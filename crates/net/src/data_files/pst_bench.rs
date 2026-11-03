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
//! line (the process's bytes read and written, and its peak working set), after a `report:` line
//! (what the import's report says was left out, by reason). With `MC_VERIFY=1` it then reads every listed message back from the cache and
//! counts those present, those that parse and their attachments (counts only), and on a `kept:`
//! line what came in of what the file says of its mail (`kept.rs`), with a digest of
//! the folders, their lists and every source: two imports of the same file (one lane and many,
//! say) came out the same when their digests are (one each of the folders, the lists and the
//! sources besides, and with `MC_VERIFY=folders` a line of them for each folder, to see where
//! two differ; the folders go in the order the import gives them, and one done already gives
//! them as the account keeps them, the Sent Items folder last); and what a search index pass
//! would still have to do (none to add, no folder read, when the import indexed as it went;
//! `MC_NO_INDEX=1` imports without indexing, to compare).
//!
//! A run read from start to end also prints a `fit:` line, what an item and a byte took, fitted
//! over the run's progress (the item weight the bar should go by: `ITEM_WEIGHT`), and a `fit by
//! folder:` line, the same fitted over each folder's items, bytes and time; and `eta:` lines, how
//! far off the time left the window says would have been, with the app's weight, the fitted
//! one, bytes alone and those in `MC_WEIGHTS` (`MC_TRACE`: what it said and what was true at
//! every 5%), and how far off even the run's own average rate, known ahead, would have been.
//! `MC_SAMPLES=<file>` keeps that progress, for `eta::replay` to judge again with other weights
//! without importing. `MC_RESUME=1` (with `MC_KEEP`) goes on from where an import into
//! the cache stopped, after an `open:` line timing the data file's opening (one cut off by a kill
//! is checked then); `MC_PAUSES=n` pauses it n times at random, going on each time, and prints how
//! long each pause took to stop it, with that run's numbers (`MC_PAUSE_S`, `MC_SEED`; `MC_PAUSE_AT`
//! pauses first as so many messages are in, to aim them at one place).
//! `MC_CHECKPOINT_S` writes a checkpoint that often instead of the app's every 10 s (a big
//! number: only those at each folder's end), to time what they cost.
//!
//! Memory is watched from outside (the test process's working set), so the numbers are the
//! process's, allocator and all; `MC_PEAK_MB` fails the run when its peak is over that many MB
//! (the `MC_NO_INDEX` run's peak and 150 MB more, for an import indexing as it goes).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mailcraft_model::job::StopHow;

use eta::{judge, judge_folders};
use outlook_pst::{UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};

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
                        .map(|(name, data)| UnicodePstAttachment {
                            filename: name,
                            mime_type: "application/octet-stream",
                            content_id: None,
                            data,
                            ..Default::default()
                        })
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
                        message_class: "IPM.Note",
                        properties: &[],
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
    // The messages in so far, for the pauses that wait for them (`MC_PAUSE_AT`).
    let seen = std::sync::atomic::AtomicUsize::new(0);
    let mut progress = Watch { began, last: began, samples: Vec::new(), ends: Vec::new(), totals: (0, 0), seen: &seen };
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
    // and gone on with each time; how long each took to stop is timed. `MC_PAUSE_AT=m`: none
    // before m messages are in, the first as they are and the rest that long after.
    let pauses = knob("MC_PAUSES");
    let pause_at = knob("MC_PAUSE_AT");
    let gap_ms = u64::try_from(if env("MC_PAUSE_S").is_some() { knob("MC_PAUSE_S").max(2) } else { 8 }).unwrap_or(8) * 1000;
    let mut noise = Noise(env("MC_SEED").and_then(|v| v.parse().ok()).unwrap_or(0x2545_F491_4F6C_DD1D));
    let mut waits: Vec<u64> = Vec::new();
    let done = loop {
        if waits.len() >= pauses {
            // Gone on with to the end: the last pause's stop isn't this run's.
            opts.stop = crate::pst::Stop::default();
            break crate::pst::import_into(&path, &store, "d1", &opts, &mut progress).unwrap();
        }
        let stop = crate::pst::Stop::default();
        opts.stop = stop.clone();
        // Aimed, the first pause comes as soon as the messages are in.
        let after = if pause_at > 0 && waits.is_empty() { Duration::ZERO } else { Duration::from_millis(1000 + noise.below(gap_ms - 1000)) };
        // Told when the import ended by itself first, so it isn't paused after.
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let seen = &seen;
        let (done, took) = std::thread::scope(|s| {
            let pauser = s.spawn(move || {
                let mut clock: Option<Instant> = None;
                loop {
                    if clock.is_none() && seen.load(std::sync::atomic::Ordering::Relaxed) >= pause_at {
                        clock = Some(Instant::now());
                    }
                    if clock.is_some_and(|c| c.elapsed() >= after) {
                        stop.raise(StopHow::Pause);
                        return Some(Instant::now());
                    }
                    match rx.recv_timeout(Duration::from_millis(10)) {
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        _ => return None,
                    }
                }
            });
            let done = crate::pst::import_into(&path, &store, "d1", &opts, &mut progress).unwrap();
            let returned = Instant::now();
            let _ = tx.send(());
            let raised = pauser.join().ok().flatten();
            (done, raised.map(|r| u64::try_from(returned.saturating_duration_since(r).as_millis()).unwrap_or(u64::MAX)))
        });
        opts.resume = true;
        match (took, done.stopped) {
            (Some(ms), Some(StopHow::Pause)) => {
                // With the run's own numbers, to see where a slow stop went.
                println!("paused: {} messages in, {ms} ms to stop; {}", done.messages, serde_json::to_string(&done.stats).unwrap());
                waits.push(ms);
            }
            // Done before it was asked to stop.
            _ => break done,
        }
    };
    let secs = began.elapsed().as_secs_f64();
    let end_ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
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
    // What the import's report says was left out, and what else came in.
    println!(
        "report: contacts {}, events {}, notes {}, body not converted {}, skips {}",
        done.contacts,
        done.events,
        done.notes,
        done.body_not_converted,
        serde_json::to_string(&done.skips).unwrap()
    );
    println!("cache: {}", serde_json::to_string(&store.cache_use("d1")).unwrap());
    if let Some(io) = io_counts() {
        println!("io: {io}");
        // `MC_PEAK_MB`: the most the peak working set may be (an import indexing as it goes is
        // held to 150 MB over the same import's with `MC_NO_INDEX`).
        if let Some(limit) = env("MC_PEAK_MB").and_then(|v| v.parse::<u64>().ok()) {
            let peak = serde_json::from_str::<serde_json::Value>(&io).ok().and_then(|v| v["peak_ws_bytes"].as_u64()).unwrap_or(0);
            assert!(peak > 0 && peak <= limit << 20, "peak working set {} MB, over {limit} MB", peak >> 20);
        }
    }
    // What an item and a byte cost, and how far off the time left the window says would have
    // been: for a run read from start to end only.
    if pauses == 0 && !opts.resume {
        // `MC_SAMPLES`: the progress kept in a file too (times and counts), for `eta::replay`.
        let Watch { samples, ends, totals, .. } = progress;
        if let Some(to) = env("MC_SAMPLES") {
            let mut text = format!("{},{},{end_ms}\n", totals.0, totals.1);
            for s in &samples {
                text.push_str(&format!("{},{},{}\n", s.ms, s.items, s.bytes));
            }
            std::fs::write(to, text).unwrap();
        }
        judge_folders(&samples, ends);
        judge(&samples, totals, end_ms);
    }
    if env("MC_VERIFY").is_some() {
        let (mut present, mut parsed, mut attachments) = (0usize, 0usize, 0usize);
        let mut kept = kept::Kept::default();
        // Digests over what the import left: the whole, and the folders, their lists and the
        // sources apart, to see where two imports differ; numbers, nothing of the mail itself.
        let (mut digest, mut parts) = (Fnv::default(), [Fnv::default(), Fnv::default(), Fnv::default()]);
        // `MC_VERIFY=folders`: a line for each folder too, by its place in the list.
        let each = env("MC_VERIFY").as_deref() == Some("folders");
        for (n, f) in done.folders.iter().enumerate() {
            let json = serde_json::to_vec(f).unwrap();
            digest.mix(&json);
            parts[0].mix(&json);
            let Ok(st) = store.load_folder("d1", &f.path) else { continue };
            let (mut list, mut sources) = (Fnv::default(), Fnv::default());
            for m in &st.messages {
                let json = serde_json::to_vec(m).unwrap();
                for d in [&mut digest, &mut parts[1], &mut list] {
                    d.mix(&json);
                }
                let Some(raw) = store.read_body(&m.key) else { continue };
                for d in [&mut digest, &mut parts[2], &mut sources] {
                    d.mix(&raw);
                }
                present += 1;
                kept.add(m, &raw);
                if let Some(p) = mailcraft_mime::parse(&raw) {
                    parsed += 1;
                    attachments += p.attachments.len();
                }
            }
            if each {
                println!(
                    "  folder {n}: {} messages, folder {:016x}, list {:016x}, sources {:016x}",
                    st.messages.len(),
                    Fnv::of(&json),
                    list.0,
                    sources.0
                );
            }
        }
        println!(
            "verify: {present} present, {parsed} parsed, {} unparsed, {attachments} attachments, digest {:016x} (folders {:016x}, lists {:016x}, sources {:016x})",
            present - parsed,
            digest.0,
            parts[0].0,
            parts[1].0,
            parts[2].0
        );
        println!("kept: {}", serde_json::to_string(&kept).unwrap());
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

/// FNV-1a, 64 bits.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn mix(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn of(bytes: &[u8]) -> u64 {
        let mut f = Fnv::default();
        f.mix(bytes);
        f.0
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

mod eta;
mod kept;

/// Progress as an import told it: the time into it, and the items read and their bytes (as the
/// file gives them).
#[derive(Clone, Copy, Debug)]
struct Sample {
    ms: u64,
    items: u64,
    bytes: u64,
}

/// What the benchmark sees of an import: its progress with the time (printed every 5 s), and
/// where it was as each folder ended.
struct Watch<'a> {
    began: Instant,
    last: Instant,
    samples: Vec<Sample>,
    ends: Vec<Sample>,
    /// Items in the file and their bytes, once known.
    totals: (u64, u64),
    seen: &'a std::sync::atomic::AtomicUsize,
}

impl Watch<'_> {
    fn now(&self) -> u64 {
        u64::try_from(self.began.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

impl crate::pst::ImportSink for Watch<'_> {
    fn progress(&mut self, t: crate::pst::Progress) {
        let n = t.imported;
        self.seen.store(n, std::sync::atomic::Ordering::Relaxed);
        let ms = self.now();
        self.samples.push(Sample { ms, items: t.read as u64, bytes: t.bytes_done });
        self.totals = (t.total as u64, t.bytes_total);
        if self.last.elapsed().as_secs_f64() >= 5.0 {
            self.last = Instant::now();
            let secs = self.began.elapsed().as_secs_f64();
            println!("{n} messages, {secs:.0} s, {:.0}/s", n as f64 / secs);
        }
    }

    /// A folder ended: what had been read by then (as last told), at this time.
    fn folder(&mut self, _done: &mailcraft_model::Folder) {
        let ms = self.now();
        let at = self.samples.last().map_or(Sample { ms, items: 0, bytes: 0 }, |s| Sample { ms, ..*s });
        self.ends.push(at);
    }
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
                message_class: "IPM.Note",
                properties: &[],
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
    std::sync::Arc::make_mut(&mut st.messages[5]).flags.seen = true;
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
