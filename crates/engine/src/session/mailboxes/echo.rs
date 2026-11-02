//! The session: its own flag and category changes to a server account's mail, coming back.
//!
//! Such a change is made in the cache in memory at once and the list patched (`patch.rs`); the
//! pool then makes it in the cache on disk and says the folder changed (`FolderChanged`). That
//! echo let go of the folder in memory, read all of it again from disk and built the list again:
//! on a folder of ten thousand messages, every Down that opened one did, a moment after.
//!
//! The changes sent are remembered, by folder, in the order they were sent (the pool makes them
//! in that order, each folder said changed once it's on disk). A `FolderChanged` while some wait
//! is taken for the oldest's echo: the folder in memory has it already, and is kept. Another
//! change on disk meanwhile (a sync) says the folder changed too, and as one more is said than
//! waited for, the last is read from disk, with everything. A change the cache on disk couldn't
//! take comes with a notice, which lets go of the account's waiting ones.

use std::collections::VecDeque;

use mailcraft_model::Flags;

use super::*;

/// Changes waiting for their echo in one folder, at most: past it the oldest is forgotten (its
/// echo then reads the folder from disk, as before).
const ECHOES_MAX: usize = 1024;
/// Seconds a change waits for its echo at most (a front end that never sends one).
const ECHO_WAIT: i64 = 60;

/// Flag and category changes sent for a server account's mail and not yet said back, by
/// (account, folder): when each was sent, and the change.
#[derive(Default)]
pub(crate) struct Echoes(HashMap<(String, String), VecDeque<(i64, Op)>>);

impl Echoes {
    /// Whether any is waiting for `account` (tests).
    #[cfg(test)]
    pub(crate) fn waiting(&self, account: &str) -> usize {
        self.0.iter().filter(|((a, _), _)| a == account).map(|(_, q)| q.len()).sum()
    }
}

impl Session {
    /// A flag or category change sent to a server account (`Job::Apply`): its echo is waited for
    /// in each folder it changes.
    pub(crate) fn expect_echo(&mut self, account: &str, op: &Op) {
        if !matches!(op, Op::SetFlag { .. } | Op::SetKeyword { .. }) {
            return;
        }
        let folders: BTreeSet<&str> = op.keys().iter().filter(|k| k.account == account).map(|k| k.folder.as_str()).collect();
        let now = self.clock.now;
        for f in folders {
            let q = self.echoes.0.entry((account.to_string(), f.to_string())).or_default();
            if q.len() >= ECHOES_MAX {
                q.pop_front();
            }
            q.push_back((now, op.clone()));
        }
    }

    /// The changes waiting for `account` forgotten: what it says changed next is read from disk.
    pub(crate) fn forget_echoes(&mut self, account: &str) {
        self.echoes.0.retain(|(a, _), _| a != account);
    }

    /// Whether `folder` changing is the echo of one of the session's own changes, which the
    /// folder in memory has already: then it's kept as it is, the list with it. A folder in
    /// memory read from disk again before the change was there gets it the way the list does,
    /// each change as its echo comes (not where a later one waiting changes the same, which says
    /// how it ends).
    pub(crate) fn own_echo(&mut self, account: &str, folder: &str) -> bool {
        let k = (account.to_string(), folder.to_string());
        let now = self.clock.now;
        let Some(q) = self.echoes.0.get_mut(&k) else { return false };
        while q.front().is_some_and(|(at, _)| now.saturating_sub(*at) > ECHO_WAIT) {
            q.pop_front();
        }
        let Some((_, op)) = q.pop_front() else {
            self.echoes.0.remove(&k);
            return false;
        };
        let in_folder = |m: &&MessageKey| m.account == account && m.folder == folder;
        let superseded: HashSet<u32> =
            q.iter().filter(|(_, o)| same_change(o, &op)).flat_map(|(_, o)| o.keys().iter().filter(in_folder).map(|m| m.uid)).collect();
        if q.is_empty() {
            self.echoes.0.remove(&k);
        }
        self.mail_changed();
        let Some(st) = self.cache.get_mut(&k) else { return false };
        let uids: HashSet<u32> =
            op.keys().iter().filter(in_folder).filter(|m| m.validity == st.validity && !superseded.contains(&m.uid)).map(|m| m.uid).collect();
        let has = |f: &Flags| match &op {
            Op::SetFlag { flag, on, .. } => flag.get(f) == *on,
            Op::SetKeyword { keyword, on, .. } => f.has(keyword) == *on,
            _ => true,
        };
        if st.messages.iter().filter(|m| uids.contains(&m.key.uid)).all(|m| has(&m.flags)) {
            return true;
        }
        for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
            match &op {
                Op::SetFlag { flag, on, .. } => flag.set(&mut m.flags, *on),
                Op::SetKeyword { keyword, on, .. } => m.flags.set_keyword(keyword, *on),
                _ => {}
            }
        }
        let visible = self.search.is_some() || self.view_folders().iter().any(|(a, f)| a == account && f == folder);
        if visible && !self.patch_list(&op) {
            self.refresh_list();
        }
        true
    }
}

/// Whether `a` and `b` change the same flag or category (of the messages both name).
fn same_change(a: &Op, b: &Op) -> bool {
    match (a, b) {
        (Op::SetFlag { flag: x, .. }, Op::SetFlag { flag: y, .. }) => x == y,
        (Op::SetKeyword { keyword: x, .. }, Op::SetKeyword { keyword: y, .. }) => x.eq_ignore_ascii_case(y),
        _ => false,
    }
}
