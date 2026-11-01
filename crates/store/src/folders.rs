//! The store: folders' lists of messages, and changes applied to them.

use crate::*;

impl Store {
    pub fn load_folder(&self, account: &str, folder: &str) -> Result<FolderState> {
        let dir = self.folder_dir(account, folder);
        let file: StateFile = self.read_json(&dir.join("state.json"))?;
        let mut st = FolderState {
            validity: file.validity,
            uid_next: file.uid_next,
            highest_modseq: file.highest_modseq,
            messages: file.messages,
            window: file.window,
        };
        // A big folder's list in pages (by number): put together, newest first again.
        if !file.pages.is_empty() {
            for i in 0..file.pages.len() {
                let page: Vec<Summary> = self.read_json(&dir.join(format!("state-{i}.json")))?;
                st.messages.extend(page);
            }
            sort_newest_first(&mut st.messages);
        }
        // Keys are rebuilt rather than trusted: a renamed or copied cache stays consistent.
        for m in &mut st.messages {
            m.key.account = account.to_string();
            m.key.folder = folder.to_string();
            m.key.validity = st.validity;
        }
        Ok(st)
    }

    /// Keep a folder's list. A big one goes in pages of [`PAGE_MESSAGES`] by number, and only
    /// the pages that changed are written: one message marked read in a folder of hundreds of
    /// thousands rewrote all of it (hundreds of megabytes, held in memory twice over).
    pub fn save_folder(&self, account: &str, folder: &str, state: &FolderState) -> Result<()> {
        self.save_folder_counted(account, folder, state, None).map(drop)
    }

    /// [`Store::save_folder`] after only these messages' flags changed (the same messages, in
    /// number): only their pages are made again (a big folder's other pages aren't even turned
    /// into text to see they're the same).
    pub fn save_folder_changed(&self, account: &str, folder: &str, state: &FolderState, changed: &std::collections::HashSet<u32>) -> Result<()> {
        self.save_folder_counted(account, folder, state, Some(changed)).map(drop)
    }

    /// [`Store::save_folder`], saying how many files it wrote (the list and the pages that
    /// changed).
    pub(crate) fn save_folder_counted(
        &self,
        account: &str,
        folder: &str,
        state: &FolderState,
        changed: Option<&std::collections::HashSet<u32>>,
    ) -> Result<usize> {
        let dir = self.folder_dir(account, folder);
        let head = dir.join("state.json");
        let json_err = |p: &Path| {
            let p = p.display().to_string();
            move |source| StoreError::Json { path: p, source }
        };
        let old: StateFile = self.read_json(&head).unwrap_or_default();
        let mut file = StateFile {
            validity: state.validity,
            uid_next: state.uid_next,
            highest_modseq: state.highest_modseq,
            messages: Vec::new(),
            window: state.window,
            pages: Vec::new(),
            count: 0,
        };
        let mut writes: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        if state.messages.len() <= PAGE_MESSAGES {
            file.messages = state.messages.clone();
        } else {
            let mut by_number: Vec<&Summary> = state.messages.iter().collect();
            by_number.sort_by_key(|m| m.key.uid);
            file.count = by_number.len();
            // The same messages as last time, some flags changed: the other pages stand.
            let same = changed.filter(|_| old.count == by_number.len() && old.pages.len() == by_number.len().div_ceil(PAGE_MESSAGES));
            for (i, page) in by_number.chunks(PAGE_MESSAGES).enumerate() {
                if let Some(changed) = same
                    && let Some(&hash) = old.pages.get(i)
                    && !page.iter().any(|m| changed.contains(&m.key.uid))
                {
                    file.pages.push(hash);
                    continue;
                }
                let path = dir.join(format!("state-{i}.json"));
                let bytes = serde_json::to_vec(page).map_err(json_err(&path))?;
                let hash = page_hash(&bytes);
                if old.pages.get(i) != Some(&hash) || !self.files.is_file(&path) {
                    writes.push((path, bytes));
                }
                file.pages.push(hash);
            }
        }
        writes.push((head.clone(), serde_json::to_vec(&file).map_err(json_err(&head))?));
        // The list and its pages together (one transaction in a data file).
        let written = writes.len();
        self.write_files(writes)?;
        // Pages the list no longer has.
        for i in file.pages.len()..old.pages.len() {
            let _ = self.files.remove_file(&dir.join(format!("state-{i}.json")));
        }
        Ok(written)
    }

    /// Write a folder's list a page at a time (importing a data file, the whole list needn't be
    /// held): messages are handed over in rising number order, then [`ListWriter::finish`]. The
    /// pages come out as [`Store::save_folder`] makes them.
    pub fn list_writer(&self, account: &str, folder: &str) -> ListWriter {
        ListWriter { account: account.to_string(), folder: folder.to_string(), pages: Vec::new(), page: Vec::new(), total: 0, unread: 0 }
    }

    /// Several files at once (one transaction in a data file).
    pub(super) fn write_files(&self, items: Vec<(PathBuf, Vec<u8>)>) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.files.write_many(items)
        }
        #[cfg(target_arch = "wasm32")]
        {
            for (p, bytes) in &items {
                self.files.write(p, bytes)?;
            }
            Ok(())
        }
    }

    /// Forget a folder's cache (UIDVALIDITY changed, or the folder was deleted on the server).
    pub fn clear_folder(&self, account: &str, folder: &str) -> Result<()> {
        let dir = self.folder_dir(account, folder);
        match self.files.remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(&dir)(e)),
            _ => Ok(()),
        }
    }

    /// Apply a change to the cached folders at once (optimistic update). Returns the folders that
    /// changed. Moves only remove from the source: the destination learns the new UID at its
    /// next sync.
    pub fn apply_op(&self, account: &str, op: &Op) -> Result<Vec<String>> {
        if op.is_folder_op() {
            return self.apply_folder_op(account, op);
        }
        if let Op::EmptyFolder { path } = op {
            let mut st = self.load_folder(account, path)?;
            // The list first: one that can't be saved still has its messages' sources.
            let gone = std::mem::take(&mut st.messages);
            self.save_folder(account, path, &st)?;
            for m in &gone {
                self.remove_body(&m.key);
            }
            let mut list = self.load_folders(account)?;
            for f in list.iter_mut().filter(|f| &f.path == path) {
                f.total = 0;
                f.unread = 0;
            }
            self.save_folders(account, &list)?;
            return Ok(vec![path.clone()]);
        }
        let mut folders: Vec<String> = op.keys().iter().filter(|k| k.account == account).map(|k| k.folder.clone()).collect();
        folders.sort();
        folders.dedup();
        for folder in &folders {
            let mut st = self.load_folder(account, folder)?;
            let mut sources_gone: Vec<&MessageKey> = Vec::new();
            let in_folder = |k: &MessageKey| k.account == account && &k.folder == folder && k.validity == st.validity;
            match op {
                Op::SetFlag { keys, flag, on } => {
                    // A set: every message of a folder may change at once (looked up in a list, that was quadratic).
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        flag.set(&mut m.flags, *on);
                    }
                }
                Op::SetKeyword { keys, keyword, on } => {
                    // A set: every message of a folder may change at once (looked up in a list, that was quadratic).
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        m.flags.set_keyword(keyword, *on);
                    }
                }
                Op::Move { keys, to } => {
                    if to == folder {
                        continue;
                    }
                    // A set: every message of a folder may change at once (looked up in a list, that was quadratic).
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    st.messages.retain(|m| !uids.contains(&m.key.uid));
                }
                Op::Expunge { keys } => {
                    sources_gone = keys.iter().filter(|k| in_folder(k)).collect();
                    let uids: HashSet<u32> = sources_gone.iter().map(|k| k.uid).collect();
                    st.messages.retain(|m| !uids.contains(&m.key.uid));
                }
                Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } | Op::EmptyFolder { .. } => {}
            }
            self.save_folder(account, folder, &st)?;
            // Once the list no longer has them: one that can't be saved keeps their sources.
            for k in sources_gone {
                self.remove_body(k);
            }
        }
        Ok(folders)
    }

    /// Folder-list changes: `folders.json` is updated and each folder's cache moves with it (or
    /// goes, for a deleted folder). Returns the folders whose contents should be shown afresh.
    pub(super) fn apply_folder_op(&self, account: &str, op: &Op) -> Result<Vec<String>> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut list = self.load_folders(account)?;
        let delimiter = list.iter().find_map(|f| f.delimiter);
        let changed = match op {
            Op::CreateFolder { path } => {
                if !list.iter().any(|f| &f.path == path) {
                    // Made in another mailbox (a shared one), it's in that mailbox too.
                    let parent = delimiter.and_then(|d| path.rsplit_once(d)).map(|(p, _)| p);
                    let mailbox = list.iter().find(|f| Some(f.path.as_str()) == parent).and_then(|f| f.mailbox.clone());
                    list.push(Folder {
                        account: account.to_string(),
                        path: path.clone(),
                        delimiter,
                        role: mailcraft_model::FolderRole::Other,
                        selectable: true,
                        mailbox,
                        ..Default::default()
                    });
                }
                vec![path.clone()]
            }
            Op::RenameFolder { from, to } => {
                let mut moved = Vec::new();
                for f in list.iter_mut() {
                    if let Some(new) = mailcraft_model::rebase_path(&f.path, from, to, f.delimiter.or(delimiter)) {
                        let (src, dst) = (self.folder_dir(account, &f.path), self.folder_dir(account, &new));
                        if self.files.exists(&src) {
                            let _ = self.files.remove_dir_all(&dst);
                            self.files.rename(&src, &dst).map_err(io_err(&src))?;
                        }
                        f.path = new.clone();
                        moved.push(new);
                    }
                }
                moved
            }
            Op::DeleteFolder { path } => {
                let gone: Vec<String> =
                    list.iter().filter(|f| mailcraft_model::is_within(&f.path, path, f.delimiter.or(delimiter))).map(|f| f.path.clone()).collect();
                for g in &gone {
                    self.clear_folder(account, g)?;
                }
                list.retain(|f| !gone.contains(&f.path));
                Vec::new()
            }
            Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Move { .. } | Op::Expunge { .. } | Op::EmptyFolder { .. } => Vec::new(),
        };
        mailcraft_model::sort_folders(&mut list);
        self.write_folders(account, &list)?;
        Ok(changed)
    }

    /// Add `folder` to the account's list, or, when the list has a folder at its path, set only
    /// that one's counts: an import adds each folder as it finishes, and what the window changed
    /// meanwhile (a folder renamed, another's counts) stays. Nothing is ever taken out.
    pub fn upsert_folder(&self, account: &str, folder: &Folder) -> Result<()> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut list = self.load_folders(account)?;
        match list.iter_mut().find(|f| f.path == folder.path) {
            Some(f) => {
                f.unread = folder.unread;
                f.total = folder.total;
            }
            None => list.push(Folder { account: account.to_string(), ..folder.clone() }),
        }
        self.write_folders(account, &list)
    }

    /// Set the counts (`path`, unread, total) of the account's folders that are in its list, as
    /// the list stands now: a list held elsewhere (the window's) written whole would drop the
    /// folders an import added since it was read. Says how many of them the list has.
    pub fn update_folder_counts(&self, account: &str, counts: &[(String, u32, u32)]) -> Result<usize> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut list = self.load_folders(account)?;
        let (mut found, mut changed) = (0, false);
        for (path, unread, total) in counts {
            if let Some(f) = list.iter_mut().find(|f| f.path == *path) {
                found += 1;
                changed |= f.unread != *unread || f.total != *total;
                f.unread = *unread;
                f.total = *total;
            }
        }
        if changed {
            self.write_folders(account, &list)?;
        }
        Ok(found)
    }
}
