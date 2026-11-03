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
/// Summaries' texts a tick makes ahead of any search ([`Session::make_texts_ahead`]): about a
/// millisecond's worth of a real file's summaries.
pub(crate) const AHEAD: usize = 256;
/// Room in [`TEXT_BYTES`] texts made ahead leave: they stop short of it and are never trimmed
/// for, so they push out no texts a search made; and the folders they're made for count as used
/// least lately, so a search needing room lets them go before any it used.
const AHEAD_ROOM: usize = 2 << 20;

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
    pub(crate) fn texts(
        &mut self,
        account: &str,
        folder: &str,
        messages: &[SharedSummary],
        range: std::ops::Range<usize>,
        make: usize,
    ) -> &[SummaryText] {
        let k = (account.to_string(), folder.to_string());
        let start = range.start;
        let end = self.make(&k, messages, range, make, false);
        let start = start.min(end);
        self.folders.get(&k).and_then(|ft| ft.texts.get(start..end)).unwrap_or_default()
    }

    /// Up to `make` (at least one) of a folder's texts made, from where they're kept to, ahead of
    /// any search: the folder isn't counted as used, and they stop short of [`AHEAD_ROOM`] (one
    /// bigger than that room alone may go past it, with nothing trimmed for it).
    fn ahead(&mut self, k: &(String, String), messages: &[SharedSummary], make: usize) {
        self.make(k, messages, 0..messages.len(), make, true);
    }

    /// The texts of `range` made (up to `make`), for a search or `ahead`: where those made end.
    fn make(&mut self, k: &(String, String), messages: &[SharedSummary], range: std::ops::Range<usize>, make: usize, ahead: bool) -> usize {
        if !ahead {
            self.tick = self.tick.wrapping_add(1);
        }
        let ft = self.folders.entry(k.clone()).or_default();
        if !ahead {
            ft.used = self.tick;
        }
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
        let room = TEXT_BYTES.saturating_sub(AHEAD_ROOM);
        for m in messages.get(ft.texts.len()..end).unwrap_or_default() {
            if ahead && self.bytes.saturating_add(added) >= room {
                break;
            }
            let st = SummaryText::of(m);
            added = added.saturating_add(st.bytes());
            ft.uids.push(m.key.uid);
            ft.texts.push(st);
            self.made = self.made.wrapping_add(1);
        }
        ft.bytes = ft.bytes.saturating_add(added);
        let made = ft.texts.len().min(end);
        self.bytes = self.bytes.saturating_add(added);
        if !ahead && self.bytes > TEXT_BYTES {
            self.trim(k);
        }
        made
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

    /// How many of a folder's texts are kept, from its list's start (asked of every folder each
    /// frame: its key is borrowed, nothing allocated).
    fn made_of(&self, k: &(String, String)) -> usize {
        self.folders.get(k).map_or(0, |ft| ft.texts.len())
    }

    /// Whether texts made ahead still fit, pushing none out.
    fn room_ahead(&self) -> bool {
        self.bytes.saturating_add(AHEAD_ROOM) < TEXT_BYTES
    }
}

impl Session {
    /// Make summaries' texts ahead of any search, [`AHEAD`] a tick, while searches run in the
    /// background and none is running: a search in a session just opened made every summary's
    /// text as it went (on a real file, 110 ms of a word's 125-140 ms the first time). Made in
    /// the ticks after the window opens instead (about 140 frames there), within the room texts
    /// are kept in, the first search finds them made, as the index is opened ahead.
    pub(crate) fn make_texts_ahead(&mut self) {
        if !self.texts_ahead_allowed() {
            return;
        }
        let texts = &mut self.search_state.texts;
        let next = self.cache.iter().filter(|(k, st)| texts.made_of(k) < st.messages.len()).min_by(|x, y| x.0.cmp(y.0));
        if let Some((k, st)) = next {
            texts.ahead(k, &st.messages, AHEAD);
        }
    }

    /// Whether a tick still has summaries' texts to make ahead of any search: the window asks for
    /// frames meanwhile.
    pub fn texts_ahead_due(&self) -> bool {
        let texts = &self.search_state.texts;
        self.texts_ahead_allowed() && self.cache.iter().any(|(k, st)| texts.made_of(k) < st.messages.len())
    }

    /// Whether texts may be made ahead now: searching in the background, none running, and room.
    fn texts_ahead_allowed(&self) -> bool {
        self.search_state.background && !self.search_state.run.as_ref().is_some_and(|r| !r.done()) && self.search_state.texts.room_ahead()
    }
}

#[cfg(test)]
mod tests;
