//! What a data file keeps raw that compresses, compressed again: the first half of compacting
//! one (the second, the database's own `compact`, gives the space back to the drive).
//!
//! A build before 0.32 had flate2's zlib-rs backend turned on for it by another crate, whose
//! fastest level kept base64 at 0.995 of its size, so most attachments were kept raw: 34.9 GiB of
//! a 47 GB data file, which compresses by about a quarter (`pack` now calls miniz itself).
//!
//! It goes through `FILES` in key order a batch at a time, in the background, and can be stopped
//! between any two: each batch is read in a read transaction of its own, compressed away from the
//! file (several ways at once), then written in a short write transaction that also records the
//! last key it got to (`recompress.cursor` in `META`), so a stopped or killed run goes on from
//! there. A batch holds at most [`crate::BACKGROUND_COMMIT`] as kept, so a write from the window
//! waits at most for one; a file bigger than that is a batch on its own. What was read is written
//! back only if the file is still what it was (its length and a hash of every piece): one written
//! again meanwhile, by a sync or a move, is left as written. The search index's files (packed
//! already) are passed over. Once every file has been through it, `META` says the file is all kept
//! as this build keeps it (`deflate`), and the health scan stops sampling it.

use std::borrow::Cow;
use std::hash::Hasher;
use std::ops::Bound;
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_model::job::progress::Work;

use super::*;
use crate::Recompressed;

/// A batch holds at most this much as kept (as read; what's written is smaller).
const BATCH_BYTES: u64 = crate::BACKGROUND_COMMIT as u64;
/// A batch looks at most at this many files: most are compressed already, or small, and the
/// cursor is written after each batch, so a run stopped there doesn't look at them again.
const BATCH_FILES: usize = 4096;
/// Files bigger than this as kept are left as they are: one is read whole, and compressed, in
/// memory (a crafted note can say anything up to the most a file may come to).
const BIGGEST: u64 = 256 << 20;
/// Compressed at most this many ways at once.
const WAYS: usize = 8;

/// A file as read: its length and a hash of every piece (or of the whole value). The same
/// fingerprint when its batch is written means nothing wrote it meanwhile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Print {
    len: u64,
    hash: u64,
}

/// A file read to be compressed again.
struct Raw {
    key: String,
    bytes: Vec<u8>,
    print: Print,
}

/// The files of one batch, and where it got to.
#[derive(Default)]
struct Batch {
    files: Vec<Raw>,
    /// The last key looked at (none: nothing was).
    last: Option<String>,
    /// Files looked at, and their bytes as kept.
    looked: u64,
    looked_bytes: u64,
    /// `FILES` ends with this batch.
    end: bool,
}

/// How a run writes: a batch (with what it holds, as kept), or its last commit, durable and
/// sealed (at the end, or once stopped, so the cursor is on the disk).
#[derive(Clone, Copy, Debug)]
pub(super) enum Step {
    Batch(usize),
    Last,
}

/// What a run works on: an account's data file in the store, or a database the fuzz target made.
pub(super) trait Target {
    /// The database, as it is now (an account's file is taken up for each batch, not held for the
    /// whole run: it can be let go of, or opened again with another cache, between them).
    fn db(&self) -> io::Result<Arc<Database>>;
    /// A write transaction, committed as `step` says.
    fn commit(&self, step: Step, f: &mut dyn FnMut(&mut Tables<'_>, &redb::WriteTransaction) -> io::Result<()>) -> io::Result<()>;
}

/// A file's fingerprint from its value in `FILES` (a whole value, or the note of one in pieces,
/// whose pieces are read from `pieces`), and its bytes as kept when `keep`. `None` when it's
/// damaged: pieces missing, or more than its note says.
fn print_of<P>(key: &str, v: &[u8], pieces: Option<&P>, keep: bool) -> io::Result<Option<(Print, Vec<u8>)>>
where
    P: ReadableTable<(&'static str, u32), &'static [u8]>,
{
    let mut h = std::hash::DefaultHasher::new();
    let Some(len) = pieced_len(v) else {
        h.write_usize(v.len());
        h.write(v);
        let bytes = if keep { v.to_vec() } else { Vec::new() };
        return Ok(Some((Print { len: v.len() as u64, hash: h.finish() }, bytes)));
    };
    let Some(pieces) = pieces else { return Ok(None) };
    let mut bytes = Vec::new();
    if keep {
        bytes.reserve(usize::try_from(len).unwrap_or(0).min(8 << 20));
    }
    let mut seen = 0u64;
    for item in pieces.range((key, 0)..=(key, u32::MAX)).map_err(disk)? {
        let (_, piece) = item.map_err(disk)?;
        let piece = piece.value();
        seen = seen.saturating_add(piece.len() as u64);
        if seen > len {
            return Ok(None);
        }
        h.write_usize(piece.len());
        h.write(piece);
        if keep {
            bytes.extend_from_slice(piece);
        }
    }
    Ok((seen == len).then(|| (Print { len, hash: h.finish() }, bytes)))
}

/// Whether the file at `key` (its value in `FILES`: whole, or a note) is kept raw: its first bytes
/// (of its first piece) don't say it's compressed. `None` when it's damaged (no first piece).
fn kept_raw<P>(key: &str, v: &[u8], pieces: Option<&P>) -> io::Result<Option<bool>>
where
    P: ReadableTable<(&'static str, u32), &'static [u8]>,
{
    if pieced_len(v).is_none() {
        return Ok(Some(!v.starts_with(DEFLATED)));
    }
    let Some(pieces) = pieces else { return Ok(None) };
    Ok(pieces.get((key, 0)).map_err(disk)?.map(|g| !g.value().starts_with(DEFLATED)))
}

/// The next batch after `after` (from the start: `None`): files of at least [`DEFLATE_MIN`] kept
/// raw, read whole, up to [`BATCH_BYTES`] (one bigger alone) and [`BATCH_FILES`] looked at.
fn read_batch(db: &Database, after: Option<&str>) -> io::Result<Batch> {
    let mut b = Batch::default();
    let tx = db.begin_read().map_err(disk)?;
    let files = match tx.open_table(FILES) {
        Ok(t) => t,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Batch { end: true, ..b }),
        Err(e) => return Err(disk(e)),
    };
    let pieces = match tx.open_table(PIECES) {
        Ok(t) => Some(t),
        Err(redb::TableError::TableDoesNotExist(_)) => None,
        Err(e) => return Err(disk(e)),
    };
    let mut bytes = 0u64;
    let (mut from, mut past_index) = (after.map(str::to_string), false);
    'seek: loop {
        let lower = match (&from, past_index) {
            (None, _) => Bound::Unbounded,
            (Some(k), false) => Bound::Excluded(k.as_str()),
            (Some(k), true) => Bound::Included(k.as_str()),
        };
        for item in files.range::<&str>((lower, Bound::Unbounded)).map_err(disk)? {
            let (k, v) = item.map_err(disk)?;
            let (key, v) = (k.value(), v.value());
            if key.starts_with(SEARCH) {
                // The index's files, all together: on past them ('0' comes right after '/').
                from = Some(format!("{}0", SEARCH.trim_end_matches('/')));
                past_index = true;
                continue 'seek;
            }
            if b.looked >= BATCH_FILES as u64 {
                return Ok(b);
            }
            let kept = pieced_len(v).unwrap_or(v.len() as u64);
            let wanted = (DEFLATE_MIN as u64..=BIGGEST).contains(&kept) && kept_raw(key, v, pieces.as_ref())? == Some(true);
            // Full: this file starts the next batch.
            if wanted && !b.files.is_empty() && bytes.saturating_add(kept) > BATCH_BYTES {
                return Ok(b);
            }
            b.looked = b.looked.saturating_add(1);
            b.looked_bytes = b.looked_bytes.saturating_add(kept.min(INFLATE_MAX));
            b.last = Some(key.to_string());
            if !wanted {
                continue;
            }
            if let Some((print, raw)) = print_of(key, v, pieces.as_ref(), true)? {
                bytes = bytes.saturating_add(kept);
                b.files.push(Raw { key: key.to_string(), bytes: raw, print });
            }
        }
        break;
    }
    b.end = true;
    Ok(b)
}

/// A file compressed as `pack` keeps it, when that saves a tenth or more.
fn packed(bytes: &[u8]) -> Option<Vec<u8>> {
    match pack(bytes) {
        Cow::Owned(v) => Some(v),
        Cow::Borrowed(_) => None,
    }
}

/// Each file of a batch compressed, up to [`WAYS`] at once (on this thread when a thread can't be
/// had); `None` for one that stays as it is.
fn compress(files: &[Raw]) -> Vec<Option<Vec<u8>>> {
    let alone = |c: &[Raw]| c.iter().map(|f| packed(&f.bytes)).collect::<Vec<_>>();
    let ways = std::thread::available_parallelism().map_or(1, |n| n.get()).min(WAYS).min(files.len());
    if ways <= 1 {
        return alone(files);
    }
    let size = files.len().div_ceil(ways);
    std::thread::scope(|s| {
        let running: Vec<_> = files
            .chunks(size)
            .map(|c| (c, std::thread::Builder::new().name("data-file-recompress".into()).spawn_scoped(s, move || alone(c))))
            .collect();
        running
            .into_iter()
            .flat_map(|(c, t)| match t {
                // A thread that failed leaves its files as they are.
                Ok(t) => t.join().unwrap_or_else(|_| vec![None; c.len()]),
                Err(_) => alone(c),
            })
            .collect()
    })
}

/// How many files and bytes (as kept) `FILES` has, the search index's left out, and how many of
/// them come up to `cursor`: a run's whole, and what an earlier one went through.
fn count(db: &Database, cursor: Option<&str>) -> io::Result<Work> {
    let mut w = Work::default();
    let tx = db.begin_read().map_err(disk)?;
    let files = match tx.open_table(FILES) {
        Ok(t) => t,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(w),
        Err(e) => return Err(disk(e)),
    };
    for item in files.iter().map_err(disk)? {
        let (k, v) = item.map_err(disk)?;
        if k.value().starts_with(SEARCH) {
            continue;
        }
        let kept = pieced_len(v.value()).unwrap_or(v.value().len() as u64).min(INFLATE_MAX);
        w.total = w.total.saturating_add(1);
        w.bytes_total = w.bytes_total.saturating_add(kept);
        if cursor.is_some_and(|c| k.value() <= c) {
            w.done = w.done.saturating_add(1);
            w.bytes_done = w.bytes_done.saturating_add(kept);
        }
    }
    Ok(w)
}

/// Compress again what `target` keeps raw, from where a run before stopped, until it's all done or
/// `cancel` is set (looked at between batches). `progress` hears after each batch.
pub(super) fn recompress(target: &dyn Target, cancel: &AtomicBool, progress: &mut dyn FnMut(Work)) -> io::Result<Recompressed> {
    let mut report = Recompressed::default();
    let (meta, mut work) = {
        let db = target.db()?;
        let meta = read_meta(&db.begin_read().map_err(disk)?)?;
        if meta.deflate >= DEFLATE_NOW {
            return Ok(Recompressed { saved_in_all: meta.saved, finished: true, ..report });
        }
        let work = count(&db, meta.cursor.as_deref())?;
        (meta, work)
    };
    let (mut cursor, mut stayed_raw, mut saved) = (meta.cursor, meta.stayed_raw, meta.saved);
    progress(work);
    loop {
        if cancel.load(Ordering::Relaxed) {
            // The cursor written so far made durable, and the file sealed.
            target.commit(Step::Last, &mut |_, _| Ok(()))?;
            return Ok(Recompressed { saved_in_all: saved, ..report });
        }
        let batch = read_batch(&*target.db()?, cursor.as_deref())?;
        let Some(last) = batch.last.clone() else { break };
        let out = compress(&batch.files);
        let (mut run, mut written) = (report.clone(), 0usize);
        let (mut run_raw, mut run_saved) = (stayed_raw, saved);
        for (f, p) in batch.files.iter().zip(&out) {
            match p {
                Some(p) => written = written.saturating_add(p.len()),
                None => {
                    run.stayed_raw = run.stayed_raw.saturating_add(1);
                    run.stayed_raw_bytes = run.stayed_raw_bytes.saturating_add(f.print.len);
                    run_raw = run_raw.saturating_add(f.print.len);
                }
            }
        }
        target.commit(Step::Batch(written), &mut |t, tx| {
            // Counted afresh each time it's tried (a commit that failed tried nothing).
            let (mut r, mut s) = (run.clone(), run_saved);
            for (f, p) in batch.files.iter().zip(&out) {
                let Some(p) = p else { continue };
                let now = match t.files.get(f.key.as_str()).map_err(disk)? {
                    Some(v) => print_of(&f.key, v.value(), Some(&t.pieces), false)?.map(|(print, _)| print),
                    None => None,
                };
                if now != Some(f.print) {
                    r.changed = r.changed.saturating_add(1);
                    continue;
                }
                t.put_kept(&f.key, p)?;
                r.files = r.files.saturating_add(1);
                r.before = r.before.saturating_add(f.print.len);
                r.after = r.after.saturating_add(p.len() as u64);
                s = s.saturating_add(f.print.len.saturating_sub(p.len() as u64));
            }
            write_meta(tx, &[(CURSOR, last.as_bytes()), (STAYED_RAW, run_raw.to_le_bytes().as_slice()), (SAVED, s.to_le_bytes().as_slice())])?;
            run = r;
            run_saved = s;
            Ok(())
        })?;
        (report, cursor, stayed_raw, saved) = (run, Some(last), run_raw, run_saved);
        work.done = work.done.saturating_add(batch.looked).min(work.total);
        work.bytes_done = work.bytes_done.saturating_add(batch.looked_bytes).min(work.bytes_total);
        progress(work);
        if batch.end {
            break;
        }
        // Between batches, the file is anyone's: a write from the window goes first.
        std::thread::yield_now();
    }
    target.commit(Step::Last, &mut |_, tx| {
        write_meta(tx, &[(DEFLATE, DEFLATE_NOW.to_le_bytes().as_slice())])?;
        tx.open_table(META).map_err(disk)?.remove(CURSOR).map_err(disk)?;
        Ok(())
    })?;
    Ok(Recompressed { saved_in_all: saved, finished: true, ..report })
}

/// An account's data file in the store, as a run's [`Target`].
pub(super) struct Account<'a> {
    pub(super) packs: &'a Packs,
    pub(super) account: &'a str,
    /// How many times the file had been let go of after a failed write as the run began: a batch
    /// not waited for to the disk may be gone once that changes, with the cursor it recorded,
    /// and the run stops rather than go on past it.
    pub(super) reopened: u64,
}

impl Target for Account<'_> {
    fn db(&self) -> io::Result<Arc<Database>> {
        self.packs.db(self.account, false)?.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn commit(&self, step: Step, f: &mut dyn FnMut(&mut Tables<'_>, &redb::WriteTransaction) -> io::Result<()>) -> io::Result<()> {
        let (how, bytes) = match step {
            Step::Batch(n) => (Commit::BATCH, n),
            Step::Last => (Commit::SEALED, 0),
        };
        self.packs.change_in_tx(self.account, how, bytes, Some(self.reopened), |t, tx| f(t, tx))
    }
}

/// The account's place in [`Packs::recompressing`], given up however the run ends.
struct Running<'a>(&'a Packs, &'a str);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        Packs::lock(&self.0.recompressing).remove(self.1);
    }
}

impl Packs {
    /// What `account`'s data file keeps raw compressed again ([`crate::Store::recompress_data_file`]);
    /// `None` when it has none. One run at a time per file: another asking meanwhile is refused.
    pub(crate) fn recompress(&self, account: &str, cancel: &AtomicBool, progress: &mut dyn FnMut(Work)) -> io::Result<Option<Recompressed>> {
        if self.file(account).is_none() || self.db(account, false)?.is_none() {
            return Ok(None);
        }
        if !Packs::lock(&self.recompressing).insert(account.to_string()) {
            return Err(io::Error::new(io::ErrorKind::ResourceBusy, "the data file is being compressed again already"));
        }
        let _running = Running(self, account);
        let target = Account { packs: self, account, reopened: self.reopened(account) };
        recompress(&target, cancel, progress).map(Some)
    }
}

/// A database of its own, in memory, as a fuzz target's [`Target`].
struct InMemory(Arc<Database>);

impl Target for InMemory {
    fn db(&self) -> io::Result<Arc<Database>> {
        Ok(self.0.clone())
    }

    fn commit(&self, _: Step, f: &mut dyn FnMut(&mut Tables<'_>, &redb::WriteTransaction) -> io::Result<()>) -> io::Result<()> {
        let tx = self.0.begin_write().map_err(disk)?;
        {
            let mut t = Tables { files: tx.open_table(FILES).map_err(disk)?, pieces: tx.open_table(PIECES).map_err(disk)? };
            f(&mut t, &tx)?;
        }
        tx.commit().map_err(disk)
    }
}

/// The most a fuzz input's values come to, repeated: room for files in pieces and batches.
const FUZZ_MAX: usize = 8 << 20;

/// A data file from a fuzz input (`fuzz/fuzz_targets/store/data_file.rs`), in memory: records one
/// after another, each a letter saying where it goes (`F` a value in `FILES`: a whole file, or the
/// note of one in pieces; `P` a piece; `M` a value in `META`), its key's length (a byte) and key,
/// for a piece its number (a byte), then its value's length (two bytes, little-endian), a count of
/// times it's repeated (a byte) and the value.
fn fuzz_db(input: &[u8]) -> io::Result<Database> {
    let db = Database::builder().create_with_backend(redb::backends::InMemoryBackend::new()).map_err(other)?;
    let tx = db.begin_write().map_err(disk)?;
    {
        let mut files = tx.open_table(FILES).map_err(disk)?;
        let mut pieces = tx.open_table(PIECES).map_err(disk)?;
        let mut meta = tx.open_table(META).map_err(disk)?;
        let (mut at, mut total) = (0usize, 0usize);
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = input.get(at..at.checked_add(n)?)?;
            at += n;
            Some(s)
        };
        while let Some(&[kind]) = take(1) {
            let Some(&[klen]) = take(1) else { break };
            let Some(key) = take(usize::from(klen)).map(|k| String::from_utf8_lossy(k).into_owned()) else { break };
            let piece = if kind == b'P' {
                let Some(&[n]) = take(1) else { break };
                u32::from(n)
            } else {
                0
            };
            let Some(&[lo, hi, times]) = take(3) else { break };
            let Some(one) = take(usize::from(u16::from_le_bytes([lo, hi]))) else { break };
            let value = one.repeat(usize::from(times.max(1)));
            total = total.saturating_add(value.len());
            if total > FUZZ_MAX {
                break;
            }
            match kind {
                b'F' => drop(files.insert(key.as_str(), value.as_slice()).map_err(disk)?),
                b'P' => drop(pieces.insert((key.as_str(), piece), value.as_slice()).map_err(disk)?),
                b'M' => drop(meta.insert(key.as_str(), value.as_slice()).map_err(disk)?),
                _ => {}
            }
        }
    }
    tx.commit().map_err(disk)?;
    Ok(db)
}

/// Every file of `db` as it reads back (`None`: it doesn't).
fn read_all(db: &Database) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    let Ok(tx) = db.begin_read() else { return out };
    let Ok(files) = tx.open_table(FILES) else { return out };
    let Ok(iter) = files.iter() else { return out };
    let keys: Vec<String> = iter.filter_map(|i| i.ok().map(|(k, _)| k.value().to_string())).collect();
    for k in keys {
        let v = read_file(&tx, &files, &k).ok().flatten();
        out.insert(k, v);
    }
    out
}

/// A hostile data file (see [`fuzz_db`]) measured as its health is, compressed again (stopped
/// after its first batch, gone on with, then once more with nothing left to do), and read back
/// whole: whether every file that read back before reads back the same.
#[doc(hidden)]
pub fn fuzz_data_file(input: &[u8]) -> bool {
    let Ok(db) = fuzz_db(input) else { return true };
    let db = Arc::new(db);
    if let Ok(tx) = db.begin_read() {
        let _ = read_meta(&tx);
        let _ = walk(&tx, true);
    }
    let before = read_all(&db);
    let target = InMemory(db.clone());
    let (stop, mut heard) = (AtomicBool::new(false), 0);
    let _ = recompress(&target, &stop, &mut |_| {
        heard += 1;
        // Heard once as it starts, then after each batch.
        stop.store(heard >= 2, Ordering::Relaxed);
    });
    let go = AtomicBool::new(false);
    let _ = recompress(&target, &go, &mut |_| {});
    let _ = recompress(&target, &go, &mut |_| {});
    let after = read_all(&db);
    before.into_iter().all(|(k, v)| v.is_none_or(|v| after.get(&k).is_some_and(|a| a.as_ref() == Some(&v))))
}
