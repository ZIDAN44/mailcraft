//! Keeping the index up: merges (against a model of what it should find), ids given again, a
//! removed message's words leaving the file, and plans that bring it up to the mail.

use std::collections::BTreeMap;

use mailcraft_model::{Folder, Summary};
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
    w.add_no_source(&mk("INBOX", 21), 21, 21).unwrap();
    w.flush(&s).unwrap();
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    for uid in 1..=16 {
        w.remove(w.find(&mk("INBOX", uid)).unwrap());
    }
    let epoch = load_meta(&s, "a1").unwrap().unwrap().epoch;
    assert!(!w.merge_step(&s, 0, 1).unwrap());
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    assert_eq!((meta.next_doc, meta.live, meta.dead), (5, 5, 0));
    assert_ne!(meta.epoch, epoch);
    assert!(r.refresh(&s).unwrap());
    let common: Vec<u32> = found(&mut r, &s, Field::Body, "common");
    assert_eq!(common.iter().filter_map(|&d| r.key(d)).map(|k| k.uid).collect::<Vec<_>>(), [17, 18, 19, 20]);
    let d = r.doc(&mk("INBOX", 19)).unwrap();
    assert_eq!(r.text(&s, d).unwrap().unwrap().names, ["f19.pdf"]);
    assert_eq!(found(&mut r, &s, Field::Names, "f19"), [d]);
    let none = r.doc(&mk("INBOX", 21)).unwrap();
    assert!(r.info(none).unwrap().no_source && r.text(&s, none).unwrap().is_none());
    assert_eq!(w.find(&mk("INBOX", 20)), r.doc(&mk("INBOX", 20)), "the writer goes on with the new ids");
    w.add(&mk("INBOX", 30), 30, 30, &text("later", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    assert!(r.refresh(&s).unwrap());
    assert_eq!(found(&mut r, &s, Field::Body, "later"), [5]);
    drop(w);
    assert_eq!(files(&s).1, BTreeSet::new(), "nothing left over");
}

/// A message with no source kept is in the index, with nothing of it but that, read back.
#[test]
fn a_message_with_no_source_is_known_as_such() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let d = w.add_no_source(&mk("INBOX", 1), 9, 99).unwrap();
    w.flush(&s).unwrap();
    drop(w);
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!(r.doc(&mk("INBOX", 1)), Some(d));
    assert_eq!(r.info(d), Some(DocInfo { files: 0, has_text: false, truncated: false, no_source: true, msgid_hash: 9, size: 99 }));
    assert_eq!(r.text(&s, d).unwrap(), None);
    assert!(found(&mut r, &s, Field::Body, "").is_empty());
}

// Plans.

fn summary(folder: &str, uid: u32, id: &str, size: u32, date: i64) -> Summary {
    Summary { key: mk(folder, uid), message_id: id.into(), size, date, ..Summary::default() }
}

/// An account with these folders and messages, the sources of those `kept`.
fn mail(s: &Store, lists: &[(&str, Vec<Summary>)], kept: &[(&str, u32)]) {
    let folders: Vec<Folder> =
        lists.iter().map(|(f, _)| Folder { account: "a1".into(), path: f.to_string(), selectable: true, ..Folder::default() }).collect();
    s.save_folders("a1", &folders).unwrap();
    for (f, messages) in lists {
        s.save_folder("a1", f, &FolderState { validity: 7, uid_next: 100, messages: messages.clone(), ..FolderState::default() }).unwrap();
    }
    for &(f, uid) in kept {
        s.write_body(&mk(f, uid), b"Subject: x\r\n\r\nbody\r\n").unwrap();
    }
}

/// A plan carried out as the indexer does: what reads nothing, the adds, the marks.
fn carry_out(s: &Store, w: &mut Writer, plan: &Plan) {
    plan.apply(w).unwrap();
    for m in &plan.add {
        w.add(&m.key, m.msgid_hash, m.size, &text(&format!("text of {}", m.key.uid), &[], 0)).unwrap();
    }
    for (f, rev) in &plan.marks {
        w.mark(f, *rev).unwrap();
    }
    w.flush(s).unwrap();
}

/// Messages with a source are to add (newest first), those without recorded as such; the plan
/// reads no message; carried out, the next plan has nothing, and passes the folders over.
#[test]
fn a_plan_adds_what_has_a_source_and_records_what_hasnt() {
    let s = Store::memory();
    let inbox = vec![summary("INBOX", 3, "c@x", 30, 300), summary("INBOX", 2, "b@x", 20, 200), summary("INBOX", 1, "a@x", 10, 100)];
    mail(&s, &[("INBOX", inbox), ("Archive", vec![])], &[("INBOX", 1), ("INBOX", 3)]);
    let mut w = Writer::open(&s, "a1").unwrap();
    let reads = s.reads();
    let plan = plan(&s, &w, "a1", &[]).unwrap();
    assert_eq!(s.reads(), reads, "no message read");
    assert_eq!(plan.add.iter().map(|m| m.key.uid).collect::<Vec<_>>(), [3, 1], "newest first");
    assert_eq!(plan.add.first().map(|m| (m.msgid_hash, m.size)), Some((msgid_hash("c@x"), 30)));
    assert_eq!(plan.no_source.iter().map(|m| m.key.uid).collect::<Vec<_>>(), [2]);
    assert_eq!((plan.remove.len(), plan.retarget.len(), plan.total), (0, 0, 3));
    assert_eq!(plan.marks.iter().map(|(f, _)| f.as_str()).collect::<Vec<_>>(), ["Archive", "INBOX"]);
    carry_out(&s, &mut w, &plan);
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert!(r.info(r.doc(&mk("INBOX", 2)).unwrap()).unwrap().no_source);
    assert_eq!(found(&mut r, &s, Field::Body, "text").len(), 2);
    drop(w);

    // Marked: passed over, its list not read again, across a reopen.
    let w = Writer::open(&s, "a1").unwrap();
    assert_eq!(plan_of(&s, &w), Plan { epoch: w.epoch(), ..Plan::default() });
    // Its list changing, or a source kept, brings a folder back.
    s.write_body(&mk("INBOX", 2), b"Subject: y\r\n\r\nbody\r\n").unwrap();
    let again = plan_of(&s, &w);
    assert_eq!((again.total, again.add.len(), again.remove.len()), (3, 1, 1), "the doc with no source goes, the message comes");
    assert_eq!(again.marks.iter().map(|(f, _)| f.as_str()).collect::<Vec<_>>(), ["INBOX"], "the other passed over");
}

fn plan_of(s: &Store, w: &Writer) -> Plan {
    plan(s, w, "a1", &[]).unwrap()
}

/// A message moved (gone from one folder, in another under a new uid, the same Message-ID and
/// size) takes its doc along; one replaced at its uid by another, or one of two alike, is
/// removed and added; a folder gone takes its docs.
#[test]
fn a_plan_moves_removes_and_adds() {
    let s = Store::memory();
    let inbox = vec![summary("INBOX", 1, "a@x", 10, 1), summary("INBOX", 2, "b@x", 20, 2), summary("INBOX", 3, "same@x", 5, 3)];
    let old = vec![summary("Old", 1, "o@x", 1, 1), summary("Old", 2, "same@x", 5, 2)];
    mail(&s, &[("INBOX", inbox), ("Archive", vec![]), ("Old", old)], &[("INBOX", 1), ("INBOX", 2), ("INBOX", 3), ("Old", 1), ("Old", 2)]);
    let mut w = Writer::open(&s, "a1").unwrap();
    let first = plan_of(&s, &w);
    carry_out(&s, &mut w, &first);
    let a = w.find(&mk("INBOX", 1)).unwrap();
    let b = w.find(&mk("INBOX", 2)).unwrap();

    // a moves to Archive; b's uid now holds another message; Old is gone, and its "same@x"
    // matches INBOX's, which moves to Archive too (two alike: no move).
    let inbox = vec![summary("INBOX", 2, "new@x", 20, 4)];
    let archive = vec![summary("Archive", 9, "a@x", 10, 1), summary("Archive", 10, "same@x", 5, 3)];
    mail(&s, &[("INBOX", inbox), ("Archive", archive)], &[("INBOX", 2), ("Archive", 9), ("Archive", 10)]);
    let reads = s.reads();
    let plan = plan_of(&s, &w);
    assert_eq!(s.reads(), reads);
    assert_eq!(plan.retarget, [(a, mk("Archive", 9))]);
    assert_eq!(plan.add.iter().map(|m| m.key.clone()).collect::<Vec<_>>(), [mk("INBOX", 2), mk("Archive", 10)]);
    assert_eq!(plan.remove.len(), 4, "b, both same@x, and Old's other: {:?}", plan.remove);
    assert!(plan.remove.contains(&b));
    assert!(plan.marks.contains(&("Old".to_string(), 0)), "a folder gone is forgotten");
    carry_out(&s, &mut w, &plan);
    assert_eq!(w.marked("Old"), None);
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!(r.doc(&mk("Archive", 9)), Some(a), "the same doc");
    assert_eq!(r.live(), 3);
    assert_eq!(
        found(&mut r, &s, Field::Body, "text").iter().filter_map(|&d| r.key(d)).map(|k| k.uid).collect::<BTreeSet<_>>(),
        BTreeSet::from([9, 2, 10])
    );
    assert_eq!(plan_of(&s, &w), Plan { epoch: w.epoch(), ..Plan::default() }, "up to date");
}

/// A plan of some folders looks at those only.
#[test]
fn a_plan_of_some_folders() {
    let s = Store::memory();
    mail(&s, &[("INBOX", vec![summary("INBOX", 1, "a@x", 1, 1)]), ("Archive", vec![summary("Archive", 1, "b@x", 1, 1)])], &[]);
    let w = Writer::open(&s, "a1").unwrap();
    let plan = plan(&s, &w, "a1", &["Archive".to_string()]).unwrap();
    assert_eq!(plan.no_source.iter().map(|m| m.key.clone()).collect::<Vec<_>>(), [mk("Archive", 1)]);
    assert_eq!(plan.total, 1);
}

/// Two messages swapping uids under a new validity (the server numbered them again): each doc
/// goes to its message's new uid, neither lost.
#[test]
fn a_plan_swapping_uids_keeps_both() {
    let s = Store::memory();
    mail(&s, &[("INBOX", vec![summary("INBOX", 1, "a@x", 10, 1), summary("INBOX", 2, "b@x", 20, 2)])], &[("INBOX", 1), ("INBOX", 2)]);
    let mut w = Writer::open(&s, "a1").unwrap();
    let first = plan_of(&s, &w);
    carry_out(&s, &mut w, &first);
    let (a, b) = (w.find(&mk("INBOX", 1)).unwrap(), w.find(&mk("INBOX", 2)).unwrap());
    let at = |uid, id: &str, size| Summary { key: MessageKey { validity: 8, ..mk("INBOX", uid) }, ..summary("INBOX", uid, id, size, 1) };
    let st = FolderState { validity: 8, uid_next: 100, messages: vec![at(1, "b@x", 20), at(2, "a@x", 10)], ..FolderState::default() };
    s.save_folder("a1", "INBOX", &st).unwrap();
    for (uid, id) in [(1, "b@x"), (2, "a@x")] {
        s.write_body(&MessageKey { validity: 8, ..mk("INBOX", uid) }, id.as_bytes()).unwrap();
    }
    let plan = plan_of(&s, &w);
    assert_eq!((plan.retarget.len(), plan.add.len(), plan.remove.len()), (2, 0, 0), "{plan:?}");
    carry_out(&s, &mut w, &plan);
    assert_eq!(w.live(), 2);
    assert_eq!(w.find(&MessageKey { validity: 8, ..mk("INBOX", 1) }), Some(b));
    assert_eq!(w.find(&MessageKey { validity: 8, ..mk("INBOX", 2) }), Some(a));
    let r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!(r.live(), 2);
    assert_eq!(r.key(a).map(|k| (k.validity, k.uid)), Some((8, 2)));
}

/// A plan made before the ids were given again is refused, nothing changed.
#[test]
fn a_plan_from_before_new_ids_is_refused() {
    let s = Store::memory();
    mail(&s, &[("INBOX", vec![summary("INBOX", 1, "a@x", 10, 1)])], &[("INBOX", 1)]);
    let mut w = Writer::open(&s, "a1").unwrap();
    w.renumber_slack = 0;
    for uid in 10..14 {
        w.add(&mk("INBOX", uid), 0, uid, &text("old", &[], 0)).unwrap();
    }
    w.flush(&s).unwrap();
    let plan = plan_of(&s, &w);
    assert_eq!(plan.remove.len(), 4);
    for uid in 10..13 {
        w.remove(w.find(&mk("INBOX", uid)).unwrap());
    }
    merge_all(&mut w, &s, 1);
    assert_ne!(w.epoch(), plan.epoch, "the ids were given again");
    let live = w.live();
    assert!(plan.apply(&mut w).is_err());
    assert_eq!(w.live(), live, "nothing changed");
}

/// A removed message that had no source never had words: no merge is due for it.
#[test]
fn a_removed_message_with_no_source_needs_no_merge() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    w.add(&mk("INBOX", 1), 1, 1, &text("kept", &[], 0)).unwrap();
    let none = w.add_no_source(&mk("INBOX", 2), 2, 2).unwrap();
    w.flush(&s).unwrap();
    w.remove(none);
    let segments = load_meta(&s, "a1").unwrap().unwrap().segments;
    assert!(!w.merge_step(&s, 0, 1000).unwrap());
    assert!(!w.merge_step(&s, 0, 1000 + TOMBSTONE_AGE).unwrap());
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    assert_eq!((meta.segments, meta.dead_since), (segments, 0), "the segment left as it was");
}
