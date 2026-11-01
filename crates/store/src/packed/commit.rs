//! How a data file commits: waiting for the disk or not, and sealed or not.
//!
//! A sealed commit (the database's quick-repair) records where the file's free space is, so a
//! crash after it reopens at once. It costs every commit that does it: on a copy of a 37 GB data
//! file a one-key commit took 15.6 ms sealed against 4.6 ms (medians), and a 16 MiB one 173 ms
//! against 119 ms. So commits aren't sealed one by one: a data file is sealed by an empty commit
//! once its writes have gone quiet for [`QUIET`], by closing it, and at once where a writer asks
//! ([`Store::seal`](crate::Store::seal): an import's end). A crash between a write and its seal
//! costs a full check of the file when it next opens (305 s for that file), which the window shows.
//!
//! Writers in the background commit batches that don't wait for the disk; the next durable commit
//! writes them out. So that a write from the window isn't the one left writing out a long run of
//! them, a batch that would take the run past [`UNDURABLE_MAX`] is made durable itself.

use std::sync::{Condvar, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use super::*;

/// How long a data file's writes have to stop before it's sealed.
pub(super) const QUIET: Duration = Duration::from_secs(2);
/// The most a data file holds in batches not yet on the disk.
const UNDURABLE_MAX: usize = 4 * crate::WRITE_BUFFER;

/// How a write transaction commits.
#[derive(Clone, Copy, Debug)]
pub(super) struct Commit {
    /// Wait for the disk. Otherwise the next durable commit (or closing the file) makes it so.
    durable: bool,
    /// Sealed: a crash after it needs no repair.
    seal: bool,
}

impl Commit {
    /// A write from the window, a folder list: on the disk when the commit returns.
    pub(super) const PLAIN: Commit = Commit { durable: true, seal: false };
    /// A batch in the background (an import's sources): durable with the next durable commit.
    pub(super) const BATCH: Commit = Commit { durable: false, seal: false };
}

/// What each data file has had committed: what isn't on the disk yet, and what isn't sealed. Data
/// files are sealed by a thread of their own once quiet.
#[derive(Debug, Default)]
pub(super) struct Seals {
    state: Mutex<Waiting>,
    wake: Condvar,
    /// How long each durable write waited for the writer.
    #[cfg(test)]
    pub(super) waits: Mutex<Vec<Duration>>,
}

#[derive(Debug, Default)]
struct Waiting {
    /// Each account's data file (not kept open by this), and when it was last written.
    due: HashMap<String, (Weak<Database>, Instant)>,
    /// Bytes each account's data file has in batches not yet on the disk.
    undurable: HashMap<String, usize>,
    running: bool,
    stop: bool,
}

impl Seals {
    fn lock(&self) -> MutexGuard<'_, Waiting> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `account`'s data file had a commit that wasn't sealed, of `undurable` bytes not waited for.
    fn written(self: &Arc<Self>, account: &str, db: &Arc<Database>, undurable: Option<usize>) {
        let mut w = self.lock();
        match undurable {
            Some(n) => {
                let run = w.undurable.entry(account.to_string()).or_default();
                *run = run.saturating_add(n);
            }
            None => {
                w.undurable.remove(account);
            }
        }
        if w.stop {
            return;
        }
        w.due.insert(account.to_string(), (Arc::downgrade(db), Instant::now()));
        if !w.running {
            let seals = self.clone();
            match std::thread::Builder::new().name("data-file-seal".into()).spawn(move || seals.run()) {
                Ok(_) => w.running = true,
                Err(e) => log::warn!("data files won't be sealed as they go quiet: {e}"),
            }
        }
        drop(w);
        self.wake.notify_one();
    }

    /// Whether `bytes` more would take `account`'s run of batches not on the disk too far.
    fn too_far(&self, account: &str, bytes: usize) -> bool {
        self.lock().undurable.get(account).copied().unwrap_or(0).saturating_add(bytes) > UNDURABLE_MAX
    }

    /// The thread: each data file sealed once it has been quiet for [`QUIET`].
    fn run(&self) {
        let mut w = self.lock();
        while !w.stop {
            let now = Instant::now();
            let quiet = |at: &Instant| now.saturating_duration_since(*at) >= QUIET;
            let ready: Vec<String> = w.due.iter().filter(|(_, (_, at))| quiet(at)).map(|(a, _)| a.clone()).collect();
            if ready.is_empty() {
                let next = w.due.values().map(|(_, at)| QUIET.saturating_sub(now.saturating_duration_since(*at))).min();
                w = match next {
                    Some(wait) => self.wake.wait_timeout(w, wait).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0),
                    None => self.wake.wait(w).unwrap_or_else(PoisonError::into_inner),
                };
                continue;
            }
            let files: Vec<(String, Weak<Database>)> = ready.into_iter().filter_map(|a| w.due.remove(&a).map(|(db, _)| (a, db))).collect();
            drop(w);
            for (account, db) in files {
                // A file let go of since closed itself, sealed.
                if let Some(db) = db.upgrade() {
                    match seal(&db) {
                        Ok(()) => {
                            self.lock().undurable.remove(&account);
                        }
                        Err(e) => log::warn!("{account}'s data file couldn't be sealed: {e}"),
                    }
                }
            }
            w = self.lock();
        }
        w.running = false;
    }

    /// No more sealing (the data files are closing, which seals them).
    pub(super) fn stop(&self) {
        self.lock().stop = true;
        self.wake.notify_all();
    }
}

/// An empty sealed commit (durable: it also writes out the batches before it).
fn seal(db: &Database) -> io::Result<()> {
    let mut tx = db.begin_write().map_err(other)?;
    tx.set_quick_repair(true);
    tx.commit().map_err(other)
}

impl Packs {
    /// Change an account's data file in one transaction, committed `how`. A batch that doesn't
    /// wait for the disk names its size, `undurable`.
    pub(super) fn change_with(
        &self,
        account: &str,
        how: Commit,
        undurable: usize,
        f: impl FnOnce(&mut Tables<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        let Some(db) = self.db(account, true)? else { return Err(io::Error::from(io::ErrorKind::NotFound)) };
        #[cfg(test)]
        let start = Instant::now();
        let mut tx = db.begin_write().map_err(other)?;
        #[cfg(test)]
        if how.durable {
            self.seals.waits.lock().unwrap_or_else(PoisonError::into_inner).push(start.elapsed());
        }
        if !how.durable {
            tx.set_durability(redb::Durability::None).map_err(other)?;
        }
        tx.set_quick_repair(how.seal);
        {
            let mut tables = Tables { files: tx.open_table(FILES).map_err(other)?, pieces: tx.open_table(PIECES).map_err(other)? };
            f(&mut tables)?;
        }
        tx.commit().map_err(other)?;
        if !how.seal {
            self.seals.written(account, &db, (!how.durable).then_some(undurable));
        }
        Ok(())
    }

    /// Sources made ready away from the writer, moved in: one transaction per data file, that only
    /// stores them (compressing them in it held the file several times as long).
    pub(crate) fn write_prepared(&self, items: Vec<(PathBuf, Prepared)>, durable: bool) -> Result<()> {
        let mut by_account: BTreeMap<String, Vec<(String, Prepared)>> = BTreeMap::new();
        for (p, prepared) in items {
            match self.place(&p) {
                Place::In(a, k) => by_account.entry(a).or_default().push((k, prepared)),
                Place::Account(_) | Place::Disk => write_atomic(&p, &prepared.into_raw().map_err(io_err(&p))?)?,
            }
        }
        for (a, files) in by_account {
            let bytes = files.iter().map(|(_, p)| p.kept_len()).fold(0usize, usize::saturating_add);
            let how = if durable || self.seals.too_far(&a, bytes) { Commit::PLAIN } else { Commit::BATCH };
            self.change_with(&a, how, bytes, |t| {
                for (k, prepared) in &files {
                    if prepared.kept {
                        t.put_kept(k, &prepared.bytes)?;
                    } else {
                        t.put(k, &prepared.bytes)?;
                    }
                }
                Ok(())
            })
            .map_err(|source| StoreError::Io { path: a.clone(), source })?;
        }
        Ok(())
    }

    /// Seal the data file of the account folder `dir`, when it has one.
    pub(crate) fn seal(&self, dir: &Path) -> Result<()> {
        let Place::Account(a) = self.place(dir) else { return Ok(()) };
        let Some(db) = self.db(&a, false).map_err(io_err(dir))? else { return Ok(()) };
        seal(&db).map_err(io_err(dir))?;
        self.seals.lock().undurable.remove(&a);
        Ok(())
    }
}

impl Drop for Packs {
    fn drop(&mut self) {
        self.seals.stop();
    }
}
