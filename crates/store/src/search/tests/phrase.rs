//! Phrases settled from their words' places: the same answers as the text gives, on texts of
//! every kind, carried through merges and ids given again, read a few pages a call, and made
//! for an index from before places were kept. The parts of dotted words are found as words but
//! keep no places.

use std::collections::BTreeMap;

use mailcraft_model::search::{TextField, TextSource, Tri, fold_all};
use proptest::prelude::*;

use super::*;

/// What the index's places say of `words` (folded, the last a prefix) in `field`, for each doc
/// with every word of it (as the search looks them up): yes, no, or `None` (the text must tell).
/// Looked up a page a call, each call's store reads counted against it.
pub(super) fn phrase_docs(r: &mut Reader, s: &Store, field: Field, words: &[String]) -> BTreeMap<u32, Option<bool>> {
    let mut within = Bits::new(r.len());
    for d in 0..r.len() {
        within.set(d);
    }
    for w in words {
        let mut found = r.word(s, field, w).unwrap();
        found.and(&within);
        within = found;
    }
    let mut scan = PhraseScan::default();
    let mut calls = 0;
    loop {
        let before = r.pages_read();
        let done = r.scan_phrase(s, field, words, &within, &mut scan, 1).unwrap();
        assert!(r.pages_read() - before <= 2, "{} pages in a call", r.pages_read() - before);
        calls += 1;
        assert!(calls < 100_000, "a phrase never settled");
        if done {
            break;
        }
    }
    within.ones().map(|d| (d, scan.found().has(d))).collect()
}

/// What the text says of the phrase: `field`'s text holds it.
fn in_text(t: &str, words: &[String]) -> bool {
    t.phrase(TextField::Body, words, "") == Tri::Yes
}

/// Whether a text has a word too long to index (its places can't tell what it starts with).
fn has_long(t: &str) -> bool {
    fold_all(t).split(' ').any(|w| w.chars().count() > mailcraft_model::search::MAX_TOKEN_CHARS)
}

const VOCAB: [&str; 22] = [
    "thank",
    "you",
    "for",
    "the",
    "update",
    "updated",
    "Résumé",
    "resume",
    "東京",
    "大阪",
    "site.word",
    "word",
    "l'avion",
    "Straße",
    "strasse",
    "a",
    "you.the",
    "UPDATE",
    "fullwidth：Ｗord",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "thank-you",
    "ⓧ",
];
const GAPS: [&str; 5] = [" ", ", ", ". ", "\n", " — "];

fn text_of(picks: &[(usize, usize)]) -> String {
    let mut t = String::new();
    for &(w, g) in picks {
        t.push_str(VOCAB[w % VOCAB.len()]);
        t.push_str(GAPS[g % GAPS.len()]);
    }
    t
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    /// Texts of accents, CJK, dotted words, repeated words and words too long to index, in the
    /// text and in the files' names, over two segments and then merged into one: what the places
    /// say of a phrase (taken from a text, its last word cut to a prefix, or made up) is what the
    /// text says, but where a long word may be its last word's.
    #[test]
    fn places_say_what_the_text_says(
        docs in prop::collection::vec((prop::collection::vec((0usize..22, 0usize..5), 0..24), prop::collection::vec(prop::collection::vec((0usize..22, 0usize..5), 0..4), 0..3)), 1..6),
        from in 0usize..64, len in 1usize..5, cut in 0usize..4, made in prop::collection::vec(0usize..22, 1..4), pick_made in any::<bool>(),
    ) {
        let s = Store::memory();
        let mut w = Writer::open(&s, "a1").unwrap();
        let mut kept = Vec::new();
        for (i, (body, names)) in docs.iter().enumerate() {
            let body = text_of(body);
            let names: Vec<String> = names.iter().map(|n| text_of(n)).collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let doc = w.add(&mk("INBOX", i as u32 + 1), 1, 1, &text(&body, &refs, 0)).unwrap();
            kept.push((doc, body, names));
            if i == docs.len() / 2 {
                w.flush(&s).unwrap();
            }
        }
        w.flush(&s).unwrap();
        // The phrase: words of a text, or made up of the vocabulary.
        let all: Vec<String> = fold_all(&kept.iter().map(|(_, b, n)| format!("{b} {}", n.join(" "))).collect::<Vec<_>>().join(" ")).split(' ').filter(|w| !w.is_empty()).map(String::from).collect();
        let mut words: Vec<String> = if pick_made || all.is_empty() {
            made.iter().flat_map(|&i| fold_all(VOCAB[i]).split(' ').map(String::from).collect::<Vec<_>>()).filter(|w| !w.is_empty()).collect()
        } else {
            let start = from % all.len();
            all[start..(start + len).min(all.len())].to_vec()
        };
        if let Some(last) = words.last_mut() {
            let keep = last.chars().count().saturating_sub(cut).max(1);
            *last = last.chars().take(keep).collect();
        }
        prop_assume!(!words.is_empty());
        let mut r = Reader::open(&s, "a1").unwrap().unwrap();
        for (merged, field) in [(false, Field::Body), (false, Field::Names), (true, Field::Body), (true, Field::Names)] {
            if merged && field == Field::Body {
                fill_to_merge(&mut w, &s);
                r.refresh(&s).unwrap();
            }
            let got = phrase_docs(&mut r, &s, field, &words);
            for (doc, body, names) in &kept {
                let want = match field {
                    Field::Body => in_text(body, &words),
                    _ => names.iter().any(|n| in_text(n, &words)),
                };
                let long = match field {
                    Field::Body => has_long(body),
                    _ => names.iter().any(|n| has_long(n)),
                };
                match got.get(doc) {
                    // Not with every word: the search says no without the places.
                    None => prop_assert!(!want || long || words.iter().any(|w| w.chars().count() > 40), "{field:?} {words:?} in {body:?} {names:?}"),
                    Some(Some(yes)) => prop_assert_eq!(*yes, want, "{:?} {:?} in {:?} {:?}", field, words, body, names),
                    Some(None) => prop_assert!(long, "{field:?} {words:?}: left to the text of {body:?} {names:?}"),
                }
            }
        }
    }
}

/// The names' words go on from one name to the next with a place between: a phrase is in one
/// name, never across two; an empty name is passed over, the names after it kept (they were left
/// out, the message taken as cut short). A part of a dotted word starts no phrase, and a repeated
/// word's every place counts.
#[test]
fn a_phrase_is_in_one_name_and_starts_at_a_word() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let a = w.add(&mk("INBOX", 1), 1, 1, &text("see site.word now, the the update", &["", "Q3 plan", "plan b.pdf"], 1)).unwrap();
    let b = w.add(&mk("INBOX", 2), 1, 1, &text("the word now", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert!(!r.info(a).unwrap().truncated);
    assert_eq!(r.text(&s, a).unwrap().unwrap().names, ["Q3 plan", "plan b.pdf"]);
    let ws = |p: &str| p.split(' ').map(String::from).collect::<Vec<_>>();
    let ask = |r: &mut Reader, field, p: &str| phrase_docs(r, &s, field, &ws(p));
    assert_eq!(ask(&mut r, Field::Names, "q3 plan").get(&a), Some(&Some(true)));
    assert_eq!(ask(&mut r, Field::Names, "plan plan").get(&a), Some(&Some(false)), "across two names");
    assert_eq!(ask(&mut r, Field::Names, "plan b.p").get(&a), Some(&Some(true)), "the last word a prefix");
    assert_eq!(ask(&mut r, Field::Body, "word now"), BTreeMap::from([(a, Some(false)), (b, Some(true))]), "a part starts no phrase");
    assert_eq!(ask(&mut r, Field::Body, "site.word now").get(&a), Some(&Some(true)));
    assert_eq!(ask(&mut r, Field::Body, "the update").get(&a), Some(&Some(true)), "the second of two");
    assert_eq!(ask(&mut r, Field::Body, "the the u").get(&a), Some(&Some(true)));
    assert_eq!(ask(&mut r, Field::Body, "now the the up").get(&a), Some(&Some(true)), "across punctuation");
    assert_eq!(ask(&mut r, Field::Body, "now the up").get(&a), Some(&Some(false)));
    assert_eq!(ask(&mut r, Field::Body, "update the").get(&a), Some(&Some(false)), "out of order");
}

/// Segments of a filler doc each (no word of the tests') flushed until the segments are enough
/// to merge, then merged into one.
fn fill_to_merge(w: &mut Writer, s: &Store) {
    for uid in 1..=8 {
        w.add(&mk("Filler", uid), 1, 1, &text("zq", &[], 0)).unwrap();
        w.flush(s).unwrap();
    }
    while w.merge_step(s, 0, 1).unwrap() {}
    assert_eq!(load_meta(s, "a1").unwrap().unwrap().segments.len(), 1, "merged into one");
}

/// A part of a dotted word is found as a word, but keeps no place: a word only ever a part in a
/// segment has no places there (and its docs aren't read for a phrase), one that's also whole
/// somewhere has a block of none for each doc with it only as a part. Phrases with it are told
/// from the whole words' places all the same, after a merge too.
#[test]
fn a_dotted_words_parts_are_words_without_places() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let a = w.add(&mk("INBOX", 1), 1, 1, &text("see site.word now, only.dotted here", &["plan.v2 final.pdf"], 1)).unwrap();
    let b = w.add(&mk("INBOX", 2), 1, 1, &text("the word now", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let c = w.add(&mk("INBOX", 3), 1, 1, &text("dotted here", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    for (field, word, want) in [
        (Field::Body, "word", vec![a, b]),
        (Field::Body, "site", vec![a]),
        (Field::Body, "dotted", vec![a, c]),
        (Field::Body, "only", vec![a]),
        (Field::Names, "v2", vec![a]),
    ] {
        assert_eq!(found(&mut r, &s, field, word), want, "{word}");
    }
    // Each segment's words: which keep places, and how many each doc has.
    let entries = |s: &Store| -> BTreeMap<(u32, Vec<u8>), Option<Vec<usize>>> {
        let meta = load_meta(s, "a1").unwrap().unwrap();
        let mut out = BTreeMap::new();
        for seg in &meta.segments {
            let base = dir(s, "a1");
            let pos: Vec<u8> = (0..seg.pos_pages)
                .flat_map(|p| decode_pos_page(&s.files.read(&Name::SegPos(seg.id, p).path(&base)).unwrap()).unwrap().to_vec())
                .collect();
            for p in 0..seg.lex_pages {
                let lex = s.files.read(&Name::SegLex(seg.id, p).path(&base)).unwrap();
                for e in decode_lex(&lex, meta.next_doc, seg.post_pages, seg.pos_pages).unwrap() {
                    let counts = e.pos.map(|at| {
                        let start = at.page as usize * POS_PAGE + at.offset as usize;
                        let bytes = &pos[start..start + at.len as usize];
                        split_places(bytes, e.df as usize).unwrap().into_iter().map(|(n, _)| n).collect()
                    });
                    out.insert((seg.id, e.key), counts);
                }
            }
        }
        out
    };
    let of = |all: &BTreeMap<(u32, Vec<u8>), Option<Vec<usize>>>, word: &str| -> Vec<Option<Vec<usize>>> {
        all.iter().filter(|((_, k), _)| *k == key(Field::Body, word)).map(|(_, v)| v.clone()).collect()
    };
    let all = entries(&s);
    assert_eq!(of(&all, "site"), [None], "only ever a part: no places");
    assert_eq!(of(&all, "only"), [None]);
    assert_eq!(of(&all, "word"), [Some(vec![0, 1])], "a block of none where it's only a part");
    assert_eq!(of(&all, "site.word"), [Some(vec![1])]);
    assert_eq!(of(&all, "dotted"), [None, Some(vec![1])], "a part in one segment, whole in the next");
    assert_eq!(all.iter().filter(|((_, k), _)| *k == key(Field::Names, "v2")).map(|(_, v)| v.clone()).collect::<Vec<_>>(), [None], "a name's part");
    let ws = |p: &str| p.split(' ').map(String::from).collect::<Vec<_>>();
    for when in ["", "merged"] {
        assert_eq!(phrase_docs(&mut r, &s, Field::Body, &ws("word now")), BTreeMap::from([(a, Some(false)), (b, Some(true))]), "{when}");
        assert_eq!(phrase_docs(&mut r, &s, Field::Body, &ws("dotted here")), BTreeMap::from([(a, Some(false)), (c, Some(true))]), "{when}");
        assert_eq!(phrase_docs(&mut r, &s, Field::Body, &ws("only.dotted he")), BTreeMap::from([(a, Some(true))]), "{when}");
        assert_eq!(
            phrase_docs(&mut r, &s, Field::Body, &ws("now si")),
            BTreeMap::from([(a, Some(false))]),
            "{when}: a part isn't a last word's start"
        );
        assert_eq!(phrase_docs(&mut r, &s, Field::Names, &ws("v2 final")).get(&a), Some(&Some(false)), "{when}");
        assert_eq!(phrase_docs(&mut r, &s, Field::Names, &ws("plan.v2 final")).get(&a), Some(&Some(true)), "{when}");
        if when.is_empty() {
            fill_to_merge(&mut w, &s);
            assert!(r.refresh(&s).unwrap());
        }
    }
    let all = entries(&s);
    assert_eq!(of(&all, "dotted"), [Some(vec![0, 1])], "merged: a block of none for the doc with it only as a part");
    assert_eq!(of(&all, "site"), [None]);
}

/// A word too long to index where the phrase's last word would be leaves the doc to its text
/// (it may start with it); one elsewhere doesn't, and a long word in the phrase leaves every doc.
#[test]
fn a_long_word_leaves_it_to_the_text() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let long = "y".repeat(60);
    let a = w.add(&mk("INBOX", 1), 1, 1, &text(&format!("see {long} and yyz see"), &[], 0)).unwrap();
    let b = w.add(&mk("INBOX", 2), 1, 1, &text(&format!("{long} see me, yyz"), &[], 0)).unwrap();
    let c = w.add(&mk("INBOX", 3), 1, 1, &text(&format!("see yyz {long}"), &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    let words = vec!["see".to_string(), "yy".to_string()];
    let got = phrase_docs(&mut r, &s, Field::Body, &words);
    assert_eq!(got, BTreeMap::from([(a, None), (b, Some(false)), (c, Some(true))]), "only a long word just after `see` may start with it");
    let mut scan = PhraseScan::default();
    let within = r.word(&s, Field::Body, "see").unwrap();
    assert!(r.scan_phrase(&s, Field::Body, &[long.clone(), "see".into()], &within, &mut scan, 1).unwrap());
    assert_eq!((scan.found().has(a), scan.found().has(b)), (None, None), "a phrase word too long to index");
}

/// Merges and ids given again carry the places: a phrase found the same before and after, the
/// reader following each; a removed doc's places go with its words.
#[test]
fn merges_carry_the_places() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    w.renumber_slack = 4;
    let mut uid = 0;
    let mut docs = Vec::new();
    for round in 0..9 {
        for i in 0..6 {
            uid += 1;
            let t = if (round + i) % 3 == 0 { format!("thank you for the update {uid}") } else { format!("the update for you {uid}, thank you") };
            docs.push((w.add(&mk("INBOX", uid), 1, 1, &text(&t, &[], 0)).unwrap(), uid, t));
        }
        w.flush(&s).unwrap();
    }
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    let words: Vec<String> = ["thank", "you", "for", "the", "up"].map(String::from).to_vec();
    let want = |r: &Reader, docs: &[(u32, u32, String)]| -> BTreeMap<u32, Option<bool>> {
        docs.iter().filter_map(|(_, u, t)| Some((r.doc(&mk("INBOX", *u))?, Some(in_text(t, &words))))).collect()
    };
    assert_eq!(phrase_docs(&mut r, &s, Field::Body, &words), want(&r, &docs));
    // Some removed, merged a day on (their words purged), then most removed: ids given again.
    for (d, ..) in docs.iter().step_by(4) {
        w.remove(*d);
    }
    let kept: Vec<_> = docs.iter().enumerate().filter(|(i, _)| i % 4 != 0).map(|(_, d)| d.clone()).collect();
    while w.merge_step(&s, 0, 1).unwrap() {}
    while w.merge_step(&s, 0, 1 + TOMBSTONE_AGE).unwrap() {}
    assert!(r.refresh(&s).unwrap());
    assert_eq!(phrase_docs(&mut r, &s, Field::Body, &words), want(&r, &kept), "merged");
    let few: Vec<_> = kept.iter().take(5).cloned().collect();
    for (_, u, _) in kept.iter().skip(5) {
        let d = w.find(&mk("INBOX", *u)).unwrap();
        w.remove(d);
    }
    while w.merge_step(&s, 0, 2 + 2 * TOMBSTONE_AGE).unwrap() {}
    r.refresh(&s).unwrap();
    assert_eq!(load_meta(&s, "a1").unwrap().unwrap().next_doc, 5, "ids given again");
    assert_eq!(phrase_docs(&mut r, &s, Field::Body, &words), want(&r, &few), "made anew");
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    assert!(meta.segments.iter().all(|seg| seg.pos_pages > 0), "{:?}", meta.segments);
}

/// An index from before places were kept (its tokenizer the one before) has its words made
/// again from the texts it keeps, places and all, when its writer opens: no message is read.
#[test]
fn an_index_from_before_places_is_made_again_from_its_texts() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let a = w.add(&mk("INBOX", 1), 1, 1, &text("thank you for the update", &[], 0)).unwrap();
    let b = w.add(&mk("INBOX", 2), 1, 1, &text("the update, thank you for it", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    drop(w);
    // As a build before places wrote it: the tokenizer before, no places pages named.
    let mut meta = load_meta(&s, "a1").unwrap().unwrap();
    meta.tokenizer = 2;
    for seg in &mut meta.segments {
        seg.pos_pages = 0;
    }
    let mut old = encode_meta(&meta);
    old.truncate(old.len() - 1 - meta.segments.len());
    s.files.write(&meta_path(&s, "a1"), &old).unwrap();
    assert_eq!(decode_meta(&old).unwrap(), meta, "a meta from before places reads");
    assert!(Reader::open(&s, "a1").unwrap().is_none(), "nothing to read until its words are made again");
    let w = Writer::open(&s, "a1").unwrap();
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    assert_eq!(meta.tokenizer, mailcraft_model::search::TOKENIZER_VERSION);
    assert!(meta.segments.iter().all(|seg| seg.pos_pages > 0));
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    let words: Vec<String> = ["thank", "you", "for", "the", "update"].map(String::from).to_vec();
    assert_eq!(phrase_docs(&mut r, &s, Field::Body, &words), BTreeMap::from([(a, Some(true)), (b, Some(false))]));
    drop(w);
    assert_eq!(files(&s).1, BTreeSet::new(), "nothing of the old segments left");
}

/// A common word's places over several places pages are read a page a call, never all at once.
#[test]
fn a_common_words_places_are_read_a_page_a_call() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    for i in 0..1500u32 {
        let t = format!("{} thank you {}", "the a ".repeat(120), if i % 2 == 0 { "for the update" } else { "for an update" });
        w.add(&mk("INBOX", i + 1), 1, 1, &text(&t, &[], 0)).unwrap();
    }
    w.flush(&s).unwrap();
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    assert!(meta.segments[0].pos_pages >= 2, "{:?}", meta.segments[0]);
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    let words: Vec<String> = ["thank", "you", "for", "the", "update"].map(String::from).to_vec();
    let got = phrase_docs(&mut r, &s, Field::Body, &words);
    assert_eq!(got.len(), 1500);
    assert_eq!(got.values().filter(|v| **v == Some(true)).count(), 750);
    assert!(got.values().all(Option::is_some));
}
