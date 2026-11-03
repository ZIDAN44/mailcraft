//! A phrase settled from its words' places ([`Reader::scan_phrase`]), a few pages a call: no text
//! is read for a doc whose places tell.
//!
//! Segment by segment, the places of the phrase's first word are taken for each doc asked about
//! (as where the phrase may start), then each next word's keep those it follows at the next
//! place; the last word is a prefix, so the places of every word starting so are looked through.
//! A doc's words are all in one segment, so a doc no segment finds the phrase in hasn't it.
//!
//! What the places can't tell is left to the text: a word of the phrase too long to index (or with
//! a space folded into it), and a long word (one too long to index) just where the last word would
//! be, which may start with it. A word without places (a segment from before they were kept, or
//! one damaged) leaves its docs to the text too.

use std::collections::BTreeMap;

use mailcraft_model::search::MAX_TOKEN_CHARS;

use super::super::format::{PART, PlacesStream, decode_places, decode_pos_page, long_key};
use super::*;

/// What a phrase's places said: the docs that hold it, and those they can't tell of.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhraseFound {
    yes: Bits,
    unsure: Bits,
}

impl PhraseFound {
    /// Whether `doc` (one with every word of the phrase, as each was looked up) holds the phrase;
    /// `None` when its places can't tell (its text must).
    pub fn has(&self, doc: u32) -> Option<bool> {
        if self.yes.get(doc) {
            Some(true)
        } else if self.unsure.get(doc) {
            None
        } else {
            Some(false)
        }
    }
}

/// Where a [`Reader::scan_phrase`] got to, and what it found so far.
#[derive(Debug, Default)]
pub struct PhraseScan {
    found: PhraseFound,
    begun: bool,
    done: bool,
    /// The highest doc asked about: a word's places past it aren't read.
    last: u32,
    seg: usize,
    stage: Stage,
    /// In the segment under way: each doc that may hold the phrase, and where it may start in it
    /// (the places of its first word, as the words after it go on keeping them).
    starts: BTreeMap<u32, Vec<u32>>,
    /// Those the word under way keeps.
    next: BTreeMap<u32, Vec<u32>>,
    /// The word found whose places are read next.
    entry: Option<LexEntry>,
    stream: Option<Stream>,
}

impl PhraseScan {
    /// What it found (all of it, once it's done).
    pub fn found(&self) -> &PhraseFound {
        &self.found
    }

    pub fn into_found(self) -> PhraseFound {
        self.found
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stage {
    /// A segment to begin.
    #[default]
    Segment,
    /// The phrase's word `n` (not its last), whole.
    Word(usize),
    /// The words starting with its last word: the lexicon page and the entry next looked at.
    Last { page: Option<u32>, entry: usize },
    /// Its field's long words.
    Long,
}

/// A word's places being read, a page at a time.
#[derive(Debug)]
struct Stream {
    docs: Vec<u32>,
    places: PlacesStream,
    page: u32,
    offset: usize,
}

/// Whether a phrase's words can be told from places: each a word the index keeps whole.
fn placeable(field: Field, words: &[String]) -> bool {
    !words.is_empty()
        && words.iter().all(|w| !w.is_empty() && !w.contains(' ') && w.chars().count() <= MAX_TOKEN_CHARS && key(field, w).len() <= MAX_KEY)
}

impl Reader {
    /// Settle the phrase `words` (folded, the last a word's start) of `field` for the docs of
    /// `within` (those with every word of it), from its words' places: reading about `max_pages`
    /// pages a call, at least one, `true` once done, `false` to be called again with the same
    /// scan. Then [`PhraseScan::found`] says, for each doc of `within`, whether it holds it, or
    /// that its text must tell.
    pub fn scan_phrase(
        &mut self,
        store: &Store,
        field: Field,
        words: &[String],
        within: &Bits,
        scan: &mut PhraseScan,
        max_pages: usize,
    ) -> Result<bool> {
        if scan.done {
            return Ok(true);
        }
        if !scan.begun {
            scan.begun = true;
            scan.found = PhraseFound { yes: Bits::new(self.len()), unsure: Bits::new(self.len()) };
            scan.last = within.ones().last().unwrap_or(0);
            if !placeable(field, words) {
                scan.found.unsure = within.clone();
                scan.done = true;
                return Ok(true);
            }
        }
        let n = words.len();
        let mut read = 0usize;
        loop {
            if read >= max_pages.max(1) {
                return Ok(false);
            }
            let Some((info, _)) = self.heads.get(scan.seg) else {
                scan.done = true;
                return Ok(true);
            };
            let info = *info;
            match scan.stage {
                Stage::Segment => {
                    scan.stage = match within.any_in(info.lo, info.hi) {
                        false => {
                            scan.seg += 1;
                            Stage::Segment
                        }
                        true if n > 1 => Stage::Word(0),
                        true => Stage::Last { page: None, entry: 0 },
                    };
                }
                // A word found: its docs read, and its places to read (each a unit of its own).
                _ if scan.stream.is_none() && scan.entry.is_some() => {
                    let Some(e) = scan.entry.take() else { continue };
                    scan.stream = self.places_of(store, &info, &e, &mut read)?;
                    if scan.stream.is_none() {
                        unsure_in(scan, within, &info);
                        end_segment(scan);
                    }
                }
                Stage::Word(i) if scan.stream.is_none() => {
                    let word = words.get(i).map_or("", String::as_str);
                    scan.entry = self.entry(store, &info, &key(field, word), &mut read)?;
                    // No doc of this segment has it: none here holds the phrase.
                    if scan.entry.is_none() {
                        end_segment(scan);
                    }
                }
                Stage::Word(i) => {
                    if self.pump(store, &info, scan, within, n, &mut read)? {
                        if i > 0 {
                            scan.starts = std::mem::take(&mut scan.next);
                        }
                        scan.stage = if i + 2 < n { Stage::Word(i + 1) } else { Stage::Last { page: None, entry: 0 } };
                        if scan.starts.is_empty() {
                            end_segment(scan);
                        }
                    }
                }
                Stage::Last { page, entry } if scan.stream.is_none() => {
                    let target = key(field, words.last().map_or("", String::as_str));
                    let Some((_, firsts)) = self.heads.get(scan.seg) else { continue };
                    // The last page starting at or before the prefix: its words may run into it.
                    let first = || firsts.partition_point(|f| f.as_slice() <= target.as_slice()).saturating_sub(1);
                    let page = page.unwrap_or_else(|| u32::try_from(first()).unwrap_or(u32::MAX));
                    if page >= info.lex_pages {
                        scan.stage = Stage::Long;
                        continue;
                    }
                    let decoded = self.lex_decoded;
                    let entries = self.lex_page(store, &info, page)?;
                    if self.lex_decoded != decoded {
                        read += 1;
                    }
                    let mut at = entry;
                    while entries.get(at).is_some_and(|e| e.key.as_slice() < target.as_slice()) {
                        at += 1;
                    }
                    scan.stage = match entries.get(at) {
                        None => Stage::Last { page: Some(page.saturating_add(1)), entry: 0 },
                        Some(e) if !e.key.starts_with(&target) => Stage::Long,
                        Some(e) => {
                            scan.entry = Some(e.clone());
                            Stage::Last { page: Some(page), entry: at.saturating_add(1) }
                        }
                    };
                }
                Stage::Last { .. } => {
                    self.pump(store, &info, scan, within, n, &mut read)?;
                }
                Stage::Long if scan.stream.is_none() => {
                    scan.entry = self.entry(store, &info, &long_key(field), &mut read)?;
                    if scan.entry.is_none() {
                        end_segment(scan);
                    }
                }
                Stage::Long => {
                    if self.pump(store, &info, scan, within, n, &mut read)? {
                        end_segment(scan);
                    }
                }
            }
        }
    }

    /// The entry of `key` in a segment, when it has it; `read` counts a lexicon page decoded.
    fn entry(&mut self, store: &Store, info: &SegInfo, key: &[u8], read: &mut usize) -> Result<Option<LexEntry>> {
        let Some((_, firsts)) = self.heads.iter().find(|(s, _)| s == info) else { return Ok(None) };
        let page = firsts.partition_point(|f| f.as_slice() <= key).saturating_sub(1);
        let page = u32::try_from(page).unwrap_or(u32::MAX);
        if page >= info.lex_pages {
            return Ok(None);
        }
        let decoded = self.lex_decoded;
        let entries = self.lex_page(store, info, page)?;
        if self.lex_decoded != decoded {
            *read += 1;
        }
        Ok(entries.binary_search_by(|e| e.key.as_slice().cmp(key)).ok().and_then(|i| entries.get(i)).cloned())
    }

    /// A word's places to read, its docs read first; `None` when it has none kept.
    fn places_of(&mut self, store: &Store, info: &SegInfo, e: &LexEntry, read: &mut usize) -> Result<Option<Stream>> {
        let Some(at) = e.pos else { return Ok(None) };
        let (docs, pages) = self.entry_docs(store, info, e)?;
        *read += pages;
        Ok(Some(Stream { docs, places: PlacesStream::new(at, e.df), page: at.page, offset: at.offset as usize }))
    }

    /// One page more of the word under way's places, each doc's block in it weighed: whether the
    /// word's places are all read.
    fn pump(&mut self, store: &Store, info: &SegInfo, scan: &mut PhraseScan, within: &Bits, n: usize, read: &mut usize) -> Result<bool> {
        let Some(mut st) = scan.stream.take() else { return Ok(true) };
        let path = Name::SegPos(info.id, st.page).path(&self.base);
        // Past the last doc that can tell anything more, the rest isn't read (a common word's
        // places, read whole, were most of a rare phrase's time).
        let stage = scan.stage;
        let Some(last) = last_wanted(scan, stage, n) else { return Ok(true) };
        let past = |st: &Stream| st.docs.get(st.places.next_index()).is_none_or(|d| *d > last);
        if past(&st) {
            return Ok(true);
        }
        if st.places.wants() > 0 {
            if st.page >= info.pos_pages {
                return Err(damaged(&path, "places past the segment"));
            }
            let raw = self.places.get(store, self.meta.epoch, &path)?;
            *read += 1;
            let data = decode_pos_page(&raw).map_err(|what| damaged(&path, what))?;
            let rest = data.get(st.offset..).filter(|r| !r.is_empty()).ok_or_else(|| damaged(&path, "places past the page"))?;
            st.places.feed(rest);
            (st.page, st.offset) = (st.page.saturating_add(1), 0);
        }
        while let Some((i, count, bytes)) = st.places.next_block().map_err(|what| damaged(&path, what))? {
            let doc = *st.docs.get(i).ok_or_else(|| damaged(&path, "places past the docs"))?;
            if doc > last {
                return Ok(true);
            }
            if !wanted(scan, stage, within, n, doc) {
                continue;
            }
            let places = decode_places(bytes, count).map_err(|what| damaged(&path, what))?;
            weigh(scan, stage, n, doc, &places);
        }
        if st.places.wants() > 0 {
            scan.stream = Some(st);
            return Ok(false);
        }
        st.places.finish().map_err(|what| damaged(&path, what))?;
        Ok(true)
    }
}

/// The highest doc whose places of the word under way can tell anything more; `None`: none can.
fn last_wanted(scan: &PhraseScan, stage: Stage, n: usize) -> Option<u32> {
    let started = || scan.starts.last_key_value().map(|(d, _)| *d);
    match stage {
        Stage::Segment => None,
        Stage::Word(0) => Some(scan.last),
        Stage::Word(_) => started(),
        Stage::Last { .. } | Stage::Long if n > 1 => started(),
        Stage::Last { .. } | Stage::Long => Some(scan.last),
    }
}

/// Whether a doc's places of the word under way can tell anything more.
fn wanted(scan: &PhraseScan, stage: Stage, within: &Bits, n: usize, doc: u32) -> bool {
    let asked = if n > 1 { scan.starts.contains_key(&doc) } else { within.get(doc) };
    match stage {
        Stage::Segment => false,
        Stage::Word(0) => within.get(doc),
        Stage::Word(_) => scan.starts.contains_key(&doc),
        Stage::Last { .. } => asked && !scan.found.yes.get(doc),
        Stage::Long => asked && !scan.found.yes.get(doc) && !scan.found.unsure.get(doc),
    }
}

/// A doc's places (rising) of the word under way weighed: where the phrase may start kept, or
/// the doc found to hold it, or left to its text.
fn weigh(scan: &mut PhraseScan, stage: Stage, n: usize, doc: u32, places: &[u32]) {
    let whole: Vec<u32> = places.iter().filter(|p| *p & PART == 0).map(|p| p >> 1).collect();
    // Whether the word is at `at` places after a place the phrase may start at (any, alone).
    let follows = |starts: Option<&Vec<u32>>, at: usize| match starts {
        None => !whole.is_empty(),
        Some(s) => s.iter().any(|p| p.checked_add(u32::try_from(at).unwrap_or(u32::MAX)).is_some_and(|q| whole.binary_search(&q).is_ok())),
    };
    let starts = if n > 1 { scan.starts.get(&doc) } else { None };
    match stage {
        Stage::Segment => {}
        Stage::Word(0) => {
            if !whole.is_empty() {
                scan.starts.insert(doc, whole);
            }
        }
        Stage::Word(i) => {
            let Some(s) = scan.starts.get(&doc) else { return };
            let kept: Vec<u32> = s
                .iter()
                .copied()
                .filter(|p| p.checked_add(u32::try_from(i).unwrap_or(u32::MAX)).is_some_and(|q| whole.binary_search(&q).is_ok()))
                .collect();
            if !kept.is_empty() {
                scan.next.insert(doc, kept);
            }
        }
        Stage::Last { .. } => {
            if follows(starts, n - 1) {
                scan.found.yes.set(doc);
            }
        }
        Stage::Long => {
            if follows(starts, n - 1) {
                scan.found.unsure.set(doc);
            }
        }
    }
}

/// The docs of `within` in a segment's span left to their texts: its places can't tell.
fn unsure_in(scan: &mut PhraseScan, within: &Bits, info: &SegInfo) {
    for d in within.ones().skip_while(|d| *d < info.lo).take_while(|d| *d <= info.hi) {
        scan.found.unsure.set(d);
    }
}

fn end_segment(scan: &mut PhraseScan) {
    scan.starts.clear();
    scan.next.clear();
    scan.entry = None;
    scan.stream = None;
    scan.seg += 1;
    scan.stage = Stage::Segment;
}
