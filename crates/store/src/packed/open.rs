//! Opening an account's data file, which can be a long check of it after MailCraft wasn't closed
//! properly: only that account waits for it, never the others (nothing is locked meanwhile but
//! the account's place in [`Open::opening`]), and [`Packs::progress`] says how it goes.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

/// The data files open, and the accounts whose file is being opened.
#[derive(Debug, Default)]
pub(super) struct Open {
    pub(super) dbs: HashMap<String, Arc<Database>>,
    /// Being opened, by a thread holding no lock: others asking for it wait ([`Packs::opened`]).
    opening: HashSet<String>,
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
        self.settled(account).dbs.remove(account).is_some()
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
            return Ok(Some(db.clone()));
        }
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
        let db = match self.builder(&name, &checked).create(&path) {
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
                self.builder(&name, &checked).create(&path).map_err(other)?
            }
        };
        if checked.load(Ordering::Acquire) {
            // Not sealed, it's checked again next time; it opened all the same.
            if let Err(e) = seal(&db, &self.seals, account) {
                log::warn!("{}: checked, but couldn't be sealed: {e}", path.display());
            }
        }
        let db = Arc::new(db);
        Packs::lock(&self.open).dbs.insert(account.to_string(), db.clone());
        Ok(Some(db))
    }

    /// How a data file named `name` opens: its cache, and its check (if it needs one) told to
    /// [`Packs::progress`] and marked in `checked`.
    fn builder(&self, name: &str, checked: &Arc<AtomicBool>) -> redb::Builder {
        let mut b = Database::builder();
        let (progress, name, checked) = (self.progress.clone(), name.to_string(), checked.clone());
        b.set_cache_size(CACHE).set_repair_callback(move |s| {
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
