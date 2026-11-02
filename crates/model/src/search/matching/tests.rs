//! Tests of how a summary's text is made: as it was made before it was made fast, byte for byte.

use proptest::prelude::*;

use super::*;
use crate::search::fold::AsciiWords;
use proptest::strategy::ValueTree;

/// How [`SummaryText::with_parts`] made a summary's text before: a string for each part, word
/// and piece, each word folded on its own and each part lower-cased by `to_lowercase`.
fn made_before(m: &Summary, want: [bool; 4]) -> SummaryText {
    fn fold_before(w: &str) -> String {
        let mut out = String::new();
        for c in w.chars() {
            for l in c.to_lowercase().filter(|l| !('\u{300}'..='\u{36F}').contains(l)) {
                match super::super::fold_table::FOLD.binary_search_by(|(k, _)| k.cmp(&l)).ok().and_then(|i| super::super::fold_table::FOLD.get(i)) {
                    Some((_, s)) => out.push_str(s),
                    None => out.push(l),
                }
            }
        }
        out
    }
    let mut st = SummaryText::default();
    let wanted = |p: Part| want[p as usize];
    let from = if wanted(Part::From) { addr_text(std::slice::from_ref(&m.from)) } else { String::new() };
    let mut to = String::new();
    if wanted(Part::To) {
        to = addr_text(&m.to);
        to.push_str(&addr_text(&m.cc));
    }
    let subject = if wanted(Part::Subject) { m.subject.as_str() } else { "" };
    let preview = if wanted(Part::Preview) { m.preview.as_str() } else { "" };
    for (i, text) in [subject, &from, &to, preview].into_iter().enumerate() {
        let mut part = String::new();
        let words = unicode_segmentation::UnicodeSegmentation::unicode_words(text)
            .map(|w| match w.is_ascii() {
                true if w.bytes().any(|b| b.is_ascii_uppercase()) => Cow::Owned(w.to_ascii_lowercase()),
                true => Cow::Borrowed(w),
                false => Cow::Owned(fold_before(w)),
            })
            .filter(|w| !w.is_empty());
        join(words, &mut part);
        let pieces: Vec<String> = part.split(' ').flat_map(parts).map(str::to_string).collect();
        if !pieces.is_empty() {
            part.push_str(" \u{1}");
            join(pieces.into_iter().map(Cow::Owned), &mut part);
        }
        st.folded.push_str(&part);
        st.ends[i] = st.folded.len();
        if i < 3 {
            st.raw.push_str(&text.to_lowercase());
            st.raw_ends[i] = st.raw.len();
        }
    }
    st
}

/// A piece of a summary's text: words of every kind (dotted, accented, written apart, Greek with
/// its capital sigma, CJK, too long to index), what joins and splits them, and anything at all.
fn piece() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9]{1,12}",
        "[a-z]{1,8}[.:'_][a-zA-Z]{1,8}",
        "[a-z]{1,6}[\u{b7}\u{2019}\u{2027}\u{ff0e}\u{ff07}\u{ff1a}][a-z]{1,6}",
        "[a-zàáâãäåæçèéêëìíîïñòóôõöøùúûüýÿßÀÉÎÕÜÆØŁłŀİıĳ]{1,10}",
        "[a-zA-Z][\u{300}-\u{36f}]{1,3}[a-z]{0,4}",
        "[\u{300}-\u{36f}\u{345}]{1,3}",
        "[ΑΒΓΔΣσςάέΐΰ]{1,8}",
        "[東京都日本語の会議資料한국어]{1,6}",
        "[ａ-ｚＡ-Ｚ０-９]{1,6}",
        "[a-z]{41,70}",
        "[.,;:!?@<>()\"'_ \t\n-]{1,4}",
        r"\PC{1,10}",
    ]
}

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(piece(), 0..12).prop_map(|ps| ps.join(" "))
}

fn address() -> impl Strategy<Value = EmailAddress> {
    (text(), r"[a-z.]{0,8}@?[a-z]{0,8}(\.[a-z]{2,3})?|\PC{0,12}").prop_map(|(name, email)| EmailAddress { name, email })
}

fn summary() -> impl Strategy<Value = Summary> {
    (text(), address(), prop::collection::vec(address(), 0..3), prop::collection::vec(address(), 0..3), text())
        .prop_map(|(subject, from, to, cc, preview)| Summary { subject, from, to, cc, preview, ..Default::default() })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Any summary's text, whichever parts are made, is what it was made as before.
    #[test]
    fn a_summary_text_is_made_as_before(m in summary(), want in any::<[bool; 4]>()) {
        prop_assert_eq!(SummaryText::with_parts(&m, want), made_before(&m, want));
    }

    /// An ASCII text is split into the words unicode-segmentation splits it into: any bytes, and
    /// ones made of what its rules tell apart (letters, digits, `:.',;_`, spaces and line ends).
    #[test]
    fn ascii_words_are_unicode_words(text in r"[\x00-\x7f]{0,40}|[aZ09:.',;_ \r\n\x0b\x0c\t@-]{0,40}|([a-z0-9_]{1,3}[:.',;_ ]{1,2}){0,10}") {
        let ours: Vec<&str> = AsciiWords::new(&text).collect();
        let theirs: Vec<&str> = unicode_segmentation::UnicodeSegmentation::unicode_words(text.as_str()).collect();
        prop_assert_eq!(ours, theirs);
    }
}

/// Summaries the proptest may not happen on: a final sigma (lower-cased by what's around it),
/// a word that is only accents, pieces that join, a capital I with a dot.
#[test]
fn edge_summaries_are_made_as_before() {
    let texts = [
        "ΟΔΟΣ ΟΔΟΣ. Σ ΣΑ aΣ Σ́ Ὀδυσσεύς",
        "\u{301}\u{302} a\u{301} e\u{301}cole",
        "site.word l'avion ratio:word snake_case l\u{b7}l ŀ x.y.z don't",
        "İstanbul ǅ ﬁle Ｗｏｒｄ．ｓｉｔｅ",
        "東京の会議 10,000 and 3.14",
        "",
    ];
    for t in texts {
        let who = EmailAddress::new(t, t);
        let m = Summary { subject: t.into(), from: who.clone(), to: vec![who.clone()], cc: vec![who], preview: t.into(), ..Default::default() };
        for want in [[true; 4], [false, true, false, true], [false; 4]] {
            assert_eq!(SummaryText::with_parts(&m, want), made_before(&m, want), "{t}");
        }
    }
}

/// No letter of the table is ASCII: an ASCII letter folds by being lower-cased alone.
#[test]
fn no_ascii_letter_folds() {
    assert!(super::super::fold_table::FOLD.iter().all(|(k, _)| !k.is_ascii()));
}

/// Hand-run, in release (`cargo test -p mailcraft-model --release --lib how_long_a_summary -- --ignored
/// --nocapture`): how long a summary's text takes to make, now and as before.
#[test]
#[ignore]
fn how_long_a_summary_text_takes() {
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let ms: Vec<Summary> = (0..20_000).map(|_| summary().new_tree(&mut runner).unwrap().current()).collect();
    let plain: Vec<Summary> = (0..20_000)
        .map(|i| Summary {
            subject: format!("Re: Quarterly budget review {i} for the studio"),
            from: EmailAddress::new("Zoe Park", "zoe.park@studio.example"),
            to: vec![EmailAddress::new("Ann Lee", "ann@studio.example"), EmailAddress::new("", "team@studio.example")],
            preview: format!(
                "Hi all, attached are the numbers for week {i}. Let's go through them on Thursday at 10:30 and decide what we cut. Best, Zoe"
            ),
            ..Default::default()
        })
        .collect();
    for (name, ms) in [("random", &ms), ("plain", &plain)] {
        for _ in 0..3 {
            let began = std::time::Instant::now();
            let n: usize = ms.iter().map(|m| made_before(m, [true; 4]).folded.len()).sum();
            let before = began.elapsed();
            let began = std::time::Instant::now();
            let k: usize = ms.iter().map(|m| SummaryText::of(m).folded.len()).sum();
            let now = began.elapsed();
            assert_eq!(n, k);
            eprintln!("{name}: {} summaries: before {before:?}, now {now:?}", ms.len());
        }
    }
}
