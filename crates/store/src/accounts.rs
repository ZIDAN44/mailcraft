//! The store: accounts' data, data files (kept or online only), and removing them.

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
        let dir = self.account_dir(account);
        let tmp = self.root.join(format!(".import-{}.tmp", safe_segment(account)));
        packed::read_data_file(path, &tmp, &mut |batch| {
            // Paths come from a file: kept only when they stay inside the account. The old
            // account's changes still to make on its server, and its unsent mail, stay behind.
            let items: Vec<(PathBuf, Vec<u8>)> = batch
                .into_iter()
                .filter(|(k, _)| k != "ops.json" && !k.starts_with("outbox/"))
                .filter_map(|(k, v)| inner_path(&k).map(|rel| (dir.join(rel), v)))
                .collect();
            self.files.write_many(&items)
        })?;
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
        self.delete_account(account).map(|()| None)
    }

    /// Forget everything about an account, its data file too (one that never held anyone's mail:
    /// a file that failed to open, an account starting again).
    pub fn delete_account(&self, account: &str) -> Result<()> {
        if self.forget_online(account) {
            return Ok(());
        }
        let dir = self.account_dir(account);
        match self.files.remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(&dir)(e)),
            _ => Ok(()),
        }
    }
}
