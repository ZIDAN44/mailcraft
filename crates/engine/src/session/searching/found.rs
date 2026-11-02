//! Search: a message as a run's sets hold it (found, let go of, to check again, missing, cut
//! short, what its text said): its folder by its place in the run's folders, and its validity
//! and number there, no strings. Keyed by its `MessageKey`, each message recorded copied its
//! account's and folder's names into each set it went into, and hashed them: most of what a
//! step that records thousands found cost. A key is made only where something asks for one.

use super::super::mailboxes::{folder_hash, key_hash, key_hash_in};
use super::run::SearchRun;
use super::*;

/// A message found and not shown yet, with its key's hash (the list's index is built of it),
/// made from its folder's: hashing each key's strings as the list took them was most of the
/// frame that showed tens of thousands.
#[derive(Clone, Debug)]
pub(crate) struct Fresh {
    pub(crate) hash: u64,
    pub(crate) m: SharedSummary,
}

impl Fresh {
    /// A message not of the run's folders (a server's find): its key hashed whole.
    pub(crate) fn of(m: SharedSummary) -> Fresh {
        Fresh { hash: key_hash(&m.key), m }
    }
}

/// Each of `folders`' [`folder_hash`].
pub(crate) fn folder_hashes(folders: &[(String, String)]) -> Vec<u64> {
    folders.iter().map(|(a, f)| folder_hash(a, f)).collect()
}

/// A message of one of a run's folders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct RunKey {
    /// Its folder: an index into [`SearchRun::folders`].
    fi: u32,
    validity: u32,
    uid: u32,
}

impl RunKey {
    /// `key`, of the run's folder `fi`.
    pub(crate) fn of(fi: usize, key: &MessageKey) -> RunKey {
        RunKey { fi: u32::try_from(fi).unwrap_or(u32::MAX), validity: key.validity, uid: key.uid }
    }

    /// Its folder: an index into [`SearchRun::folders`].
    pub(crate) fn folder(self) -> usize {
        usize::try_from(self.fi).unwrap_or(usize::MAX)
    }

    pub(crate) fn uid(self) -> u32 {
        self.uid
    }
}

impl SearchRun {
    /// `key` as the run's sets hold it, when it's in one of the run's folders.
    pub(crate) fn run_key(&self, key: &MessageKey) -> Option<RunKey> {
        let fi = self.folders.iter().position(|(a, f)| *a == key.account && *f == key.folder)?;
        Some(RunKey::of(fi, key))
    }

    /// The message `k` is.
    pub(crate) fn message_key(&self, k: RunKey) -> Option<MessageKey> {
        let (account, folder) = self.folders.get(k.folder())?;
        Some(MessageKey { account: account.clone(), folder: folder.clone(), validity: k.validity, uid: k.uid })
    }

    /// `m`, of the run's folder `fi`, found.
    pub(crate) fn fresh(&self, fi: usize, m: SharedSummary) -> Fresh {
        let hash = match self.folder_hashes.get(fi) {
            Some(&f) => key_hash_in(f, m.key.validity, m.key.uid),
            None => key_hash(&m.key),
        };
        Fresh { hash, m }
    }

    /// Whether the run found `key`.
    pub(crate) fn has_found(&self, key: &MessageKey) -> bool {
        !self.found.is_empty() && self.run_key(key).is_some_and(|k| self.found.contains(&k))
    }
}
