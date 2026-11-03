//! A doc's words as the writer keeps them before it writes a segment: each word's docs, and each
//! doc's places of it (`format/pos.rs`); and the lists of a word in several segments made one.

use std::ops::Range;

use mailcraft_model::search::{Placed, placed_tokens};

use super::super::format::{R, long_key, needs_places, put_places, put_var, split_places};
use super::*;

/// A word's docs, rising, and their places: a block a doc, in the same order (none for an
/// address).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Posting {
    pub(crate) docs: Vec<u32>,
    pub(crate) places: Vec<u8>,
}

/// The words to write: each word's docs and places.
pub(crate) type Words = BTreeMap<Vec<u8>, Posting>;

/// A doc's words, each once, with its block of places (empty for an address, which has none):
/// the text's, the names' and both's addresses. A word longer than [`MAX_KEY`] is kept as a long
/// word (its place alone), as one too long to index is. A part of a dotted word is a word with no
/// place: one the doc has only so has a block of none.
pub(crate) fn doc_words<S: AsRef<str>>(text: &str, names: &[S]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut placed: Vec<(Vec<u8>, Option<u32>)> = Vec::new();
    place(Field::Body, text, 0, &mut placed);
    // A place left between names: no phrase runs from one into the next.
    let mut at = 0u32;
    for n in names {
        at = place(Field::Names, n.as_ref(), at, &mut placed).saturating_add(1);
    }
    placed.sort_unstable();
    placed.dedup();
    let mut out: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut places: Vec<u32> = Vec::new();
    let mut it = placed.into_iter().peekable();
    while let Some((k, p)) = it.next() {
        places.extend(p);
        if it.peek().is_some_and(|(next, _)| *next == k) {
            continue;
        }
        let mut block = Vec::with_capacity(places.len().saturating_mul(2).saturating_add(4));
        put_places(&mut block, &places);
        places.clear();
        out.push((k, block));
    }
    let mut addresses = BTreeSet::new();
    address_words(text, &mut addresses);
    for n in names {
        address_words(n.as_ref(), &mut addresses);
    }
    for a in addresses {
        let k = key(Field::Address, &a);
        if k.len() <= MAX_KEY {
            out.push((k, Vec::new()));
        }
    }
    out.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The words of `text` in `field`, each with its place (from `from` on; none for a part, which no
/// phrase reads), added to `out`; one past the last place.
fn place(field: Field, text: &str, from: u32, out: &mut Vec<(Vec<u8>, Option<u32>)>) -> u32 {
    let mut end = from;
    for p in placed_tokens(text) {
        let (k, at, part) = match p {
            Placed::Word(at, w) => (key(field, &w), at, false),
            Placed::Part(at, w) => (key(field, &w), at, true),
            Placed::Long(at) => (long_key(field), at, false),
        };
        let at = from.saturating_add(at);
        end = end.max(at.saturating_add(1));
        match (k.len() <= MAX_KEY, part) {
            (true, true) => out.push((k, None)),
            (true, false) => out.push((k, Some(at))),
            // Too long to keep: a part goes, a word is a long one.
            (false, false) => out.push((long_key(field), Some(at))),
            (false, true) => {}
        }
    }
    end
}

/// A doc's words, added to the words to write. Returns the bytes the words took.
pub(crate) fn add_words(words: &mut Words, doc: u32, text: &str, names: &[String]) -> usize {
    doc_words(text, names).into_iter().map(|(k, block)| file_word(words, doc, k, &block)).fold(0, usize::saturating_add)
}

/// What a new word takes in the words to write besides its key's, its list's and its places'
/// bytes: its key and posting in a node of the map, about two-thirds full, with their nodes'
/// links.
const WORD_ENTRY: usize = 104;

/// A word of `doc` (its places `block`) added to the words to write; the bytes it took as the heap
/// holds them (its key's, its list's and its places' room, not their length, and its place in the
/// map), so what a writer holds is what [`Writer::should_flush`] goes by.
pub(crate) fn file_word(words: &mut Words, doc: u32, k: Vec<u8>, block: &[u8]) -> usize {
    let mut added = 0usize;
    let room = k.capacity();
    let p = words.entry(k).or_insert_with(|| {
        added = room.saturating_add(WORD_ENTRY);
        Posting::default()
    });
    if p.docs.last() != Some(&doc) {
        let (docs, places) = (p.docs.capacity(), p.places.capacity());
        p.docs.push(doc);
        p.places.extend_from_slice(block);
        added = added.saturating_add(p.docs.capacity().saturating_sub(docs).saturating_mul(4));
        added = added.saturating_add(p.places.capacity().saturating_sub(places));
    }
    added
}

/// A word's lists from several segments (its docs, and their places when it has them) made one:
/// docs rising, each once (its block from the first list that has it), only those `keep` keeps.
/// A word that should have places and whose list hasn't is a part in each of those docs (a block
/// of no places each).
pub(crate) fn merge_lists(key: &[u8], lists: Vec<(Vec<u32>, Option<Vec<u8>>)>, keep: impl Fn(u32) -> bool) -> R<Posting> {
    if !needs_places(key) {
        let mut docs: Vec<u32> = lists.into_iter().flat_map(|(d, _)| d).collect();
        docs.sort_unstable();
        docs.dedup();
        docs.retain(|&d| keep(d));
        return Ok(Posting { docs, places: Vec::new() });
    }
    // Each doc's block, as (doc, list, its count, where its bytes are), from every list.
    let mut blocks: Vec<(u32, usize, usize, Range<usize>)> = Vec::new();
    for (i, (docs, places)) in lists.iter().enumerate() {
        let blocks_of = match places {
            Some(p) => split_places(p, docs.len())?,
            None => vec![(0, 0..0); docs.len()],
        };
        for (&doc, (count, at)) in docs.iter().zip(blocks_of) {
            if keep(doc) {
                blocks.push((doc, i, count, at));
            }
        }
    }
    blocks.sort_unstable_by_key(|(doc, i, _, _)| (*doc, *i));
    blocks.dedup_by_key(|(doc, _, _, _)| *doc);
    let mut out = Posting { docs: Vec::with_capacity(blocks.len()), places: Vec::new() };
    for (doc, i, count, at) in blocks {
        let bytes = match lists.get(i).map(|(_, p)| p.as_ref()) {
            Some(Some(p)) => p.get(at).ok_or("places past the list")?,
            Some(None) => &[],
            None => return Err("places past the list"),
        };
        out.docs.push(doc);
        put_var(&mut out.places, count as u64);
        out.places.extend_from_slice(bytes);
    }
    Ok(out)
}
