//! Search: the texts of messages checked for a phrase, folded, kept between searches.
//!
//! A phrase is settled against each candidate's text as the index keeps it: read (a page of
//! texts at a time), then folded a slice at a time, which was nearly all a phrase search cost
//! (0.6 s reading and 0.5 s folding for one of the 41 GB file). Typing the phrase on, or
//! searching it again, checks the same messages: their texts are kept folded here, by message,
//! and checked without reading or folding anything.
//!
//! A key can come back as another message (a local folder deleted and made again numbers its
//! mail from 1 again; the index then takes it for another doc), so each text is kept with the
//! doc it was read from, in the index's epoch: a text whose doc isn't the candidate's any more
//! isn't used, and goes. A folder's texts go when the folder leaves the listing.
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

/// The doc a text was read from: its id in the index's epoch (another epoch numbers docs again,
/// and a message the index takes for another gets another id).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextOf {
    pub(crate) epoch: u64,
    pub(crate) doc: u32,
}

/// The folded texts kept, by message: the search that used each last, and the doc it's of.
#[derive(Debug, Default)]
pub(crate) struct FoldedTexts {
    texts: HashMap<MessageKey, (u64, TextOf, Arc<Folded>)>,
    bytes: usize,
    /// Which search is running (each starts the next).
    search: u64,
}

impl FoldedTexts {
    /// Another search starts: what it doesn't use may go to make room.
    pub(crate) fn begin(&mut self) {
        self.search = self.search.wrapping_add(1);
    }

    /// `key`'s text, folded, when it's kept and was read from `of`. One read from another doc
    /// was another message's: it goes.
    pub(crate) fn get(&mut self, key: &MessageKey, of: TextOf) -> Option<Arc<Folded>> {
        let (used, was, f) = self.texts.get_mut(key)?;
        if *was != of {
            if let Some((_, _, old)) = self.texts.remove(key) {
                self.bytes = self.bytes.saturating_sub(old.bytes());
            }
            return None;
        }
        *used = self.search;
        Some(f.clone())
    }

    /// Keep `key`'s text folded, as read from `of`, room allowing.
    pub(crate) fn keep(&mut self, key: &MessageKey, of: TextOf, folded: Folded) {
        let size = folded.bytes();
        if size > FOLDED_BYTES {
            return;
        }
        if self.bytes.saturating_add(size) > FOLDED_BYTES {
            let search = self.search;
            let mut gone = 0usize;
            self.texts.retain(|_, (used, _, f)| {
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
        if let Some((_, _, old)) = self.texts.insert(key.clone(), (self.search, of, Arc::new(folded))) {
            self.bytes = self.bytes.saturating_sub(old.bytes());
        }
        self.bytes = self.bytes.saturating_add(size);
    }

    /// Keep only the texts of the folders `keep` keeps (account, folder).
    pub(crate) fn retain(&mut self, keep: impl Fn(&str, &str) -> bool) {
        let mut gone = 0usize;
        self.texts.retain(|k, (_, _, f)| {
            let kept = keep(&k.account, &k.folder);
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
