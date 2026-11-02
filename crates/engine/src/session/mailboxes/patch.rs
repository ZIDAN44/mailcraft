//! The session: a flag or category changed on mail in the list is changed in the list as it
//! stands, not by building the list again. Marking a message read built the whole list again
//! (each message copied out of the folder, filtered, arranged, its conversations worked out, its
//! rows laid out): on a folder of ten thousand messages, every Down that opened one did.

use mailcraft_model::{Flags, PINNED};

use super::*;

impl Session {
    /// Take a changed flag or category (in the cache already: `apply_op_memory`) into the list
    /// as it stands; false when it must be built again ([`Session::refresh_list`]), the same
    /// list either way.
    ///
    /// With a search, the changed messages are weighed against it again, alone: built again
    /// only when what it finds changed (`is:unread` and a message marked read). Built again: for
    /// the Outbox; for deleted mail and junk marks (junk moves, and what's focused can change);
    /// for a list changed by hand since it was built; and when the change lets mail in that the
    /// filter left out (marked unread, shown Unread only). Mail the filter now leaves out goes from the list
    /// (but for the message being read, kept as `refresh_list` keeps it); a list arranged by
    /// the flag is arranged again; a pin lays the rows out again; a read mark changes the
    /// unread count of its row only.
    pub(crate) fn patch_list(&mut self, op: &Op) -> bool {
        let filter = self.filter;
        // What the change decides: whether mail passes the filter, where it's arranged, whether
        // it's pinned, what its row counts unread.
        let (keys, filtered, rearranged, repinned, unread) = match op {
            Op::SetFlag { flag: Flag::Deleted, .. } => return false,
            Op::SetFlag { keys, flag, .. } => (
                keys,
                matches!((flag, filter), (Flag::Seen, ListFilter::Unread) | (Flag::Flagged, ListFilter::Flagged)),
                *flag == Flag::Flagged && self.prefs.arrange == Arrange::Flag,
                false,
                *flag == Flag::Seen,
            ),
            Op::SetKeyword { keyword, .. } if keyword.to_ascii_lowercase().contains("junk") => return false,
            Op::SetKeyword { keys, keyword, .. } => (keys, filter == ListFilter::Categorized, false, keyword.eq_ignore_ascii_case(PINNED), false),
            _ => return false,
        };
        if self.view == View::Outbox || !self.nav_fresh() || (self.search.is_some() && !self.search_patched(keys)) {
            return false;
        }
        // The changed messages as the cache has them, each folder shown looked through once (a
        // select-all marks every message of a big folder at once).
        let shown = match (&self.search, &self.search_state.run) {
            (Some(_), Some(run)) => run.folders.clone(),
            _ => self.view_folders(),
        };
        let mut uids: HashMap<(&str, &str), HashSet<u32>> = HashMap::new();
        for k in keys {
            if shown.iter().any(|(a, f)| *a == k.account && *f == k.folder) {
                uids.entry((k.account.as_str(), k.folder.as_str())).or_default().insert(k.uid);
            }
        }
        let mut changed: Vec<(usize, Flags)> = Vec::new();
        for ((a, f), uids) in &uids {
            // Let go of since (a change saved to disk read it again): built again from disk.
            let Some(st) = self.cache.get(&(a.to_string(), f.to_string())) else { return false };
            for m in st.messages.iter().filter(|m| uids.contains(&m.key.uid)) {
                match self.index_of(&m.key) {
                    Some(i) => changed.push((i, m.flags.clone())),
                    // Let in by the filter now: where it goes is the list's to work out.
                    None if filtered && !m.flags.deleted && self.filter_keeps(filter, m) => return false,
                    None => {}
                }
            }
        }
        let reading = self.reading.as_ref().map(|r| r.key.clone());
        let mut gone = Vec::new();
        for (i, flags) in &changed {
            let Some(m) = self.list.get_mut(*i) else { continue };
            m.flags = flags.clone();
            if filtered && !self.list.get(*i).is_some_and(|m| self.filter_keeps(filter, m) || reading.as_ref() == Some(&m.key)) {
                gone.push(*i);
            }
        }
        if !gone.is_empty() || rearranged {
            let mut list = std::mem::take(&mut self.list);
            if !gone.is_empty() {
                gone.sort_unstable();
                let mut at = 0usize;
                list.retain(|_| {
                    let keep = gone.binary_search(&at).is_err();
                    at = at.saturating_add(1);
                    keep
                });
            }
            if rearranged {
                arrange_list(&mut list, self.prefs.arrange, self.prefs.reverse_sort);
            }
            self.take_list(list);
            return true;
        }
        if repinned {
            self.rebuild_rows();
        } else if unread {
            for (i, _) in &changed {
                if !self.patch_unread(*i) {
                    return false;
                }
            }
        }
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// The index found which of a folder's messages carry files (`with`) and which only show
    /// pictures in their text (`without`), where their summaries said otherwise; the folder's
    /// list on disk is put right already ([`Event::AttachmentsKnown`]). The summaries the
    /// window holds are put right too, so the paperclip, the Has files filter and
    /// `has:attachment` agree, and a save of the folder from the window keeps it right. The list
    /// is built again only when what it shows depends on files (filtered or arranged by them);
    /// otherwise its rows are patched as they stand.
    pub(crate) fn files_known(&mut self, account: &str, folder: &str, with: &[u32], without: &[u32]) {
        let with: HashSet<u32> = with.iter().copied().collect();
        let without: HashSet<u32> = without.iter().copied().collect();
        // Whether `m` is one of them, and if so whether it changed.
        let set = |m: &mut Summary| {
            let files = if with.contains(&m.key.uid) {
                true
            } else if without.contains(&m.key.uid) {
                false
            } else {
                return false;
            };
            let changed = m.has_attachments != files;
            m.has_attachments = files;
            changed
        };
        self.mail_changed();
        if let Some(st) = self.cache.get_mut(&(account.to_string(), folder.to_string())) {
            for m in &mut st.messages {
                set(m);
            }
        }
        let mut shown = false;
        for m in self.list.iter_mut().filter(|m| m.key.account == account && m.key.folder == folder) {
            shown |= set(m);
        }
        let folders = match (&self.search, &self.search_state.run) {
            (Some(_), Some(run)) => run.folders.clone(),
            _ => self.view_folders(),
        };
        let visible = folders.iter().any(|(a, f)| a == account && f == folder);
        if visible && (self.filter == ListFilter::Attachments || self.prefs.arrange == Arrange::Attachments) {
            self.refresh_list();
        } else if shown {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// A message downloaded: its summary as the cache on disk has it now, filled from its source
    /// ([`Event::Filled`]). The folder in memory takes it, with the flags it has here (a mark made
    /// since is newer), and the list its entry, where nothing it shows depends on what changed;
    /// built again where something does (its conversation, a list filtered or arranged by files,
    /// size or importance, the Focused split). It read all of the folder from disk again and built
    /// the list: 24 ms on a folder of 10,941 messages, every Down that landed on mail not
    /// downloaded yet. A folder not in memory is read with it when it's wanted.
    pub(crate) fn filled(&mut self, mut now: Summary) {
        let k = (now.key.account.clone(), now.key.folder.clone());
        let Some(st) = self.cache.get_mut(&k).filter(|st| st.validity == now.key.validity) else { return };
        let Some(m) = st.find_mut(now.key.uid) else { return };
        now.flags.clone_from(&m.flags);
        let threads = m.in_reply_to != now.in_reply_to || m.references != now.references;
        let files = m.has_attachments != now.has_attachments;
        let (size, importance, bulk) = (m.size != now.size, m.importance != now.importance, m.bulk != now.bulk);
        m.clone_from(&now);
        self.mail_changed();
        // A search holds what it found as it was: it's run again over the folder.
        if self.search.is_some() {
            self.search_folder_changed(&k.0, &k.1);
            self.refresh_list();
            return;
        }
        let Some(i) = self.index_of(&now.key) else { return };
        let (filter, arrange) = (self.filter, self.prefs.arrange);
        let shown_by = (threads && self.prefs.conversations)
            || (files && (filter == ListFilter::Attachments || arrange == Arrange::Attachments))
            || (size && arrange == Arrange::Size)
            || (importance && filter == ListFilter::Important)
            || bulk;
        if shown_by || !self.nav_fresh() {
            self.refresh_list();
            if let Some(r) = self.reading.as_ref().filter(|r| r.key.account == k.0 && r.key.folder == k.1) {
                let key = r.key.clone();
                let thread = self.thread_of(&key);
                if let Some(r) = self.reading.as_mut() {
                    r.thread = thread;
                }
            }
            return;
        }
        if let Some(m) = self.list.get_mut(i) {
            now.flags.clone_from(&m.flags);
            *m = now;
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// Count the unread mail of the row the message at `i` in the list is on again (its own, or
    /// its conversation's); false when it has none to count.
    fn patch_unread(&mut self, i: usize) -> bool {
        let Some(key) = self.list.get(i).map(|m| m.key.clone()) else { return false };
        let unread = |j: &usize| self.list.get(*j).is_some_and(|m| !m.flags.seen);
        let count = if self.prefs.conversations && !self.threads.is_empty() {
            match self.thread_at(i) {
                Some(t) => t.messages.iter().filter(|j| unread(j)).count(),
                None => return false,
            }
        } else {
            usize::from(unread(&i))
        };
        match self.row_index_of(&key).and_then(|r| self.nav.rows.get_mut(r)) {
            Some(Row::Message { unread, .. }) => {
                *unread = count;
                true
            }
            _ => false,
        }
    }

    /// The message read before goes from the list, where it was kept only while it was read
    /// (`refresh_list`); the rest stays as it is.
    pub(crate) fn drop_from_list(&mut self, key: &MessageKey) {
        let at = self.index_of(key);
        match at {
            Some(i) if self.search.is_none() && self.view != View::Outbox && self.nav_fresh() => {
                let mut list = std::mem::take(&mut self.list);
                if i < list.len() {
                    list.remove(i);
                }
                self.take_list(list);
            }
            _ => self.refresh_list(),
        }
    }
}
