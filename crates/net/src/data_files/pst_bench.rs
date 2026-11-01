//! Big data files, on demand: making one, and importing it the way the app does, timed. Both are
//! ignored tests (they take minutes and gigabytes), run by hand:
//!
//! ```text
//! MC_PST=D:\big.pst MC_MESSAGES=200000 MC_ATTACH_KB=300 cargo test -p mailcraft-net --release --lib pst_bench::make -- --ignored --nocapture
//! MC_PST=D:\big.pst MC_CACHE=D:\cache cargo test -p mailcraft-net --release --lib pst_bench::import -- --ignored --nocapture
//! ```
//!
//! The import's own knobs come from `MC_LANES`, `MC_BUDGET_MB`, `MC_CHUNK_KB`, `MC_RECYCLE`, `MC_HANDLE_CACHE_KB`,
//! `MC_BULK_CACHE_MB` (the data file's cache meanwhile) and `MC_BATCH_KB` (unset: the app's), and
//! it ends with one `stats:` line of numbers (`pst::ImportStats`), where its time went, a `cache:`
//! line (how the data file's cache was used, `mailcraft_store::CacheUse`) and on Windows an `io:`
//! line (the process's bytes read and written, and its peak working set). With `MC_VERIFY=1` it then reads every listed message back from the cache and
//! counts those present, those that parse and their attachments (counts only), with a digest of
//! the folders, their lists and every source: two imports of the same file (one lane and many,
//! say) came out the same when their digests are; and what a search index pass would still have
//! to do (none to add, no folder read, when the import indexed as it went; `MC_NO_INDEX=1` imports
//! without indexing, to compare).
//!
//! A run read from start to end also prints a `fit:` line, what an item and a byte took, fitted
//! over the run's progress (the item weight the bar should go by: `ITEM_WEIGHT`), and `eta:`
//! lines, how far off the time left the window says would have been, with the app's weight, the
//! fitted one and bytes alone. `MC_RESUME=1` (with `MC_KEEP`) goes on from where an import into
//! the cache stopped, after an `open:` line timing the data file's opening (one cut off by a kill
//! is checked then); `MC_PAUSES=n` pauses it n times at random, going on each time, and prints how
//! long each pause took to stop it (`MC_PAUSE_S`, `MC_SEED`). `MC_CHECKPOINT_S` writes a
//! checkpoint that often instead of the app's every 10 s (a big number: almost none), to time
//! what they cost.
//!
//! Memory is watched from outside (the test process's working set), so the numbers are the
//! process's, allocator and all.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mailcraft_model::job::{Eta, ITEM_WEIGHT, StopHow, Work};

use pst_rw::{UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// A cheap repeatable stream of bytes (xorshift), so attachments aren't all zeros.
struct Noise(u64);

impl Noise {
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            out.extend_from_slice(&self.0.to_le_bytes());
        }
        out.truncate(n);
        out
    }

    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

const WORDS: [&str; 24] = [
    "storyboard",
    "budget",
    "review",
    "meeting",
    "poster",
    "invoice",
    "schedule",
    "draft",
    "mix",
    "camera",
    "festival",
    "proof",
    "deadline",
    "client",
    "studio",
    "render",
    "colour",
    "sound",
    "edit",
    "shoot",
    "contract",
    "travel",
    "lunch",
    "plan",
];

fn sentence(n: &mut Noise, words: usize) -> String {
    (0..words).map(|_| WORDS[n.below(WORDS.len() as u64) as usize]).collect::<Vec<_>>().join(" ")
}

/// Make a data file of `MC_MESSAGES` messages (default 20 000) across a few folders, one in three
/// with an attachment of about `MC_ATTACH_KB` (default 200 KB), at `MC_PST`.
#[test]
#[ignore]
fn make() {
    let path = PathBuf::from(env("MC_PST").expect("MC_PST: where to write the data file"));
    let total: usize = env("MC_MESSAGES").and_then(|v| v.parse().ok()).unwrap_or(20_000);
    let attach_kb: usize = env("MC_ATTACH_KB").and_then(|v| v.parse().ok()).unwrap_or(200);
    // Messages per write: the writer rewrites a folder's table each time, so few and big.
    let batch: usize = env("MC_BATCH").and_then(|v| v.parse().ok()).unwrap_or(5000);
    let _ = std::fs::remove_file(&path);
    let folders: [(&[&str], usize); 5] =
        [(&["Inbox"], 55), (&["Sent Items"], 15), (&["Inbox", "Projects"], 15), (&["Archive", "2024"], 10), (&["Archive", "2023"], 5)];
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    let began = Instant::now();
    let mut written = 0usize;
    let mut created = false;
    for (folder, share) in folders {
        let count = total * share / 100;
        let mut done = 0;
        while done < count {
            let n = (count - done).min(batch);
            // Owned first, borrowed by the writer's batch.
            let mut owned = Vec::with_capacity(n);
            for i in 0..n {
                let k = written + i;
                let subject = format!("{} #{k}", sentence(&mut noise, 4));
                let body = format!("Hello,\r\n\r\n{}\r\n\r\nThanks,\r\nRiley\r\n", sentence(&mut noise, 120));
                let html = format!("<html><body><p>Hello,</p><p>{}</p><p>Thanks,<br>Riley</p></body></html>", sentence(&mut noise, 120));
                let file = k.is_multiple_of(3).then(|| {
                    let size = attach_kb * 1024 / 2 + noise.below((attach_kb * 1024) as u64) as usize;
                    (format!("file-{k}.bin"), noise.bytes(size))
                });
                // Delivery times are Windows file times (100 ns since 1601).
                let unix = 1_700_000_000 + k as i64 * 60;
                owned.push((subject, body, html, format!("<m{k}@bench.example>"), file, (unix + 11_644_473_600) * 10_000_000));
            }
            let recipients = [UnicodePstRecipient { name: "Zoe Park", email: "zoe@studio.example", recipient_type: UnicodePstRecipientType::To }];
            let files: Vec<Vec<UnicodePstAttachment<'_>>> = owned
                .iter()
                .map(|o| {
                    o.4.iter()
                        .map(|(name, data)| UnicodePstAttachment { filename: name, mime_type: "application/octet-stream", content_id: None, data })
                        .collect()
                })
                .collect();
            let batch: Vec<UnicodePstBatchMessage<'_>> = owned
                .iter()
                .zip(&files)
                .map(|(o, f)| UnicodePstBatchMessage {
                    message: UnicodePstMessage {
                        subject: &o.0,
                        sender_name: "Ana Lima",
                        sender_email: "ana@soundlab.example",
                        recipients: &recipients,
                        body: &o.1,
                        html_body: Some(&o.2),
                        message_id: &o.3,
                        delivery_time: o.5,
                    },
                    attachments: f,
                })
                .collect();
            let (first, rest) = if created { (None, batch.as_slice()) } else { (batch.first(), batch.get(1..).unwrap_or(&[])) };
            if let Some(first) = first {
                UnicodePstFile::create_in_folder_with_attachments(&path, folder, &first.message, first.attachments).unwrap();
                created = true;
            }
            if !rest.is_empty() {
                UnicodePstFile::append_many_in_folder_with_attachments(&path, folder, rest).unwrap();
            }
            done += n;
            written += n;
            if written.is_multiple_of(10_000) || done == count {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                println!("{written} messages, {:.2} GB, {:.0} s", size as f64 / 1e9, began.elapsed().as_secs_f64());
            }
        }
    }
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("done: {written} messages, {:.2} GB in {:.0} s", size as f64 / 1e9, began.elapsed().as_secs_f64());
}

/// Import the data file at `MC_PST` into a fresh packed cache at `MC_CACHE` (as the app keeps
/// accounts), printing progress with the time, then where it went.
#[test]
#[ignore]
fn import() {
    let path = PathBuf::from(env("MC_PST").expect("MC_PST: the data file to import"));
    let cache = PathBuf::from(env("MC_CACHE").expect("MC_CACHE: an empty folder for the cache"));
    // `MC_KEEP`: over what's there (an import stopped part-way, say).
    if env("MC_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&cache);
    }
    // Opening a data file an import was cut off in (MC_KEEP) checks it first: timed, to its
    // folder list read.
    let opened = Instant::now();
    let store = mailcraft_store::Store::open_packed(cache.join("root"), cache.join("files")).unwrap();
    let known = store.load_folders("d1").map(|f| f.len()).unwrap_or(0);
    println!("open: {} ms, {known} folders", opened.elapsed().as_millis());
    let began = Instant::now();
    let mut last = Instant::now();
    let mut samples: Vec<Sample> = Vec::new();
    let mut totals = (0u64, 0u64);
    let mut progress = |t: crate::pst::Progress| {
        let n = t.imported;
        let ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
        samples.push(Sample { ms, items: t.read as u64, bytes: t.bytes_done });
        totals = (t.total as u64, t.bytes_total);
        if last.elapsed().as_secs_f64() >= 5.0 {
            last = Instant::now();
            println!("{n} messages, {:.0} s, {:.0}/s", began.elapsed().as_secs_f64(), n as f64 / began.elapsed().as_secs_f64());
        }
    };
    let knob = |name: &str| env(name).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let mut opts = crate::pst::ImportOptions {
        lanes: knob("MC_LANES"),
        budget_bytes: knob("MC_BUDGET_MB").saturating_mul(1024 * 1024),
        chunk_kb: knob("MC_CHUNK_KB"),
        recycle: knob("MC_RECYCLE"),
        handle_cache_bytes: u64::try_from(knob("MC_HANDLE_CACHE_KB")).unwrap_or(0).saturating_mul(1024),
        cache_bytes: knob("MC_BULK_CACHE_MB").saturating_mul(1024 * 1024),
        batch_bytes: knob("MC_BATCH_KB").saturating_mul(1024),
        // `MC_NO_INDEX`: without the search index, to see what indexing as it goes costs.
        no_index: env("MC_NO_INDEX").is_some(),
        // `MC_RESUME`: go on from where the import into this cache stopped (one cut off, say).
        resume: env("MC_RESUME").is_some(),
        // `MC_CHECKPOINT_S`: a checkpoint this often instead of the app's, to see what they cost.
        checkpoint_every: env("MC_CHECKPOINT_S").and_then(|v| v.parse().ok()).map(Duration::from_secs),
        ..Default::default()
    };
    // `MC_PAUSES=n`: paused n times, each between 1 s and `MC_PAUSE_S` (default 8) into its run,
    // and gone on with each time; how long each took to stop is timed.
    let pauses = knob("MC_PAUSES");
    let gap_ms = u64::try_from(if env("MC_PAUSE_S").is_some() { knob("MC_PAUSE_S").max(2) } else { 8 }).unwrap_or(8) * 1000;
    let mut noise = Noise(env("MC_SEED").and_then(|v| v.parse().ok()).unwrap_or(0x2545_F491_4F6C_DD1D));
    let mut waits: Vec<u64> = Vec::new();
    let done = loop {
        if waits.len() >= pauses {
            break crate::pst::import_with(&path, &store, "d1", &opts, &mut progress).unwrap();
        }
        let stop = crate::pst::Stop::default();
        opts.stop = stop.clone();
        let after = Duration::from_millis(1000 + noise.below(gap_ms - 1000));
        // Told when the import ended by itself first, so it isn't paused after.
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let (done, took) = std::thread::scope(|s| {
            let pauser = s.spawn(move || match rx.recv_timeout(after) {
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    stop.raise(StopHow::Pause);
                    Some(Instant::now())
                }
                _ => None,
            });
            let done = crate::pst::import_with(&path, &store, "d1", &opts, &mut progress).unwrap();
            let returned = Instant::now();
            let _ = tx.send(());
            let raised = pauser.join().ok().flatten();
            (done, raised.map(|r| u64::try_from(returned.saturating_duration_since(r).as_millis()).unwrap_or(u64::MAX)))
        });
        opts.resume = true;
        match (took, done.stopped) {
            (Some(ms), Some(StopHow::Pause)) => {
                println!("paused: {} messages in, {ms} ms to stop", done.messages);
                waits.push(ms);
            }
            // Done before it was asked to stop.
            _ => break done,
        }
    };
    let secs = began.elapsed().as_secs_f64();
    if !waits.is_empty() {
        waits.sort_unstable();
        let at = |q: usize| waits.get((waits.len() * q / 100).min(waits.len() - 1)).copied().unwrap_or(0);
        println!("pauses: {}, to stop p50 {} ms, p99 {} ms, max {} ms", waits.len(), at(50), at(99), waits.last().copied().unwrap_or(0));
    }
    let listed: usize = done.folders.iter().map(|f| store.load_folder("d1", &f.path).map(|st| st.messages.len()).unwrap_or(0)).sum();
    assert_eq!(listed, done.messages, "every imported message listed once");
    println!(
        "done: {} messages ({} skipped) in {:.0} s, {:.0}/s; cache {:.2} GB",
        done.messages,
        done.skipped,
        secs,
        done.messages as f64 / secs,
        dir_size(&cache) as f64 / 1e9
    );
    println!("stats: {}", serde_json::to_string(&done.stats).unwrap());
    println!("cache: {}", serde_json::to_string(&store.cache_use("d1")).unwrap());
    if let Some(io) = io_counts() {
        println!("io: {io}");
    }
    // What an item and a byte cost, and how far off the time left the window says would have
    // been: for a run read from start to end only.
    if pauses == 0 && !opts.resume {
        let end_ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
        let fitted = fit(&samples, FIT_SPAN_MS);
        if let Some(f) = fitted {
            println!(
                "fit: {:.3} ms an item, {:.2} ns a byte, item weight {} bytes, r2 {:.3}, {} spans",
                f.item_ms,
                f.byte_ns,
                f.weight(),
                f.r2,
                f.spans
            );
        }
        let mut weights = vec![("app", ITEM_WEIGHT), ("bytes only", 0)];
        weights.extend(fitted.map(|f| ("fitted", f.weight())));
        for (name, weight) in weights {
            let e = eta_error(&samples, totals, end_ms, weight);
            println!(
                "eta ({name}, item weight {weight}): worst {:.1}% off from 10% done, {:.1}% from 50%; first shown at {:.1}% done, {} s in",
                e.from_10 * 100.0,
                e.from_50 * 100.0,
                e.first_permille as f64 / 10.0,
                e.first_ms / 1000
            );
        }
    }
    if env("MC_VERIFY").is_some() {
        let (mut present, mut parsed, mut attachments) = (0usize, 0usize, 0usize);
        // FNV-1a over what the import left: a number, nothing of the mail itself.
        let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |bytes: &[u8]| {
            for b in bytes {
                digest = (digest ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
            }
        };
        for f in &done.folders {
            mix(&serde_json::to_vec(f).unwrap());
            let Ok(st) = store.load_folder("d1", &f.path) else { continue };
            for m in &st.messages {
                mix(&serde_json::to_vec(m).unwrap());
                let Some(raw) = store.read_body(&m.key) else { continue };
                mix(&raw);
                present += 1;
                if let Some(p) = mailcraft_mime::parse(&raw) {
                    parsed += 1;
                    attachments += p.attachments.len();
                }
            }
        }
        println!("verify: {present} present, {parsed} parsed, {} unparsed, {attachments} attachments, digest {digest:016x}", present - parsed);
        // What a pass would still have to do for search, now the import is in.
        let reads = store.reads();
        let w = mailcraft_store::search::Writer::open(&store, "d1").unwrap();
        let plan = mailcraft_store::search::plan(&store, &w, "d1", &[]).unwrap();
        println!(
            "index: {} live, then to add {}, no source {}, to remove {}, folders read {}, messages read {}",
            w.live(),
            plan.add.len(),
            plan.no_source.len(),
            plan.remove.len(),
            plan.total,
            store.reads() - reads
        );
    }
}

/// This process's bytes read and written, and its peak working set, as Windows counts them
/// (asked of PowerShell: the bench needs no system calls of its own). None elsewhere.
fn io_counts() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let pid = std::process::id();
    let script = format!(
        "$p = Get-CimInstance Win32_Process -Filter 'ProcessId={pid}'; $w = Get-Process -Id {pid}; \
         '{{\"read_bytes\":' + $p.ReadTransferCount + ',\"write_bytes\":' + $p.WriteTransferCount + ',\"peak_ws_bytes\":' + $w.PeakWorkingSet64 + '}}'"
    );
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return std::fs::metadata(p).map(|m| m.len()).unwrap_or(0) };
    rd.flatten().map(|e| if e.path().is_dir() { dir_size(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) }).sum()
}

/// Progress as an import told it: the time into it, and the items read and their bytes (as the
/// file gives them).
#[derive(Clone, Copy, Debug)]
struct Sample {
    ms: u64,
    items: u64,
    bytes: u64,
}

/// Spans shorter than this are left out of the fit: one big item can take most of a second.
const FIT_SPAN_MS: u64 = 2000;

/// What an item and a byte took over an import, fitted: a span of it took `item_ms` for each
/// item read in it and `byte_ns` for each byte; `r2` is the share of the spans' times it
/// explains (through the origin), over `spans` spans.
#[derive(Clone, Copy, Debug)]
struct Fit {
    item_ms: f64,
    byte_ns: f64,
    r2: f64,
    spans: usize,
}

impl Fit {
    /// The bytes an item weighs: those that take as long to import as an item does besides them
    /// (`ITEM_WEIGHT`, fitted). 0 when the fit says nothing sensible.
    fn weight(&self) -> u64 {
        let w = self.item_ms * 1e6 / self.byte_ns;
        if w.is_finite() && w > 0.0 { w as u64 } else { 0 }
    }
}

/// Least squares, through the origin, over the import cut into spans of at least `span_ms` from
/// one sample to a later one: the time each took against the items and bytes read in it. None
/// with fewer than two spans, or when items and bytes went up together throughout.
fn fit(samples: &[Sample], span_ms: u64) -> Option<Fit> {
    let mut spans = Vec::new();
    let mut from = *samples.first()?;
    for s in samples.iter().skip(1) {
        if s.ms.saturating_sub(from.ms) >= span_ms {
            let d = |a: u64, b: u64| a.saturating_sub(b) as f64;
            spans.push((d(s.items, from.items), d(s.bytes, from.bytes), d(s.ms, from.ms)));
            from = *s;
        }
    }
    let (mut xx, mut xy, mut yy, mut xt, mut yt, mut tt) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for &(x, y, t) in &spans {
        (xx, xy, yy) = (xx + x * x, xy + x * y, yy + y * y);
        (xt, yt, tt) = (xt + x * t, yt + y * t, tt + t * t);
    }
    let det = xx * yy - xy * xy;
    if spans.len() < 2 || det <= 1e-9 * xx * yy {
        return None;
    }
    let (a, b) = ((xt * yy - yt * xy) / det, (yt * xx - xt * xy) / det);
    let sse: f64 = spans.iter().map(|&(x, y, t)| (t - a * x - b * y).powi(2)).sum();
    Some(Fit { item_ms: a, byte_ns: b * 1e6, r2: if tt > 0.0 { 1.0 - sse / tt } else { 0.0 }, spans: spans.len() })
}

/// How far off the time left would have been: the worst share it was off by from 10% done and
/// from 50% (by the bar), and the bar and the time when it was first shown.
#[derive(Clone, Copy, Debug, Default)]
struct EtaError {
    from_10: f64,
    from_50: f64,
    first_permille: u16,
    first_ms: u64,
}

/// Below a minute left the window says "less than a minute" whatever the estimate, so only
/// times of a minute or more, said or true, are judged.
const JUDGED_FROM_S: f64 = 60.0;

/// The time left as the window works it out (`Eta`, items weighing `weight` bytes besides their
/// own) from an import's samples, with `totals` (items, bytes), against the time it really took
/// to end at `end_ms`. The window's clock is taken to be the worker's.
fn eta_error(samples: &[Sample], totals: (u64, u64), end_ms: u64, weight: u64) -> EtaError {
    let mut eta = Eta::default();
    let mut out = EtaError::default();
    let mut shown = false;
    for s in samples {
        let work = Work { done: s.items, total: totals.0, bytes_done: s.bytes, bytes_total: totals.1 };
        eta.sample(s.ms, work.weighted_done(weight), work.weighted_total(weight), 0, s.ms);
        let Some(left) = eta.remaining_s(s.ms) else { continue };
        let permille = work.permille(weight).unwrap_or(0);
        if !shown {
            (shown, out.first_permille, out.first_ms) = (true, permille, s.ms);
        }
        let truth = end_ms.saturating_sub(s.ms) as f64 / 1000.0;
        let left = left as f64;
        if truth < JUDGED_FROM_S && left < JUDGED_FROM_S {
            continue;
        }
        let off = (left - truth).abs() / truth.max(1.0);
        if permille >= 100 {
            out.from_10 = out.from_10.max(off);
        }
        if permille >= 500 {
            out.from_50 = out.from_50.max(off);
        }
    }
    out
}

/// Samples of an import every 200 items of `items` (sizes in bytes), where an item takes
/// `item_ms` and a byte `byte_ns`.
fn simulated(items: &[u64], item_ms: f64, byte_ns: f64) -> Vec<Sample> {
    let (mut t, mut bytes, mut out) = (0.0f64, 0u64, Vec::new());
    for (k, size) in items.iter().enumerate() {
        t += item_ms + *size as f64 * byte_ns / 1e6;
        bytes += size;
        if (k + 1) % 200 == 0 {
            out.push(Sample { ms: t as u64, items: k as u64 + 1, bytes });
        }
    }
    out
}

/// Folders of many small messages, then of few big ones, then small again.
fn mixed_items() -> Vec<u64> {
    let mut items = vec![4_000u64; 20_000];
    items.extend(std::iter::repeat_n(3_000_000u64, 2_000));
    items.extend(std::iter::repeat_n(6_000u64, 20_000));
    items
}

/// The fit finds what an item and a byte cost from the progress alone, however the folders mix
/// small and big items.
#[test]
fn the_fit_finds_what_an_item_and_a_byte_cost() {
    let f = fit(&simulated(&mixed_items(), 2.0, 18.0), FIT_SPAN_MS).unwrap();
    assert!((f.item_ms - 2.0).abs() < 0.05, "{f:?}");
    assert!((f.byte_ns - 18.0).abs() < 0.5, "{f:?}");
    assert!((105_000..118_000).contains(&f.weight()) && f.r2 > 0.99, "{f:?}");
    // Every item alike: nothing tells an item's cost from its bytes'.
    assert!(fit(&simulated(&[5_000; 10_000], 2.0, 18.0), FIT_SPAN_MS).is_none());
    assert!(fit(&[], FIT_SPAN_MS).is_none());
}

/// The time left is judged against when the import really ended: right with items weighed as
/// they cost, and far off when they weigh nothing and small items come first.
#[test]
fn the_time_left_is_judged_against_the_real_end() {
    let items = mixed_items();
    let samples = simulated(&items, 2.0, 18.0);
    let totals = (items.len() as u64, items.iter().sum());
    let end = samples.last().unwrap().ms;
    let right = eta_error(&samples, totals, end, 111_111);
    assert!(right.from_10 < 0.05 && right.from_50 < 0.05, "{right:?}");
    assert!(right.first_ms >= 20_000 && right.first_permille >= 20, "nothing said early: {right:?}");
    let bytes_only = eta_error(&samples, totals, end, 0);
    assert!(bytes_only.from_10 > 0.2, "{bytes_only:?}");
    assert_eq!(eta_error(&[], totals, end, 0).first_ms, 0);
}

/// A data file with a folder bigger than a page of the list: imported, the folder lists every
/// message once, newest first, with the right counts; its list is kept in pages (the import never
/// held it whole), and changing it afterwards writes as any folder does.
#[test]
fn a_big_folder_comes_in_a_page_at_a_time() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-pages-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pst = dir.join("big.pst");
    let n = mailcraft_store::PAGE_MESSAGES * 2 + 808;
    let owned: Vec<(String, String, i64)> =
        (0..n).map(|k| (format!("Report #{k}"), format!("<p{k}@bench.example>"), 1_700_000_000 + k as i64 * 60)).collect();
    let recipients = [UnicodePstRecipient { name: "Zoe", email: "zoe@studio.example", recipient_type: UnicodePstRecipientType::To }];
    let batch: Vec<UnicodePstBatchMessage<'_>> = owned
        .iter()
        .map(|o| UnicodePstBatchMessage {
            message: UnicodePstMessage {
                subject: &o.0,
                sender_name: "Ana",
                sender_email: "ana@soundlab.example",
                recipients: &recipients,
                body: "Hello.",
                html_body: None,
                message_id: &o.1,
                delivery_time: o.2,
            },
            attachments: &[],
        })
        .collect();
    UnicodePstFile::create_in_folder_with_attachments(&pst, &["Inbox"], &batch[0].message, batch[0].attachments).unwrap();
    for part in batch[1..].chunks(4000) {
        UnicodePstFile::append_many_in_folder_with_attachments(&pst, &["Inbox"], part).unwrap();
    }
    let store = mailcraft_store::Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let done = crate::pst::import(&pst, &store, "d1", &mut |_| {}).unwrap();
    assert_eq!(done.messages, n);
    // Its writes counted: the batches of sources, and the checkpoints (the last as it ended).
    let s = done.stats;
    assert!(s.checkpoints >= 1 && s.batches + s.checkpoints >= 2, "{s:?}");
    assert!(s.checkpoint_ms + s.batch_ms <= s.total_ms, "{s:?}");
    let inbox = done.folders.iter().find(|f| f.path == "INBOX").unwrap();
    let st = store.load_folder("d1", "INBOX").unwrap();
    assert_eq!(st.messages.len(), n);
    assert_eq!((inbox.total as usize, inbox.unread), (n, st.unread()), "counted as they went out");
    assert_eq!(st.messages[0].subject, format!("Report #{}", n - 1), "newest first");
    let mut uids: Vec<u32> = st.messages.iter().map(|m| m.key.uid).collect();
    uids.sort_unstable();
    uids.dedup();
    assert_eq!(uids.len(), n, "every message once");
    assert!(st.uid_next as usize > n);
    assert!(store.read_body(&st.messages[n / 2].key).is_some());
    // Changed afterwards: a flag, saved and read back.
    let mut st = st;
    st.messages[5].flags.seen = true;
    let key = st.messages[5].key.clone();
    store.save_folder("d1", "INBOX", &st).unwrap();
    assert!(store.load_folder("d1", "INBOX").unwrap().find(key.uid).unwrap().flags.seen);
    // Its items read through a handle opened again every 3: the same list, every message once.
    let opts = crate::pst::ImportOptions { recycle: 3, lanes: 1, ..Default::default() };
    let again = crate::pst::import_with(&pst, &store, "d2", &opts, &mut |_| {}).unwrap();
    assert_eq!((again.messages, again.stats.reopens as usize), (n, n.div_ceil(3) - 1));
    let listed = |account: &str| -> Vec<(String, u32)> {
        store.load_folder(account, "INBOX").unwrap().messages.iter().map(|m| (m.subject.clone(), m.key.uid)).collect()
    };
    assert_eq!(listed("d2"), listed("d1"));
    let _ = std::fs::remove_dir_all(&dir);
}
