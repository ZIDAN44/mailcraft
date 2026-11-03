//! How search reads words: split at UAX #29 word boundaries, then folded, lower-cased with their
//! accents gone and a few letters spelled out (ß → ss, æ → ae, ø → o, ł → l, ё → е, fullwidth
//! forms → ASCII), so `resume` finds "Résumé" and `strasse` "Straße". The same fold is applied to
//! what's searched and to what's typed. Cyrillic keeps its letters but ё (й isn't и), and other
//! scripts (Arabic, Hebrew, Indic, CJK) pass through as they are.
//!
//! The letters come from a table made by `cargo xtask fold` from Unicode's decompositions
//! ([`super::fold_table`]), so no normalizer is carried at run time (nor in the browser build).

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

use super::fold_table::FOLD;

mod ascii;

pub(crate) use ascii::AsciiWords;
#[cfg(test)]
pub(crate) use ascii::pieces;

/// Changes whenever what the index keeps of a text's words changes: what was indexed with another
/// is read again (2: the parts of words joined by `.`, `:`, `'`, `_`; 3: each word's places,
/// [`placed_tokens`]).
pub const TOKENIZER_VERSION: u32 = 3;
/// Longer "words" (base64 debris, long links) aren't indexed: they only bloat it.
pub const MAX_TOKEN_CHARS: usize = 40;

/// An accent that letters carry (combining diacritical marks), dropped: so text written with
/// its accents apart (macOS file names, NFD) folds as written together, and `İ` (lower-cased to
/// `i` and a dot) is `i`.
fn is_mark(c: char) -> bool {
    ('\u{300}'..='\u{36F}').contains(&c)
}

/// A word folded: lower-case, accents gone, letters spelled out. Borrowed when it's lower-case
/// ASCII already, the common case.
pub fn fold_word(w: &str) -> Cow<'_, str> {
    if w.is_ascii() {
        return if w.bytes().any(|b| b.is_ascii_uppercase()) { Cow::Owned(w.to_ascii_lowercase()) } else { Cow::Borrowed(w) };
    }
    let mut out = String::with_capacity(w.len());
    fold_chars(w, &mut out);
    Cow::Owned(out)
}

/// Each letter of `w` folded, added to `out`. An ASCII letter (most of a word with an accent) is
/// only lower-cased: no accent and no letter of the table is ASCII.
fn fold_chars(w: &str, out: &mut String) {
    for c in w.chars() {
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
            continue;
        }
        for l in c.to_lowercase() {
            if l.is_ascii() {
                out.push(l);
                continue;
            }
            if is_mark(l) {
                continue;
            }
            match FOLD.binary_search_by(|(k, _)| k.cmp(&l)).ok().and_then(|i| FOLD.get(i)) {
                Some((_, s)) => out.push_str(s),
                None => out.push(l),
            }
        }
    }
}

/// A text's words, folded, as search indexes them: none empty, none longer than
/// [`MAX_TOKEN_CHARS`].
pub fn tokens(text: &str) -> impl Iterator<Item = Cow<'_, str>> {
    all_words(text).filter(|w| w.len() <= MAX_TOKEN_CHARS || w.chars().count() <= MAX_TOKEN_CHARS)
}

/// A text's words, folded, long ones too (what a message's own fields are checked against).
pub(super) fn all_words(text: &str) -> impl Iterator<Item = Cow<'_, str>> {
    text.unicode_words().map(fold_word).filter(|w| !w.is_empty())
}

/// A text's [`tokens`], each followed by its [`parts`]: the words search indexes, so `word` finds
/// "site.word".
pub fn index_tokens(text: &str) -> impl Iterator<Item = Cow<'_, str>> {
    with_parts(all_words(text)).filter(|w| w.len() <= MAX_TOKEN_CHARS || w.chars().count() <= MAX_TOKEN_CHARS)
}

/// A word search indexes, with its place: which of the text's words (as a phrase reads them, long
/// ones too, counted from 0) it is, or is a part of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placed<'a> {
    /// A word, at most [`MAX_TOKEN_CHARS`] long.
    Word(u32, Cow<'a, str>),
    /// One of the [`parts`] of the word at that place (a phrase never starts or goes on there).
    Part(u32, String),
    /// A word too long to index: only its place is kept, so a phrase whose last word may start it
    /// is checked against the text.
    Long(u32),
}

/// A text's [`index_tokens`], each with its place, and the places of its words too long to index:
/// what a phrase is told from without the text.
pub fn placed_tokens(text: &str) -> impl Iterator<Item = Placed<'_>> {
    let fits = |w: &str| w.len() <= MAX_TOKEN_CHARS || w.chars().count() <= MAX_TOKEN_CHARS;
    all_words(text).enumerate().flat_map(move |(i, w)| {
        let at = u32::try_from(i).unwrap_or(u32::MAX);
        let parts: Vec<Placed<'_>> = parts(&w).filter(|p| fits(p)).map(|p| Placed::Part(at, p.to_string())).collect();
        let word = if fits(&w) { Placed::Word(at, w) } else { Placed::Long(at) };
        std::iter::once(word).chain(parts)
    })
}

/// Words, each followed by its [`parts`].
pub(super) fn with_parts<'a>(words: impl Iterator<Item = Cow<'a, str>>) -> impl Iterator<Item = Cow<'a, str>> {
    words.flat_map(|w| {
        let parts: Vec<Cow<'a, str>> = parts(&w).map(|p| Cow::Owned(p.to_string())).collect();
        std::iter::once(w).chain(parts)
    })
}

/// Whether UAX #29 keeps `c` inside a word, between letters (MidLetter, MidNumLet) or joining
/// its pieces (ExtendNumLet): "site.word", "l'avion", "ratio:word" and "snake_case" are each
/// one word. Between digits only (MidNum: `10,000`) isn't split.
fn joins(c: char) -> bool {
    matches!(
        c,
        '.' | ':'
            | '\''
            | '_'
            | '\u{b7}'
            | '\u{387}'
            | '\u{55f}'
            | '\u{5f4}'
            | '\u{2018}'
            | '\u{2019}'
            | '\u{2024}'
            | '\u{2027}'
            | '\u{203f}'
            | '\u{2040}'
            | '\u{2054}'
            | '\u{fe13}'
            | '\u{fe33}'
            | '\u{fe34}'
            | '\u{fe4d}'..='\u{fe4f}' | '\u{fe52}' | '\u{fe55}' | '\u{ff07}' | '\u{ff0e}' | '\u{ff1a}' | '\u{ff3f}'
    )
}

/// The pieces of a folded word that joins them ([`joins`]), each found by itself too: a word
/// seen only as "xx.word" was never found by `word` (nine messages of the 41 GB file). None for
/// a word that doesn't, and none of one letter (the `t` of "don't", the `l` of "l'avion").
pub fn parts(w: &str) -> impl Iterator<Item = &str> {
    let split = w.contains(joins);
    w.split(joins).filter(move |p| split && p.chars().nth(1).is_some())
}

/// Whether a folded word has a character that [`joins`] its pieces: most don't, and aren't
/// split for [`parts`] at all. A look at its bytes first: one all ASCII with none of `.:'_` has
/// none.
pub(super) fn has_joins(text: &str) -> bool {
    text.bytes().any(|b| matches!(b, b'.' | b':' | b'\'' | b'_') || !b.is_ascii()) && text.contains(joins)
}

/// A text's [`tokens`] joined by spaces, added to `out`.
pub fn fold_text(text: &str, out: &mut String) {
    join(tokens(text), out);
}

/// Words joined by spaces, added to `out` (a space first when `out` has something).
pub(super) fn join<'a>(words: impl Iterator<Item = Cow<'a, str>>, out: &mut String) {
    for w in words {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&w);
    }
}

/// A text's words folded, long ones too, joined by spaces, added to `out` (a space before each
/// but one at `start`, where the text they're part of starts): [`join`] of [`all_words`] into a
/// string of its own, with none for each word. Each word's [`parts`] go to `pieces`, a space
/// before each, as it's folded, rather than the text being looked through again for them.
pub(super) fn fold_all_into(text: &str, start: usize, out: &mut String, pieces: &mut String) {
    // An ASCII text (most summaries' parts) split a byte at a time, into the same words.
    if text.is_ascii() {
        AsciiWords::new(text).for_each(|w| fold_one_into(w, true, start, out, pieces));
    } else {
        // Most of a text that isn't ASCII all through still is, between its spaces.
        for piece in ascii::pieces(text) {
            if piece.is_ascii() {
                AsciiWords::new(piece).for_each(|w| fold_one_into(w, true, start, out, pieces));
            } else {
                piece.unicode_words().for_each(|w| fold_one_into(w, w.is_ascii(), start, out, pieces));
            }
        }
    }
}

/// A word of [`fold_all_into`], folded into `out` (whose text starts at `start`), and its
/// [`parts`] into `pieces`. An ASCII word (`ascii`: most) is lower-cased in place and split at its
/// ASCII join characters, its bytes looked at no more than that.
fn fold_one_into(w: &str, ascii: bool, start: usize, out: &mut String, pieces: &mut String) {
    let before = out.len();
    if before > start {
        out.push(' ');
    }
    let at = out.len();
    if ascii {
        out.push_str(w);
        if let Some(folded) = out.get_mut(at..) {
            folded.make_ascii_lowercase();
            if folded.bytes().any(|b| matches!(b, b'.' | b':' | b'\'' | b'_')) {
                for p in folded.split(['.', ':', '\'', '_']).filter(|p| p.len() > 1) {
                    pieces.push(' ');
                    pieces.push_str(p);
                }
            }
        }
    } else {
        fold_chars(w, out);
        if let Some(folded) = out.get(at..)
            && has_joins(folded)
        {
            for p in parts(folded) {
                pieces.push(' ');
                pieces.push_str(p);
            }
        }
    }
    // A word of accents alone folds to nothing: not a word.
    if out.len() == at {
        out.truncate(before);
    }
}

/// A text's words folded, long ones too, joined by spaces: what a phrase is looked for in
/// ([`folded_has_phrase`]), made once to be looked through again.
pub fn fold_all(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    join(all_words(text), &mut out);
    out
}

/// Whether a text folded by [`fold_all`] has these folded words one after another, the last as
/// a word's start: what the text itself says of them (`TextSource::phrase` of a `&str`).
pub fn folded_has_phrase(folded: &str, phrase: &[String]) -> bool {
    has_phrase(folded, &phrase.join(" "))
}

/// Whether a folded text (words split by single spaces) has a word starting with `prefix`. Most
/// texts looked through don't hold it at all: a quick look for it anywhere comes first.
pub(super) fn has_prefix(text: &str, prefix: &str) -> bool {
    !prefix.is_empty() && text.contains(prefix) && text.match_indices(prefix).any(|(i, _)| starts_word(text, i))
}

/// Whether a folded text has these folded words one after another (`phrase`, joined by spaces),
/// the last as a word's start, as a lone word is (`"q3 plan"` finds "Q3 Plan.pdf").
pub(super) fn has_phrase(text: &str, phrase: &str) -> bool {
    has_prefix(text, phrase)
}

fn starts_word(text: &str, i: usize) -> bool {
    i.checked_sub(1).is_none_or(|before| text.as_bytes().get(before) == Some(&b' '))
}
