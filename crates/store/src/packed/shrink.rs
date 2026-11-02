//! A data file made smaller, giving its free room back to the drive: compacted in place by the
//! database, or copied into a new file that takes its place.
//!
//! Either way the account's file is had to itself at the end ([`Packs::take`]): taken out of the
//! open files with the account marked as opening, so whoever asks for it meanwhile (a sync, a
//! removal, letting it go) waits until it's back (but for a read from the window's thread, refused
//! at once: [`super::no_waiting`]), and taken up only once nothing holds it (a call
//! under way holds it for milliseconds; one that holds it longer than [`TAKE_WAIT`] has it given
//! back, and the compaction refused). Should it fail or panic meanwhile, the file is closed rather
//! than given back, and opened again (checked, when it has to be) the next time it's asked for.
//!
//! In place ([`Packs::compact_in_place`]), the database moves what it keeps to the front of the
//! file and cuts off the end, which can't be stopped and has the file the whole time (125 s,
//! cold, on a 44 GB file). The commit after it is sealed (the compaction records nowhere what's
//! free), and how long it took against the measured cost is kept as the file's pace
//! (`compact.pace` in `META`) for the next one's estimate.
//!
//! By a copy ([`Packs::compact_by_copy`]), every file goes into a new database beside it in key
//! order, in batches as a background writer makes them, compressed again on the way when kept raw
//! (as a build before 0.32 kept attachments): the account goes on being read and written
//! meanwhile, and the copy can be stopped at any batch (it's thrown away). Every key written
//! meanwhile is noted ([`Journal`]); once the copy has every file, the account's file is had to
//! itself, those keys are copied again (compressed as the batches are), the copy sealed, and put
//! in its place, with what recompressing had counted in the old file's `META`. A crash before that
//! leaves the old file as it was (and the copy, removed when the data files next open); the
//! rename that puts it in place is the drive's own, all or nothing, tried again while another
//! program has the file open for a moment, and made to last (`crate::atomic::rename`). A file in
//! pieces that don't add up (damaged) can't be copied as it is: the copy is refused rather than
//! made without it.

use std::collections::BTreeSet;
use std::ops::Bound;
use std::time::{Duration, Instant};

use mailcraft_model::job::{PACE_ONE, Work, compact_ms, next_pace};

use super::compact::{BIGGEST, compress, count};
use super::*;
use crate::Shrunk;

/// How long a data file is waited for to be let go of by whoever holds it.
const TAKE_WAIT: Duration = Duration::from_secs(5);
/// The most keys a copy notes as written meanwhile: past it, so much changed that the copy is
/// given up (and tried again later).
const JOURNAL_MAX: usize = 200_000;
/// A batch of a copy holds at most this much as kept (one file bigger alone), and this many files.
const COPY_BYTES: usize = crate::BACKGROUND_COMMIT;
const COPY_FILES: usize = 4096;
/// A copy being made of a data file: `<name>.compacting` beside it.
const COPY_EXTENSION: &str = "compacting";

/// The keys written to an account's data file while it's copied.
#[derive(Debug, Default)]
pub(super) struct Journal {
    keys: BTreeSet<String>,
    /// More than [`JOURNAL_MAX`]: the copy is given up.
    full: bool,
}

/// An account's data file had to itself ([`Packs::take`]), until it's given back, or closed.
struct Taken<'a> {
    packs: &'a Packs,
    account: &'a str,
    db: Option<Database>,
    /// Dropped last: those waiting for the file look again.
    _opening: Opening<'a>,
}

impl Taken<'_> {
    /// The file open again for everyone, as it is.
    fn give_back(mut self) {
        if let Some(db) = self.db.take() {
            Packs::lock(&self.packs.open).dbs.insert(self.account.to_string(), Arc::new(db));
        }
    }
}

impl Drop for Taken<'_> {
    fn drop(&mut self) {
        // Not given back (an error, a panic, or a copy put in its place): closed, opened again
        // when next asked for (after a panic the database writes nothing, and it's checked then).
        if let Some(db) = self.db.take() {
            Packs::lock(&self.packs.open).caches.remove(self.account);
            drop(db);
        }
    }
}

/// An account's copy noting what's written, given up however the copy ends.
struct Journaling<'a>(&'a Packs, &'a str);

impl Drop for Journaling<'_> {
    fn drop(&mut self) {
        Packs::lock(&self.0.copying).remove(self.1);
    }
}

/// A copy being made, removed unless it took the data file's place.
struct CopyFile {
    path: PathBuf,
    kept: bool,
}

impl Drop for CopyFile {
    fn drop(&mut self) {
        if !self.kept
            && let Err(e) = fs::remove_file(&self.path)
            && e.kind() != io::ErrorKind::NotFound
        {
            log::warn!("{}: not removed: {e}", self.path.display());
        }
    }
}

/// Where a copy of the data file at `path` is made.
pub(super) fn copy_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(std::ffi::OsStr::to_os_string).unwrap_or_default();
    name.push(".");
    name.push(COPY_EXTENSION);
    path.with_file_name(name)
}

/// Whether `name` is a copy left by a compaction that didn't end (a crash).
pub(super) fn is_copy(name: &str) -> bool {
    let data_file = name.strip_suffix(COPY_EXTENSION).and_then(|n| n.strip_suffix('.'));
    data_file.and_then(|n| n.strip_suffix(EXTENSION)).and_then(|n| n.strip_suffix('.')).is_some_and(|stem| !stem.is_empty())
}

/// A file's bytes as kept (`v`, its value in `FILES`: whole, or the note of one in pieces read
/// from `pieces`); `None` when it's damaged.
fn kept_bytes<P>(key: &str, v: &[u8], pieces: Option<&P>) -> io::Result<Option<Vec<u8>>>
where
    P: ReadableTable<(&'static str, u32), &'static [u8]>,
{
    let Some(len) = pieced_len(v) else { return Ok(Some(v.to_vec())) };
    let Some(pieces) = pieces else { return Ok(None) };
    let mut out = Vec::with_capacity(usize::try_from(len).unwrap_or(0).min(8 << 20));
    for item in pieces.range((key, 0)..=(key, u32::MAX)).map_err(disk)? {
        let (_, piece) = item.map_err(disk)?;
        if (out.len() as u64).saturating_add(piece.value().len() as u64) > len {
            return Ok(None);
        }
        out.extend_from_slice(piece.value());
    }
    Ok((out.len() as u64 == len).then_some(out))
}

/// The files of a copy's batch, as kept, and where it got to.
#[derive(Default)]
struct CopyBatch {
    files: Vec<(String, Vec<u8>)>,
    last: Option<String>,
    looked_bytes: u64,
    /// Damaged files (pieces missing): a copy would be without them, so none is made.
    damaged: u64,
    end: bool,
}

/// A copy refused because the data file has files in pieces that don't add up.
fn damaged_refused(damaged: u64) -> io::Error {
    io::Error::other(format!("{damaged} file(s) in the data file are damaged and a copy would be without them, so it wasn't copied"))
}

/// `files` (key, as kept) written to a copy, what an earlier build kept raw compressed on the way.
fn put_copied(t: &mut Tables<'_>, files: &[(String, Vec<u8>)]) -> io::Result<()> {
    let raw: Vec<usize> = files.iter().enumerate().filter(|(_, (k, v))| to_compress(k, v)).map(|(i, _)| i).collect();
    let packed = compress(&raw.iter().filter_map(|i| files.get(*i).map(|(_, v)| v.as_slice())).collect::<Vec<_>>());
    let mut packed = raw.into_iter().zip(packed).filter_map(|(i, p)| Some((i, p?))).peekable();
    for (i, (key, kept)) in files.iter().enumerate() {
        match packed.next_if(|(at, _)| *at == i) {
            Some((_, p)) => t.put_kept(key, &p)?,
            None => t.put_kept(key, kept)?,
        }
    }
    Ok(())
}

/// The next batch of every file after `after` (from the start: `None`), up to [`COPY_BYTES`] (one
/// file bigger alone) and [`COPY_FILES`].
fn read_copy_batch(db: &Database, after: Option<&str>) -> io::Result<CopyBatch> {
    let mut b = CopyBatch::default();
    let tx = db.begin_read().map_err(disk)?;
    let files = match tx.open_table(FILES) {
        Ok(t) => t,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(CopyBatch { end: true, ..b }),
        Err(e) => return Err(disk(e)),
    };
    let pieces = match tx.open_table(PIECES) {
        Ok(t) => Some(t),
        Err(redb::TableError::TableDoesNotExist(_)) => None,
        Err(e) => return Err(disk(e)),
    };
    let lower = after.map_or(Bound::Unbounded, Bound::Excluded);
    let mut bytes = 0usize;
    for item in files.range::<&str>((lower, Bound::Unbounded)).map_err(disk)? {
        let (k, v) = item.map_err(disk)?;
        let (key, v) = (k.value(), v.value());
        let kept = usize::try_from(pieced_len(v).unwrap_or(v.len() as u64)).unwrap_or(usize::MAX);
        if b.files.len() >= COPY_FILES || (!b.files.is_empty() && bytes.saturating_add(kept) > COPY_BYTES) {
            return Ok(b);
        }
        b.last = Some(key.to_string());
        b.looked_bytes = b.looked_bytes.saturating_add(kept as u64);
        match kept_bytes(key, v, pieces.as_ref())? {
            Some(file) => {
                bytes = bytes.saturating_add(file.len());
                b.files.push((key.to_string(), file));
            }
            None => b.damaged = b.damaged.saturating_add(1),
        }
    }
    b.end = true;
    Ok(b)
}

/// Every batch a copy of `db` would read, as many as it takes (the fuzz target's), with how
/// many files they hold.
pub(super) fn read_all_batches(db: &Database) -> io::Result<usize> {
    let (mut after, mut files) = (None::<String>, 0usize);
    loop {
        let b = read_copy_batch(db, after.as_deref())?;
        files = files.saturating_add(b.files.len());
        // Each batch moves on, or ends: a key repeated would be a loop.
        if b.end || b.last.is_none() || b.last == after {
            return Ok(files);
        }
        after = b.last;
    }
}

/// Whether a file as kept is kept raw and would be compressed again on the way.
fn to_compress(key: &str, kept: &[u8]) -> bool {
    kept.len() >= DEFLATE_MIN && kept.len() as u64 <= BIGGEST && !kept.starts_with(DEFLATED) && !key.starts_with(SEARCH)
}

/// A write transaction of the copy `db`, not waited for to the disk (the seal at its end is).
fn write_copy(db: &Database, f: impl FnOnce(&mut Tables<'_>) -> io::Result<()>) -> io::Result<()> {
    let mut tx = db.begin_write().map_err(disk)?;
    tx.set_durability(redb::Durability::None).map_err(other)?;
    {
        let mut t = Tables { files: tx.open_table(FILES).map_err(disk)?, pieces: tx.open_table(PIECES).map_err(disk)?, touched: None };
        f(&mut t)?;
    }
    tx.commit().map_err(disk)
}

impl Packs {
    /// Whether `account`'s data file is being copied: what's written to it is noted.
    pub(super) fn copying(&self, account: &str) -> bool {
        Packs::lock(&self.copying).contains_key(account)
    }

    /// `keys` were written to `account`'s data file while it's copied.
    pub(super) fn copied_again(&self, account: &str, keys: Vec<String>) {
        if let Some(j) = Packs::lock(&self.copying).get_mut(account) {
            j.keys.extend(keys);
            if j.keys.len() > JOURNAL_MAX {
                j.full = true;
                j.keys.clear();
            }
        }
    }

    /// `account`'s data file had to itself: opened first (checked, if it has to be), then taken
    /// from the open files once nothing holds it (`None`: it has none, or was let go of). Refused
    /// ([`io::ErrorKind::ResourceBusy`]) when something holds it past [`TAKE_WAIT`].
    fn take<'a>(&'a self, account: &'a str) -> io::Result<Option<Taken<'a>>> {
        if self.db(account, false)?.is_none() {
            return Ok(None);
        }
        let mut open = self.settled(account);
        let Some(mut db) = open.dbs.remove(account) else { return Ok(None) };
        open.opening.insert(account.to_string());
        open.taken.insert(account.to_string());
        drop(open);
        let opening = Opening { packs: self, account };
        // The sealing thread lets go of it (a seal under way ends first); nothing waits to seal it.
        self.seals.forget(account);
        let end = Instant::now().checked_add(TAKE_WAIT);
        loop {
            match Arc::try_unwrap(db) {
                Ok(db) => return Ok(Some(Taken { packs: self, account, db: Some(db), _opening: opening })),
                Err(held) if end.is_some_and(|end| Instant::now() < end) => {
                    db = held;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(held) => {
                    Packs::lock(&self.open).dbs.insert(account.to_string(), held);
                    return Err(io::Error::new(io::ErrorKind::ResourceBusy, "the data file is in use; try again in a moment"));
                }
            }
        }
    }

    /// `account`'s data file compacted in place by the database ([`crate::Store::compact_data_file`]);
    /// `None` when it has none.
    pub(crate) fn compact_in_place(&self, account: &str) -> io::Result<Option<Shrunk>> {
        let Some(path) = self.file(account) else { return Ok(None) };
        let Some(mut taken) = self.take(account)? else { return Ok(None) };
        let Some(db) = taken.db.as_mut() else { return Ok(None) };
        let start = Instant::now();
        let before = fs::metadata(&path)?.len();
        let pace = read_meta(&db.begin_read().map_err(disk)?)?.pace;
        #[cfg(test)]
        if let Some(hook) = Packs::lock(&self.compact_hook).as_ref() {
            hook()?;
        }
        db.compact().map_err(other)?;
        let took = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        let moved = before.saturating_sub(fs::metadata(&path)?.len());
        let live = walk(&db.begin_read().map_err(disk)?, false)?.live;
        let overhead = overhead_ppm(live, fs::metadata(&path)?.len());
        let pace = next_pace(pace, took, compact_ms(before, moved, PACE_ONE));
        // The compaction records nowhere what's free: sealed, a crash after it reopens at once.
        seal_with(db, &self.seals, account, |tx| {
            write_meta(tx, &[(OVERHEAD, overhead.to_le_bytes().as_slice()), (PACE, pace.to_le_bytes().as_slice())])
        })?;
        let after = fs::metadata(&path)?.len();
        taken.give_back();
        Ok(Some(Shrunk { before, after, elapsed_ms: took, ..Shrunk::default() }))
    }

    /// `account`'s data file copied into a new one that takes its place
    /// ([`crate::Store::copy_data_file`]); `None` when it has none, or is removed meanwhile.
    /// `go_on` hears how far it has come before each batch, and stops it by saying no; `swap`
    /// hears it's about to have the file to itself, and stops it too by saying no.
    pub(crate) fn compact_by_copy(
        &self,
        account: &str,
        go_on: &mut dyn FnMut(Work) -> bool,
        swap: &mut dyn FnMut() -> bool,
    ) -> io::Result<Option<Shrunk>> {
        let Some(path) = self.file(account) else { return Ok(None) };
        let Some(db) = self.db(account, false)? else { return Ok(None) };
        let start = Instant::now();
        let before = fs::metadata(&path)?.len();
        let mut copy = CopyFile { path: copy_path(&path), kept: false };
        match fs::remove_file(&copy.path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        // What's written from here on is noted. The writer taken once after: a write that began
        // before and didn't see it has committed by then, and the copy reads what it wrote.
        if Packs::lock(&self.copying).insert(account.to_string(), Journal::default()).is_some() {
            return Err(io::Error::new(io::ErrorKind::ResourceBusy, "the data file is being copied already"));
        }
        let _journaling = Journaling(self, account);
        drop(db.begin_write().map_err(disk)?);
        let mut work = count(&db, None)?;
        drop(db);
        let new = Database::builder().set_cache_size(CACHE).create(&copy.path).map_err(other)?;
        let (mut after, mut damaged) = (None::<String>, 0u64);
        loop {
            if !go_on(work) {
                return Ok(Some(Shrunk { before, after: before, elapsed_ms: 0, cancelled: true, ..Shrunk::default() }));
            }
            // Let go of between batches: a removal, or a write from the window, goes first.
            let Some(db) = self.db(account, false)? else { return Ok(None) };
            let batch = read_copy_batch(&db, after.as_deref())?;
            drop(db);
            damaged = damaged.saturating_add(batch.damaged);
            if damaged > 0 {
                return Err(damaged_refused(damaged));
            }
            write_copy(&new, |t| put_copied(t, &batch.files))?;
            work.done = work.done.saturating_add(batch.files.len() as u64).saturating_add(batch.damaged).min(work.total);
            work.bytes_done = work.bytes_done.saturating_add(batch.looked_bytes).min(work.bytes_total);
            if batch.last.is_some() {
                after = batch.last;
            }
            if batch.end {
                break;
            }
            std::thread::yield_now();
        }
        // The rest with the file to itself: what was written meanwhile copied again, the copy
        // sealed, closed and put in its place.
        if !swap() {
            return Ok(Some(Shrunk { before, after: before, elapsed_ms: 0, cancelled: true, ..Shrunk::default() }));
        }
        let Some(mut taken) = self.take(account)? else { return Ok(None) };
        let journal = Packs::lock(&self.copying).get_mut(account).map(std::mem::take).unwrap_or_default();
        if journal.full {
            taken.give_back();
            return Err(io::Error::other("too much was written to the data file while it was copied; it's tried again later"));
        }
        let Some(db) = taken.db.take() else { return Ok(None) };
        let meta = {
            let tx = db.begin_read().map_err(disk)?;
            let files = tx.open_table(FILES).map_err(disk)?;
            let pieces = match tx.open_table(PIECES) {
                Ok(t) => Some(t),
                Err(redb::TableError::TableDoesNotExist(_)) => None,
                Err(e) => return Err(disk(e)),
            };
            let (mut now, mut gone) = (Vec::new(), Vec::new());
            for key in &journal.keys {
                match files.get(key.as_str()).map_err(disk)? {
                    Some(v) => match kept_bytes(key, v.value(), pieces.as_ref())? {
                        Some(kept) => now.push((key.clone(), kept)),
                        None => return Err(damaged_refused(1)),
                    },
                    None => gone.push(key),
                }
            }
            write_copy(&new, |t| {
                put_copied(t, &now)?;
                for key in gone {
                    t.remove(key)?;
                }
                Ok(())
            })?;
            read_meta(&tx)?
        };
        // Everything in it is kept as this build keeps it (what recompressing counted in the old
        // file carried over); sealed, and on the disk.
        let mut tx = new.begin_write().map_err(disk)?;
        tx.set_quick_repair(true);
        let counted = [(DEFLATE, DEFLATE_NOW), (PACE, meta.pace), (STAYED_RAW, meta.stayed_raw), (SAVED, meta.saved), (PASSED, meta.passed)];
        let counted: Vec<(&str, [u8; 8])> = counted.iter().map(|(k, v)| (*k, v.to_le_bytes())).collect();
        write_meta(&tx, &counted.iter().map(|(k, v)| (*k, v.as_slice())).collect::<Vec<_>>())?;
        tx.commit().map_err(disk)?;
        drop(new);
        drop(db);
        // A test's: something holding the file as it's put in place.
        #[cfg(test)]
        if let Some(hook) = Packs::lock(&self.compact_hook).as_ref() {
            hook()?;
        }
        crate::atomic::rename(&copy.path, &path)?;
        copy.kept = true;
        crate::atomic::sync_folder_of(&path);
        drop(taken);
        // What the new file has over what it keeps is the least it can have.
        if let Err(e) = self.compacted(account) {
            log::warn!("{account}: its data file was copied, but what it has over its mail wasn't measured: {e}");
        }
        let after = fs::metadata(&path)?.len();
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(Some(Shrunk { before, after, elapsed_ms, copied: true, ..Shrunk::default() }))
    }

    /// Copies left by compactions that didn't end (a crash), removed: the data files they were of
    /// are as they were.
    pub(super) fn remove_copies(&self) {
        let Ok(rd) = fs::read_dir(&self.dir) else { return };
        for e in rd.flatten() {
            if e.file_name().to_str().is_some_and(is_copy) && e.file_type().is_ok_and(|t| t.is_file()) {
                match fs::remove_file(e.path()) {
                    Ok(()) => log::info!("{}: a copy left by a compaction that didn't end, removed", e.path().display()),
                    Err(e2) => log::warn!("{}: not removed: {e2}", e.path().display()),
                }
            }
        }
    }
}
