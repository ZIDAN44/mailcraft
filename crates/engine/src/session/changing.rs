//! The session: changing mail (flags, moves, deletions, undo).

use super::*;

impl Session {
    /// Apply a change: to the in-memory list at once, then to the cache and server (through a
    /// job), or straight to the cache for local accounts.
    pub fn apply_op(&mut self, account: &str, op: Op) {
        if let Err(e) = self.apply_op_checked(account, op) {
            self.notice(e, true);
        }
    }

    /// [`Session::apply_op`], saying when a local move didn't happen (the mail stays where it
    /// was) rather than putting it in a notice: a move command fails with it.
    pub(super) fn apply_op_checked(&mut self, account: &str, op: Op) -> Result<(), String> {
        // Outbox entries aren't on a server: mail commands leave them alone.
        let op = match op {
            Op::SetFlag { keys, flag, on } => Op::SetFlag { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), flag, on },
            Op::SetKeyword { keys, keyword, on } => Op::SetKeyword { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), keyword, on },
            Op::Move { keys, to } => Op::Move { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), to },
            Op::Expunge { keys } => Op::Expunge { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect() },
            other => other,
        };
        if !op.is_folder_op() && !matches!(op, Op::EmptyFolder { .. }) && op.keys().is_empty() {
            return Ok(());
        }
        let local = self.account(account).is_none_or(AccountState::is_local);
        self.apply_op_memory(account, &op);
        let done = if local { self.apply_op_local(account, &op) } else { Ok(()) };
        // A flag or category is changed in the list as it stands, where that gives the same list.
        if !self.patch_list(&op) {
            self.refresh_list();
        }
        if !local {
            self.expect_echo(account, &op);
            self.jobs.push(Job::Apply { account: account.to_string(), op });
        }
        done
    }

    pub(super) fn apply_op_memory(&mut self, account: &str, op: &Op) {
        // Search reads no flags or categories as text: those keep what it read; mail leaving or
        // coming into a folder has the folder scanned again.
        match op {
            Op::Move { keys, to } => {
                let folders: HashSet<&str> = keys.iter().map(|k| k.folder.as_str()).chain([to.as_str()]).collect();
                for f in folders {
                    self.search_folder_changed(account, f);
                }
            }
            Op::Expunge { keys } => {
                let folders: HashSet<&str> = keys.iter().map(|k| k.folder.as_str()).collect();
                for f in folders {
                    self.search_folder_changed(account, f);
                }
            }
            Op::EmptyFolder { path } => self.search_folder_changed(account, path),
            Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } => {}
        }
        if op.is_folder_op() {
            self.apply_folder_op_memory(account, op);
            return;
        }
        if let Op::EmptyFolder { path } = op {
            if let Some(st) = self.cache.get_mut(&(account.to_string(), path.clone())) {
                st.messages.clear();
            }
            if let Some(f) = self.account_mut(account).and_then(|a| a.folders.iter_mut().find(|f| &f.path == path)) {
                f.unread = 0;
                f.total = 0;
            }
            self.selection.retain(|k| !(k.account == account && &k.folder == path));
            if self.reading.as_ref().is_some_and(|r| r.key.account == account && &r.key.folder == path) {
                self.reading = None;
            }
            return;
        }
        let mut unread_delta: HashMap<String, i64> = HashMap::new();
        self.mail_changed();
        for ((a, f), st) in self.cache.iter_mut() {
            if a != account {
                continue;
            }
            // A set: every message of a big folder may change at once.
            let mine: HashSet<u32> = op.keys().iter().filter(|k| &k.folder == f && k.validity == st.validity).map(|k| k.uid).collect();
            if mine.is_empty() {
                continue;
            }
            match op {
                Op::SetFlag { flag, on, .. } => {
                    for m in st.messages.iter_mut().filter(|m| mine.contains(&m.key.uid) && flag.get(&m.flags) != *on) {
                        if *flag == Flag::Seen {
                            *unread_delta.entry(f.clone()).or_default() += if *on { -1 } else { 1 };
                        }
                        flag.set(&mut SharedSummary::make_mut(m).flags, *on);
                    }
                }
                Op::SetKeyword { keyword, on, .. } => {
                    for m in st.messages.iter_mut().filter(|m| mine.contains(&m.key.uid) && m.flags.has(keyword) != *on) {
                        SharedSummary::make_mut(m).flags.set_keyword(keyword, *on);
                    }
                }
                Op::Move { to, .. } if to == f => {}
                Op::Move { .. } | Op::Expunge { .. } => {
                    let unread = st.messages.iter().filter(|m| mine.contains(&m.key.uid) && !m.flags.seen).count();
                    *unread_delta.entry(f.clone()).or_default() -= i64::try_from(unread).unwrap_or(0);
                    st.messages.retain(|m| !mine.contains(&m.key.uid));
                }
                Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } | Op::EmptyFolder { .. } => {}
            }
        }
        if let Some(a) = self.account_mut(account) {
            for (f, d) in unread_delta {
                if let Some(folder) = a.folders.iter_mut().find(|x| x.path == f) {
                    folder.unread = u32::try_from((i64::from(folder.unread) + d).max(0)).unwrap_or(0);
                }
            }
        }
    }

    /// Local accounts: change the cache directly, including filing moved messages in their
    /// destination with new UIDs.
    pub(super) fn apply_op_local(&mut self, account: &str, op: &Op) -> Result<(), String> {
        let Some(store) = self.store.clone() else { return Ok(()) };
        // The folders it touches (counted again after; all of them for a folder change).
        let mut touched: Vec<String> = op.keys().iter().map(|k| k.folder.clone()).collect();
        match op {
            Op::Move { to, .. } => touched.push(to.clone()),
            Op::EmptyFolder { path } => touched.push(path.clone()),
            _ => {}
        }
        touched.sort();
        touched.dedup();
        // A flag or category: the lists in memory have it already (`apply_op_memory`), so the
        // flags they have now are handed to the store's thread for marks with the folders'
        // counts, written there in one commit (`store/src/marking.rs`): not in the window's frame.
        // One not in memory is changed on disk.
        if matches!(op, Op::SetFlag { .. } | Op::SetKeyword { .. })
            && touched.iter().all(|f| self.cache.contains_key(&(account.to_string(), f.clone())))
        {
            let counts = self.count_local(account, Some(&touched));
            let lists: Vec<FolderMarks> = touched
                .iter()
                .filter_map(|f| {
                    let st = self.cache.get(&(account.to_string(), f.clone()))?;
                    let uids: HashSet<u32> = op.keys().iter().filter(|k| k.folder == *f && k.validity == st.validity).map(|k| k.uid).collect();
                    let flags = st.messages.iter().filter(|m| uids.contains(&m.key.uid)).map(|m| (m.key.uid, m.flags.clone())).collect();
                    Some(FolderMarks { folder: f.clone(), validity: st.validity, flags })
                })
                .collect();
            store.mark_later(account, lists, counts);
            return Ok(());
        }
        // Anything else reads the lists from disk: the account's marks handed over before are
        // there first.
        store.flush_marks_of(account, None);
        // A long move's write ([`Store::long_write`]) lasts until the lists and counts after it are
        // saved too: each of them is sealed.
        let _long = if let Op::Move { keys, to } = op {
            // Not filed in the destination: the messages stay where they were (taken from the
            // sources after all, they were lost), and the lists in memory are read again.
            match Self::file_moved(&store, account, keys, to) {
                Ok(long) => {
                    self.reload_folder(account, to);
                    long
                }
                Err(e) => {
                    for f in &touched {
                        self.reload_folder(account, f);
                    }
                    self.recount_local(account, Some(&touched));
                    return Err(format!("Your mail couldn't be moved, so it stays where it was: {e}"));
                }
            }
        } else {
            None
        };
        match store.apply_op(account, op) {
            Ok(changed) => {
                for f in changed {
                    self.reload_folder(account, &f);
                }
            }
            Err(e) => self.notice(e.to_string(), true),
        }
        if op.is_folder_op()
            && let Ok(mut folders) = store.load_folders(account)
        {
            sort_folders(&mut folders);
            if let Some(a) = self.account_mut(account) {
                a.folders = folders;
            }
        }
        self.recount_local(account, (!op.is_folder_op()).then_some(&touched));
        // The search index follows a message moved or deleted (no server tells it here). A
        // flag or category changes nothing it keeps.
        if op.is_folder_op() {
            self.want_index(account, None);
        } else if !matches!(op, Op::SetFlag { .. } | Op::SetKeyword { .. }) {
            for f in &touched {
                self.want_index(account, Some(f));
            }
        }
        Ok(())
    }

    /// A local move's first half: the messages filed in `to` with new numbers, their sources with
    /// them. Every list is read before anything is written, and any error but a source that
    /// isn't here stops it (a list read as empty was saved over the destination's mail).
    ///
    /// A move of more than a batch of sources is a long write ([`Store::long_write`], returned to
    /// last until the rest of the move is saved): its batches don't wait for the disk, and the
    /// last, which the destination's list counts on, is a sealed commit that waits for it, refused
    /// if the data file lost what didn't (opened again after a failed write) meanwhile. Each
    /// batch waiting for the disk unsealed, a crash part-way checked the whole file as it opened
    /// again (105 s for 37 GB); the batches before the last are lost to a crash, but no list names
    /// them yet.
    fn file_moved<'s>(store: &'s Store, account: &str, keys: &[MessageKey], to: &str) -> Result<Option<LongWrite<'s>>, mailcraft_store::StoreError> {
        let mut dest = store.load_folder(account, to)?;
        if dest.validity == 0 {
            dest.validity = 1;
        }
        // Each source folder read once, its messages found by number; the bodies written
        // together (a folder's every message may go at once: per message, all of this was
        // read again, and each body was a write of its own).
        let mut sources: HashMap<String, HashMap<u32, SharedSummary>> = HashMap::new();
        for k in keys.iter().filter(|k| k.folder != to) {
            if !sources.contains_key(&k.folder) {
                let st = store.load_folder(account, &k.folder)?;
                sources.insert(k.folder.clone(), st.messages.into_iter().map(|m| (m.key.uid, m)).collect());
            }
        }
        let mut uid = dest.free_uid(account, to)?;
        let mut bodies: Vec<(MessageKey, Vec<u8>)> = Vec::new();
        let mut pending = 0usize;
        let packs = store.packs(account);
        let prepare = |bodies: Vec<(MessageKey, Vec<u8>)>| bodies.into_iter().map(|(k, raw)| (k, Store::prepare(packs, raw))).collect();
        // The long write, and how often its data file had been let go of after a failed write
        // as it began.
        let mut long: Option<(LongWrite<'s>, u64)> = None;
        for k in keys.iter().filter(|k| k.folder != to) {
            let Some(m) = sources.get(&k.folder).and_then(|src| src.get(&k.uid)) else { continue };
            let mut moved = Summary::clone(m);
            moved.key = MessageKey {
                account: account.to_string(),
                folder: to.to_string(),
                validity: dest.validity,
                uid: FolderState::take_uid(&mut uid, account, to)?,
            };
            // One never downloaded moves without it.
            if let Some(raw) = store.load_body(k)? {
                // A full batch goes before this one, so the last batch is never empty.
                if !bodies.is_empty() && (bodies.len() >= 1000 || pending.saturating_add(raw.len()) > 16 << 20) {
                    if long.is_none() {
                        let began = store.long_write(account);
                        long = Some((began, store.data_file_reopened(account)));
                    }
                    store.write_prepared(prepare(std::mem::take(&mut bodies)), false)?;
                    pending = 0;
                }
                pending = pending.saturating_add(raw.len());
                bodies.push((moved.key.clone(), raw));
            }
            dest.messages.push(moved.into());
        }
        if !bodies.is_empty() {
            match &long {
                Some((_, reopened)) => store.write_batch(account, prepare(bodies), Vec::new(), *reopened)?,
                None => store.write_prepared(prepare(bodies), true)?,
            }
        }
        dest.uid_next = uid;
        sort_newest_first(&mut dest.messages);
        store.save_folder(account, to, &dest)?;
        Ok(long.map(|(l, _)| l))
    }

    /// A folder-list change, in memory: the account's folders, the open view, cached folder
    /// contents and Favorites follow a renamed folder; a deleted one disappears.
    pub(super) fn apply_folder_op_memory(&mut self, account: &str, op: &Op) {
        self.mail_changed();
        let delimiter = self.delimiter(account);
        if let Some(a) = self.account_mut(account) {
            match op {
                Op::CreateFolder { path } => {
                    if !a.folders.iter().any(|f| &f.path == path) {
                        // Made in another mailbox (a shared one), it's in that mailbox too.
                        let parent = delimiter.and_then(|d| path.rsplit_once(d)).map(|(p, _)| p);
                        let mailbox = a.folders.iter().find(|f| Some(f.path.as_str()) == parent).and_then(|f| f.mailbox.clone());
                        a.folders.push(Folder {
                            account: account.to_string(),
                            path: path.clone(),
                            delimiter,
                            role: FolderRole::Other,
                            selectable: true,
                            mailbox,
                            ..Default::default()
                        });
                    }
                }
                Op::RenameFolder { from, to } => {
                    for f in a.folders.iter_mut() {
                        if let Some(new) = mailcraft_model::rebase_path(&f.path, from, to, f.delimiter.or(delimiter)) {
                            f.path = new;
                        }
                    }
                }
                Op::DeleteFolder { path } => a.folders.retain(|f| !mailcraft_model::is_within(&f.path, path, f.delimiter.or(delimiter))),
                Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Move { .. } | Op::Expunge { .. } | Op::EmptyFolder { .. } => {}
            }
            sort_folders(&mut a.folders);
        }
        match op {
            Op::CreateFolder { .. } => {}
            Op::RenameFolder { from, to } => {
                self.cache.retain(|(acc, f), _| acc != account || !mailcraft_model::is_within(f, from, delimiter));
                if let View::Folder { account: va, folder } = &mut self.view
                    && va == account
                    && let Some(new) = mailcraft_model::rebase_path(folder, from, to, delimiter)
                {
                    *folder = new;
                }
                if let Some(favs) = self.prefs.favorites.as_mut() {
                    for fav in favs.iter_mut().filter(|f| f.account == account) {
                        if let Some(new) = mailcraft_model::rebase_path(&fav.folder, from, to, delimiter) {
                            fav.folder = new;
                        }
                    }
                }
            }
            Op::DeleteFolder { path } => {
                self.cache.retain(|(acc, f), _| acc != account || !mailcraft_model::is_within(f, path, delimiter));
                if let Some(favs) = self.prefs.favorites.as_mut() {
                    favs.retain(|f| f.account != account || !mailcraft_model::is_within(&f.folder, path, delimiter));
                }
                if matches!(&self.view, View::Folder { account: va, folder } if va == account && mailcraft_model::is_within(folder, path, delimiter))
                {
                    self.view = self.default_view();
                    self.reading = None;
                    self.selection.clear();
                    self.focus = None;
                }
            }
            Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Move { .. } | Op::Expunge { .. } | Op::EmptyFolder { .. } => {}
        }
        self.search_folders_changed(account);
    }
}
