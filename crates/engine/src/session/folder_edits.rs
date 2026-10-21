//! The session: making, renaming, moving and deleting folders.

use super::*;

impl Session {
    /// The account's hierarchy delimiter (from any folder that has one).
    pub(super) fn delimiter(&self, account: &str) -> Option<char> {
        self.account(account).and_then(|a| a.folders.iter().find_map(|f| f.delimiter))
    }

    /// Where top-level folders go: the root, or `INBOX` on servers that keep every folder under
    /// it (`INBOX.Sent`, `INBOX.Projects`).
    pub(super) fn top_parent(&self, account: &str) -> Option<String> {
        let a = self.account(account)?;
        let d = self.delimiter(account)?;
        let others: Vec<&Folder> = a.folders.iter().filter(|f| !f.path.eq_ignore_ascii_case("INBOX")).collect();
        let inbox = a.folders.iter().find(|f| f.path.eq_ignore_ascii_case("INBOX"))?;
        (!others.is_empty() && others.iter().all(|f| f.path.starts_with(&format!("{}{d}", inbox.path)))).then(|| inbox.path.clone())
    }

    pub(super) fn parent_of(path: &str, delimiter: Option<char>) -> Option<&str> {
        delimiter.and_then(|d| path.rsplit_once(d)).map(|(p, _)| p)
    }

    /// A user folder (not Inbox or another special folder) that exists, for changing.
    pub(super) fn user_folder(&self, account: &str, path: &str) -> Result<Folder, CmdError> {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        let f = self.folder(account, path).cloned().ok_or_else(|| CmdError::Params(format!("no folder `{path}`")))?;
        if f.is_fixed() {
            return Err(CmdError::Failed(format!("{} can't be renamed, moved or deleted", f.display_name())));
        }
        Ok(f)
    }

    /// Create a folder called `name` inside `parent` (`None`: at the top).
    pub fn create_folder(&mut self, account: &str, parent: Option<&str>, name: &str) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if let Some(p) = parent
            && self.folder(account, p).is_none()
        {
            return Err(CmdError::Params(format!("no folder `{p}`")));
        }
        let parent = parent.map(str::to_string).or_else(|| self.top_parent(account));
        let path = mailcraft_model::child_path(parent.as_deref(), self.delimiter(account), name).map_err(CmdError::Params)?;
        if self.folder(account, &path).is_some() {
            return Err(CmdError::Params(format!("there is already a folder called {}", name.trim())));
        }
        self.apply_op(account, Op::CreateFolder { path: path.clone() });
        Ok(serde_json::json!({ "created": path }))
    }

    /// Rename a folder (it stays where it is).
    pub fn rename_folder(&mut self, account: &str, path: &str, name: &str) -> CmdResult {
        let f = self.user_folder(account, path)?;
        let d = f.delimiter.or(self.delimiter(account));
        let to = mailcraft_model::child_path(Self::parent_of(path, d), d, name).map_err(CmdError::Params)?;
        if to == path {
            return Ok(serde_json::json!({ "renamed": to }));
        }
        if self.folder(account, &to).is_some() {
            return Err(CmdError::Params(format!("there is already a folder called {}", name.trim())));
        }
        self.apply_op(account, Op::RenameFolder { from: path.to_string(), to: to.clone() });
        Ok(serde_json::json!({ "renamed": to }))
    }

    /// Move a folder inside another (`None`: to the top).
    pub fn move_folder(&mut self, account: &str, path: &str, parent: Option<&str>) -> CmdResult {
        let f = self.user_folder(account, path)?;
        let d = f.delimiter.or(self.delimiter(account));
        let d = d.ok_or_else(|| CmdError::Failed("this server has no subfolders".into()))?;
        let parent = match parent {
            Some(p) => {
                if self.folder(account, p).is_none() {
                    return Err(CmdError::Params(format!("no folder `{p}`")));
                }
                if mailcraft_model::is_within(p, path, Some(d)) {
                    return Err(CmdError::Params("a folder can't be moved into itself".into()));
                }
                Some(p.to_string())
            }
            None => self.top_parent(account),
        };
        let leaf = path.rsplit(d).next().unwrap_or(path);
        let to = match &parent {
            Some(p) => format!("{p}{d}{leaf}"),
            None => leaf.to_string(),
        };
        if to == path {
            return Ok(serde_json::json!({ "moved": to }));
        }
        if self.folder(account, &to).is_some() {
            return Err(CmdError::Params(format!("there is already a folder called {} there", f.display_name())));
        }
        self.apply_op(account, Op::RenameFolder { from: path.to_string(), to: to.clone() });
        Ok(serde_json::json!({ "moved": to }))
    }

    /// Delete a folder: into Deleted Items, or for good when it is already there (or
    /// `permanent`, or there is no Deleted Items folder). Subfolders go with it.
    pub fn delete_folder(&mut self, account: &str, path: &str, permanent: bool) -> CmdResult {
        let f = self.user_folder(account, path)?;
        let d = f.delimiter.or(self.delimiter(account));
        let trash = self.account(account).and_then(|a| a.folder(FolderRole::Trash)).map(|t| t.path.clone());
        let in_trash = trash.as_deref().is_some_and(|t| mailcraft_model::is_within(path, t, d));
        if let (Some(trash), Some(d), false, false) = (&trash, d, in_trash, permanent) {
            // Like deleting mail: the folder waits in Deleted Items.
            let leaf = path.rsplit(d).next().unwrap_or(path);
            let mut to = format!("{trash}{d}{leaf}");
            let mut n = 2;
            while self.folder(account, &to).is_some() {
                to = format!("{trash}{d}{leaf} ({n})");
                n += 1;
            }
            self.apply_op(account, Op::RenameFolder { from: path.to_string(), to: to.clone() });
            return Ok(serde_json::json!({ "moved": to }));
        }
        // Deepest first: servers refuse to delete a folder that still has children.
        let mut doomed: Vec<String> = self
            .account(account)
            .map(|a| a.folders.iter().filter(|x| mailcraft_model::is_within(&x.path, path, d)).map(|x| x.path.clone()).collect())
            .unwrap_or_default();
        doomed.sort_by_key(|p| std::cmp::Reverse(p.len()));
        let n = doomed.len();
        for p in doomed {
            self.apply_op(account, Op::DeleteFolder { path: p });
        }
        Ok(serde_json::json!({ "deleted": n }))
    }

    /// Favorites as shown: the user's list, or the defaults.
    pub fn favorites(&self) -> Vec<FolderRef> {
        match &self.prefs.favorites {
            Some(list) => list.iter().filter(|r| self.folder(&r.account, &r.folder).is_some()).cloned().collect(),
            None if self.accounts.len() == 1 => self
                .accounts
                .first()
                .map(|a| {
                    [FolderRole::Inbox, FolderRole::Sent, FolderRole::Drafts]
                        .iter()
                        .filter_map(|r| a.folder(*r))
                        .map(|f| FolderRef { account: a.config.id.clone(), folder: f.path.clone() })
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// Add a folder to Favorites or take it off (`on: None` toggles).
    pub fn set_favorite(&mut self, account: &str, folder: &str, on: Option<bool>) -> CmdResult {
        if self.folder(account, folder).is_none() {
            return Err(CmdError::Params(format!("no folder `{folder}` in account `{account}`")));
        }
        let mut list = self.favorites();
        let r = FolderRef { account: account.to_string(), folder: folder.to_string() };
        let present = list.contains(&r);
        let on = on.unwrap_or(!present);
        if on && !present {
            list.push(r);
        } else if !on {
            list.retain(|x| *x != r);
        }
        self.prefs.favorites = Some(list);
        Ok(serde_json::json!({ "on": on }))
    }

    /// Count a local account's folders again (`only` these, or all of them).
    pub(super) fn recount_local(&mut self, account: &str, only: Option<&[String]>) {
        let paths: Vec<String> = match only {
            Some(list) => list.to_vec(),
            None => self.account(account).map(|a| a.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default(),
        };
        let mut counts = Vec::new();
        for p in paths {
            let st = self.folder_state(account, &p);
            counts.push((p, st.unread(), u32::try_from(st.messages.len()).unwrap_or(u32::MAX)));
        }
        let mut changed = false;
        if let Some(a) = self.account_mut(account) {
            for (p, unread, total) in counts {
                if let Some(f) = a.folders.iter_mut().find(|f| f.path == p)
                    && (f.unread != unread || f.total != total)
                {
                    f.unread = unread;
                    f.total = total;
                    changed = true;
                }
            }
        }
        // Local accounts have no server to report counts: keep them on disk.
        if changed
            && let (Some(store), Some(a)) = (&self.store, self.account(account))
            && let Err(e) = store.save_folders(account, &a.folders)
        {
            log::warn!("{account}: folder counts not saved: {e}");
        }
    }

    /// The message to show after `gone` leave the list: the next one down, else the one above.
    pub(super) fn successor(&self, gone: &[MessageKey]) -> Option<MessageKey> {
        let gone: HashSet<&MessageKey> = gone.iter().collect();
        let last = self.list.iter().rposition(|m| gone.contains(&m.key))?;
        self.list
            .iter()
            .skip(last + 1)
            .find(|m| !gone.contains(&m.key))
            .or_else(|| self.list.iter().take(last).rev().find(|m| !gone.contains(&m.key)))
            .map(|m| m.key.clone())
    }

    /// Move the selection to a folder of the same role in each message's account (Archive,
    /// Deleted Items, Junk), or to an explicit folder.
    pub fn move_selection(&mut self, target: MoveTarget) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let next = self.successor(&self.selection.clone());
        let mut by_account: HashMap<String, Vec<MessageKey>> = HashMap::new();
        for m in &sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
        }
        let mut moved = 0usize;
        let mut accounts: Vec<String> = by_account.keys().cloned().collect();
        accounts.sort();
        for a in accounts {
            let keys = by_account.remove(&a).unwrap_or_default();
            let dest = match &target {
                MoveTarget::Folder(f) => Some(f.clone()),
                MoveTarget::Role(r) => self.account(&a).and_then(|acc| acc.folder(*r)).map(|f| f.path.clone()),
            };
            let Some(dest) = dest else {
                let what = match target {
                    MoveTarget::Role(r) => r.label().unwrap_or("that folder"),
                    MoveTarget::Folder(_) => "that folder",
                };
                return Err(CmdError::Failed(format!("this account has no {what} folder")));
            };
            if self.folder(&a, &dest).is_none() {
                return Err(CmdError::Params(format!("no folder `{dest}`")));
            }
            let keys: Vec<MessageKey> = keys.into_iter().filter(|k| k.folder != dest).collect();
            moved += keys.len();
            if !keys.is_empty() {
                // Undo finds them again by Message-ID (those without one can't come back).
                let back: Vec<(String, String)> = keys
                    .iter()
                    .filter_map(|k| self.summary(k).filter(|m| !m.message_id.is_empty()).map(|m| (m.message_id.clone(), k.folder.clone())))
                    .collect();
                if !back.is_empty() {
                    self.remember(Undo::Move { account: a.clone(), to: dest.clone(), back });
                }
                self.apply_op(&a, Op::Move { keys, to: dest });
            }
        }
        self.after_removal(next);
        Ok(serde_json::json!({ "moved": moved }))
    }

    /// Delete permanently.
    pub fn expunge_selection(&mut self) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let next = self.successor(&self.selection.clone());
        let mut by_account: HashMap<String, Vec<MessageKey>> = HashMap::new();
        for m in &sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
        }
        let n = sel.len();
        for (a, keys) in by_account {
            self.apply_op(&a, Op::Expunge { keys });
        }
        self.after_removal(next);
        Ok(serde_json::json!({ "deleted": n }))
    }

    pub(super) fn after_removal(&mut self, next: Option<MessageKey>) {
        self.reading = None;
        self.selection.clear();
        self.focus = None;
        if let Some(k) = next
            && self.list.iter().any(|m| m.key == k)
        {
            // Show the next message, but don't mark it read just because something was deleted.
            self.show_message(&k, false);
        }
    }

    /// Set or clear a flag on the selection (`on: None` toggles, following the first message).
    pub fn flag_selection(&mut self, flag: Flag, on: Option<bool>) -> CmdResult {
        let sel = self.selected();
        let first = sel.first().ok_or_else(|| CmdError::Disabled("no message selected".into()))?;
        let current = flag.get(&first.flags);
        let on = on.unwrap_or(!current);
        let mut by_account: HashMap<String, Vec<MessageKey>> = HashMap::new();
        // Undo puts back only the ones that changed.
        let mut changed: HashMap<String, Vec<MessageKey>> = HashMap::new();
        for m in &sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
            if flag.get(&m.flags) != on {
                changed.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
        }
        for (a, keys) in changed {
            self.remember(Undo::Ops { account: a, ops: vec![Op::SetFlag { keys, flag, on: !on }] });
        }
        for (a, keys) in by_account {
            self.apply_op(&a, Op::SetFlag { keys, flag, on });
        }
        // Unflagged: no longer due.
        if flag == Flag::Flagged && !on {
            for m in &sel {
                self.prefs.follow_up.remove(&follow_up_id(m));
            }
        }
        Ok(serde_json::json!({ "on": on, "count": sel.len() }))
    }

    /// Flag the selection for follow-up by a day (`None`: no date), or (`complete`) mark it
    /// done: unflagged, its date forgotten.
    pub fn follow_up_selection(&mut self, due: Option<i64>, complete: bool) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        self.flag_selection(Flag::Flagged, Some(!complete))?;
        for m in &sel {
            let id = follow_up_id(m);
            match due.filter(|_| !complete) {
                Some(d) if self.prefs.follow_up.len() < 10_000 || self.prefs.follow_up.contains_key(&id) => {
                    self.prefs.follow_up.insert(id, d);
                }
                _ => {
                    self.prefs.follow_up.remove(&id);
                }
            }
        }
        Ok(serde_json::json!({ "flagged": !complete, "due": due.filter(|_| !complete).map(mailcraft_model::dates::day_string), "count": sel.len() }))
    }

    /// When a flagged message is due, if a day was given.
    pub fn follow_up_due(&self, m: &Summary) -> Option<i64> {
        if !m.flags.flagged {
            return None;
        }
        self.prefs.follow_up.get(&follow_up_id(m)).copied()
    }

    pub(super) fn remember(&mut self, u: Undo) {
        self.undo.push(u);
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
    }

    /// Put the last change back (Undo).
    pub fn undo_last(&mut self) -> CmdResult {
        let u = self.undo.pop().ok_or_else(|| CmdError::Disabled("nothing to undo".into()))?;
        match u {
            Undo::Ops { account, ops } => {
                for op in ops {
                    self.apply_op(&account, op);
                }
                Ok(serde_json::json!({ "undone": "changes" }))
            }
            Undo::Move { account, to, back } => {
                // Where each one is now: the destination's messages with that Message-ID.
                let here = self.folder_state(&account, &to).messages.clone();
                let mut by_folder: std::collections::BTreeMap<String, Vec<MessageKey>> = std::collections::BTreeMap::new();
                let mut used: BTreeSet<MessageKey> = BTreeSet::new();
                for (id, from) in &back {
                    if let Some(m) = here.iter().find(|m| &m.message_id == id && !used.contains(&m.key)) {
                        used.insert(m.key.clone());
                        by_folder.entry(from.clone()).or_default().push(m.key.clone());
                    }
                }
                if by_folder.is_empty() {
                    // Not there yet (the server is still moving them): try again in a moment.
                    self.undo.push(Undo::Move { account, to, back });
                    return Err(CmdError::Failed("still being moved; try Undo again in a moment".into()));
                }
                let n = used.len();
                for (from, keys) in by_folder {
                    self.apply_op(&account, Op::Move { keys, to: from });
                }
                Ok(serde_json::json!({ "undone": "move", "count": n }))
            }
        }
    }
}
