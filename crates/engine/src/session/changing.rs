//! The session: changing mail (flags, moves, deletions, undo).

use super::*;

impl Session {
    /// Apply a change: to the in-memory list at once, then to the cache and server (through a
    /// job), or straight to the cache for local accounts.
    pub fn apply_op(&mut self, account: &str, op: Op) {
        // Outbox entries aren't on a server: mail commands leave them alone.
        let op = match op {
            Op::SetFlag { keys, flag, on } => Op::SetFlag { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), flag, on },
            Op::SetKeyword { keys, keyword, on } => Op::SetKeyword { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), keyword, on },
            Op::Move { keys, to } => Op::Move { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), to },
            Op::Expunge { keys } => Op::Expunge { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect() },
            other => other,
        };
        if !op.is_folder_op() && !matches!(op, Op::EmptyFolder { .. }) && op.keys().is_empty() {
            return;
        }
        let local = self.account(account).is_none_or(AccountState::is_local);
        self.apply_op_memory(account, &op);
        if local {
            self.apply_op_local(account, &op);
        } else {
            self.jobs.push(Job::Apply { account: account.to_string(), op });
        }
        self.index = None;
        self.refresh_list();
    }

    pub(super) fn apply_op_memory(&mut self, account: &str, op: &Op) {
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
                    for m in st.messages.iter_mut().filter(|m| mine.contains(&m.key.uid)) {
                        if *flag == Flag::Seen && m.flags.seen != *on {
                            *unread_delta.entry(f.clone()).or_default() += if *on { -1 } else { 1 };
                        }
                        flag.set(&mut m.flags, *on);
                    }
                }
                Op::SetKeyword { keyword, on, .. } => {
                    for m in st.messages.iter_mut().filter(|m| mine.contains(&m.key.uid)) {
                        m.flags.set_keyword(keyword, *on);
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
    pub(super) fn apply_op_local(&mut self, account: &str, op: &Op) {
        let Some(store) = self.store.clone() else { return };
        // The folders it touches (counted again after; all of them for a folder change).
        let mut touched: Vec<String> = op.keys().iter().map(|k| k.folder.clone()).collect();
        match op {
            Op::Move { to, .. } => touched.push(to.clone()),
            Op::EmptyFolder { path } => touched.push(path.clone()),
            _ => {}
        }
        touched.sort();
        touched.dedup();
        // A flag or category: the lists in memory have it already (`apply_op_memory`), so they're
        // saved as they are (a big folder's changed page only), not read from disk, changed and
        // read again. One not in memory is changed on disk.
        if matches!(op, Op::SetFlag { .. } | Op::SetKeyword { .. })
            && touched.iter().all(|f| self.cache.contains_key(&(account.to_string(), f.clone())))
        {
            let changed: HashSet<u32> = op.keys().iter().map(|k| k.uid).collect();
            for f in &touched {
                if let Some(st) = self.cache.get(&(account.to_string(), f.clone()))
                    && let Err(e) = store.save_folder_changed(account, f, st, &changed)
                {
                    self.notice(e.to_string(), true);
                }
            }
            self.recount_local(account, Some(&touched));
            return;
        }
        if let Op::Move { keys, to } = op {
            let mut dest = store.load_folder(account, to).unwrap_or_default();
            if dest.validity == 0 {
                dest.validity = 1;
            }
            // Each source folder read once, its messages found by number; the bodies written
            // together (a folder's every message may go at once: per message, all of this was
            // read again, and each body was a write of its own).
            let mut sources: HashMap<String, HashMap<u32, Summary>> = HashMap::new();
            let mut uid = dest.uid_next.max(dest.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1);
            let mut bodies: Vec<(MessageKey, Vec<u8>)> = Vec::new();
            let mut pending = 0usize;
            for k in keys {
                if k.folder == *to {
                    continue;
                }
                let src = sources.entry(k.folder.clone()).or_insert_with(|| {
                    store.load_folder(account, &k.folder).map(|st| st.messages.into_iter().map(|m| (m.key.uid, m)).collect()).unwrap_or_default()
                });
                let Some(m) = src.get(&k.uid) else { continue };
                let mut moved = m.clone();
                moved.key = MessageKey { account: account.to_string(), folder: to.clone(), validity: dest.validity, uid };
                uid += 1;
                if let Some(raw) = store.read_body(k) {
                    pending += raw.len();
                    bodies.push((moved.key.clone(), raw));
                }
                if bodies.len() >= 1000 || pending >= 16 << 20 {
                    let _ = store.write_bodies(&std::mem::take(&mut bodies));
                    pending = 0;
                }
                dest.messages.push(moved);
            }
            if !bodies.is_empty() {
                let _ = store.write_bodies(&bodies);
            }
            dest.uid_next = uid;
            sort_newest_first(&mut dest.messages);
            if let Err(e) = store.save_folder(account, to, &dest) {
                self.notice(e.to_string(), true);
            }
            self.reload_folder(account, to);
        }
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
    }

    /// A folder-list change, in memory: the account's folders, the open view, cached folder
    /// contents and Favorites follow a renamed folder; a deleted one disappears.
    pub(super) fn apply_folder_op_memory(&mut self, account: &str, op: &Op) {
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
        self.index = None;
    }
}
