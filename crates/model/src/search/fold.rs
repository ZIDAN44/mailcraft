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

/// Changes whenever what [`index_tokens`] gives changes: what was indexed with another is read
/// again (2: the parts of words joined by `.`, `:`, `'`, `_`).
pub const TOKENIZER_VERSION: u32 = 2;
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
    for c in w.chars() {
        for l in c.to_lowercase().filter(|l| !is_mark(*l)) {
            match FOLD.binary_search_by(|(k, _)| k.cmp(&l)).ok().and_then(|i| FOLD.get(i)) {
                Some((_, s)) => out.push_str(s),
                None => out.push(l),
            }
        }
    }
    Cow::Owned(out)
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

/// Whether a folded text (words split by single spaces) has a word starting with `prefix`.
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
