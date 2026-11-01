//! How a data file commits: waiting for the disk or not, and sealed or not.
//!
//! A sealed commit (the database's quick-repair) records where the file's free space is, so a
//! crash after it reopens at once. It costs every commit that does it: on a copy of a 37 GB data
//! file a one-key commit took 15.6 ms sealed against 4.6 ms (medians), and a 16 MiB one 173 ms
//! against 119 ms. So commits aren't sealed one by one: a data file is sealed by an empty commit
//! once its writes have gone quiet for [`QUIET`], and at once where a writer asks
//! ([`Store::seal`](crate::Store::seal): an import's end). As MailCraft closes,
//! [`Store::seal_all`](crate::Store::seal_all) seals every one and makes each commit after it
//! sealed: closing a file would seal it, but the threads holding the store (an account's worker, an
//! import) are let go rather than waited for, so the process seldom ends with the files closed.
//! A crash between a write and its seal costs a full check of the file when it next opens (305 s
//! for that file). Nothing shows that check yet: the account's first read waits for it (the boot
//! screen that will show it is still to come).
//!
//! Writers in the background commit batches that don't wait for the disk; the next durable commit
//! writes them out. So that a write from the window isn't the one left writing out a long run of
//! them, a batch that would take the run past [`UNDURABLE_MAX`] is made durable itself. What's
//! waiting is counted while the commit's writer still holds the file, so the count follows the
//! order the commits were made in, and two writers can't both take the run past the cap.

use std::sync::{Condvar, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use super::*;

/// How long a data file's writes have to stop before it's sealed.
pub(super) const QUIET: Duration = Duration::from_secs(2);
/// The most a data file holds in batches not yet on the disk: what a write from the window may
/// have to write out with its own commit (a few background batches).
pub(super) const UNDURABLE_MAX: usize = 4 * crate::BACKGROUND_COMMIT;

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
    /// Durable and sealed: every commit once MailCraft is closing.
    const SEALED: Commit = Commit { durable: true, seal: true };
}

/// What each data file has had committed: what isn't on the disk yet, and what isn't sealed. Data
/// files are sealed by a thread of their own once quiet.
#[derive(Debug, Default)]
pub(super) struct Seals {
    state: Mutex<Waiting>,
    wake: Condvar,
    /// A seal by the thread ended.
    sealed: Condvar,
    /// How long each write from the window waited for the writer.
    #[cfg(test)]
    pub(super) waits: Mutex<Vec<Duration>>,
}

#[derive(Debug, Default)]
struct Waiting {
    /// Each account's data file (not kept open by this), and when it was last written.
    due: HashMap<String, (Weak<Database>, Instant)>,
    /// Bytes each account's data file has in batches not yet on the disk.
    undurable: HashMap<String, usize>,
    /// The account whose data file the thread is sealing now (it holds the file meanwhile).
    sealing: Option<String>,
    running: bool,
    stop: bool,
    /// MailCraft is closing ([`Packs::seal_all`]): every commit is sealed.
    closing: bool,
    /// The most any data file has had waiting, and the biggest batch.
    #[cfg(test)]
    peak: (usize, usize),
}

impl Seals {
    fn lock(&self) -> MutexGuard<'_, Waiting> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What `account`'s data file has waiting, as a commit about to be made changes it: a batch of
    /// `Some(bytes)` not waited for adds to it, a durable commit (`None`) writes it all out. Called
    /// with the file's writer held, so these come in the order of the commits; returns what was
    /// waiting before, to put back if the commit fails ([`Seals::restore`]).
    fn count(&self, account: &str, undurable: Option<usize>) -> usize {
        let mut w = self.lock();
        let before = w.undurable.get(account).copied().unwrap_or(0);
        match undurable {
            Some(n) => {
                let run = before.saturating_add(n);
                w.undurable.insert(account.to_string(), run);
                #[cfg(test)]
                {
                    w.peak = (w.peak.0.max(run), w.peak.1.max(n));
                }
            }
            None => {
                w.undurable.remove(account);
            }
        }
        before
    }

    /// A commit counted with [`Seals::count`] failed: what was waiting before it still is (its
    /// writer is still held, so nothing was counted since).
    fn restore(&self, account: &str, before: usize) {
        let mut w = self.lock();
        if before == 0 {
            w.undurable.remove(account);
        } else {
            w.undurable.insert(account.to_string(), before);
        }
    }

    /// Whether `bytes` more would take `account`'s run of batches not on the disk past the cap.
    /// Asked with the file's writer held, so no other batch comes in between.
    fn too_far(&self, account: &str, bytes: usize) -> bool {
        self.lock().undurable.get(account).copied().unwrap_or(0).saturating_add(bytes) > UNDURABLE_MAX
    }

    /// `account`'s data file had a commit that wasn't sealed: it's sealed once quiet.
    fn written(self: &Arc<Self>, account: &str, db: &Arc<Database>) {
        let mut w = self.lock();
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

    /// The thread: each data file sealed once it has been quiet for [`QUIET`], one at a time.
    fn run(&self) {
        let mut w = self.lock();
        while !w.stop {
            let now = Instant::now();
            let quiet = |at: &Instant| now.saturating_duration_since(*at) >= QUIET;
            let ready = w.due.iter().find(|(_, (_, at))| quiet(at)).map(|(a, _)| a.clone());
            let Some((account, db)) = ready.and_then(|a| w.due.remove(&a).map(|(db, _)| (a, db))) else {
                let next = w.due.values().map(|(_, at)| QUIET.saturating_sub(now.saturating_duration_since(*at))).min();
                w = match next {
                    Some(wait) => self.wake.wait_timeout(w, wait).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0),
                    None => self.wake.wait(w).unwrap_or_else(PoisonError::into_inner),
                };
                continue;
            };
            w.sealing = Some(account.clone());
            drop(w);
            // A file let go of since closed itself, sealed. This thread lets go of the file before
            // anyone waiting for the seal to end ([`Seals::forget`]) hears of it.
            let done = db.upgrade().map(|db| seal(&db, self, &account));
            w = self.lock();
            if let Some(Err(e)) = done {
                log::warn!("{account}'s data file couldn't be sealed: {e}");
            }
            w.sealing = None;
            self.sealed.notify_all();
        }
        w.running = false;
    }

    /// `account`'s data file is being let go of (closed, to be opened again or deleted): no seal
    /// waits for it any more, and one under way has ended, so nothing here holds it open.
    pub(super) fn forget(&self, account: &str) {
        let mut w = self.lock();
        w.due.remove(account);
        w.undurable.remove(account);
        while w.sealing.as_deref() == Some(account) {
            w = self.sealed.wait(w).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// As if the thread were sealing `account`'s data file (`None`: it has ended).
    #[cfg(test)]
    pub(super) fn pretend_sealing(&self, account: Option<&str>) {
        self.lock().sealing = account.map(str::to_string);
        self.sealed.notify_all();
    }

    /// Whether `account`'s data file waits for a seal, or has batches not on the disk.
    #[cfg(test)]
    pub(super) fn waiting_for(&self, account: &str) -> bool {
        let w = self.lock();
        w.due.contains_key(account) || w.undurable.contains_key(account)
    }

    /// What `account`'s data file has waiting now, the most any file has had, and the biggest
    /// batch any had.
    #[cfg(test)]
    pub(super) fn undurable(&self, account: &str) -> (usize, usize, usize) {
        let w = self.lock();
        (w.undurable.get(account).copied().unwrap_or(0), w.peak.0, w.peak.1)
    }

    /// No more sealing (the data files are closing, which seals them).
    pub(super) fn stop(&self) {
        self.lock().stop = true;
        self.wake.notify_all();
    }
}

/// An empty sealed commit of `account`'s data file (durable: it also writes out the batches
/// before it).
fn seal(db: &Database, seals: &Seals, account: &str) -> io::Result<()> {
    let mut tx = db.begin_write().map_err(other)?;
    tx.set_quick_repair(true);
    let before = seals.count(account, None);
    tx.commit().map_err(|e| {
        seals.restore(account, before);
        other(e)
    })
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
        // Writes from the window only (a background batch names its size).
        #[cfg(test)]
        if how.durable && undurable == 0 {
            self.seals.waits.lock().unwrap_or_else(PoisonError::into_inner).push(start.elapsed());
        }
        // Asked once this writer has the file: whatever began before the closing seal committed
        // before it, and whatever begins after is sealed itself. Likewise, no other batch can
        // come in between this one's look at the run waiting and its commit.
        let how = if self.seals.lock().closing {
            Commit::SEALED
        } else if !how.durable && self.seals.too_far(account, undurable) {
            Commit::PLAIN
        } else {
            how
        };
        if !how.durable {
            tx.set_durability(redb::Durability::None).map_err(other)?;
        }
        tx.set_quick_repair(how.seal);
        {
            let mut tables = Tables { files: tx.open_table(FILES).map_err(other)?, pieces: tx.open_table(PIECES).map_err(other)? };
            f(&mut tables)?;
        }
        let before = self.seals.count(account, (!how.durable).then_some(undurable));
        if let Err(e) = tx.commit() {
            self.seals.restore(account, before);
            return Err(other(e));
        }
        if !how.seal {
            self.seals.written(account, &db);
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
            let how = if durable { Commit::PLAIN } else { Commit::BATCH };
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
        seal(&db, &self.seals, &a).map_err(io_err(dir))
    }

    /// MailCraft is closing: every open data file sealed, and every commit from here on sealed
    /// too (a thread still writing as the process ends leaves its file reopening at once).
    pub(crate) fn seal_all(&self) -> Result<()> {
        {
            let mut w = self.seals.lock();
            w.closing = true;
            w.due.clear();
        }
        let open: Vec<(String, Arc<Database>)> = Packs::lock(&self.open).iter().map(|(a, db)| (a.clone(), db.clone())).collect();
        let mut done = Ok(());
        for (a, db) in open {
            if let Err(e) = seal(&db, &self.seals, &a) {
                log::warn!("{a}'s data file couldn't be sealed: {e}");
                if done.is_ok() {
                    done = Err(StoreError::Io { path: a, source: e });
                }
            }
        }
        done
    }
}

impl Drop for Packs {
    fn drop(&mut self) {
        self.seals.stop();
    }
}
