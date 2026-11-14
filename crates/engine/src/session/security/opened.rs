//! What was decrypted of messages, held in memory only, in the order it came.

use std::collections::VecDeque;

use super::*;

/// The most decrypted messages held in memory at once.
pub(super) const OPENED_MAX: usize = 64;

/// Decrypted messages: for each, the MIME entity inside and whether it was authenticated, and
/// the order they were decrypted in (oldest first), for the oldest to be let go first.
#[derive(Debug, Default)]
pub(crate) struct Opened {
    held: HashMap<MessageKey, (Vec<u8>, bool)>,
    order: VecDeque<MessageKey>,
}

impl Opened {
    /// What was decrypted of `key`, and whether it was authenticated.
    pub(crate) fn get(&self, key: &MessageKey) -> Option<&(Vec<u8>, bool)> {
        self.held.get(key)
    }

    /// `key`'s let go of.
    pub(crate) fn remove(&mut self, key: &MessageKey) {
        if self.held.remove(key).is_some() {
            self.order.retain(|k| k != key);
        }
    }

    /// Those `keep` says no to let go of.
    pub(crate) fn retain(&mut self, keep: impl Fn(&MessageKey) -> bool) {
        self.held.retain(|k, _| keep(k));
        self.order.retain(|k| keep(k));
    }

    /// Hold what was decrypted of `key`, as the newest. Past [`OPENED_MAX`] the oldest one that
    /// isn't `shown` goes first (none goes while every one is), and is given back.
    pub(super) fn keep(&mut self, key: &MessageKey, entity: Vec<u8>, authenticated: bool, shown: impl Fn(&MessageKey) -> bool) -> Option<MessageKey> {
        let mut gone = None;
        if self.held.contains_key(key) {
            self.order.retain(|k| k != key);
        } else if self.held.len() >= OPENED_MAX
            && let Some(at) = self.order.iter().position(|k| !shown(k))
            && let Some(k) = self.order.remove(at)
        {
            self.held.remove(&k);
            gone = Some(k);
        }
        self.held.insert(key.clone(), (entity, authenticated));
        self.order.push_back(key.clone());
        gone
    }
}
