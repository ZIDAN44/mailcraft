//! The session: the user's own replies in a conversation (issue #148), as Outlook's conversations
//! show the messages from other folders: those in Sent Items that answer one of the
//! conversation's messages, or that one of them answers, found by their Message-IDs.

use std::collections::HashMap;

use mailcraft_model::SharedSummary;
use mailcraft_model::thread::norm_id;

use super::*;

/// The newest of a Sent folder's messages looked through at most.
const INDEXED_MAX: usize = 50_000;
/// Sent messages shown in one conversation at most.
const ASIDE_MAX: usize = 100;

/// An account's Sent folder, looked up by Message-ID: each message by its own, and by those it
/// answers (In-Reply-To and References).
#[derive(Default)]
pub(crate) struct SentIndex {
    /// What it was made from: the folder's message count and its newest key, to know when the
    /// folder changed.
    made_of: (usize, Option<MessageKey>),
    messages: Vec<SharedSummary>,
    own: HashMap<String, usize>,
    answering: HashMap<String, Vec<usize>>,
}

impl SentIndex {
    fn of(messages: &[SharedSummary]) -> SentIndex {
        let messages: Vec<SharedSummary> = messages.iter().take(INDEXED_MAX).cloned().collect();
        let mut own = HashMap::new();
        let mut answering: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, m) in messages.iter().enumerate() {
            let id = norm_id(&m.message_id);
            if !id.is_empty() {
                own.entry(id).or_insert(i);
            }
            for r in m.references.iter().chain(std::iter::once(&m.in_reply_to)).map(|r| norm_id(r)).filter(|r| !r.is_empty()) {
                let at = answering.entry(r).or_default();
                if !at.contains(&i) && at.len() < ASIDE_MAX {
                    at.push(i);
                }
            }
        }
        SentIndex { made_of: (0, None), messages, own, answering }
    }
}

impl Session {
    /// The Sent Items messages of `key`'s conversation (its other messages here, `thread`) that
    /// aren't in the list: shown with it, the oldest first. None when conversations are off, in a
    /// search, or in a folder the user's own mail is already in (Sent Items, Drafts, Junk, the
    /// Outbox).
    pub(crate) fn sent_aside(&mut self, key: &MessageKey, thread: &[MessageKey]) -> Vec<SharedSummary> {
        if !self.prefs.conversations || self.search.is_some() {
            return Vec::new();
        }
        let role = self.folder(&key.account, &key.folder).map(|f| f.role);
        if key.folder == OUTBOX || matches!(role, Some(FolderRole::Sent | FolderRole::Drafts | FolderRole::Junk)) {
            return Vec::new();
        }
        let Some(sent) = self.account(&key.account).and_then(|a| a.folder(FolderRole::Sent)).map(|f| f.path.clone()) else {
            return Vec::new();
        };
        // The conversation as the list has it: its own Message-IDs, and those it answers.
        let shared = |k: &MessageKey| self.summary(k).map(|m| std::sync::Arc::new(m.clone()));
        let members: Vec<SharedSummary> =
            if thread.is_empty() { shared(key).into_iter().collect() } else { thread.iter().filter_map(shared).collect() };
        let own: Vec<String> = members.iter().map(|m| norm_id(&m.message_id)).filter(|i| !i.is_empty()).collect();
        let answered: Vec<String> = members
            .iter()
            .flat_map(|m| m.references.iter().chain(std::iter::once(&m.in_reply_to)).map(|r| norm_id(r)).collect::<Vec<_>>())
            .filter(|r| !r.is_empty())
            .collect();
        let index = self.sent_index(&key.account, &sent);
        let mut found: Vec<usize> = Vec::new();
        for i in answered.iter().filter_map(|r| index.own.get(r).copied()).chain(own.iter().filter_map(|i| index.answering.get(i)).flatten().copied())
        {
            if !found.contains(&i) {
                found.push(i);
            }
        }
        let mut out: Vec<SharedSummary> = found
            .into_iter()
            .filter_map(|i| index.messages.get(i).cloned())
            // The same message here as well (Bcc to oneself): shown once, here.
            .filter(|m| !own.contains(&norm_id(&m.message_id)))
            .take(ASIDE_MAX)
            .collect();
        out.sort_by_key(|m| m.when());
        out
    }

    /// The account's Sent folder looked up by Message-ID: made again once the folder changed.
    fn sent_index(&mut self, account: &str, sent: &str) -> &SentIndex {
        let made_of = {
            let st = self.folder_state(account, sent);
            (st.messages.len(), st.messages.first().map(|m| m.key.clone()))
        };
        let stale = self.sent_indexes.get(account).is_none_or(|x| x.made_of != made_of);
        if stale {
            let messages = self.folder_state(account, sent).messages.clone();
            let mut index = SentIndex::of(&messages);
            index.made_of = made_of;
            self.sent_indexes.insert(account.to_string(), index);
        }
        self.sent_indexes.entry(account.to_string()).or_default()
    }
}
