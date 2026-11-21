//! The session: changing mail (flags, moves, deletions, undo).

use std::collections::VecDeque;

use super::*;

mod folder_edits;
mod follow_up;
pub(crate) use folder_edits::keys_by_account;

/// The most moves into an Inbox or Junk remembered ([`MovedIn`]) before the oldest are
/// forgotten: the server's answer takes each out again, so only those it never confirms stay.
pub(crate) const MOVED_IN_MAX: usize = 50_000;

/// Mail the user put in a server account's Inbox or Junk (a move, an undo, a snooze over), by
/// (account, folder, Message-ID): the server gives it a new UID there, and it isn't mail
/// arriving. Past [`MOVED_IN_MAX`] the oldest are forgotten first (all of them were, at once,
/// so a move still on its way counted as mail arriving).
#[derive(Debug, Default)]
pub(crate) struct MovedIn {
    /// Each one remembered and when (a count of insertions).
    at: HashMap<(String, String, String), u64>,
    /// The same, oldest first; one taken out since, or remembered again later, is skipped as it
    /// comes up.
    order: VecDeque<(u64, (String, String, String))>,
    count: u64,
}

impl MovedIn {
    pub(crate) fn insert(&mut self, k: (String, String, String)) {
        self.count = self.count.saturating_add(1);
        self.at.insert(k.clone(), self.count);
        self.order.push_back((self.count, k));
        while self.at.len() > MOVED_IN_MAX {
            let Some((when, k)) = self.order.pop_front() else { break };
            if self.at.get(&k) == Some(&when) {
                self.at.remove(&k);
            }
        }
        // What was taken out stays in the order until it comes up: not past twice the cap.
        if self.order.len() > MOVED_IN_MAX.saturating_mul(2) {
            let at = &self.at;
            self.order.retain(|(when, k)| at.get(k) == Some(when));
        }
    }

    /// Forget one; whether it was remembered.
    pub(crate) fn remove(&mut self, k: &(String, String, String)) -> bool {
        self.at.remove(k).is_some()
    }

    /// Each one remembered.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &(String, String, String)> {
        self.at.keys()
    }
}

impl Session {
    /// Apply a change: to the in-memory list at once, then to the cache and server (through a
    /// job), or straight to the cache for local accounts.
    pub fn apply_op(&mut self, account: &str, op: Op) {
        if let Err(e) = self.apply_op_checked(account, op) {
            self.notice(e, true);
        }
    }

    /// Make the changes gathered for many messages: flags and categories first, so they're set
    /// on the mail before it moves, then the moves.
    pub(super) fn apply_changes(&mut self, batch: Batch) {
        let (moves, marks): (Vec<_>, Vec<_>) = batch.0.into_iter().partition(|(_, op)| matches!(op, Op::Move { .. }));
        for (account, op) in marks.into_iter().chain(moves) {
            self.apply_op(&account, op);
        }
    }

    /// [`Session::apply_op`], saying when a local change didn't happen (a move's mail stays where
    /// it was, or is in both folders when only its copies were filed) rather than putting it in a
    /// notice: a move command fails with it.
    pub(super) fn apply_op_checked(&mut self, account: &str, op: Op) -> Result<(), String> {
        // Outbox entries aren't on a server: mail commands leave them alone.
        let op = match op {
            Op::SetFlag { keys, flag, on } => Op::SetFlag { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), flag, on },
            Op::SetKeyword { keys, keyword, on } => Op::SetKeyword { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), keyword, on },
            Op::SetFollowUp { keys, due } => Op::SetFollowUp { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), due },
            Op::SetReminder { keys, at } => Op::SetReminder { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), at },
            Op::Move { keys, to } => Op::Move { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), to },
            Op::Expunge { keys } => Op::Expunge { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect() },
            other => other,
        };
        if !op.is_folder_op() && op.whole_folder().is_none() && op.keys().is_empty() {
            return Ok(());
        }
        let local = self.account(account).is_none_or(AccountState::is_local);
        if !local {
            self.note_moved_in(account, &op);
        }
        self.apply_op_memory(account, &op);
        let done = if local { self.apply_op_local(account, &op) } else { Ok(()) };
        self.todo_op(account, &op);
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

    /// Mail moved into an Inbox, Junk or Sent on a server is remembered ([`Session::moved_in`]):
    /// seen there under a new UID, it was taken for mail arriving, so a rule filed it straight
    /// back where it was dragged from, an undone delete was announced as new, and "Not junk" on a
    /// blocked sender's message sent it back to Junk. (Mail put in Sent wasn't sent: the rules
    /// for mail sent leave it alone, issue #202.)
    fn note_moved_in(&mut self, account: &str, op: &Op) {
        let Op::Move { keys, to } = op else { return };
        if !matches!(self.folder(account, to).map(|f| f.role), Some(FolderRole::Inbox | FolderRole::Junk | FolderRole::Sent)) {
            return;
        }
        // Each source folder looked through once (per message, 5,000 out of 50,000 took 80 ms).
        let mut from: BTreeMap<(&str, &str), HashSet<&MessageKey>> = BTreeMap::new();
        for k in keys.iter().filter(|k| k.folder != *to) {
            from.entry((k.account.as_str(), k.folder.as_str())).or_default().insert(k);
        }
        for ((a, folder), keys) in from {
            let ids: Vec<String> = self
                .folder_state(a, folder)
                .messages
                .iter()
                .filter(|m| !m.message_id.is_empty() && keys.contains(&m.key))
                .map(|m| m.message_id.clone())
                .collect();
            for id in ids {
                self.moved_in.insert((account.to_string(), to.clone(), id));
            }
        }
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
            Op::MarkFolderRead { .. }
            | Op::SetFlag { .. }
            | Op::SetFollowUp { .. }
            | Op::SetReminder { .. }
            | Op::SetKeyword { .. }
            | Op::CreateFolder { .. }
            | Op::RenameFolder { .. }
            | Op::DeleteFolder { .. } => {}
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
        // The folder's mail kept here is marked with `SetFlag`; this is the rest, on the server
        // (issue #197): its count goes, and anything kept still unread is read.
        if let Op::MarkFolderRead { path } = op {
            self.mail_changed();
            if let Some(st) = self.cache.get_mut(&(account.to_string(), path.clone())) {
                for m in st.messages.iter_mut().filter(|m| !m.flags.seen) {
                    SharedSummary::make_mut(m).flags.seen = true;
                }
            }
            if let Some(f) = self.account_mut(account).and_then(|a| a.folders.iter_mut().find(|f| &f.path == path)) {
                f.unread = 0;
            }
            return;
        }
        let mut unread_delta: HashMap<String, i64> = HashMap::new();
        // Mail gone from a folder is gone from its server's count too (it said there was more on
        // the server, a draft sent and gone, until the next sync).
        let mut gone: HashMap<String, u32> = HashMap::new();
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
                Op::SetFollowUp { due, .. } => {
                    let day = due.map(|d| d.day);
                    for m in st.messages.iter_mut().filter(|m| mine.contains(&m.key.uid) && m.flags.follow_up_due != day) {
                        SharedSummary::make_mut(m).flags.follow_up_due = day;
                    }
                }
                Op::SetReminder { at, .. } => {
                    for m in st.messages.iter_mut().filter(|m| mine.contains(&m.key.uid) && m.flags.reminder != *at) {
                        SharedSummary::make_mut(m).flags.reminder = *at;
                    }
                }
                Op::Move { to, .. } if to == f => {}
                Op::Move { .. } | Op::Expunge { .. } => {
                    let unread = st.messages.iter().filter(|m| mine.contains(&m.key.uid) && !m.flags.seen).count();
                    *unread_delta.entry(f.clone()).or_default() -= i64::try_from(unread).unwrap_or(0);
                    let before = st.messages.len();
                    st.messages.retain(|m| !mine.contains(&m.key.uid));
                    *gone.entry(f.clone()).or_default() += u32::try_from(before - st.messages.len()).unwrap_or(u32::MAX);
                }
                Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } | Op::EmptyFolder { .. } | Op::MarkFolderRead { .. } => {}
            }
        }
        if let Some(a) = self.account_mut(account) {
            for (f, d) in unread_delta {
                if let Some(folder) = a.folders.iter_mut().find(|x| x.path == f) {
                    folder.unread = u32::try_from((i64::from(folder.unread) + d).max(0)).unwrap_or(0);
                }
            }
            for (f, n) in gone {
                if let Some(folder) = a.folders.iter_mut().find(|x| x.path == f) {
                    folder.total = folder.total.saturating_sub(n);
                }
            }
        }
    }

    /// Local accounts: change the cache directly, including filing moved messages in their
    /// destination with new UIDs. An error says what the cache couldn't do.
    pub(super) fn apply_op_local(&mut self, account: &str, op: &Op) -> Result<(), String> {
        let Some(store) = self.store.clone() else { return Ok(()) };
        // The folders it touches (counted again after; all of them for a folder change).
        let mut touched: Vec<String> = op.keys().iter().map(|k| k.folder.clone()).collect();
        match op {
            Op::Move { to, .. } => touched.push(to.clone()),
            Op::EmptyFolder { path } | Op::MarkFolderRead { path } => touched.push(path.clone()),
            _ => {}
        }
        touched.sort();
        touched.dedup();
        // A flag or category: the lists in memory have it already (`apply_op_memory`), so the
        // flags they have now are handed to the store's thread for marks with the folders'
        // counts, written there in one commit (`store/src/marking.rs`): not in the window's frame.
        // One not in memory is changed on disk.
        if matches!(op, Op::SetFlag { .. } | Op::SetFollowUp { .. } | Op::SetReminder { .. } | Op::SetKeyword { .. })
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
        // Not done (a move's sources not taken out after its copies were filed): the lists in
        // memory are read again, and the caller is told (`apply_op` says it in a notice).
        let failed = match store.apply_op(account, op) {
            Ok(changed) => {
                for f in changed {
                    self.reload_folder(account, &f);
                }
                None
            }
            Err(e) => {
                for f in &touched {
                    self.reload_folder(account, f);
                }
                Some(match op {
                    // The folder as its name reads (its path is kept in modified UTF-7).
                    Op::Move { to, .. } => {
                        format!("Your mail was copied to {}, but couldn't be taken out of where it was: {e}", mailcraft_model::utf7::decode(to))
                    }
                    _ => e.to_string(),
                })
            }
        };
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
        } else if !matches!(op, Op::SetFlag { .. } | Op::SetFollowUp { .. } | Op::SetReminder { .. } | Op::SetKeyword { .. }) {
            for f in &touched {
                self.want_index(account, Some(f));
            }
        }
        failed.map_or(Ok(()), Err)
    }

    /// A local move's first half: the messages filed in `to` with new numbers, their sources with
    /// them ([`Filing`]: a long write past a batch of them, returned to last until the rest of the
    /// move is saved). Every list is read before anything is written, and any error but a source
    /// that isn't here stops it (a list read as empty was saved over the destination's mail).
    fn file_moved<'s>(store: &'s Store, account: &str, keys: &[MessageKey], to: &str) -> Result<Option<LongWrite<'s>>, mailcraft_store::StoreError> {
        let mut filing = Filing::new(store, account, to)?;
        // Each source folder read once, its messages found by number.
        let mut sources: HashMap<String, HashMap<u32, SharedSummary>> = HashMap::new();
        for k in keys.iter().filter(|k| k.folder != to) {
            if !sources.contains_key(&k.folder) {
                let st = store.load_folder(account, &k.folder)?;
                sources.insert(k.folder.clone(), st.messages.into_iter().map(|m| (m.key.uid, m)).collect());
            }
        }
        for k in keys.iter().filter(|k| k.folder != to) {
            let Some(m) = sources.get(&k.folder).and_then(|src| src.get(&k.uid)) else { continue };
            let mut moved = Summary::clone(m);
            moved.key = filing.key()?;
            // One never downloaded moves without it.
            filing.add(moved, store.load_body(k)?)?;
        }
        filing.finish()
    }

    /// File a message whose source is `raw`, marked `flags`, arrived at `date`; its key.
    pub(crate) fn file_raw(
        filing: &mut Filing,
        raw: Vec<u8>,
        flags: mailcraft_model::Flags,
        date: i64,
    ) -> Result<MessageKey, mailcraft_store::StoreError> {
        let key = filing.key()?;
        let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        filing.add(mime::summary_from_full(&raw, key.clone(), flags, size, date), Some(raw))?;
        Ok(key)
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
                        let parent = Self::parent_of(path, delimiter);
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
                Op::SetFlag { .. }
                | Op::SetFollowUp { .. }
                | Op::SetReminder { .. }
                | Op::SetKeyword { .. }
                | Op::Move { .. }
                | Op::Expunge { .. }
                | Op::EmptyFolder { .. }
                | Op::MarkFolderRead { .. } => {}
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
                self.organizing_follows_folder(account, from, to, delimiter);
                // Its place in the user's order too (issue #183).
                self.order_follows_folder(account, from, Some(to), delimiter);
                self.folds_follow_folder(account, from, Some(to), delimiter);
                // A folder's own AutoArchive setting goes with it (issue #180).
                for f in self.prefs.auto_archive.folders.iter_mut().filter(|f| f.account == account) {
                    if let Some(new) = mailcraft_model::rebase_path(&f.folder, from, to, delimiter) {
                        f.folder = new;
                    }
                }
            }
            Op::DeleteFolder { path } => {
                self.cache.retain(|(acc, f), _| acc != account || !mailcraft_model::is_within(f, path, delimiter));
                if let Some(favs) = self.prefs.favorites.as_mut() {
                    favs.retain(|f| f.account != account || !mailcraft_model::is_within(&f.folder, path, delimiter));
                }
                self.prefs.auto_archive.folders.retain(|f| f.account != account || !mailcraft_model::is_within(&f.folder, path, delimiter));
                self.order_follows_folder(account, path, None, delimiter);
                self.folds_follow_folder(account, path, None, delimiter);
                if matches!(&self.view, View::Folder { account: va, folder } if va == account && mailcraft_model::is_within(folder, path, delimiter))
                {
                    self.view = self.default_view();
                    self.reading = None;
                    self.selection.clear();
                    self.focus = None;
                }
            }
            Op::SetFlag { .. }
            | Op::SetFollowUp { .. }
            | Op::SetReminder { .. }
            | Op::SetKeyword { .. }
            | Op::Move { .. }
            | Op::Expunge { .. }
            | Op::EmptyFolder { .. }
            | Op::MarkFolderRead { .. } => {}
        }
        self.search_folders_changed(account);
    }
}

/// Changes to many messages, gathered as they're worked out (rules, junk lists and ignored
/// conversations on mail arriving) and made by [`Session::apply_changes`]: the same flag,
/// category or destination in one account is one change for all its messages, as a selection's
/// is, not one per message (each rebuilt the list and asked the server again).
#[derive(Default)]
pub(super) struct Batch(Vec<(String, Op)>);

impl Batch {
    /// `op` (a flag, category or move) for messages of `account`, joined to the same change
    /// asked for already. Messages are added one at a time, all of one message's changes
    /// together, so one named twice for a change (two rules flagging it) is that change's last.
    pub(super) fn add(&mut self, account: &str, op: Op) {
        let mut op = op;
        for (a, have) in &mut self.0 {
            if a != account {
                continue;
            }
            op = match (have, op) {
                (Op::SetFlag { keys, flag, on }, Op::SetFlag { keys: more, flag: f, on: o }) if *flag == f && *on == o => return join(keys, more),
                (Op::SetKeyword { keys, keyword, on }, Op::SetKeyword { keys: more, keyword: k, on: o }) if *keyword == k && *on == o => {
                    return join(keys, more);
                }
                (Op::Move { keys, to }, Op::Move { keys: more, to: t }) if *to == t => return join(keys, more),
                (_, op) => op,
            };
        }
        self.0.push((account.to_string(), op));
    }
}

fn join(keys: &mut Vec<MessageKey>, more: Vec<MessageKey>) {
    for k in more {
        if keys.last() != Some(&k) {
            keys.push(k);
        }
    }
}
