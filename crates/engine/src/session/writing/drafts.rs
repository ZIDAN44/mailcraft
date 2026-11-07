//! What Drafts says about the rest of the mail (issue #52): the messages a draft answers, marked
//! `[Draft]` in the list as Outlook marks a message whose reply was begun and left unsent.

use super::*;

/// Drafts read for the messages they answer, at most per account (a Drafts folder of years).
const MAX_DRAFTS_READ: usize = 5_000;

/// What [`Session::answered_by_drafts`] last found, and from which drafts: the list asks every
/// frame, and the set made again each time was thousands of strings a frame.
#[derive(Default)]
pub(crate) struct Answered(std::cell::RefCell<Option<(u64, Arc<HashSet<(String, String)>>)>>);

impl Session {
    /// The messages a draft answers, by account and Message-ID (without brackets): those the
    /// drafts in each account's Drafts answer (kept on this computer too, while a server can't
    /// take them), and those whose drafts are on the way there ([`Session::saving`]). Made again
    /// only when those drafts changed.
    pub fn answered_by_drafts(&self) -> Arc<HashSet<(String, String)>> {
        use std::hash::{Hash, Hasher};
        let drafts: Vec<(&str, &FolderState)> = self
            .accounts
            .iter()
            .filter_map(|a| {
                let path = &a.folder(FolderRole::Drafts)?.path;
                self.cache.get(&(a.config.id.clone(), path.clone())).map(|st| (a.config.id.as_str(), st))
            })
            .collect();
        // Which drafts they are: a message's in-reply-to never changes under its UID.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (account, st) in &drafts {
            (account, st.validity, st.messages.len()).hash(&mut h);
            for m in st.messages.iter().take(MAX_DRAFTS_READ) {
                m.key.uid.hash(&mut h);
            }
        }
        for c in &self.saving {
            (&c.account, &c.in_reply_to).hash(&mut h);
        }
        let key = h.finish();
        if let Ok(memo) = self.answered.0.try_borrow()
            && let Some((at, set)) = memo.as_ref()
            && *at == key
        {
            return set.clone();
        }
        let mut out = HashSet::new();
        let mut add = |account: &str, in_reply_to: &str| {
            let id = mailcraft_model::thread::norm_id(in_reply_to);
            if !id.is_empty() {
                out.insert((account.to_string(), id));
            }
        };
        for (account, st) in &drafts {
            for m in st.messages.iter().take(MAX_DRAFTS_READ) {
                add(account, &m.in_reply_to);
            }
        }
        for c in &self.saving {
            add(&c.account, &c.in_reply_to);
        }
        let set = Arc::new(out);
        if let Ok(mut memo) = self.answered.0.try_borrow_mut() {
            *memo = Some((key, set.clone()));
        }
        set
    }

    /// Have the account's Drafts read, for the messages its drafts answer: when its folders are
    /// known, and kept read while the cache lets other folders go ([`Session::trim_cache`]).
    pub(crate) fn read_drafts(&mut self, account: &str) {
        let Some(path) = self.account(account).and_then(|a| a.folder(FolderRole::Drafts)).map(|f| f.path.clone()) else { return };
        if let Err(e) = self.load_cached(account, &path) {
            log::info!("{account}: Drafts not read yet for its replies: {e}");
        }
    }

    /// Whether `k` is an account's Drafts.
    pub(crate) fn is_drafts(&self, k: &(String, String)) -> bool {
        self.folder(&k.0, &k.1).is_some_and(|f| f.role == FolderRole::Drafts)
    }
}
