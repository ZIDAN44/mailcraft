//! Where a text has what a search looks for (issue #173), to mark in the message found: its words
//! read as the index reads them, folded, a word sought as a word's start (the pieces of a joined
//! word, "site.word", too), and a phrase as its words one after another.

use std::borrow::Cow;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::fold::{fold_word, parts};
use super::{Field, Query, Term};

/// The most hits marked in a text: a common word sought in a long message marks those.
pub const MAX_HITS: usize = 10_000;

impl Query {
    /// What it looks for in `field`'s text, or in any field, folded, to mark where a message has
    /// it: each word alone, a phrase as its words. Nothing it looks for not to be there.
    pub fn sought(&self, field: &Field) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        for c in self.clauses.iter().filter(|c| !c.negated) {
            sought_in(&c.term, field, &mut out);
        }
        out
    }
}

fn sought_in(term: &Term, field: &Field, out: &mut Vec<Vec<String>>) {
    let here = |f: &Field| *f == Field::Any || f == field;
    match term {
        Term::Words(f, words) if here(f) => {
            out.extend(words.iter().map(|w| fold_word(w).into_owned()).filter(|w| !w.is_empty()).map(|w| vec![w]));
        }
        Term::Phrase(f, phrase) if here(f) => {
            let words: Vec<String> = phrase.unicode_words().map(|w| fold_word(w).into_owned()).filter(|w| !w.is_empty()).collect();
            if !words.is_empty() {
                out.push(words);
            }
        }
        Term::AnyOf(any) => {
            for t in any {
                sought_in(t, field, out);
            }
        }
        _ => {}
    }
}

/// A word of the text: where it starts, as written, and folded.
struct Word<'a> {
    at: usize,
    text: &'a str,
    folded: Cow<'a, str>,
}

/// Where `text` has what's `sought` ([`Query::sought`]): byte ranges, in order, none overlapping,
/// at most [`MAX_HITS`]. A word is marked as far as it was typed (`proj` in "project") when the
/// word is ASCII, and whole otherwise (folding may change its length).
pub fn hits(text: &str, sought: &[Vec<String>]) -> Vec<Range<usize>> {
    if sought.is_empty() || sought.iter().all(Vec::is_empty) {
        return Vec::new();
    }
    let words: Vec<Word<'_>> =
        text.unicode_word_indices().map(|(at, w)| Word { at, text: w, folded: fold_word(w) }).filter(|w| !w.folded.is_empty()).collect();
    let mut out: Vec<Range<usize>> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        for s in sought {
            let found = match s.as_slice() {
                [] => None,
                [one] => starts(w, one),
                [first @ .., last] => {
                    let run = words.get(i..i.saturating_add(s.len()));
                    run.filter(|run| run.iter().zip(first).all(|(w, f)| *w.folded == **f))
                        .and_then(|run| run.last())
                        .and_then(|end| prefix(end, last))
                        .map(|end| w.at..end.end)
                }
            };
            if let Some(r) = found {
                out.push(r);
            }
        }
        if out.len() >= MAX_HITS {
            break;
        }
    }
    out.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(out.len());
    for r in out {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged.truncate(MAX_HITS);
    merged
}

/// The word, or one of its pieces, starting with `sought`: as far as that goes.
fn starts(w: &Word<'_>, sought: &str) -> Option<Range<usize>> {
    if let Some(r) = prefix(w, sought) {
        return Some(r);
    }
    if w.text.is_ascii() {
        // The pieces of a joined word, where they are in it (folding ASCII keeps its length).
        let mut off = 0usize;
        for piece in w.folded.split(['.', ':', '\'', '_']) {
            if piece.len() > 1 && piece.starts_with(sought) {
                let start = w.at.saturating_add(off);
                return Some(start..start.saturating_add(sought.len()));
            }
            off = off.saturating_add(piece.len()).saturating_add(1);
        }
        return None;
    }
    parts(&w.folded).any(|p| p.starts_with(sought)).then(|| w.at..w.at.saturating_add(w.text.len()))
}

/// The word starting with `sought`: as far as that goes, or the whole word when it isn't ASCII.
fn prefix(w: &Word<'_>, sought: &str) -> Option<Range<usize>> {
    if !w.folded.starts_with(sought) {
        return None;
    }
    let end = if w.text.is_ascii() { sought.len() } else { w.text.len() };
    Some(w.at..w.at.saturating_add(end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(text: &str, query: &str) -> Vec<String> {
        let q = Query::parse(query);
        hits(text, &q.sought(&Field::Body)).into_iter().map(|r| text.get(r).unwrap_or("?").to_string()).collect()
    }

    /// The reporter's: a number inside a longer string, found by itself and marked there.
    #[test]
    fn what_was_found_is_marked() {
        assert_eq!(marked("Order 201124-J006965361-S/N000164396 shipped", "J006965361"), ["J006965361"]);
        assert_eq!(marked("The Project and the projector", "proj"), ["Proj", "proj"]);
        assert_eq!(marked("Résumé attached; see resume.pdf", "resume"), ["Résumé", "resume"]);
        assert_eq!(marked("open site.word now", "word"), ["word"]);
        assert_eq!(marked("the budget review on Thursday, a review", "\"budget review\""), ["budget review"]);
        assert_eq!(marked("budget for the review", "\"budget review\""), Vec::<String>::new());
        // Not what's negated, nor another field's words.
        assert_eq!(marked("alpha beta", "alpha -beta"), ["alpha"]);
        assert_eq!(marked("from ana about ana", "from:ana"), Vec::<String>::new());
        assert_eq!(marked("from ana about ana", "body:ana"), ["ana", "ana"]);
        assert_eq!(marked("nothing here", "is:unread"), Vec::<String>::new());
        // Overlapping ones, one mark.
        assert_eq!(marked("projects", "proj project"), ["project"]);
    }

    proptest::proptest! {
        /// Any text, anything sought: the marks are in the text, in order, apart.
        #[test]
        fn marks_are_always_in_the_text(text in "\\PC{0,64}", query in "\\PC{0,16}") {
            let ranges = hits(&text, &Query::parse(&query).sought(&Field::Body));
            for r in &ranges {
                proptest::prop_assert!(text.get(r.clone()).is_some(), "{:?} in {:?}", r, text);
            }
            proptest::prop_assert!(ranges.windows(2).all(|w| w[0].end < w[1].start));
        }
    }

    #[test]
    fn hits_are_capped_and_never_panic() {
        let text = "a ".repeat(MAX_HITS * 2);
        assert_eq!(hits(&text, &[vec!["a".into()]]).len(), MAX_HITS);
        for text in ["", "ü", "\u{301}", "x.y:z'_", "日本語のテキスト", "١٢٣ ٤٥٦"] {
            for s in [vec![], vec!["".to_string()], vec!["x".into(), "y".into()], vec!["日本".into()]] {
                for r in hits(text, std::slice::from_ref(&s)) {
                    assert!(text.get(r).is_some(), "{text:?} {s:?}");
                }
            }
        }
    }
}
