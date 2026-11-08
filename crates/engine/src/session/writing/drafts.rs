//! What Drafts says about the rest of the mail (issue #52): the messages a draft answers or
//! forwards, marked `[Draft]` in the list as Outlook marks a message whose reply or forward was
//! begun and left unsent.

use super::*;

/// Drafts read for the messages they answer, at most per account (a Drafts folder of years).
const MAX_DRAFTS_READ: usize = 5_000;

/// The messages a draft was begun to ([`Session::drafted`]).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Drafted {
    /// Those a draft answers, by account and Message-ID (without brackets): a reply names the
    /// message it answers by its In-Reply-To.
    answered: HashSet<(String, String)>,
    /// Those a draft forwards, by their place: a forward names it only in its own copy
    /// ([`mime::SOURCE_HEADER`], [`Summary::forward_of`]).
    forwarded: HashSet<MessageKey>,
}

impl Drafted {
    /// Whether a draft answers or forwards `m`: the list marks it `[Draft]`.
    pub fn marks(&self, m: &Summary) -> bool {
        self.forwarded.contains(&m.key) || {
            let id = mailcraft_model::thread::norm_id(&m.message_id);
            !id.is_empty() && self.answered.contains(&(m.key.account.clone(), id))
        }
    }

    /// Whether a draft answers the message with Message-ID `id` in `account`.
    pub fn answers(&self, account: &str, id: &str) -> bool {
        self.answered.contains(&(account.to_string(), mailcraft_model::thread::norm_id(id)))
    }

    /// Whether a draft forwards the message at `key`.
    pub fn forwards(&self, key: &MessageKey) -> bool {
        self.forwarded.contains(key)
    }

    fn add_reply(&mut self, account: &str, in_reply_to: &str) {
        let id = mailcraft_model::thread::norm_id(in_reply_to);
        if !id.is_empty() {
            self.answered.insert((account.to_string(), id));
        }
    }
}

/// What [`Session::drafted`] last found, and from which drafts: the list asks every frame, and
/// the set made again each time was thousands of strings a frame.
#[derive(Default)]
pub(crate) struct DraftedMemo(std::cell::RefCell<Option<(u64, Arc<Drafted>)>>);

impl Session {
    /// The messages a draft answers or forwards: those the drafts in each account's Drafts were
    /// begun to (kept on this computer too, while a server can't take them), and those whose
    /// drafts are on the way there ([`Session::saving`]). Made again only when those drafts
    /// changed.
    pub fn drafted(&self) -> Arc<Drafted> {
        use std::hash::{Hash, Hasher};
        let drafts: Vec<(&str, &FolderState)> = self
            .accounts
            .iter()
            .filter_map(|a| {
                let path = &a.folder(FolderRole::Drafts)?.path;
                self.cache.get(&(a.config.id.clone(), path.clone())).map(|st| (a.config.id.as_str(), st))
            })
            .collect();
        // Which drafts they are: what a message answers or forwards never changes under its UID.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (account, st) in &drafts {
            (account, st.validity, st.messages.len()).hash(&mut h);
            for m in st.messages.iter().take(MAX_DRAFTS_READ) {
                m.key.uid.hash(&mut h);
            }
        }
        for c in &self.saving {
            (&c.account, &c.in_reply_to, &c.source, c.mode == Some(ReplyMode::Forward)).hash(&mut h);
        }
        let key = h.finish();
        if let Ok(memo) = self.drafted.0.try_borrow()
            && let Some((at, found)) = memo.as_ref()
            && *at == key
        {
            return found.clone();
        }
        let mut out = Drafted::default();
        for (account, st) in &drafts {
            for m in st.messages.iter().take(MAX_DRAFTS_READ) {
                out.add_reply(account, &m.in_reply_to);
                if let Some(k) = MessageKey::from_id(&m.forward_of) {
                    out.forwarded.insert(k);
                }
            }
        }
        for c in &self.saving {
            out.add_reply(&c.account, &c.in_reply_to);
            if let (Some(k), Some(ReplyMode::Forward)) = (&c.source, c.mode) {
                out.forwarded.insert(k.clone());
            }
        }
        let found = Arc::new(out);
        if let Ok(mut memo) = self.drafted.0.try_borrow_mut() {
            *memo = Some((key, found.clone()));
        }
        found
    }

    /// Have the account's Drafts read, for the messages its drafts answer or forward: when its
    /// folders are known, and kept read while the cache lets other folders go
    /// ([`Session::trim_cache`]).
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
