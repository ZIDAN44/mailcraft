//! The store: accounts' data, data files (kept or online only), and removing them.

use std::sync::atomic::AtomicBool;

use mailcraft_model::job::progress::Work;

use crate::*;

impl Store {
    /// The data file an account is kept in, when the cache keeps them so.
    pub fn data_file(&self, account: &str) -> Option<PathBuf> {
        if self.is_online_only(account) {
            return None;
        }
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.file(&safe_segment(account)),
            _ => None,
        }
    }

    /// How an account's data file is doing: how much of it is kept mail, and what compacting it
    /// would give back. Only read, and quick (one walk of the file's list of what it keeps, not
    /// of the mail); `None` when the account has no data file.
    pub fn data_file_health(&self, account: &str) -> Result<Option<DataFileHealth>> {
        if self.is_online_only(account) {
            return Ok(None);
        }
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => {
                let a = safe_segment(account);
                x.health(&a).map_err(|source| StoreError::Io { path: a, source })
            }
            _ => Ok(None),
        }
    }

    /// An account's data file was just compacted: it's sealed, and what it has over its kept mail
    /// now is taken as the least it can have, so [`DataFileHealth::reclaimable`] counts from it.
    pub fn data_file_compacted(&self, account: &str) -> Result<()> {
        if self.is_online_only(account) {
            return Ok(());
        }
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => {
                let a = safe_segment(account);
                x.compacted(&a).map_err(|source| StoreError::Io { path: a, source })
            }
            _ => Ok(()),
        }
    }

    /// What an account's data file keeps raw that compresses (a build before 0.32 kept base64
    /// attachments so) compressed again, a few files at a time in the background: each batch is
    /// compressed away from the file, then written in a short transaction of its own, so a write
    /// from the window waits at most for one. `cancel` stops it between batches; the next call goes
    /// on from where it got to. `progress` hears how far it has come after each batch. The file
    /// gets smaller only once compacted (the space freed is used again meanwhile). `None` when the
    /// account has no data file, or is removed while it runs (its file isn't made again).
    pub fn recompress_data_file(&self, account: &str, cancel: &AtomicBool, progress: &mut dyn FnMut(Work)) -> Result<Option<Recompressed>> {
        if self.is_online_only(account) {
            return Ok(None);
        }
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => {
                let a = safe_segment(account);
                // A removal waits for the batch being written, and the run stops after it.
                let seen = self.removals(account);
                let kept = |write: &mut dyn FnMut() -> io::Result<()>| self.while_kept(account, seen, write);
                match x.recompress(&a, cancel, progress, &kept) {
                    Err(_) if self.removals(account) != seen => Ok(None),
                    done => done.map_err(|source| StoreError::Io { path: a, source }),
                }
            }
            _ => {
                let _ = (cancel, progress);
                Ok(None)
            }
        }
    }

    /// The folder the data files are in, when the cache keeps them so.
    pub fn data_files_dir(&self) -> Option<PathBuf> {
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => Some(x.dir().to_path_buf()),
            _ => None,
        }
    }

    /// Keep an account online only (issue #4): its mail, folder lists, changes waiting for the
    /// server, Outbox, calendars and contacts are held in memory and never written anywhere, so
    /// nothing of it is left on this computer once MailCraft closes. Turned on, what was kept on
    /// disk comes into memory (nothing is lost while MailCraft runs) and its data file goes;
    /// turned off, what's held is written out to disk again.
    pub fn set_online_only(&self, account: &str, on: bool) -> Result<()> {
        if self.is_online_only(account) == on {
            return Ok(());
        }
        let dir = self.account_dir(account);
        let (from, to) = if on { (&self.files.main, &self.files.held) } else { (&self.files.held, &self.files.main) };
        let mut items = Vec::new();
        for p in from.walk(&dir) {
            if let Ok(bytes) = from.read(&p) {
                items.push((p, bytes));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        to.write_many(&items)?;
        #[cfg(target_arch = "wasm32")]
        for (p, bytes) in &items {
            to.write(p, bytes)?;
        }
        {
            let mut online = self.files.online();
            if on {
                online.insert(safe_segment(account));
            } else {
                online.remove(&safe_segment(account));
            }
        }
        // Only once it's all on the other side: a failure above leaves it where it was. A data
        // file the system won't let go of yet (a sync writing to it) is switched all the same;
        // the next start, holding the account in memory from the outset, removes it.
        match from.remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => log::warn!("{}: left for the next start: {e}", dir.display()),
            _ => {}
        }
        Ok(())
    }

    /// Whether an account is kept online only.
    pub fn is_online_only(&self, account: &str) -> bool {
        self.files.online().contains(&safe_segment(account))
    }

    /// Drop an online-only account's held files; `false` when it isn't one.
    pub(super) fn forget_online(&self, account: &str) -> bool {
        if !self.files.online().remove(&safe_segment(account)) {
            return false;
        }
        let _ = self.files.held.remove_dir_all(&self.account_dir(account));
        true
    }

    /// Where an account's mail is kept (to show the user): its data file, or its folder. Before
    /// anything is written, the folder its data file goes in (its own folder never exists then).
    pub fn account_path(&self, account: &str) -> PathBuf {
        self.data_file(account).or_else(|| self.data_files_dir()).unwrap_or_else(|| self.account_dir(account))
    }

    pub(super) fn account_dir(&self, account: &str) -> PathBuf {
        self.root.join(safe_segment(account))
    }

    /// Fill an account from an existing MailCraft data file (`.mailcraft`): its folders, mail,
    /// calendar and contacts, as they were. The file is only read (from a copy). Messages are
    /// renumbered as this account's; returns how many there are.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn import_data_file(&self, account: &str, path: &Path) -> Result<usize> {
        self.import_data_file_until(account, path, &|| false)
    }

    /// [`Store::import_data_file`], asking `stop` as the file is copied to be read and before each
    /// batch: once it says to, nothing more is read, the copy goes, and it ends in an error
    /// ("interrupted"). What came in before is left as it was, not yet this account's (a part of
    /// a data file, in no order a folder list keeps): the caller removes it. The copy it's read
    /// from is made beside the account's data file (in the data files' folder before it has one),
    /// so the room a copy in takes, the copy read from and the data file it fills, is all on the
    /// drive the data files are on.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn import_data_file_until(&self, account: &str, path: &Path, stop: &dyn Fn() -> bool) -> Result<usize> {
        let dir = self.account_dir(account);
        let beside = self.data_file(account).and_then(|f| f.parent().map(Path::to_path_buf)).or_else(|| self.data_files_dir());
        let tmp = beside.unwrap_or_else(|| self.root.clone()).join(format!(".import-{}.tmp", safe_segment(account)));
        let packs = self.packs(account);
        let read = packed::read_data_file(path, &tmp, stop, &mut |batch| {
            if stop() {
                return Err(StoreError::Io { path: path.display().to_string(), source: io::Error::new(io::ErrorKind::Interrupted, "stopped") });
            }
            // Paths come from a file: kept only when they stay inside the account. The old
            // account's changes still to make on its server, and its unsent mail, stay behind.
            // Compressed here, away from the writer, and written as a background import writes:
            // in short batches that don't wait for the disk (the seal below makes them durable).
            let mut items: Vec<(PathBuf, Prepared)> = Vec::new();
            let mut bytes = 0usize;
            for (k, v) in batch {
                // Nor the note an import into it kept of how far it got: this account's import
                // is a copy, not that one going on.
                if k == "ops.json" || k == "import.json" || k.starts_with("outbox/") {
                    continue;
                }
                let Some(rel) = inner_path(&k) else { continue };
                let prepared = Store::prepare(packs, v);
                if bytes.saturating_add(prepared.kept_len()) > crate::BACKGROUND_COMMIT && !items.is_empty() {
                    self.files.write_prepared(std::mem::take(&mut items), false)?;
                    bytes = 0;
                }
                bytes = bytes.saturating_add(prepared.kept_len());
                items.push((dir.join(rel), prepared));
            }
            if items.is_empty() { Ok(()) } else { self.files.write_prepared(items, false) }
        });
        let sealed = self.seal(account);
        read?;
        sealed?;
        // Folders and messages name the account they belong to: this one now.
        let mut folders = self.load_folders(account)?;
        if folders.iter().any(|f| f.account != account) {
            for f in &mut folders {
                f.account = account.to_string();
            }
            self.save_folders(account, &folders)?;
        }
        let mut messages = 0;
        for f in folders {
            let mut st = self.load_folder(account, &f.path)?;
            messages += st.messages.len();
            if st.messages.iter().any(|m| m.key.account != account) {
                for m in &mut st.messages {
                    m.key.account = account.to_string();
                }
                self.save_folder(account, &f.path, &st)?;
            }
        }
        Ok(messages)
    }

    /// Bytes an account's cache takes on disk (message sources, summaries, the Outbox).
    pub fn account_size(&self, account: &str) -> u64 {
        self.files.size(&self.account_dir(account))
    }

    /// An account removed, as Outlook removes one: its data file stays where it is, closed and
    /// no one's (returned, when there is one), and what's kept outside data files goes.
    pub fn remove_account(&self, account: &str) -> Result<Option<PathBuf>> {
        let _gone = self.removing(account);
        // Its search index, being written in the background, isn't written back after.
        self.search_gone(account);
        if self.forget_online(account) {
            return Ok(None);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Files::Packed(x) = &self.files.main {
            let kept = x.let_go(&safe_segment(account));
            // A folder from before data files, if one is still there.
            let _ = fs::remove_dir_all(self.account_dir(account));
            return Ok(kept);
        }
        self.delete_files(account).map(|()| None)
    }

    /// Forget everything about an account, its data file too (one that never held anyone's mail:
    /// a file that failed to open, an account starting again).
    pub fn delete_account(&self, account: &str) -> Result<()> {
        let _gone = self.removing(account);
        self.delete_files(account)
    }

    /// [`Store::delete_account`] once it's counted as removed.
    fn delete_files(&self, account: &str) -> Result<()> {
        self.search_gone(account);
        if self.forget_online(account) {
            return Ok(());
        }
        let dir = self.account_dir(account);
        match self.files.remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(&dir)(e)),
            _ => Ok(()),
        }
    }

    /// How many times `account` has been removed (or deleted): taken when a change for it is
    /// handed to another thread, to make it there with [`Store::while_kept`].
    pub fn removals(&self, account: &str) -> u64 {
        self.removals.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(account).copied().unwrap_or(0)
    }

    /// `write` run unless `account` was removed since [`Store::removals`] said `seen`: a removal
    /// waits for one running, so what a change queued before it writes doesn't bring back the
    /// files of an account gone.
    pub fn while_kept<T>(&self, account: &str, seen: u64, write: impl FnOnce() -> T) -> Option<T> {
        let removals = self.removals.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        if removals.get(account).copied().unwrap_or(0) != seen {
            return None;
        }
        Some(write())
    }

    /// `account` counted as removed, held until its files are gone.
    fn removing(&self, account: &str) -> std::sync::RwLockWriteGuard<'_, HashMap<String, u64>> {
        let mut removals = self.removals.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let n = removals.entry(account.to_string()).or_insert(0);
        *n = n.saturating_add(1);
        removals
    }
}

/// What compressing a data file again did ([`Store::recompress_data_file`]), in this run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recompressed {
    /// Files compressed again: how many, and their bytes as they were kept and as they are now.
    pub files: u64,
    pub before: u64,
    pub after: u64,
    /// Files tried that stayed as they were (they don't compress: pictures, archives), and their
    /// bytes.
    pub stayed_raw: u64,
    pub stayed_raw_bytes: u64,
    /// Files written again while they were being compressed: left as written.
    pub changed: u64,
    /// Files kept raw passed over, and their bytes as kept: damaged ones, and those bigger than a
    /// run takes (64 MiB). The data file says what every run passed over (`recompress.passed`).
    pub passed_over: u64,
    pub passed_over_bytes: u64,
    /// What every run of it has saved, as the data file records it.
    pub saved_in_all: u64,
    /// Everything was looked at: the data file says it's kept as this build keeps it (what's left
    /// raw doesn't compress, or was passed over). Otherwise it was stopped, and the next run goes
    /// on from where it got to.
    pub finished: bool,
}
