//! What compacting a big data file costs, measured on copies of a real one (counts and timings
//! only): how compacting is offered and when it runs are decided from these numbers. None of these
//! runs on its own; each is an ignored test given its files by the environment, run alone in
//! release with `--ignored --nocapture`.
//!
//! The data file a build before 0.32 made in the app kept most attachments raw (34.9 GiB of a
//! 47 GB file); compressed again, it keeps about 9 GiB less in the same file, the room scattered
//! through it. One like it is made from a copy of a data file ([`a_data_file_copied_to_a_new_one`]
//! with `MC_COPY_RAW`), compressed again as the app will ([`a_real_data_file_is_compressed_again`]),
//! then compacted cold, in place ([`a_data_file_compacts_in_place`]) or by a copy into a new file
//! ([`a_data_file_copied_to_a_new_one`]); [`a_data_files_manifest`] says every file reads back the
//! same through all of it.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use redb::{ReadableDatabase, ReadableTable};

use super::*;
use crate::packed::{DEFLATED, FILES, META, PIECES, Tables, pieced_len, unpack};

const MIB: f64 = (1u64 << 20) as f64;

fn mib(n: u64) -> f64 {
    n as f64 / MIB
}

fn var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// What the process has read and written (every read and write it asked for, the system's cache
/// included) and the most memory it has held, from the system.
#[derive(Clone, Copy, Debug, Default)]
struct Process {
    read: u64,
    written: u64,
    peak: u64,
}

impl Process {
    #[cfg(windows)]
    fn now() -> Process {
        let script = format!(
            "$p = Get-CimInstance Win32_Process -Filter 'ProcessId={}'; \"$($p.ReadTransferCount) $($p.WriteTransferCount) $($p.PeakWorkingSetSize)\"",
            std::process::id()
        );
        let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let n: Vec<u64> = text.split_whitespace().filter_map(|w| w.parse().ok()).collect();
        // The peak working set is in KiB.
        Process { read: n[0], written: n[1], peak: n[2] << 10 }
    }

    #[cfg(not(windows))]
    fn now() -> Process {
        let field = |text: &str, name: &str| {
            text.lines().find_map(|l| l.strip_prefix(name)).and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok()).unwrap_or(0)
        };
        let io = std::fs::read_to_string("/proc/self/io").unwrap_or_default();
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        Process { read: field(&io, "rchar:"), written: field(&io, "wchar:"), peak: field(&status, "VmHWM:") << 10 }
    }

    fn since(self, before: Process) -> Process {
        Process { read: self.read - before.read, written: self.written - before.written, peak: self.peak }
    }
}

/// `file` opened past the system's cache on Windows (for writing: made new, and written through
/// to the drive); elsewhere it goes through the cache, so purge it there before a cold run.
fn uncached(file: &Path, write: bool) -> std::fs::File {
    let mut options = std::fs::OpenOptions::new();
    if write {
        options.write(true).create(true).truncate(true);
    } else {
        options.read(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_NO_BUFFERING, and FILE_FLAG_WRITE_THROUGH for writing.
        options.custom_flags(if write { 0x2000_0000 | 0x8000_0000 } else { 0x2000_0000 });
    }
    options.open(file).unwrap()
}

/// Reads and writes past the cache need a buffer on a sector's boundary: 8 MiB of one, in `buffer`.
fn aligned(buffer: &mut Vec<u8>) -> &mut [u8] {
    const CHUNK: usize = 8 << 20;
    buffer.resize(CHUNK + 4096, 0x5a);
    let skip = buffer.as_ptr().align_offset(4096);
    &mut buffer[skip..skip + CHUNK]
}

/// The drive's own rates: the first 4 GiB of `file` read (not left in the cache, so it stays cold)
/// and 2 GiB written beside it, in bytes a second. Past the system's cache on Windows only (see
/// [`uncached`]); elsewhere these are the cache's rates.
fn drive_rates(file: &Path) -> (f64, f64) {
    let mut buffer = Vec::new();
    let chunk = aligned(&mut buffer);
    let mut f = uncached(file, false);
    let (mut read, t) = (0u64, Instant::now());
    while read < 4 << 30 {
        let n = std::io::Read::read(&mut f, chunk).unwrap();
        if n == 0 {
            break;
        }
        read += n as u64;
    }
    let read_rate = read as f64 / t.elapsed().as_secs_f64();
    let beside = file.with_extension("rate");
    let mut f = uncached(&beside, true);
    let (mut written, t) = (0u64, Instant::now());
    while written < 2 << 30 {
        std::io::Write::write_all(&mut f, chunk).unwrap();
        written += chunk.len() as u64;
    }
    f.sync_all().unwrap();
    let write_rate = written as f64 / t.elapsed().as_secs_f64();
    drop(f);
    std::fs::remove_file(&beside).unwrap();
    (read_rate, write_rate)
}

/// `file` made cold: replaced by a copy of itself read and written past the system's cache (see
/// [`uncached`]), so none of it is left there and what reads it next reads the drive, whatever
/// read it before. Nothing may have it open.
fn cold(file: &Path) -> Duration {
    let t = Instant::now();
    let len = std::fs::metadata(file).unwrap().len();
    let copy = file.with_extension("cold");
    let mut buffer = Vec::new();
    let chunk = aligned(&mut buffer);
    let (mut from, mut to) = (uncached(file, false), uncached(&copy, true));
    loop {
        let n = std::io::Read::read(&mut from, chunk).unwrap();
        if n == 0 {
            break;
        }
        // The last piece is written to a whole sector and the copy cut back to the file's length.
        std::io::Write::write_all(&mut to, &chunk[..n.next_multiple_of(4096)]).unwrap();
    }
    drop((from, to));
    let f = std::fs::OpenOptions::new().write(true).open(&copy).unwrap();
    f.set_len(len).unwrap();
    f.sync_all().unwrap();
    drop(f);
    std::fs::rename(&copy, file).unwrap();
    t.elapsed()
}

/// A file's length, looked at every 50 ms until stopped: each time it shrank, and when. redb's
/// compaction gives back the end of the file after each pass that moved pages, so this counts its
/// passes from outside (one that freed nothing at the end isn't seen).
struct Shrinks {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<Vec<(Duration, u64)>>,
}

impl Shrinks {
    fn watch(file: &Path) -> Shrinks {
        let stop = Arc::new(AtomicBool::new(false));
        let (file, stopping) = (file.to_path_buf(), stop.clone());
        let thread = std::thread::spawn(move || {
            let (t, mut seen, mut last) = (Instant::now(), Vec::new(), std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0));
            while !stopping.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
                let len = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(last);
                if len < last {
                    seen.push((t.elapsed(), len));
                }
                last = len;
            }
            seen
        });
        Shrinks { stop, thread }
    }

    fn stop(self) -> Vec<(Duration, u64)> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().unwrap()
    }
}

/// The only account of the data folder at `root` (its data files in "MailCraft Files"), and its
/// data file.
fn only_data_file(root: &Path) -> (Store, String, PathBuf) {
    let s = Store::open_packed(root, root.join("MailCraft Files")).unwrap();
    let accounts = s.load_accounts().unwrap();
    assert_eq!(accounts.len(), 1, "one account in {}", root.display());
    let id = accounts[0].id.clone();
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    let file = packs.file(&crate::safe_segment(&id)).unwrap();
    (s, id, file)
}

/// A file as kept in a data file's read transaction: whole, or its pieces put together (still
/// compressed when it was).
fn kept(pieces: &redb::ReadOnlyTable<(&'static str, u32), &'static [u8]>, key: &str, value: &[u8]) -> Vec<u8> {
    let Some(len) = pieced_len(value) else { return value.to_vec() };
    let mut out = Vec::with_capacity(len as usize);
    for item in pieces.range((key, 0)..=(key, u32::MAX)).unwrap() {
        out.extend_from_slice(item.unwrap().1.value());
    }
    assert_eq!(out.len() as u64, len, "the pieces of a file");
    out
}

/// Every file of a data file, read back: a hash of its path, its length and a hash of its bytes
/// (paths are mail's own, so only hashed), with the time the read took.
fn manifest(file: &Path) -> (BTreeMap<u64, (u64, u64)>, Duration) {
    let hash = |b: &[u8]| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        b.hash(&mut h);
        h.finish()
    };
    let t = Instant::now();
    let db = redb::ReadOnlyDatabase::open(file).unwrap();
    let tx = db.begin_read().unwrap();
    let (files, pieces) = (tx.open_table(FILES).unwrap(), tx.open_table(PIECES).unwrap());
    let mut out = BTreeMap::new();
    for item in files.iter().unwrap() {
        let (k, v) = item.unwrap();
        let bytes = unpack(kept(&pieces, k.value(), v.value())).unwrap();
        out.insert(hash(k.value().as_bytes()), (bytes.len() as u64, hash(&bytes)));
    }
    (out, t.elapsed())
}

/// Every file of a data file read back: `MC_MANIFEST_FILE` names the data file (a copy), and
/// `MC_MANIFEST` where its manifest goes. Written the first time; after that the file's is
/// compared with it, and must be the same.
#[test]
#[ignore = "needs a copy of a real data file in MC_MANIFEST_FILE"]
fn a_data_files_manifest() {
    let (Some(file), Some(at)) = (var("MC_MANIFEST_FILE"), var("MC_MANIFEST")) else { return };
    let (now, took) = manifest(&file);
    eprintln!("{} files read back in {took:?}", now.len());
    let lines: Vec<String> = now.iter().map(|(k, (len, h))| format!("{k:016x} {len} {h:016x}")).collect();
    match std::fs::read_to_string(&at) {
        Err(_) => {
            std::fs::write(&at, lines.join("\n")).unwrap();
            eprintln!("written");
        }
        Ok(before) => {
            let before: Vec<&str> = before.lines().collect();
            let differ = before.iter().zip(&lines).filter(|(a, b)| **a != b.as_str()).count();
            eprintln!("{} files before, {} now, {differ} read back otherwise", before.len(), lines.len());
            assert!(before.len() == lines.len() && differ == 0, "every file reads back as before");
        }
    }
}

/// A data file copied into a new one, file by file in key order, in batches as a background writer
/// makes them (each at most [`crate::BACKGROUND_COMMIT`] as kept, not waited for to the disk but
/// one in four, as the run a writer leaves undurable is capped; the last sealed): `MC_COPY_FROM`
/// names the data file (a copy, made [`cold`] first and then only read), `MC_COPY_TO` the new one.
/// Timed, as a compaction by copy would be.
///
/// With `MC_COPY_RAW=<r>`, a file kept compressed to `r` of its size or more is written raw, and
/// the meta table is left out: a data file as a build before 0.32 made it in the app, whose fast
/// deflate kept base64 at 0.995 of its size (miniz keeps it at about 0.76, mail's text at much
/// less). Without it, every file is copied as kept, meta and all.
#[test]
#[ignore = "needs a copy of a real data file in MC_COPY_FROM"]
fn a_data_file_copied_to_a_new_one() {
    let (Some(from), Some(to)) = (var("MC_COPY_FROM"), var("MC_COPY_TO")) else { return };
    let raw_at: Option<f64> = std::env::var("MC_COPY_RAW").ok().map(|r| r.parse().unwrap());
    assert!(!to.exists(), "{} is made new", to.display());
    eprintln!("made cold in {:.1?}", cold(&from));
    let before = Process::now();
    let t = Instant::now();
    // With the cache a data file has (redb's own is 1 GiB).
    let src = redb::Database::builder().set_cache_size(crate::CACHE).open_read_only(&from).unwrap();
    let rtx = src.begin_read().unwrap();
    let (files, pieces) = (rtx.open_table(FILES).unwrap(), rtx.open_table(PIECES).unwrap());
    let db = redb::Database::builder().set_cache_size(crate::CACHE).create(&to).unwrap();
    let (mut batch, mut held) = (Vec::new(), 0usize);
    let (mut count, mut kept_bytes, mut raw_files, mut raw_bytes, mut commits) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let write = |batch: &mut Vec<(String, Vec<u8>)>, last: bool, durable: bool| {
        let mut tx = db.begin_write().unwrap();
        if last {
            tx.set_quick_repair(true);
        } else if !durable {
            tx.set_durability(redb::Durability::None).unwrap();
        }
        {
            let mut tables = Tables { files: tx.open_table(FILES).unwrap(), pieces: tx.open_table(PIECES).unwrap(), touched: None };
            for (k, v) in batch.drain(..) {
                tables.put_kept(&k, &v).unwrap();
            }
        }
        tx.commit().unwrap();
    };
    for item in files.iter().unwrap() {
        let (k, v) = item.unwrap();
        let mut value = kept(&pieces, k.value(), v.value());
        if let Some(at) = raw_at
            && value.starts_with(DEFLATED)
        {
            let whole = unpack(value.clone()).unwrap();
            if value.len() as f64 >= at * whole.len() as f64 {
                raw_files += 1;
                raw_bytes += whole.len() as u64;
                value = whole;
            }
        }
        count += 1;
        kept_bytes += value.len() as u64;
        held += value.len();
        batch.push((k.value().to_string(), value));
        if held >= crate::BACKGROUND_COMMIT {
            commits += 1;
            write(&mut batch, false, commits % 4 == 0);
            held = 0;
        }
    }
    if raw_at.is_none() {
        let tx = db.begin_write().unwrap();
        {
            let mut meta = tx.open_table(META).unwrap();
            if let Ok(old) = rtx.open_table(META) {
                for item in old.iter().unwrap() {
                    let (k, v) = item.unwrap();
                    meta.insert(k.value(), v.value()).unwrap();
                }
            }
        }
        tx.commit().unwrap();
    }
    write(&mut batch, true, true);
    commits += 1;
    drop(db);
    let took = t.elapsed();
    let used = Process::now().since(before);
    let (from_len, to_len) = (std::fs::metadata(&from).unwrap().len(), std::fs::metadata(&to).unwrap().len());
    eprintln!(
        "copied {count} files ({:.0} MiB as kept) in {commits} commits, {took:.1?}: {:.0} MiB -> {:.0} MiB; \
         read {:.0} MiB, wrote {:.0} MiB, peak working set {:.0} MiB",
        mib(kept_bytes),
        mib(from_len),
        mib(to_len),
        mib(used.read),
        mib(used.written),
        mib(used.peak)
    );
    if raw_at.is_some() {
        eprintln!("kept raw: {raw_files} files, {:.0} MiB", mib(raw_bytes));
    }
}

/// A real data file compressed again, as the app will before compacting it: `MC_DATA` names a
/// copy of a data folder with one account (its data files in "MailCraft Files"). The file's health
/// before and after, the run's report and its time.
#[test]
#[ignore = "needs a copy of a real data folder in MC_DATA"]
fn a_real_data_file_is_compressed_again() {
    let Some(root) = var("MC_DATA") else { return };
    let (s, id, _) = only_data_file(&root);
    let before = s.data_file_health(&id).unwrap().unwrap();
    eprintln!("before: {before:?}");
    let used = Process::now();
    let (t, mut heard) = (Instant::now(), 0u64);
    let r = s.recompress_data_file(&id, &AtomicBool::new(false), &mut |_| heard += 1).unwrap().unwrap();
    let took = t.elapsed();
    let used = Process::now().since(used);
    eprintln!("compressed again in {took:.1?} ({heard} reports): {r:?}");
    eprintln!("read {:.0} MiB, wrote {:.0} MiB, peak working set {:.0} MiB", mib(used.read), mib(used.written), mib(used.peak));
    let after = s.data_file_health(&id).unwrap().unwrap();
    eprintln!("after: {after:?}");
    eprintln!(
        "live {:.0} -> {:.0} MiB ({:.0} MiB less), file {:.0} MiB, reclaimable {:.0} MiB",
        mib(before.live_bytes),
        mib(after.live_bytes),
        mib(before.live_bytes.saturating_sub(after.live_bytes)),
        mib(after.file_bytes),
        mib(after.reclaimable)
    );
    assert!(r.finished && after.recompressed);
}

/// A real data file compacted in place by redb, cold (made [`cold`] after its health is read):
/// `MC_DATA` as for [`a_real_data_file_is_compressed_again`]. Measured: the
/// time, the passes (the file shrinking, and its reads over its size), the bytes moved (written),
/// the peak working set, and the drive's own rates, fitted as `passes × file / read rate + moved /
/// write rate`; and the rate the health's count gives (what's to give back over the time), which
/// predicts how long a compaction takes before one runs.
#[test]
#[ignore = "needs a copy of a real data folder in MC_DATA"]
fn a_data_file_compacts_in_place() {
    let Some(root) = var("MC_DATA") else { return };
    let (s, id, file) = only_data_file(&root);
    let health = s.data_file_health(&id).unwrap().unwrap();
    eprintln!("before: {health:?}");
    drop(s);
    eprintln!("made cold in {:.1?}", cold(&file));
    let (read_rate, write_rate) = drive_rates(&file);
    eprintln!("the drive: reads {:.0} MiB/s, writes {:.0} MiB/s", read_rate / MIB, write_rate / MIB);
    let before = Process::now();
    let t = Instant::now();
    let mut db = redb::Database::builder().set_cache_size(crate::CACHE).create(&file).unwrap();
    let opened = t.elapsed();
    let shrinks = Shrinks::watch(&file);
    let t = Instant::now();
    let compacted = db.compact().unwrap();
    let took = t.elapsed();
    // As the app will: an empty sealed commit, so a crash after it opens without a repair.
    let t = Instant::now();
    let mut tx = db.begin_write().unwrap();
    tx.set_quick_repair(true);
    tx.commit().unwrap();
    let sealed = t.elapsed();
    drop(db);
    let shrank = shrinks.stop();
    let used = Process::now().since(before);
    let after_len = std::fs::metadata(&file).unwrap().len();
    let (s, id, _) = only_data_file(&root);
    let after = s.data_file_health(&id).unwrap().unwrap();
    s.data_file_compacted(&id).unwrap();
    let measured = s.data_file_health(&id).unwrap().unwrap();
    let passes = used.read as f64 / health.file_bytes as f64;
    let fit = passes * health.file_bytes as f64 / read_rate + used.written as f64 / write_rate;
    let given = health.reclaimable.min(health.live_bytes);
    eprintln!(
        "opened in {opened:?}; compacted ({compacted}) in {took:.1?}, sealed in {sealed:?}: {:.0} -> {:.0} MiB",
        mib(health.file_bytes),
        mib(after_len)
    );
    eprintln!(
        "read {:.0} MiB ({passes:.2} times the file), moved (wrote) {:.0} MiB, peak working set {:.0} MiB",
        mib(used.read),
        mib(used.written),
        mib(used.peak)
    );
    eprintln!(
        "the file shrank {} times{}",
        shrank.len(),
        shrank.first().zip(shrank.last()).map(|(a, b)| format!(", first at {:.1?}, last at {:.1?}", a.0, b.0)).unwrap_or_default()
    );
    eprintln!("fit: {fit:.1} s against {:.1} s ({:.2} of it)", took.as_secs_f64(), took.as_secs_f64() / fit);
    eprintln!(
        "the health said {:.0} MiB to give back: {:.0} MiB/s of it; given back {:.0} MiB",
        mib(given),
        mib(given) / took.as_secs_f64(),
        mib(health.file_bytes.saturating_sub(after_len))
    );
    eprintln!("after: {after:?}");
    eprintln!("after its overhead is measured: {measured:?}");
}
