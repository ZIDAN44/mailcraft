//! Search: each folder's summaries' text as search reads it, kept between searches.
//!
//! Folding a summary's subject, senders, recipients and preview is most of what a search of the
//! summaries costs; kept per folder, typing the next letter (or searching again from another
//! view) folds nothing again. Kept beside the folder's list, message for message, and checked
//! against it at each use: a list that changed under it (a message in or out) has its texts
//! made again from there, so a missed invalidation can't give a stale answer. A summary that
//! changed in place (its preview filled in by its download) keeps its uid at its place, which that
//! check can't see: its text is made again then ([`FolderTexts::changed`]). Flags aren't text: a
//! mark-read keeps them.

use super::*;

/// Most bytes of summary text kept, all folders told; the least recently used go first.
pub(crate) const TEXT_BYTES: usize = 48 << 20;

/// One folder's texts, aligned with its list.
#[derive(Debug, Default)]
struct FolderText {
    uids: Vec<u32>,
    texts: Vec<SummaryText>,
    bytes: usize,
    used: u64,
}

/// Each folder's summary texts, by (account, folder).
#[derive(Debug, Default)]
pub(crate) struct FolderTexts {
    folders: HashMap<(String, String), FolderText>,
    bytes: usize,
    tick: u64,
    /// Summary texts made so far.
    made: u64,
}

impl FolderTexts {
    /// The texts of `messages[range]` (the folder's list as it is now), made for those not kept
    /// or no longer the same message. A range past what's kept makes what lies between too.
    /// At most `make` are made (at least one): the texts given may stop short of the range's
    /// end, where they do.
    pub(crate) fn texts(&mut self, account: &str, folder: &str, messages: &[Summary], range: std::ops::Range<usize>, make: usize) -> &[SummaryText] {
        self.tick = self.tick.wrapping_add(1);
        let k = (account.to_string(), folder.to_string());
        let ft = self.folders.entry(k.clone()).or_default();
        ft.used = self.tick;
        // Kept only while each is still the message at its place.
        let mut keep = ft.uids.len().min(messages.len());
        if let Some(i) = ft.uids.iter().zip(messages).position(|(u, m)| *u != m.key.uid) {
            keep = keep.min(i);
        }
        if keep < ft.texts.len() {
            let gone: usize = ft.texts.get(keep..).unwrap_or_default().iter().map(SummaryText::bytes).sum();
            ft.texts.truncate(keep);
            ft.uids.truncate(keep);
            ft.bytes = ft.bytes.saturating_sub(gone);
            self.bytes = self.bytes.saturating_sub(gone);
        }
        let most = ft.texts.len().saturating_add(make).max(range.start.saturating_add(1));
        let end = range.end.min(messages.len()).min(most);
        let mut added = 0usize;
        for m in messages.get(ft.texts.len()..end).unwrap_or_default() {
            let st = SummaryText::of(m);
            added = added.saturating_add(st.bytes());
            ft.uids.push(m.key.uid);
            ft.texts.push(st);
            self.made = self.made.wrapping_add(1);
        }
        ft.bytes = ft.bytes.saturating_add(added);
        self.bytes = self.bytes.saturating_add(added);
        if self.bytes > TEXT_BYTES {
            self.trim(&k);
        }
        let start = range.start.min(end);
        self.folders.get(&k).and_then(|ft| ft.texts.get(start..end)).unwrap_or_default()
    }

    /// Let the least recently used folders' texts go until they fit, never `keep`'s.
    fn trim(&mut self, keep: &(String, String)) {
        while self.bytes > TEXT_BYTES {
            let Some(oldest) = self.folders.iter().filter(|(k, _)| *k != keep).min_by_key(|(_, ft)| ft.used).map(|(k, _)| k.clone()) else {
                break;
            };
            if let Some(ft) = self.folders.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(ft.bytes);
            }
        }
    }

    /// The text kept of `key`'s summary, at `at` in its folder's list, when it's still kept there
    /// (a list that changed since has it made again).
    pub(crate) fn kept(&self, key: &MessageKey, at: usize) -> Option<&SummaryText> {
        let ft = self.folders.get(&(key.account.clone(), key.folder.clone()))?;
        if ft.uids.get(at) != Some(&key.uid) {
            return None;
        }
        ft.texts.get(at)
    }

    /// `m`'s summary changed in place (its preview filled in by its download): the text kept for
    /// it is made again, the folder's others kept.
    pub(crate) fn changed(&mut self, m: &Summary) {
        let Some(ft) = self.folders.get_mut(&(m.key.account.clone(), m.key.folder.clone())) else { return };
        let Some(at) = ft.uids.iter().position(|u| *u == m.key.uid) else { return };
        let Some(slot) = ft.texts.get_mut(at) else { return };
        let st = SummaryText::of(m);
        let (was, now) = (slot.bytes(), st.bytes());
        *slot = st;
        ft.bytes = ft.bytes.saturating_sub(was).saturating_add(now);
        self.bytes = self.bytes.saturating_sub(was).saturating_add(now);
        self.made = self.made.wrapping_add(1);
    }

    /// A folder's list was read again or changed: its texts go.
    pub(crate) fn forget(&mut self, account: &str, folder: &str) {
        if let Some(ft) = self.folders.remove(&(account.to_string(), folder.to_string())) {
            self.bytes = self.bytes.saturating_sub(ft.bytes);
        }
    }

    /// Keep only the folders `keep` keeps.
    pub(crate) fn retain(&mut self, keep: impl Fn(&str, &str) -> bool) {
        let mut gone = 0usize;
        self.folders.retain(|(a, f), ft| {
            let kept = keep(a, f);
            if !kept {
                gone = gone.saturating_add(ft.bytes);
            }
            kept
        });
        self.bytes = self.bytes.saturating_sub(gone);
    }

    /// Summary texts made so far (what keeping them saves).
    pub(crate) fn made(&self) -> u64 {
        self.made
    }
}
