//! Opening an account's data file, which can be a long check of it after MailCraft wasn't closed
//! properly: only that account waits for it, never the others (nothing is locked meanwhile but
//! the account's place in [`Open::opening`]), and [`Packs::progress`] says how it goes.
//!
//! A data file opens with a cache of [`CACHE`], or of more while its account takes a bulk write
//! (an import: [`Packs::bulk_begin`]). A write transaction holds what it changes in half the
//! cache; past that, pages are written out, and read back when changed again. A file open with
//! the other cache is closed and opened again the next time it's asked for while nobody else holds
//! it (on a 6.9 GB file the close took 16 ms, the open 6-9 ms); one in use meanwhile goes on with
//! the cache it has. The close at an import's end also gives back the room the file had grown
//! by ahead of its contents: 8.6 GB came down to 6.9 GB at once, rather than when MailCraft closed.

use std::collections::HashSet;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::CacheUse;

/// How long a data file let go of after a failed write is waited for, while something still
/// holds it, before it's opened again.
const LET_GO_WAIT: Duration = Duration::from_secs(2);

/// The data files open, and the accounts whose file is being opened.
#[derive(Debug, Default)]
pub(super) struct Open {
    pub(super) dbs: HashMap<String, Arc<Database>>,
    /// Being opened, by a thread holding no lock: others asking for it wait ([`Packs::opened`]).
    pub(super) opening: HashSet<String>,
    /// Of those, the ones a compaction has to itself (`shrink.rs`): a read from the window's
    /// thread isn't kept waiting for them ([`no_waiting`]).
    pub(super) taken: HashSet<String>,
    /// The cache each open data file was opened with, when not [`CACHE`].
    pub(super) caches: HashMap<String, usize>,
    /// Accounts taking a bulk write, and the cache their data file has meanwhile.
    bulk: HashMap<String, usize>,
    /// Accounts taking a bulk write, whatever their cache: each commit of theirs that waits for
    /// the disk is sealed meanwhile (`commit.rs`).
    bulk_writers: HashSet<String>,
    /// Accounts taking a long write (a big local move), and how many: the same, with no other
    /// cache ([`Packs::long_begin`]).
    long_writers: HashMap<String, usize>,
    /// How the caches of the files let go of to change theirs were used (so a count over an
    /// import keeps what its file did before it was opened again).
    spent: HashMap<String, CacheUse>,
    /// How many times each account's data file was let go of after a write the disk failed (to
    /// open again at its last durable commit: what wasn't durable yet is gone).
    reopened: HashMap<String, u64>,
    /// The data file let go of so, while something may still hold it (a writer waiting its turn,
    /// a reader): it can't be opened again until that lets go too.
    failed: HashMap<String, Weak<Database>>,
}

impl CacheUse {
    fn of(db: &Database) -> CacheUse {
        let s = db.cache_stats();
        CacheUse {
            evictions: s.evictions(),
            read_misses: s.read_misses(),
            write_misses: s.write_misses(),
            used_bytes: u64::try_from(s.used_bytes()).unwrap_or(u64::MAX),
        }
    }

    fn add(self, o: CacheUse) -> CacheUse {
        CacheUse {
            evictions: self.evictions.saturating_add(o.evictions),
            read_misses: self.read_misses.saturating_add(o.read_misses),
            write_misses: self.write_misses.saturating_add(o.write_misses),
            used_bytes: o.used_bytes,
        }
    }
}

/// An account no longer being opened, however that ends (a panic included): those waiting on it
/// look again.
pub(super) struct Opening<'a> {
    pub(super) packs: &'a Packs,
    pub(super) account: &'a str,
}

impl Drop for Opening<'_> {
    fn drop(&mut self) {
        let mut open = Packs::lock(&self.packs.open);
        open.opening.remove(self.account);
        open.taken.remove(self.account);
        drop(open);
        self.packs.opened.notify_all();
    }
}

/// A check of a data file over, however its opening ends (failing included): the next is told.
struct CheckOver<'a>(&'a OpenProgress);

impl Drop for CheckOver<'_> {
    fn drop(&mut self) {
        self.0.checked_one();
    }
}

impl Open {
    /// The cache `account`'s data file should have.
    fn wanted(&self, account: &str) -> usize {
        self.bulk.get(account).copied().unwrap_or(CACHE)
    }
}

thread_local! {
    /// This thread mustn't wait for a data file a compaction has to itself ([`no_waiting`]).
    static NO_WAITING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// This thread is writing ([`ThreadWriting`]): a write always waits its turn.
    static WRITING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The calling thread (the window's) is never kept waiting by a compaction that has an account's
/// data file to itself: a read of it fails at once ([`io::ErrorKind::WouldBlock`], "being
/// compacted") rather than freezing the window for as long as that takes (125 s, cold, for a
/// 44 GB file). A write still waits its turn: it's never refused for that.
pub(crate) fn no_waiting() {
    NO_WAITING.with(|w| w.set(true));
}

/// A write under way on this thread, until dropped ([`no_waiting`] doesn't refuse it).
pub(super) struct ThreadWriting(bool);

impl ThreadWriting {
    pub(super) fn begin() -> ThreadWriting {
        ThreadWriting(WRITING.with(|w| w.replace(true)))
    }
}

impl Drop for ThreadWriting {
    fn drop(&mut self) {
        WRITING.with(|w| w.set(self.0));
    }
}

/// Why a read from a thread that doesn't wait ([`no_waiting`]) is refused.
pub(super) fn being_compacted() -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, "this data file is being compacted; it's ready in a moment")
}

impl Packs {
    /// The open files, once no thread is opening `account`'s.
    pub(super) fn settled(&self, account: &str) -> std::sync::MutexGuard<'_, Open> {
        let mut open = Packs::lock(&self.open);
        while open.opening.contains(account) {
            open = self.opened.wait(open).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        open
    }

    /// [`Packs::settled`], but a read from a thread that doesn't wait ([`no_waiting`]) for a file
    /// a compaction has to itself is refused at once ([`being_compacted`]).
    fn settled_or_busy(&self, account: &str) -> io::Result<std::sync::MutexGuard<'_, Open>> {
        let refuse = NO_WAITING.with(std::cell::Cell::get) && !WRITING.with(std::cell::Cell::get);
        let mut open = Packs::lock(&self.open);
        while open.opening.contains(account) {
            if refuse && open.taken.contains(account) {
                return Err(being_compacted());
            }
            open = self.opened.wait(open).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        Ok(open)
    }

    /// Let go of an account's data file (once it's done opening): whether it was open.
    pub(super) fn close(&self, account: &str) -> bool {
        let mut open = self.settled(account);
        open.caches.remove(account);
        open.dbs.remove(account).is_some()
    }

    /// Let go of `account`'s data file `db` after a write the disk failed, so the next use opens
    /// it again (checked as after a crash, back at its last durable commit), and count it
    /// ([`Packs::reopened`]). Only that file: one let go of already (another thread's write failed
    /// on it too) or opened again since stays. Returns whether it was let go of now.
    pub(super) fn let_go_failed(&self, account: &str, db: &Arc<Database>) -> bool {
        let mut open = self.settled(account);
        if !open.dbs.get(account).is_some_and(|d| Arc::ptr_eq(d, db)) {
            return false;
        }
        open.dbs.remove(account);
        open.caches.remove(account);
        let n = open.reopened.entry(account.to_string()).or_default();
        *n = n.saturating_add(1);
        open.failed.insert(account.to_string(), Arc::downgrade(db));
        true
    }

    /// How many times `account`'s data file was let go of after a write the disk failed: a writer
    /// that sees it change knows what it wrote without waiting for the disk may be gone.
    pub(crate) fn reopened(&self, account: &str) -> u64 {
        Packs::lock(&self.open).reopened.get(account).copied().unwrap_or(0)
    }

    /// Whether the data file let go of after a failed write is still held, waiting
    /// [`LET_GO_WAIT`] at most for whoever holds it (a writer waiting its turn fails at once, a
    /// reader ends) before it opens again: the file can't be opened twice.
    fn still_held(&self, account: &str) -> bool {
        let end = Instant::now().checked_add(LET_GO_WAIT);
        loop {
            {
                let mut open = Packs::lock(&self.open);
                match open.failed.get(account).map(Weak::strong_count) {
                    None => return false,
                    Some(0) => {
                        open.failed.remove(account);
                        return false;
                    }
                    Some(_) => {}
                }
            }
            if end.is_none_or(|end| Instant::now() >= end) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// `account` takes a bulk write (an import): its data file has a cache of `cache_bytes` until
    /// [`Packs::bulk_end`], opened again with it now if nobody holds it (otherwise as soon as
    /// nobody does). A cache no bigger than [`CACHE`] changes nothing.
    pub(crate) fn bulk_begin(&self, account: &str, cache_bytes: usize) {
        let mut open = self.settled(account);
        open.bulk_writers.insert(account.to_string());
        if cache_bytes > CACHE {
            open.bulk.insert(account.to_string(), cache_bytes);
        } else {
            open.bulk.remove(account);
        }
        drop(open);
        if !self.recache(account) && self.cache_of(account).is_some_and(|c| c != cache_bytes.max(CACHE)) {
            log::info!("{account}: its data file is in use; the bulk write starts with the cache it has");
        }
    }

    /// `account`'s bulk write is over: its data file goes back to [`CACHE`], letting go of the
    /// rest, now if nobody holds it (a clean close: it opens again at once), otherwise the next
    /// time it's asked for once nobody does.
    pub(crate) fn bulk_end(&self, account: &str) {
        {
            let mut open = self.settled(account);
            open.bulk.remove(account);
            open.bulk_writers.remove(account);
        }
        // Ended by a panic: closed now, the file would write nothing (the database doesn't while
        // a thread panics) and need its full check when opened next; the next ask for it, from an
        // ordinary moment, gives it its usual cache.
        if !std::thread::panicking() {
            self.recache(account);
        }
    }

    /// Whether `account` takes a bulk or a long write now: each of its commits that waits for the
    /// disk is sealed meanwhile, and its batches that don't run longer between them.
    pub(super) fn sealing(&self, account: &str) -> bool {
        let open = Packs::lock(&self.open);
        open.bulk_writers.contains(account) || open.long_writers.contains_key(account)
    }

    /// `account` takes a long write (a big local move) until [`Packs::long_end`]: each of its
    /// commits that waits for the disk is sealed meanwhile, as in a bulk write, so a crash
    /// part-way reopens the file at once. Written since its last seal, the file is sealed now,
    /// so a crash before the long write's first such commit does too. Whether it was written is
    /// asked with its writer held: a background commit that began before the long write did, and
    /// so wasn't sealed, has committed by then, and is sealed after.
    pub(crate) fn long_begin(&self, account: &str) {
        {
            let mut open = Packs::lock(&self.open);
            let n = open.long_writers.entry(account.to_string()).or_default();
            *n = n.saturating_add(1);
        }
        let _writing = ThreadWriting::begin();
        match self.db(account, false) {
            Ok(Some(db)) => {
                if let Err(e) = seal_if_due(&db, &self.seals, account) {
                    log::warn!("{account}'s data file couldn't be sealed: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => log::warn!("{account}'s data file couldn't be sealed: {e}"),
        }
    }

    /// One of `account`'s long writes is over.
    pub(crate) fn long_end(&self, account: &str) {
        let mut open = Packs::lock(&self.open);
        if let Some(n) = open.long_writers.get_mut(account) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                open.long_writers.remove(account);
            }
        }
    }

    /// `account`'s data file, open with another cache than it should have, closed if nobody else
    /// holds it, to be opened again with that cache. It's closed with the account marked as
    /// opening, so nobody gets it meanwhile, and with no lock held (closing writes out what waits).
    /// Whether it was.
    fn recache(&self, account: &str) -> bool {
        let mut open = self.settled(account);
        if open.caches.get(account).copied().unwrap_or(CACHE) == open.wanted(account)
            || open.dbs.get(account).is_none_or(|db| Arc::strong_count(db) > 1)
        {
            return false;
        }
        let Some(db) = open.dbs.remove(account) else { return false };
        // Taken only when this is the last hold of it: nobody (the sealing thread included, whose
        // hold is weak) can take it up again meanwhile.
        let db = match Arc::try_unwrap(db) {
            Ok(db) => db,
            Err(db) => {
                open.dbs.insert(account.to_string(), db);
                return false;
            }
        };
        open.opening.insert(account.to_string());
        drop(open);
        let _opening = Opening { packs: self, account };
        // Sealed before it's closed, rather than trusting the close (which says nothing of how it
        // went): what waited is made durable, or the file stays open and its seal still due.
        if let Err(e) = seal(&db, &self.seals, account) {
            log::warn!("{account}: its data file couldn't be sealed to change its cache: {e}");
            Packs::lock(&self.open).dbs.insert(account.to_string(), Arc::new(db));
            return false;
        }
        let used = CacheUse::of(&db);
        let mut open = Packs::lock(&self.open);
        let spent = open.spent.get(account).copied().unwrap_or_default().add(used);
        open.spent.insert(account.to_string(), spent);
        open.caches.remove(account);
        drop(open);
        drop(db);
        // Sealed and closed: nothing waits for a seal, and the free space is recorded.
        self.seals.forget(account);
        true
    }

    /// How `account`'s data file's cache has been used since the store opened ([`CacheUse`]).
    pub(crate) fn cache_use(&self, account: &str) -> CacheUse {
        let open = self.settled(account);
        let spent = open.spent.get(account).copied().unwrap_or_default();
        match open.dbs.get(account) {
            Some(db) => spent.add(CacheUse::of(db)),
            None => CacheUse { used_bytes: 0, ..spent },
        }
    }

    /// The cache `account`'s data file is open with (none: it isn't open).
    pub(crate) fn cache_of(&self, account: &str) -> Option<usize> {
        let open = self.settled(account);
        open.dbs.contains_key(account).then(|| open.caches.get(account).copied().unwrap_or(CACHE))
    }

    /// An account's open data file; `create` makes it when there's none. Looked up and named with
    /// the account marked as opening, so an account's connections starting together open it once:
    /// two threads opening the file at once had the second refused by the first's lock on it
    /// ("open in another copy of MailCraft") on Unix, or naming a second file beside it. No lock
    /// is held while it opens, which is a full check of it after an unclean exit: other accounts
    /// go on meanwhile. A file that had to be checked is sealed at once: one only read afterwards
    /// was checked again at every start until something sealed it (any way the process ended but
    /// closing, a window closed while it booted included).
    pub(super) fn db(&self, account: &str, create: bool) -> io::Result<Option<Arc<Database>>> {
        let mut open = self.settled_or_busy(account)?;
        if let Some(db) = open.dbs.get(account) {
            if open.caches.get(account).copied().unwrap_or(CACHE) == open.wanted(account) {
                return Ok(Some(db.clone()));
            }
            // Open with another cache than it should have (a bulk write began or ended while it
            // was in use): opened again with that one, once nobody holds it.
            drop(open);
            self.recache(account);
            open = self.settled_or_busy(account)?;
            if let Some(db) = open.dbs.get(account) {
                return Ok(Some(db.clone()));
            }
        }
        let cache = open.wanted(account);
        let known = Packs::lock(&self.names).contains_key(account);
        if !create && !known {
            return Ok(None);
        }
        open.opening.insert(account.to_string());
        drop(open);
        let _opening = Opening { packs: self, account };
        let name = self.name(account);
        let path = self.dir.join(&name);
        if !create && !path.exists() {
            return Ok(None);
        }
        let checked = Arc::new(AtomicBool::new(false));
        let _over = CheckOver(&self.progress);
        let held = self.still_held(account);
        // Made new: it says how it's kept (`META`).
        let mut fresh = !path.exists();
        let db = match self.create(&self.builder(&name, &checked, cache), &path) {
            Ok(db) => db,
            // Let go of after a failed write, and still held here: not another copy's.
            Err(redb::DatabaseError::DatabaseAlreadyOpen) if held => {
                return Err(other(format!("{} is being opened again after a write that failed; try again", path.display())));
            }
            Err(redb::DatabaseError::DatabaseAlreadyOpen) => {
                return Err(other(format!("{} is open in another copy of MailCraft", path.display())));
            }
            // The disk failing isn't damage (a file that isn't a data file is: InvalidData).
            Err(redb::DatabaseError::Storage(redb::StorageError::Io(e))) if e.kind() != io::ErrorKind::InvalidData => return Err(e),
            Err(e) => {
                // Damaged: set aside, and start again (the server has the mail of most accounts).
                let aside = path.with_extension(format!("{EXTENSION}.damaged"));
                log::warn!("{}: {e}; set aside as {}", path.display(), aside.display());
                fs::rename(&path, &aside)?;
                fresh = true;
                self.create(&self.builder(&name, &checked, cache), &path).map_err(other)?
            }
        };
        if checked.load(Ordering::Acquire) {
            // Not sealed, it's checked again next time; it opened all the same.
            if let Err(e) = seal(&db, &self.seals, account) {
                log::warn!("{}: checked, but couldn't be sealed: {e}", path.display());
            }
        }
        // Made new: it says at once, sealed, that everything in it is kept as this build keeps it
        // (a first commit not waited for to the disk would be lost to a crash or a full drive,
        // and the file would read as one from before for good). Not sealed, its writes say so
        // until one of them reaches the disk.
        if fresh {
            match seal_with(&db, &self.seals, account, |tx| write_meta(tx, &[(DEFLATE, DEFLATE_NOW.to_le_bytes().as_slice())])) {
                Ok(()) => {}
                Err(e) => {
                    log::warn!("{}: made, but couldn't say how it's kept yet: {e}", path.display());
                    Packs::lock(&self.fresh).insert(account.to_string());
                }
            }
        }
        let db = Arc::new(db);
        let mut open = Packs::lock(&self.open);
        open.dbs.insert(account.to_string(), db.clone());
        if cache == CACHE {
            open.caches.remove(account);
        } else {
            open.caches.insert(account.to_string(), cache);
        }
        Ok(Some(db))
    }

    /// The data file at `path` opened as `b` says (made when it isn't there), read through
    /// handles of its own (`handles.rs`).
    #[cfg(not(any(test, feature = "test-drive")))]
    fn create(&self, b: &redb::Builder, path: &Path) -> std::result::Result<Database, redb::DatabaseError> {
        b.create_with_backend(super::handles::DataFile::open(path)?.counted(&self.progress))
    }

    /// …on a drive a test can fill ([`Packs::full`]).
    #[cfg(any(test, feature = "test-drive"))]
    fn create(&self, b: &redb::Builder, path: &Path) -> std::result::Result<Database, redb::DatabaseError> {
        super::filling::create(b, path, &self.full, &self.progress)
    }

    /// How a data file named `name` opens: its cache of `cache` bytes, and its check (if it needs
    /// one) told to [`Packs::progress`] and marked in `checked`.
    fn builder(&self, name: &str, checked: &Arc<AtomicBool>, cache: usize) -> redb::Builder {
        let mut b = Database::builder();
        // Its size, for how far a pass over it has come; told by its file name (a copy beside an
        // Outlook data file is named by its whole path).
        let path = self.dir.join(name);
        let name = shown(name);
        let (progress, checked) = (self.progress.clone(), checked.clone());
        b.set_cache_size(cache).set_repair_callback(move |s| {
            if !checked.swap(true, Ordering::AcqRel) {
                progress.sized(fs::metadata(&path).map_or(0, |m| m.len()));
            }
            progress.checking(&name, s.progress());
        });
        b
    }

    /// Bring folders from before in, then open every account's data file (by file name), each
    /// told to [`Packs::progress`]. One that can't be opened is a problem told there, not the
    /// error: the others still open, and the account says why again when it's used.
    pub(crate) fn open_all(&self) -> Result<()> {
        self.bring_in_folders()?;
        self.remove_copies();
        let mut files: Vec<(String, String)> = Packs::lock(&self.names).iter().map(|(a, n)| (n.clone(), a.clone())).collect();
        files.sort();
        self.progress.start(files.len());
        for (name, account) in &files {
            let name = &shown(name);
            self.progress.opening(name);
            match self.db(account, false) {
                Ok(_) => {}
                Err(e) => {
                    log::warn!("{name}: {e}");
                    self.progress.fail(name, e.to_string());
                }
            }
            self.progress.opened();
        }
        if self.progress.checked() > 0 {
            log::info!("{} data file(s) checked as they opened (MailCraft wasn't closed properly)", self.progress.checked());
        }
        Ok(())
    }
}

/// A data file's name as it's shown: the file's own (a copy beside an Outlook data file is named
/// by its whole path).
fn shown(name: &str) -> String {
    Path::new(name).file_name().and_then(|n| n.to_str()).unwrap_or(name).to_string()
}
