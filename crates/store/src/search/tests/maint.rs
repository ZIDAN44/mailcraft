//! Keeping the index up: merges (against a model of what it should find), ids given again, a
//! removed message's words leaving the file.

use std::collections::BTreeMap;

use proptest::prelude::*;

use super::*;

const WORDS: [&str; 8] = ["alpha", "alpine", "beta", "bet", "gamma", "delta", "deltoid", "omega"];
const PREFIXES: [&str; 12] = ["", "a", "al", "alp", "b", "bet", "d", "del", "g", "o", "omega", "z"];

/// Merge until nothing is due.
fn merge_all(w: &mut Writer, s: &Store, now: u64) -> usize {
    let mut steps = 0;
    while w.merge_step(s, 0, now).unwrap() {
        steps += 1;
        assert!(steps < 1000, "merges never end");
    }
    steps
}

/// Every word in the index's segments.
fn segment_words(s: &Store) -> BTreeSet<Vec<u8>> {
    let meta = load_meta(s, "a1").unwrap().unwrap();
    let base = dir(s, "a1");
    let mut out = BTreeSet::new();
    for seg in &meta.segments {
        for p in 0..seg.lex_pages {
            let bytes = s.files.read(&Name::SegLex(seg.id, p).path(&base)).unwrap();
            out.extend(decode_lex(&bytes, meta.next_doc, seg.post_pages).unwrap().into_iter().map(|e| e.key));
        }
    }
    out
}

#[derive(Clone, Debug)]
enum Op {
    Add {
        folder: bool,
        uid: u32,
        words: Vec<usize>,
    },
    Remove(usize),
    Move {
        pick: usize,
        folder: bool,
        uid: u32,
    },
    Flush,
    /// Merge until nothing is due, this many hours on.
    Merge(u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (any::<bool>(), 1u32..40, prop::collection::vec(0usize..WORDS.len(), 0..4)).prop_map(|(folder, uid, words)| Op::Add { folder, uid, words }),
        3 => any::<usize>().prop_map(Op::Remove),
        2 => (any::<usize>(), any::<bool>(), 1u32..40).prop_map(|(pick, folder, uid)| Op::Move { pick, folder, uid }),
        2 => Just(Op::Flush),
        2 => (0u8..30).prop_map(Op::Merge),
    ]
}

fn folder(f: bool) -> &'static str {
    if f { "Archive" } else { "INBOX" }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Adds, removals, moves, flushes and merges (ids given again among them) against a map of
    /// what each message says: every prefix finds the same messages, the index read back and
    /// reopened, and nothing left over.
    #[test]
    fn merges_find_what_a_model_finds(ops in prop::collection::vec(op(), 1..60)) {
        let s = Store::memory();
        let mut w = Writer::open(&s, "a1").unwrap();
        w.renumber_slack = 6;
        let mut model: BTreeMap<(&str, u32), Vec<&str>> = BTreeMap::new();
        let mut now = 1_000_000u64;
        for op in ops {
            match op {
                Op::Add { folder: f, uid, words } => {
                    let ws: Vec<&str> = words.iter().filter_map(|&i| WORDS.get(i).copied()).collect();
                    w.add(&mk(folder(f), uid), u64::from(uid), uid, &text(&ws.join(" "), &[], 0)).unwrap();
                    model.insert((folder(f), uid), ws);
                }
                Op::Remove(pick) => {
                    let Some(k) = model.keys().nth(pick % model.len().max(1)).copied() else { continue };
                    w.remove(w.find(&mk(k.0, k.1)).unwrap());
                    model.remove(&k);
                }
                Op::Move { pick, folder: f, uid } => {
                    let Some(k) = model.keys().nth(pick % model.len().max(1)).copied() else { continue };
                    let doc = w.find(&mk(k.0, k.1)).unwrap();
                    w.retarget(doc, &mk(folder(f), uid)).unwrap();
                    let ws = model.remove(&k).unwrap();
                    model.insert((folder(f), uid), ws);
                }
                Op::Flush => {
                    w.flush(&s).unwrap();
                }
                Op::Merge(hours) => {
                    now += u64::from(hours) * 3600;
                    merge_all(&mut w, &s, now);
                    if let Some(meta) = load_meta(&s, "a1").unwrap() {
                        prop_assert!(meta.next_doc <= 2 * meta.live + 6, "{} ids for {} live", meta.next_doc, meta.live);
                    }
                }
            }
        }
        w.flush(&s).unwrap();
        for reader in [Reader::open(&s, "a1").unwrap(), { drop(w); Reader::open(&s, "a1").unwrap() }] {
            let Some(mut r) = reader else {
                prop_assert!(model.is_empty());
                continue;
            };
            for prefix in PREFIXES {
                let got: BTreeSet<(String, u32)> =
                    r.word(&s, Field::Body, prefix).unwrap().ones().filter_map(|d| r.key(d)).map(|k| (k.folder, k.uid)).collect();
                let want: BTreeSet<(String, u32)> =
                    model.iter().filter(|(_, ws)| ws.iter().any(|w| w.starts_with(prefix))).map(|((f, u), _)| (f.to_string(), *u)).collect();
                prop_assert_eq!(got, want, "prefix {:?}", prefix);
            }
            prop_assert_eq!(r.live() as usize, model.len());
        }
        drop(Writer::open(&s, "a1").unwrap());
        if load_meta(&s, "a1").unwrap().is_some() {
            prop_assert_eq!(files(&s).1, BTreeSet::new(), "nothing left over");
        }
    }

    /// Mail coming and going in bulk: once merged, the ids stay within twice the live docs and
    /// [`RENUMBER_SLACK`], and each live message still finds its word.
    #[test]
    fn churn_keeps_the_ids_few(rounds in prop::collection::vec((500u32..2500, 50u32..95), 1..5)) {
        let s = Store::memory();
        let mut w = Writer::open(&s, "a1").unwrap();
        let mut live: BTreeMap<u32, u32> = BTreeMap::new();
        let mut uid = 0u32;
        for (round, (adds, gone_pct)) in rounds.into_iter().enumerate() {
            for _ in 0..adds {
                uid += 1;
                let d = w.add(&mk("INBOX", uid), u64::from(uid), uid, &text(&format!("keep m{uid}"), &[], 0)).unwrap();
                live.insert(uid, d);
            }
            let going: Vec<u32> = live.keys().copied().filter(|u| u % 100 < gone_pct).collect();
            for u in going {
                if let Some(d) = w.find(&mk("INBOX", u)) {
                    w.remove(d);
                }
                live.remove(&u);
            }
            merge_all(&mut w, &s, 1_000_000 + round as u64 * TOMBSTONE_AGE);
            let meta = load_meta(&s, "a1").unwrap().unwrap();
            prop_assert_eq!(meta.live as usize, live.len());
            prop_assert!(meta.next_doc <= 2 * meta.live + RENUMBER_SLACK, "{} ids for {} live", meta.next_doc, meta.live);
        }
        let mut r = Reader::open(&s, "a1").unwrap().unwrap();
        prop_assert_eq!(r.word(&s, Field::Body, "keep").unwrap().count() as usize, live.len());
        if let Some(&u) = live.keys().last() {
            let found: Vec<MessageKey> = r.word(&s, Field::Body, &format!("m{u}")).unwrap().ones().filter_map(|d| r.key(d)).collect();
            prop_assert_eq!(found, vec![mk("INBOX", u)]);
        }
    }
}

/// A removed message's words stay in its segment until it has been removed a day (a merge
/// then leaves them out), on every layout; the rest is found as before.
#[test]
fn a_removed_messages_words_leave_within_a_day() {
    for (kind, dir, s) in layouts("tombstone") {
        let mut w = Writer::open(&s, "a1").unwrap();
        let gone = w.add(&mk("INBOX", 1), 1, 1, &text("secretword shared", &["secret.pdf"], 1)).unwrap();
        let kept = w.add(&mk("INBOX", 2), 2, 2, &text("plainword shared", &[], 0)).unwrap();
        w.flush(&s).unwrap();
        w.remove(gone);
        assert!(!w.merge_step(&s, 0, 5000).unwrap(), "{kind}: nothing to merge yet");
        assert_eq!(load_meta(&s, "a1").unwrap().unwrap().dead_since, 5000, "{kind}: when it was first seen");
        assert!(!w.merge_step(&s, 0, 5000 + TOMBSTONE_AGE - 1).unwrap());
        assert!(segment_words(&s).contains(&key(Field::Body, "secretword")), "{kind}: still there");
        assert!(!w.merge_step(&s, 0, 5000 + TOMBSTONE_AGE).unwrap(), "{kind}: one merge does it");
        let words = segment_words(&s);
        assert!(!words.contains(&key(Field::Body, "secretword")) && !words.contains(&key(Field::Names, "secret")), "{kind}: gone");
        assert!(words.contains(&key(Field::Body, "plainword")));
        let meta = load_meta(&s, "a1").unwrap().unwrap();
        assert_eq!(meta.dead_since, 0, "{kind}: nothing removed is left in a segment");
        let mut r = Reader::open(&s, "a1").unwrap().unwrap();
        assert_eq!(found(&mut r, &s, Field::Body, "shared"), [kept]);
        drop(w);
        assert_eq!(files(&s).1, BTreeSet::new(), "{kind}: nothing left over");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A merge that purges one removed doc and keeps another (its words in a segment not merged)
/// marks only the first purged on the page it writes: the writer reopened, the second's words
/// still leave within a day, on every layout.
#[test]
fn a_removed_doc_kept_by_a_merge_still_leaves_after_a_reopen() {
    for (kind, dir, s) in layouts("kept-tombstone") {
        let mut w = Writer::open(&s, "a1").unwrap();
        for uid in 1..=5 {
            w.add(&mk("INBOX", uid), 0, uid, &text(&format!("big{uid}"), &[], 0)).unwrap();
        }
        w.flush(&s).unwrap();
        for uid in 6..=13 {
            w.add(&mk("INBOX", uid), 0, uid, &text(&format!("small{uid}"), &[], 0)).unwrap();
            w.flush(&s).unwrap();
        }
        w.remove(w.find(&mk("INBOX", 1)).unwrap());
        w.remove(w.find(&mk("INBOX", 6)).unwrap());
        // The eight small segments merged: uid 6's words go, uid 1's stay in the big segment.
        merge_all(&mut w, &s, 1000);
        let words = segment_words(&s);
        assert!(!words.contains(&key(Field::Body, "small6")) && words.contains(&key(Field::Body, "big1")), "{kind}");
        assert_eq!(load_meta(&s, "a1").unwrap().unwrap().dead_since, 1000, "{kind}");
        drop(w);
        let s = reopen(kind, &dir, s);
        let mut w = Writer::open(&s, "a1").unwrap();
        merge_all(&mut w, &s, 1000 + TOMBSTONE_AGE);
        let words = segment_words(&s);
        assert!(!words.contains(&key(Field::Body, "big1")), "{kind}: after a reopen, its words still leave");
        assert!(words.contains(&key(Field::Body, "big2")) && words.contains(&key(Field::Body, "small7")), "{kind}");
        assert_eq!(load_meta(&s, "a1").unwrap().unwrap().dead_since, 0, "{kind}");
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Eight flushes of one size are merged into one; the merge writes its pages as they fill (it
/// holds a page or two of each segment, not the segments), and a crash before its meta leaves
/// the generation before, the pages it wrote removed when the writer next opens.
#[test]
fn a_merge_holds_pages_not_segments() {
    for (kind, dir, s) in layouts("merge") {
        let mut w = Writer::open(&s, "a1").unwrap();
        let mut uid = 0;
        for _ in 0..8 {
            for _ in 0..600 {
                uid += 1;
                let body: Vec<String> = (0..60).map(|j| format!("w{uid}x{j}")).collect();
                w.add(&mk("INBOX", uid), 0, uid, &text(&format!("common {}", body.join(" ")), &[], 0)).unwrap();
            }
            w.flush(&s).unwrap();
        }
        let before = load_meta(&s, "a1").unwrap().unwrap();
        assert_eq!(before.segments.len(), 8);
        let input: u64 = before.segments.iter().map(|s| s.bytes).sum();

        // Stopped before its meta: the generation before, then nothing of it left.
        w.stop_before_meta = true;
        assert!(w.merge_step(&s, 0, 1).is_err(), "{kind}");
        assert_eq!(load_meta(&s, "a1").unwrap().unwrap(), before, "{kind}");
        assert!(!files(&s).1.is_empty(), "{kind}: what it wrote is there");
        drop(w);
        let s = reopen(kind, &dir, s);
        let mut w = Writer::open(&s, "a1").unwrap();
        assert_eq!(files(&s).1, BTreeSet::new(), "{kind}: and removed");

        assert!(!w.merge_step(&s, 0, 1).unwrap(), "{kind}");
        let after = load_meta(&s, "a1").unwrap().unwrap();
        assert_eq!(after.segments.len(), 1, "{kind}");
        let seg = after.segments.first().unwrap();
        assert_eq!((seg.docs, seg.lo, seg.hi), (4800, 0, 4799));
        // A page or two of each of 8 segments, the pages being written (at most the budget, a
        // lexicon page), and the longest list of docs.
        let bound = 8 * (LEX_PAGE + 2 * MAX_KEY + 64 + POST_PAGE + 5) + 3 * LEX_PAGE + POST_PAGE + 4800 * 4;
        assert!(w.merge_peak <= bound && (w.merge_peak as u64) < input / 2, "{kind}: held {} of {input}", w.merge_peak);
        let mut r = Reader::open(&s, "a1").unwrap().unwrap();
        assert_eq!(found(&mut r, &s, Field::Body, "common").len(), 4800);
        assert_eq!(found(&mut r, &s, Field::Body, "w4321x59"), [4320]);
        drop(w);
        assert_eq!(files(&s).1, BTreeSet::new(), "{kind}: nothing left over");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Past twice the live docs and the slack, the ids are given again: the same messages found,
/// their texts and what they are kept, a new epoch, the old files gone.
#[test]
fn ids_are_given_again() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    w.renumber_slack = 4;
    for uid in 1..=20 {
        w.add(&mk("INBOX", uid), u64::from(uid), uid, &text(&format!("common u{uid}"), &[&format!("f{uid}.pdf")], 1)).unwrap();
    }
    w.flush(&s).unwrap();
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    for uid in 1..=16 {
        w.remove(w.find(&mk("INBOX", uid)).unwrap());
    }
    let epoch = load_meta(&s, "a1").unwrap().unwrap().epoch;
    assert!(!w.merge_step(&s, 0, 1).unwrap());
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    assert_eq!((meta.next_doc, meta.live, meta.dead), (4, 4, 0));
    assert_ne!(meta.epoch, epoch);
    assert!(r.refresh(&s).unwrap());
    let common: Vec<u32> = found(&mut r, &s, Field::Body, "common");
    assert_eq!(common.iter().filter_map(|&d| r.key(d)).map(|k| k.uid).collect::<Vec<_>>(), [17, 18, 19, 20]);
    let d = r.doc(&mk("INBOX", 19)).unwrap();
    assert_eq!(r.text(&s, d).unwrap().unwrap().names, ["f19.pdf"]);
    assert_eq!(found(&mut r, &s, Field::Names, "f19"), [d]);
    assert_eq!(w.find(&mk("INBOX", 20)), r.doc(&mk("INBOX", 20)), "the writer goes on with the new ids");
    w.add(&mk("INBOX", 30), 30, 30, &text("later", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    assert!(r.refresh(&s).unwrap());
    assert_eq!(found(&mut r, &s, Field::Body, "later"), [4]);
    drop(w);
    assert_eq!(files(&s).1, BTreeSet::new(), "nothing left over");
}
