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
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::CacheUse;

/// The data files open, and the accounts whose file is being opened.
#[derive(Debug, Default)]
pub(super) struct Open {
    pub(super) dbs: HashMap<String, Arc<Database>>,
    /// Being opened, by a thread holding no lock: others asking for it wait ([`Packs::opened`]).
    opening: HashSet<String>,
    /// The cache each open data file was opened with, when not [`CACHE`].
    caches: HashMap<String, usize>,
    /// Accounts taking a bulk write, and the cache their data file has meanwhile.
    bulk: HashMap<String, usize>,
    /// How the caches of the files let go of to change theirs were used (so a count over an
    /// import keeps what its file did before it was opened again).
    spent: HashMap<String, CacheUse>,
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
struct Opening<'a> {
    packs: &'a Packs,
    account: &'a str,
}

impl Drop for Opening<'_> {
    fn drop(&mut self) {
        Packs::lock(&self.packs.open).opening.remove(self.account);
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

impl Packs {
    /// The open files, once no thread is opening `account`'s.
    fn settled(&self, account: &str) -> std::sync::MutexGuard<'_, Open> {
        let mut open = Packs::lock(&self.open);
        while open.opening.contains(account) {
            open = self.opened.wait(open).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        open
    }

    /// Let go of an account's data file (once it's done opening): whether it was open.
    pub(super) fn close(&self, account: &str) -> bool {
        let mut open = self.settled(account);
        open.caches.remove(account);
        open.dbs.remove(account).is_some()
    }

    /// `account` takes a bulk write (an import): its data file has a cache of `cache_bytes` until
    /// [`Packs::bulk_end`], opened again with it now if nobody holds it (otherwise as soon as
    /// nobody does). A cache no bigger than [`CACHE`] changes nothing.
    pub(crate) fn bulk_begin(&self, account: &str, cache_bytes: usize) {
        let mut open = self.settled(account);
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
        self.settled(account).bulk.remove(account);
        // Ended by a panic: closed now, the file would write nothing (the database doesn't while
        // a thread panics) and need its full check when opened next; the next ask for it, from an
        // ordinary moment, gives it its usual cache.
        if !std::thread::panicking() {
            self.recache(account);
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
        let mut open = self.settled(account);
        if let Some(db) = open.dbs.get(account) {
            if open.caches.get(account).copied().unwrap_or(CACHE) == open.wanted(account) {
                return Ok(Some(db.clone()));
            }
            // Open with another cache than it should have (a bulk write began or ended while it
            // was in use): opened again with that one, once nobody holds it.
            drop(open);
            self.recache(account);
            open = self.settled(account);
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
        let db = match self.builder(&name, &checked, cache).create(&path) {
            Ok(db) => db,
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
                self.builder(&name, &checked, cache).create(&path).map_err(other)?
            }
        };
        if checked.load(Ordering::Acquire) {
            // Not sealed, it's checked again next time; it opened all the same.
            if let Err(e) = seal(&db, &self.seals, account) {
                log::warn!("{}: checked, but couldn't be sealed: {e}", path.display());
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

    /// How a data file named `name` opens: its cache of `cache` bytes, and its check (if it needs
    /// one) told to [`Packs::progress`] and marked in `checked`.
    fn builder(&self, name: &str, checked: &Arc<AtomicBool>, cache: usize) -> redb::Builder {
        let mut b = Database::builder();
        let (progress, name, checked) = (self.progress.clone(), name.to_string(), checked.clone());
        b.set_cache_size(cache).set_repair_callback(move |s| {
            checked.store(true, Ordering::Release);
            progress.checking(&name, s.progress());
        });
        b
    }

    /// Bring folders from before in, then open every account's data file (by file name), each
    /// told to [`Packs::progress`]. One that can't be opened is a problem told there, not the
    /// error: the others still open, and the account says why again when it's used.
    pub(crate) fn open_all(&self) -> Result<()> {
        self.bring_in_folders()?;
        let mut files: Vec<(String, String)> = Packs::lock(&self.names).iter().map(|(a, n)| (n.clone(), a.clone())).collect();
        files.sort();
        self.progress.start(files.len());
        for (name, account) in &files {
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
