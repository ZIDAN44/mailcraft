//! The session: making, renaming, moving and deleting folders.

use mailcraft_model::FOLLOW_UP_COMPLETE;
use mailcraft_model::organize::MAX_FOLLOW_UP;

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
        self.import_holds(account)?;
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
        self.import_holds(account)?;
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

    /// Count a local account's folders again (`only` these, or all of them). One whose list
    /// can't be read keeps its counts (not 0).
    pub(crate) fn recount_local(&mut self, account: &str, only: Option<&[String]>) {
        let counts = self.count_local(account, only);
        // Local accounts have no server to report counts: keep them on disk. Only the counts, in
        // the list as it is there (written only when they differ): an import adds its folders
        // to it as they finish, and the list held here may not have them yet. Compared with
        // the disk, not with the list here, which a flag change has counted already. Written on
        // the store's thread for marks, after the marks before them: a durable commit of a data
        // file, it waited in the window's frame for those marks' commits (seconds, now and then,
        // on a 38 GB file); a list on disk with none of them is written whole (`marks_kept`).
        let Some(store) = &self.store else { return };
        store.count_later(account, counts);
    }

    /// Count `only` (all the account's folders when `None`) and set the counts in the account's
    /// list here: the counts, to keep on disk.
    pub(super) fn count_local(&mut self, account: &str, only: Option<&[String]>) -> Vec<(String, u32, u32)> {
        let paths: Vec<String> = match only {
            Some(list) => list.to_vec(),
            None => self.account(account).map(|a| a.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default(),
        };
        let mut counts = Vec::new();
        for p in paths {
            // One whose read marks wait to be written is counted with them, by the marks' thread.
            if let Err(e) = self.load_cached(account, &p) {
                if !mailboxes::read_soon(&e) {
                    log::warn!("{account}: a folder isn't counted: {e}");
                }
                continue;
            }
            let st = self.folder_state(account, &p);
            counts.push((p, st.unread(), u32::try_from(st.messages.len()).unwrap_or(u32::MAX)));
        }
        if let Some(a) = self.account_mut(account) {
            for (p, unread, total) in &counts {
                if let Some(f) = a.folders.iter_mut().find(|f| f.path == *p) {
                    f.unread = *unread;
                    f.total = *total;
                }
            }
        }
        counts
    }

    /// What came of the read marks the store kept behind the window ([`Store::mark_later`]): one
    /// that couldn't be kept is said, and its folders read again as the disk has them; an account
    /// whose folder list on disk had none of the folders counted gets the window's written whole.
    pub(super) fn marks_kept(&mut self) {
        let Some(store) = self.store.clone() else { return };
        for after in store.take_marks_failed() {
            match after {
                MarksAfter::Failed { account, folders, message } => {
                    self.notice(message, true);
                    // A folder list not written (no folders) isn't counted again: that would
                    // write it again, and fail again, every frame.
                    let recount = !folders.is_empty();
                    for folder in folders {
                        self.apply(Event::FolderChanged { account: account.clone(), folder });
                    }
                    if recount {
                        self.recount_local(&account, None);
                    }
                }
                // Written on the marks' thread too: a failure comes back as one with no folders.
                MarksAfter::Unlisted { account } => {
                    if let Some(a) = self.account(&account).filter(|a| !a.folders.is_empty()) {
                        store.save_folders_later(&account, a.folders.clone());
                    }
                }
            }
        }
        // Folders not read while their marks were being written: read now they're written, the
        // list built again when it shows them; one that changed on disk meanwhile as it would
        // have been then.
        if self.marks_deferred.is_empty() {
            return;
        }
        let written: Vec<((String, String), bool)> =
            self.marks_deferred.iter().filter(|((a, f), _)| !store.marks_waiting_for(a, Some(f))).map(|(k, c)| (k.clone(), *c)).collect();
        if written.is_empty() {
            return;
        }
        for (k, _) in &written {
            self.marks_deferred.remove(k);
        }
        for ((a, f), _) in written.iter().filter(|(_, changed)| *changed) {
            self.folder_changed(a, f);
        }
        let shown = self.view_folders();
        if self.search.is_none() && written.iter().any(|(k, changed)| !changed && shown.contains(k)) {
            self.refresh_list();
        }
    }

    /// The message to show after `gone` leave the list: the next row down, else the one above,
    /// as the rows are shown (pinned ones first, a conversation one row). By the list's own
    /// order, deleting a message jumped up to a pinned one, or into the conversation above.
    pub(super) fn successor(&self, gone: &[MessageKey]) -> Option<MessageKey> {
        let gone: HashSet<&MessageKey> = gone.iter().collect();
        if let Some(last) = gone.iter().filter_map(|k| self.msg_row_of(k)).max() {
            let stays = |p: usize| self.row_key(p).filter(|k| !gone.contains(k));
            // Its own row first: a conversation that loses one message is still there.
            let next = stays(last).or_else(|| (last + 1..self.row_count()).find_map(stays)).or_else(|| (0..last).rev().find_map(stays));
            if next.is_some() {
                return next.cloned();
            }
        }
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
        // A data file's import writes its account's lists (and goes on writing them).
        for a in by_account.keys() {
            self.import_holds(a)?;
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
                // One that didn't happen (a local store error) has nothing to undo, and the
                // selection stays on it.
                self.apply_op_checked(&a, Op::Move { keys, to: dest.clone() }).map_err(CmdError::Failed)?;
                if !back.is_empty() {
                    self.moved_at = self.clock.now;
                    self.remember(Undo::Move { account: a.clone(), to: dest, back });
                }
            }
        }
        // All of it there already (Archive pressed in Archive): nothing moved, and the one open
        // stays open (it gave way to the next all the same).
        if moved == 0 {
            return Ok(serde_json::json!({ "moved": 0 }));
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
        // Gone from an account its import writes, it would come back as the import goes on.
        for a in by_account.keys() {
            self.import_holds(a)?;
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
    /// Flagged again or cleared, a follow-up marked complete no longer is.
    pub fn flag_selection(&mut self, flag: Flag, on: Option<bool>) -> CmdResult {
        let sel = self.selected();
        let first = sel.first().ok_or_else(|| CmdError::Disabled("no message selected".into()))?;
        let current = flag.get(&first.flags);
        let on = on.unwrap_or(!current);
        if flag == Flag::Flagged {
            let was = self.follow_up_days_of(&sel);
            let undo = self.set_flagged(&sel, on);
            self.remember_follow_up(undo, was);
            return Ok(serde_json::json!({ "on": on, "count": sel.len() }));
        }
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
        Ok(serde_json::json!({ "on": on, "count": sel.len() }))
    }

    /// Flag `sel` (or clear the flag): one marked complete no longer is, a fresh flag has no due
    /// day, and a cleared one neither. The changes that put the flags back, by account.
    ///
    /// A server that keeps one status for both (Microsoft Graph's flag) gets the
    /// [`FOLLOW_UP_COMPLETE`] keyword taken off before the flag is set, and put on after it.
    fn set_flagged(&mut self, sel: &[Summary], on: bool) -> Vec<(String, Vec<Op>)> {
        let mut by_account: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        let mut changed: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        // Carrying the keyword: complete, or flagged again elsewhere with it left behind.
        let mut completed: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        for m in sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
            if m.flags.flagged != on {
                changed.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
            if m.flags.has(FOLLOW_UP_COMPLETE) {
                completed.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
        }
        for (a, keys) in &completed {
            self.apply_op(a, Op::SetKeyword { keys: keys.clone(), keyword: FOLLOW_UP_COMPLETE.into(), on: false });
        }
        for (a, keys) in by_account {
            self.apply_op(&a, Op::SetFlag { keys, flag: Flag::Flagged, on });
        }
        for m in sel {
            let id = follow_up_id(m);
            // Unflagged: no longer due. Flagged again once complete: a fresh flag, not the one
            // completed (one still flagged keeps its day).
            if !on || m.flags.follow_up_complete() {
                self.prefs.follow_up.remove(&id);
            }
            self.prefs.follow_up_done.remove(&id);
        }
        let mut undo: BTreeMap<String, Vec<Op>> = BTreeMap::new();
        for (a, keys) in changed {
            undo.entry(a).or_default().push(Op::SetFlag { keys, flag: Flag::Flagged, on: !on });
        }
        for (a, keys) in completed {
            undo.entry(a).or_default().push(Op::SetKeyword { keys, keyword: FOLLOW_UP_COMPLETE.into(), on: true });
        }
        undo.into_iter().collect()
    }

    /// Each of `sel`'s due and done days as they are, for Undo to put back.
    fn follow_up_days_of(&self, sel: &[Summary]) -> Vec<(String, Option<i64>, Option<i64>)> {
        sel.iter()
            .map(|m| {
                let id = follow_up_id(m);
                let (due, done) = (self.prefs.follow_up.get(&id).copied(), self.prefs.follow_up_done.get(&id).copied());
                (id, due, done)
            })
            .collect()
    }

    /// Remember a change to follow-ups for Undo, when it changed anything: `ops` put the flags
    /// back, `was` the days.
    fn remember_follow_up(&mut self, ops: Vec<(String, Vec<Op>)>, was: Vec<(String, Option<i64>, Option<i64>)>) {
        let days_changed =
            was.iter().any(|(id, due, done)| self.prefs.follow_up.get(id).copied() != *due || self.prefs.follow_up_done.get(id).copied() != *done);
        if ops.iter().any(|(_, o)| !o.is_empty()) || days_changed {
            self.remember(Undo::FollowUp { ops, days: was });
        }
    }

    /// Flag the selection for follow-up by a day (`None`: no date), or (`complete`) mark it
    /// done ([`Self::complete_selection`]).
    pub fn follow_up_selection(&mut self, due: Option<i64>, complete: bool) -> CmdResult {
        if complete {
            return self.complete_selection();
        }
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let was = self.follow_up_days_of(&sel);
        let undo = self.set_flagged(&sel, true);
        for m in &sel {
            let id = follow_up_id(m);
            match due {
                Some(d) if self.prefs.follow_up.len() < MAX_FOLLOW_UP || self.prefs.follow_up.contains_key(&id) => {
                    self.prefs.follow_up.insert(id, d);
                }
                _ => {
                    self.prefs.follow_up.remove(&id);
                }
            }
        }
        self.remember_follow_up(undo, was);
        Ok(serde_json::json!({ "flagged": true, "due": due.map(mailcraft_model::dates::day_string), "count": sel.len() }))
    }

    /// Mark the selection's follow-up complete (Mark Complete): unflagged, with the
    /// [`FOLLOW_UP_COMPLETE`] keyword, and the day it was done kept (as Outlook keeps it). Its
    /// due day stays; Undo puts the flag and both days back as they were, in one step.
    pub fn complete_selection(&mut self) -> CmdResult {
        let sel = self.selected();
        self.complete_messages(sel)
    }

    /// [`Session::complete_selection`] for these messages, wherever they are (the To-Do Bar's
    /// are in any folder).
    pub(crate) fn complete_messages(&mut self, sel: Vec<Summary>) -> CmdResult {
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let was = self.follow_up_days_of(&sel);
        let today = mailcraft_model::dates::today(&self.clock);
        let mut flagged: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        let mut marked: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        for m in &sel {
            if m.flags.flagged {
                flagged.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
            if !m.flags.has(FOLLOW_UP_COMPLETE) {
                marked.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
            // Done today, unless it was already done (marked complete again).
            let id = follow_up_id(m);
            if !(m.flags.follow_up_complete() && self.prefs.follow_up_done.contains_key(&id))
                && (self.prefs.follow_up_done.len() < MAX_FOLLOW_UP || self.prefs.follow_up_done.contains_key(&id))
            {
                self.prefs.follow_up_done.insert(id, today);
            }
        }
        // Undo: the keyword off before the flag goes back on (see `set_flagged`).
        let mut undo: BTreeMap<String, Vec<Op>> = BTreeMap::new();
        for (a, keys) in &marked {
            undo.entry(a.clone()).or_default().push(Op::SetKeyword { keys: keys.clone(), keyword: FOLLOW_UP_COMPLETE.into(), on: false });
        }
        for (a, keys) in &flagged {
            undo.entry(a.clone()).or_default().push(Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on: true });
        }
        for (a, keys) in flagged {
            self.apply_op(&a, Op::SetFlag { keys, flag: Flag::Flagged, on: false });
        }
        for (a, keys) in marked {
            self.apply_op(&a, Op::SetKeyword { keys, keyword: FOLLOW_UP_COMPLETE.into(), on: true });
        }
        self.remember_follow_up(undo.into_iter().collect(), was);
        Ok(serde_json::json!({ "flagged": false, "complete": true, "count": sel.len() }))
    }

    /// When a flagged message is due, if a day was given.
    pub fn follow_up_due(&self, m: &Summary) -> Option<i64> {
        if !m.flags.flagged {
            return None;
        }
        self.prefs.follow_up.get(&follow_up_id(m)).copied()
    }

    /// When a message's follow-up was marked complete, if it was and the day is known.
    pub fn follow_up_done(&self, m: &Summary) -> Option<i64> {
        if !m.flags.follow_up_complete() {
            return None;
        }
        self.prefs.follow_up_done.get(&follow_up_id(m)).copied()
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
            Undo::FollowUp { ops, days } => {
                for (account, ops) in ops {
                    for op in ops {
                        self.apply_op(&account, op);
                    }
                }
                for (id, due, done) in days {
                    put_day(&mut self.prefs.follow_up, id.clone(), due);
                    put_day(&mut self.prefs.follow_up_done, id, done);
                }
                Ok(serde_json::json!({ "undone": "changes" }))
            }
            Undo::Move { account, to, back } => {
                if let Err(e) = self.import_holds(&account) {
                    self.undo.push(Undo::Move { account, to, back });
                    return Err(e);
                }
                if let Err(e) = self.folders_ready(&[(account.clone(), to.clone())]) {
                    self.undo.push(Undo::Move { account, to, back });
                    return Err(e);
                }
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
                    // Not there yet (the server is still moving them, and the folder they go
                    // to is read after): try again in a moment. For a couple of minutes after
                    // the move; a server account's, as a move on this computer is done at once.
                    let server = self.account(&account).is_some_and(|a| !a.is_local());
                    if server && self.clock.now.saturating_sub(self.moved_at) < 120 {
                        self.undo.push(Undo::Move { account, to, back });
                        return Err(CmdError::Failed("still being moved; try Undo again in a moment".into()));
                    }
                    // Gone from there since (deleted for good, moved on, the folder emptied):
                    // nothing to put back, and what was done before it can be undone. Kept to
                    // be tried again, it stood in the way of every Undo after.
                    return Err(CmdError::Failed("the mail isn't where it was moved to any more, so it can't be put back".into()));
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

/// A day put back as it was (`None`: none), within [`MAX_FOLLOW_UP`].
fn put_day(days: &mut BTreeMap<String, i64>, id: String, day: Option<i64>) {
    match day {
        Some(d) if days.len() < MAX_FOLLOW_UP || days.contains_key(&id) => {
            days.insert(id, d);
        }
        Some(_) => {}
        None => {
            days.remove(&id);
        }
    }
}
