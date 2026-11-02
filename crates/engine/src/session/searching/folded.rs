//! Search: the texts of messages checked for a phrase, folded, kept between searches.
//!
//! A phrase is settled against each candidate's text as the index keeps it: read (a page of
//! texts at a time), then folded a slice at a time, which was nearly all a phrase search cost
//! (0.6 s reading and 0.5 s folding for one of the 41 GB file). Typing the phrase on, or
//! searching it again, checks the same messages: their texts are kept folded here, by message
//! (a message's text never changes), and checked without reading or folding anything.
//!
//! What a search checks stays while it runs: when the texts kept reach [`FOLDED_BYTES`], those
//! no search used since the one running go; if that's not enough room, a text isn't kept (a
//! search through more than fits would otherwise let go of each text before it came round to
//! it again).

use std::sync::Arc;

use super::*;

/// Most bytes of folded text kept, all accounts told.
pub(crate) const FOLDED_BYTES: usize = 32 << 20;

/// A message's text as the index keeps it, folded.
#[derive(Debug, Default)]
pub(crate) struct Folded {
    /// Its files' names, as kept.
    pub(crate) names: Vec<String>,
    /// Its text's slices ([`VERIFY_BYTES`](super::matching::VERIFY_BYTES) each, overlapping as
    /// they're checked), each folded: a phrase is looked for in each, as in the text's.
    pub(crate) slices: Vec<String>,
}

impl Folded {
    fn bytes(&self) -> usize {
        let names: usize = self.names.iter().map(|n| n.len().saturating_add(24)).sum();
        let slices: usize = self.slices.iter().map(|s| s.len().saturating_add(24)).sum();
        names.saturating_add(slices).saturating_add(std::mem::size_of::<Folded>())
    }
}

/// The folded texts kept, by message.
#[derive(Debug, Default)]
pub(crate) struct FoldedTexts {
    texts: HashMap<MessageKey, (u64, Arc<Folded>)>,
    bytes: usize,
    /// Which search is running (each starts the next).
    search: u64,
}

impl FoldedTexts {
    /// Another search starts: what it doesn't use may go to make room.
    pub(crate) fn begin(&mut self) {
        self.search = self.search.wrapping_add(1);
    }

    /// `key`'s text, folded, when it's kept.
    pub(crate) fn get(&mut self, key: &MessageKey) -> Option<Arc<Folded>> {
        let (used, f) = self.texts.get_mut(key)?;
        *used = self.search;
        Some(f.clone())
    }

    /// Keep `key`'s text folded, room allowing.
    pub(crate) fn keep(&mut self, key: &MessageKey, folded: Folded) {
        let size = folded.bytes();
        if size > FOLDED_BYTES {
            return;
        }
        if self.bytes.saturating_add(size) > FOLDED_BYTES {
            let search = self.search;
            let mut gone = 0usize;
            self.texts.retain(|_, (used, f)| {
                let keep = *used == search;
                if !keep {
                    gone = gone.saturating_add(f.bytes());
                }
                keep
            });
            self.bytes = self.bytes.saturating_sub(gone);
            if self.bytes.saturating_add(size) > FOLDED_BYTES {
                return;
            }
        }
        if let Some((_, old)) = self.texts.insert(key.clone(), (self.search, Arc::new(folded))) {
            self.bytes = self.bytes.saturating_sub(old.bytes());
        }
        self.bytes = self.bytes.saturating_add(size);
    }

    /// Keep only the texts of accounts `keep` keeps.
    pub(crate) fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        let mut gone = 0usize;
        self.texts.retain(|k, (_, f)| {
            let kept = keep(&k.account);
            if !kept {
                gone = gone.saturating_add(f.bytes());
            }
            kept
        });
        self.bytes = self.bytes.saturating_sub(gone);
    }

    /// Bytes kept.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
}
