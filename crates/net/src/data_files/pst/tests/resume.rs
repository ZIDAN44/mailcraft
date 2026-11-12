//! An import that stops part-way goes on from its checkpoint: paused, cancelled, cut off as if
//! the process ended, or with the reader failing on a row, it comes out as the import that never
//! stopped. A file that changed since, or went, isn't gone on with; one Outlook only opened is.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use mailcraft_model::job::{Blocked, SkipReason, StopHow};
use mailcraft_model::pim::{PIM_DOC, Pim};
use mailcraft_pst::write::{self, BatchMessage, NewMessage, NewRecipient, RecipientType};
use mailcraft_store::Store;
use mailcraft_store::search::{self, Writer as IndexWriter};

use super::super::item::Item;
use super::super::lanes::tests::Fault;
use super::super::out::Out;
use super::super::resume::{self, CHECKPOINT_DOC, CheckState, Checkpoint, Found};
use super::super::{ImportOptions, Imported, Stop, import_with, keep_as_is};
use super::pipeline::{data_file, everything};
use crate::space::{Filling, LOW_SPACE};

fn store_at(dir: &Path, name: &str) -> Store {
    Store::open_packed(dir.join(name).join("app"), dir.join(name).join("Files")).unwrap()
}

fn opts(lanes: usize, fault: Option<Fault>) -> ImportOptions {
    ImportOptions { lanes, fault, ..ImportOptions::default() }
}

/// An import into `store`'s `a1` stopped as `how` once `at` items are read.
fn stopped_at(pst: &Path, store: &Store, at: usize, how: StopHow, fault: Option<Fault>) -> Imported {
    let stop = Stop::default();
    let o = ImportOptions { stop: stop.clone(), ..opts(2, fault) };
    import_with(pst, store, "a1", &o, &mut |p| {
        if p.read >= at {
            stop.raise(how);
        }
    })
    .unwrap()
}

/// Go on with the import into `store`'s `a1`; and how many items had been read at its first word
/// of progress.
fn go_on(pst: &Path, store: &Store, fault: Option<Fault>) -> (Imported, usize) {
    let mut first = None;
    let o = ImportOptions { resume: true, ..opts(2, fault) };
    let done = import_with(pst, store, "a1", &o, &mut |p| {
        first.get_or_insert(p.read);
    })
    .unwrap();
    (done, first.unwrap_or(0))
}

/// Options for an import whose checkpoints come every few milliseconds, with batches of a few
/// messages between them: a cut lands part-way through a folder, past its last checkpoint.
fn often(fault: Option<Fault>) -> ImportOptions {
    ImportOptions { batch_bytes: 3000, checkpoint_every: Some(Duration::from_millis(15)), ..opts(2, fault) }
}

/// Every folder the search index has marked as brought up to the mail has every message it lists
/// in the index (one that hasn't is left for the next pass, unmarked).
fn indexed_where_marked(store: &Store) {
    let w = IndexWriter::open(store, "a1").unwrap();
    for f in store.load_folders("a1").unwrap() {
        let rev = search::revision(store, "a1", &f.path);
        if rev.is_none() || w.marked(&f.path) != rev {
            continue;
        }
        let st = store.load_folder("a1", &f.path).unwrap();
        let missing = st.messages.iter().filter(|m| w.find(&m.key).is_none()).count();
        assert_eq!(missing, 0, "{}: marked as up to date with {missing} of {} not indexed", f.path, st.messages.len());
    }
}

fn checkpoint(store: &Store) -> Checkpoint {
    match resume::load(store, "a1") {
        Found::Some(cp) => *cp,
        _ => panic!("no checkpoint"),
    }
}

/// Paused part-way through the first folder or the second, or cancelled, and gone on with, an
/// import is the one that never stopped: the same folders, lists, numbers and sources. Going on
/// reads only what wasn't read; one done says what it came to without reading the file.
#[test]
fn paused_and_gone_on_with_is_the_same_import() {
    let (dir, pst) = data_file("resume-pause", 300, false);
    let whole = store_at(&dir, "whole");
    let all = import_with(&pst, &whole, "a1", &opts(2, None), &mut |_| {}).unwrap();
    assert_eq!(all.messages, 900);
    assert_eq!(checkpoint(&whole).state, CheckState::Done, "an import done says so");
    for (n, (at, how)) in [(200, StopHow::Pause), (400, StopHow::Pause), (600, StopHow::Cancel), (200, StopHow::Cancel)].into_iter().enumerate() {
        let store = store_at(&dir, &format!("part{n}"));
        let part = stopped_at(&pst, &store, at, how, None);
        assert_eq!(part.stopped, Some(how), "{at} {how:?}");
        let cp = checkpoint(&store);
        assert_eq!(cp.state, if how == StopHow::Pause { CheckState::Running } else { CheckState::Cancelled });
        assert!(cp.current.is_some(), "{at} {how:?}: stopped part-way through a folder");
        let (done, from) = go_on(&pst, &store, None);
        assert_eq!((done.messages, done.skipped, done.runs), (900, 0, 2), "{at} {how:?}");
        assert!(from > at, "{at} {how:?}: went on from {from}");
        assert_eq!(everything(&store, "a1"), everything(&whole, "a1"), "{at} {how:?}");
        assert_eq!(checkpoint(&store).state, CheckState::Done);
        let (again, _) = go_on(&dir.join("gone.pst"), &store, None);
        assert_eq!((again.messages, again.blocked, again.folders.len()), (900, None, all.folders.len()));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cut off as if the process ended (after a checkpoint part-way through a folder, and batches
/// past it), the import goes on from its checkpoint: what was written past it goes and is written
/// again under the same numbers, and it comes out as the import that never stopped, each message
/// once, and searchable or left for the next pass to look at.
#[test]
fn cut_off_and_gone_on_with_is_the_same_import() {
    let (dir, pst) = data_file("resume-cut", 300, false);
    let whole = store_at(&dir, "whole");
    import_with(&pst, &whole, "a1", &opts(2, None), &mut |_| {}).unwrap();
    let (mut part_way, mut written_past) = (0, 0);
    for (n, at) in [150u64, 250, 450, 520, 899].into_iter().enumerate() {
        let name = format!("cut{n}");
        {
            let store = store_at(&dir, &name);
            assert!(import_with(&pst, &store, "a1", &often(Some(Fault::CutOffAfter(at))), &mut |_| {}).is_err());
        }
        let store = store_at(&dir, &name);
        // Cut off before its first checkpoint, it starts again from the beginning.
        match resume::load(&store, "a1") {
            Found::Some(cp) => {
                assert_eq!(cp.state, CheckState::Running, "cut off at {at}");
                if let Some(c) = cp.current.as_ref().filter(|c| c.tail.is_some()) {
                    part_way += 1;
                    written_past += store.cached_bodies("a1", &c.path, 1).into_iter().filter(|u| *u >= c.uid_next).count();
                }
            }
            Found::Nothing => assert!(at < 300, "a folder done is in a checkpoint"),
            Found::Unsound => panic!("cut off at {at}: the checkpoint written doesn't add up"),
        }
        let (done, _) = go_on(&pst, &store, None);
        assert_eq!((done.messages, done.skipped, done.blocked), (900, 0, None), "cut off at {at}");
        assert_eq!(everything(&store, "a1"), everything(&whole, "a1"), "cut off at {at}");
        indexed_where_marked(&store);
    }
    assert!(part_way >= 3, "only {part_way} cuts went on from part-way through a folder");
    assert!(written_past > 0, "no source was written past a checkpoint");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Paused, gone on with and cut off part-way through, then gone on with again, the import is the
/// one that never stopped: the checkpoints the second run wrote count each row once.
#[test]
fn paused_then_cut_off_and_gone_on_with_is_the_same_import() {
    let (dir, pst) = data_file("resume-pause-cut", 300, false);
    let whole = store_at(&dir, "whole");
    import_with(&pst, &whole, "a1", &opts(2, None), &mut |_| {}).unwrap();
    {
        let store = store_at(&dir, "part");
        stopped_at(&pst, &store, 100, StopHow::Pause, None);
        let o = ImportOptions { resume: true, ..often(Some(Fault::CutOffAfter(280))) };
        assert!(import_with(&pst, &store, "a1", &o, &mut |_| {}).is_err());
    }
    let store = store_at(&dir, "part");
    let (done, _) = go_on(&pst, &store, None);
    assert_eq!((done.messages, done.skipped), (900, 0));
    assert_eq!(everything(&store, "a1"), everything(&whole, "a1"));
    indexed_where_marked(&store);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cancelled as it goes on from a pause, before the folder part-way through is reached again,
/// the import keeps that folder's list as the pause left it: gone on with, it's the one that
/// never stopped.
#[test]
fn cancelled_before_the_paused_folder_is_reached_keeps_its_list() {
    let (dir, pst) = data_file("resume-pause-cancel", 300, false);
    let whole = store_at(&dir, "whole");
    import_with(&pst, &whole, "a1", &opts(2, None), &mut |_| {}).unwrap();
    let store = store_at(&dir, "part");
    stopped_at(&pst, &store, 200, StopHow::Pause, None);
    let cp = checkpoint(&store);
    assert!(cp.current.as_ref().is_some_and(|c| c.tail.is_some()), "paused part-way through a folder");
    let source = cp.source.clone();
    {
        // Going on: the file is known, then the user cancels.
        let stop = Stop::default();
        let o = ImportOptions { stop: stop.clone(), ..opts(2, None) };
        let mut sink = |_| {};
        let mut out = Out::new(&store, "a1", &o, &mut sink);
        out.resume_from(cp);
        out.source_known(&pst, source.record_key, source.fingerprint);
        stop.raise(StopHow::Cancel);
        assert!(out.stopped());
        out.finish();
    }
    let cp = checkpoint(&store);
    assert_eq!(cp.state, CheckState::Cancelled);
    let (done, _) = go_on(&pst, &store, None);
    assert_eq!((done.messages, done.skipped), (900, 0));
    assert_eq!(everything(&store, "a1"), everything(&whole, "a1"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Gone on with from a checkpoint (after a cut, nothing at its end saved them), the import goes
/// on with the due and done days read before it: they're in what the next checkpoint writes,
/// each note cleaned of what isn't a day (a key too long, a day out of range, no key: the notes
/// are from disk).
#[test]
fn going_on_keeps_the_due_days_read_before_the_checkpoint() {
    use mailcraft_model::organize::{FOLLOW_UP_DOC, FOLLOW_UP_DONE_DOC, MAX_FOLLOW_UP_ID};
    use std::collections::BTreeMap;
    let (dir, pst) = data_file("resume-due", 5, false);
    let store = store_at(&dir, "part");
    stopped_at(&pst, &store, 3, StopHow::Pause, None);
    let cp = checkpoint(&store);
    let read = BTreeMap::from([
        ("before@x".to_string(), 20_400),
        ("x".repeat(MAX_FOLLOW_UP_ID + 1), 20_400),
        ("far@x".to_string(), i64::MAX),
        ("early@x".to_string(), -1_000_000),
    ]);
    store.save_account_doc("a1", FOLLOW_UP_DOC, &read).unwrap();
    let done = BTreeMap::from([
        ("finished@x".to_string(), 20_390),
        ("x".repeat(MAX_FOLLOW_UP_ID + 1), 20_390),
        ("far@x".to_string(), i64::MAX - 3),
        (" ".to_string(), 20_390),
    ]);
    store.save_account_doc("a1", FOLLOW_UP_DONE_DOC, &done).unwrap();
    let o = opts(2, None);
    let mut sink = |_| {};
    let mut out = Out::new(&store, "a1", &o, &mut sink);
    out.resume_from(cp);
    let docs = out.due_doc();
    let doc = |name: &str| -> BTreeMap<String, i64> {
        let (_, bytes) = docs.iter().find(|(n, _)| *n == name).expect("cleaned, written with the next checkpoint");
        serde_json::from_slice(bytes).unwrap()
    };
    assert_eq!(doc(FOLLOW_UP_DOC), BTreeMap::from([("before@x".to_string(), 20_400)]));
    assert_eq!(doc(FOLLOW_UP_DONE_DOC), BTreeMap::from([("finished@x".to_string(), 20_390)]));
    drop(out);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Paused while an item is read from a slow drive, the import stops at the read in hand, not at
/// the end of the item (each read a second here: an item takes several, the first, the file
/// opened with it, more): no read begins after the pause, nothing half read is kept or counted,
/// and gone on with, it's the import that never stopped.
#[test]
fn a_pause_stops_part_way_through_an_item() {
    let (dir, pst) = data_file("resume-slow-read", 40, true);
    let whole = store_at(&dir, "whole");
    import_with(&pst, &whole, "a1", &opts(2, None), &mut |_| {}).unwrap();
    let store = store_at(&dir, "part");
    let stop = Stop::default();
    let o = ImportOptions { stop: stop.clone(), ..opts(1, Some(Fault::SlowReads(1000))) };
    let reads = || super::super::lanes::tests::slow_reads(&pst);
    let at_pause = std::thread::scope(|s| {
        let asked = s.spawn(|| {
            // Paused part-way through the first item, its second read in hand (a guard against
            // hanging only: the reads are counted, not timed).
            let end = std::time::Instant::now() + Duration::from_secs(60);
            while reads() < 2 && std::time::Instant::now() < end {
                std::thread::sleep(Duration::from_millis(10));
            }
            stop.raise(StopHow::Pause);
            reads()
        });
        let part = import_with(&pst, &store, "a1", &o, &mut |_| {}).unwrap();
        assert_eq!((part.stopped, part.messages, part.skipped), (Some(StopHow::Pause), 0, 0));
        asked.join().unwrap()
    });
    // The one lane's reads, one after another: none begun after the pause. One may be counted
    // just after it, its look at the pause just before (the read in hand all the same).
    assert!(at_pause >= 2, "paused before the item's second read");
    assert!(reads() <= at_pause + 1, "{} reads, {at_pause} begun as it was paused", reads());
    let (done, _) = go_on(&pst, &store, None);
    assert_eq!((done.messages, done.skipped), (120, 0));
    assert_eq!(everything(&store, "a1"), everything(&whole, "a1"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Kept as it is after a pause, or after being cut off past its checkpoint, an import's folder
/// part-way through comes into the folder list with the messages its checkpoint names, numbered
/// as they came in, those written past it gone: every message listed has its source, the import
/// says it's done (gone on with, it answers without reading the file), and keeping it again
/// changes nothing.
#[test]
fn keeping_a_paused_import_lists_the_folder_part_way_through() {
    let (dir, pst) = data_file("resume-keep", 300, false);
    let mut written_past = 0;
    for (n, cut) in [None, Some(Fault::CutOffAfter(450)), Some(Fault::CutOffAfter(520))].into_iter().enumerate() {
        let store = store_at(&dir, &format!("keep{n}"));
        match cut {
            None => assert_eq!(stopped_at(&pst, &store, 400, StopHow::Pause, None).stopped, Some(StopHow::Pause)),
            Some(fault) => assert!(import_with(&pst, &store, "a1", &often(Some(fault)), &mut |_| {}).is_err()),
        }
        let cp = checkpoint(&store);
        let part = cp.current.clone().filter(|c| c.tail.is_some()).expect("part-way through a folder");
        assert!(!store.load_folders("a1").unwrap().iter().any(|f| f.path == part.path), "{n}: kept out of the list");
        written_past += store.cached_bodies("a1", &part.path, 1).into_iter().filter(|u| *u >= part.uid_next).count();
        // Its account removed meanwhile: nothing is written.
        let removed = Stop::default();
        removed.raise(StopHow::Discard);
        assert!(keep_as_is(&store, "a1", &removed).is_err(), "{n}");
        assert!(!store.load_folders("a1").unwrap().iter().any(|f| f.path == part.path), "{n}: listed when thrown away");
        assert_ne!(checkpoint(&store).state, CheckState::Done, "{n}");
        let kept = keep_as_is(&store, "a1", &Stop::default()).unwrap();
        assert!(kept.iter().any(|f| f.path == part.path), "{n}: kept {kept:?}");
        let mut listed = 0;
        for f in store.load_folders("a1").unwrap() {
            let st = store.load_folder("a1", &f.path).unwrap();
            assert_eq!(f.total as usize, st.messages.len(), "{n}: {}", f.path);
            assert!(st.messages.iter().all(|m| store.read_body(&m.key).is_some()), "{n}: {}", f.path);
            listed += st.messages.len();
        }
        assert_eq!(listed as u64, cp.counts.imported, "{n}");
        let mut uids: Vec<u32> = store.load_folder("a1", &part.path).unwrap().messages.iter().map(|m| m.key.uid).collect();
        uids.sort_unstable();
        assert_eq!(uids, (1..part.uid_next).collect::<Vec<u32>>(), "{n}");
        assert!(store.cached_bodies("a1", &part.path, 1).into_iter().all(|u| u < part.uid_next), "{n}: written past it");
        assert_eq!(checkpoint(&store).state, CheckState::Done);
        assert!(keep_as_is(&store, "a1", &Stop::default()).unwrap().is_empty(), "{n}: kept again");
        let (done, _) = go_on(&pst, &store, None);
        assert_eq!(done.messages as u64, cp.counts.imported, "{n}");
    }
    assert!(written_past > 0, "no source was written past a checkpoint");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A row the reader fails on is counted once and never read again: gone on with, the import
/// skips it, and comes out as one that never stopped with the same failures.
#[test]
fn a_row_the_reader_failed_on_is_not_read_again() {
    let (dir, pst) = data_file("resume-panic", 120, false);
    let fault = Some(Fault::PanicRow(30));
    let whole = store_at(&dir, "whole");
    let all = import_with(&pst, &whole, "a1", &opts(2, fault), &mut |_| {}).unwrap();
    assert_eq!((all.messages, all.skips.get(&SkipReason::MessageUnreadable)), (357, Some(&3)));
    let store = store_at(&dir, "part");
    stopped_at(&pst, &store, 200, StopHow::Pause, fault);
    let cp = checkpoint(&store);
    assert!(cp.failed.len() >= 2 && cp.counts.read >= 200, "{:?}", cp.failed);
    let (done, _) = go_on(&pst, &store, fault);
    assert_eq!((done.messages, done.skipped, done.skips.get(&SkipReason::MessageUnreadable)), (357, 3, Some(&3)));
    assert_eq!(everything(&store, "a1"), everything(&whole, "a1"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file with a message more since the import stopped isn't gone on with, nor one that's gone,
/// nor from a checkpoint that doesn't add up: the import waits on the user, nothing read or
/// written. One only opened by Outlook since (its time changed, nothing it holds) is gone on with.
#[test]
fn a_changed_or_missing_file_is_not_gone_on_with() {
    let (dir, pst) = data_file("resume-changed", 300, false);
    let whole = store_at(&dir, "whole");
    import_with(&pst, &whole, "a1", &opts(2, None), &mut |_| {}).unwrap();

    let touched = dir.join("touched.pst");
    std::fs::copy(&pst, &touched).unwrap();
    let copy = store_at(&dir, "copy");
    stopped_at(&touched, &copy, 400, StopHow::Pause, None);
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
    std::fs::File::options().write(true).open(&touched).unwrap().set_modified(later).unwrap();
    // Paused again before the file is planned whole: a stop, not a file that changed.
    let stop = Stop::default();
    stop.raise(StopHow::Pause);
    let o = ImportOptions { stop, resume: true, ..opts(2, None) };
    let paused = import_with(&touched, &copy, "a1", &o, &mut |_| {}).unwrap();
    assert_eq!((paused.stopped, paused.blocked), (Some(StopHow::Pause), None));
    let (done, _) = go_on(&touched, &copy, None);
    assert_eq!((done.messages, done.blocked), (900, None));
    assert_eq!(everything(&copy, "a1"), everything(&whole, "a1"));

    let changed = dir.join("changed.pst");
    std::fs::copy(&pst, &changed).unwrap();
    let store = store_at(&dir, "part");
    stopped_at(&changed, &store, 400, StopHow::Pause, None);
    let before = (checkpoint(&store), store.load_folders("a1").unwrap());
    let (done, _) = go_on(&dir.join("gone.pst"), &store, None);
    assert_eq!(done.blocked, Some(Blocked::SourceMissing));
    let to = [NewRecipient { name: "Zoe", email: "zoe@studio.example", recipient_type: RecipientType::To, ..Default::default() }];
    let more = NewMessage {
        subject: "One more",
        sender_name: "Ana",
        sender_email: "ana@soundlab.example",
        recipients: &to,
        body: "late",
        html_body: None,
        message_id: "<more@run.example>",
        delivery_time: (1_800_000_000 + 11_644_473_600) * 10_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    // A message more, in a file as big as it was and dated as it was (written in the second it
    // was last written in, or by a program that puts the date back): what it holds says so.
    let dated = std::fs::metadata(&changed).unwrap().modified().unwrap();
    let as_it_was = resume::facts(&changed);
    write::append(&changed, &["Inbox"], &[BatchMessage { message: more, attachments: &[] }]).unwrap();
    std::fs::File::options().write(true).open(&changed).unwrap().set_modified(dated).unwrap();
    assert_eq!(resume::facts(&changed), as_it_was, "the file's size and date don't tell");
    let (done, _) = go_on(&changed, &store, None);
    assert_eq!((done.blocked, done.messages), (Some(Blocked::SourceChanged), 0));
    assert_eq!((checkpoint(&store), store.load_folders("a1").unwrap()), before, "nothing written");
    store.save_account_doc("a1", CHECKPOINT_DOC, &serde_json::json!({"version": 9})).unwrap();
    assert_eq!(go_on(&pst, &store, None).0.blocked, Some(Blocked::SourceChanged));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Contacts and calendar items read before a checkpoint are in the account with it, and an import
/// gone on from it keeps them and numbers the next after them.
#[test]
fn contacts_and_calendar_survive_with_their_checkpoint() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-resume-pim-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = store_at(&dir, "s");
    let note = |text: &str| Item {
        class: "IPM.StickyNote".into(),
        subject: text.into(),
        body: text.into(),
        date: 1_700_000_000,
        extra: Some(crate::pst_pim::Extra::default()),
        ..Default::default()
    };
    {
        let mut sink = |_| {};
        let mut out = Out::new(&store, "a1", &ImportOptions::default(), &mut sink);
        out.source_known(&dir.join("in.pst"), "00ff".into(), 7);
        out.begin_pim_folder(0x8062);
        out.add_pim(note("Poster"));
        out.row_done(0x8062, 0, false);
        out.checkpoint();
        // Cut off here: the rest isn't written.
        out.add_pim(note("Lost with the crash"));
    }
    let pim: Pim = store.load_account_doc("a1", PIM_DOC).unwrap();
    assert_eq!(pim.notes.len(), 1, "written with the checkpoint");
    let cp = checkpoint(&store);
    assert!(cp.pim);
    assert_eq!(cp.current.as_ref().map(|c| (c.node, c.next_row)), Some((0x8062, 1)));
    let mut sink = |_| {};
    let mut out = Out::new(&store, "a1", &ImportOptions::default(), &mut sink);
    let skip = out.resume_from(cp);
    assert_eq!(skip.start(0x8062), 1, "the row read isn't read again");
    out.add_pim(note("Plan"));
    let done = out.finish();
    assert_eq!(done.notes, 2);
    let pim: Pim = store.load_account_doc("a1", PIM_DOC).unwrap();
    assert_eq!(pim.notes.len(), 2);
    assert_ne!(pim.notes[0].note.uid, pim.notes[1].note.uid, "numbered after those read before");
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A checkpoint read from a file is checked: one that doesn't add up isn't gone on from, and one
/// that does reads back as written.
#[test]
fn a_checkpoint_that_does_not_add_up_is_refused() {
    let inbox = mailcraft_model::Folder { path: "INBOX".into(), ..Default::default() };
    let good = serde_json::json!({"version": 1, "folders": [inbox], "done": [{"node": 1, "path": "INBOX"}], "current": {"node": 2, "nextRow": 3, "uidNext": 1}});
    let good = serde_json::to_vec(&good).unwrap();
    assert!(Checkpoint::parse(&good).is_some());
    assert_eq!(super::super::checkpoint_from_bytes(&good), Some(true));
    // The fuzz target's seed is one an import would go on from.
    let seed = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fuzz/seeds/import_checkpoint/paused.json")).unwrap();
    assert_eq!(super::super::checkpoint_from_bytes(&seed), Some(true));
    let seed = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fuzz/seeds/import_checkpoint/sent_put_off.json")).unwrap();
    let cp = Checkpoint::parse(&seed).unwrap();
    assert_eq!((cp.deferred.is_some(), cp.sent.replies.len(), cp.sent.frozen), (true, 2, true));
    assert_eq!(super::super::checkpoint_from_bytes(&seed), Some(true));
    for bad in [
        &br#"{"version":2}"#[..],
        br#"{"version":1,"folders":[{"path":"A"},{"path":"A"}]}"#,
        br#"{"version":1,"done":[{"node":1,"path":"Nowhere"}]}"#,
        br#"{"version":1,"done":[{"node":1},{"node":1}]}"#,
        br#"{"version":1,"current":{"node":2,"uidNext":0}}"#,
        br#"{"version":1,"done":[{"node":2}],"current":{"node":2,"uidNext":1}}"#,
        br#"{"version":1,"folders":[{"path":"A"}],"current":{"node":2,"path":"A","uidNext":1,"tail":{"pages":[1],"inPages":2}}}"#,
        br#"{"version":1,"current":{"node":2,"uidNext":1,"tail":{}}}"#,
        br#"not json"#,
    ] {
        assert!(Checkpoint::parse(bad).is_none(), "{}", String::from_utf8_lossy(bad));
        assert_eq!(super::super::checkpoint_from_bytes(bad), None);
    }
}

/// The drive filling up part-way: the import stops itself as a pause does (nothing counted as
/// left out, the checkpoint saying where it got) and says why; gone on with once there's room,
/// it's the import that never stopped. Too full to begin, it reads and writes nothing. "Full" is
/// less than [`LOW_SPACE`] and what the data file can grow by in one step: a drive with more than
/// `LOW_SPACE` but not that is full already.
#[test]
fn a_full_drive_pauses_and_room_again_goes_on() {
    let (dir, pst) = data_file("resume-full", 300, false);
    let whole = store_at(&dir, "whole");
    import_with(&pst, &whole, "a1", &opts(2, None), &mut |_| {}).unwrap();
    let store = store_at(&dir, "part");
    let left = LOW_SPACE + (32 << 20);
    // Full from the second ask (the first checkpoint) on: a later ask can come after the last
    // message on a slow machine, whose checkpoints bunch up, and then nothing ever fills.
    let o = ImportOptions { space: Some(Arc::new(Filling::new(1, left))), ..often(None) };
    let part = import_with(&pst, &store, "a1", &o, &mut |_| {}).unwrap();
    assert_eq!(part.blocked, Some(Blocked::DiskFull { free: left }));
    assert_eq!((part.stopped, part.skipped), (Some(StopHow::Pause), 0));
    assert!(part.skips.is_empty(), "{:?}", part.skips);
    assert!(part.messages > 0 && part.messages < 900, "stopped part-way: {}", part.messages);
    assert_eq!(checkpoint(&store).state, CheckState::Running);
    let (done, _) = go_on(&pst, &store, None);
    assert_eq!((done.messages, done.skipped, done.runs, done.blocked), (900, 0, 2, None));
    assert_eq!(everything(&store, "a1"), everything(&whole, "a1"));

    // Too full to begin: nothing is read, and no data file is made.
    let empty = store_at(&dir, "empty");
    let o = ImportOptions { space: Some(Arc::new(Filling::new(0, left))), ..opts(2, None) };
    let mut told = 0;
    let none = import_with(&pst, &empty, "a1", &o, &mut |_| told += 1).unwrap();
    assert_eq!((none.blocked, none.messages, told), (Some(Blocked::DiskFull { free: left }), 0, 0));
    assert!(empty.data_file("a1").is_none_or(|f| !f.exists()), "no data file");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The drive filling up between two looks at it (something else wrote to it meanwhile): the
/// write it refuses stops the import as a full drive does, nothing more is written and nothing
/// is counted as left out; it says the messages its last checkpoint had, and gone on with once
/// there's room, it's the import that never stopped.
#[test]
fn a_write_the_full_drive_refuses_pauses_and_room_again_goes_on() {
    let (dir, pst) = data_file("resume-refused", 300, false);
    let whole = Store::failing();
    import_with(&pst, &whole, "a1", &opts(2, None), &mut |_| {}).unwrap();
    let roomy = || Some(Arc::new(Filling::new(usize::MAX, 0)) as Arc<dyn crate::space::Space>);
    for at in [200, 450] {
        let store = Store::failing();
        let o = ImportOptions { space: roomy(), ..often(None) };
        let part = import_with(&pst, &store, "a1", &o, &mut |p| {
            if p.read >= at {
                store.fail(None, Some("#.eml"));
            }
        })
        .unwrap();
        assert!(matches!(part.blocked, Some(Blocked::DiskFull { .. })), "{at}: {:?}", part.blocked);
        assert_eq!((part.stopped, part.skipped), (Some(StopHow::Pause), 0), "{at}");
        assert!(!part.skips.contains_key(&SkipReason::NotWritten), "{at}: {:?}", part.skips);
        let cp = checkpoint(&store);
        assert_eq!((cp.state, part.messages), (CheckState::Running, usize::try_from(cp.counts.imported).unwrap()), "{at}");
        assert!(part.messages < 900, "{at}: stopped part-way: {}", part.messages);
        store.fail(None, None);
        let o = ImportOptions { resume: true, space: roomy(), ..opts(2, None) };
        let done = import_with(&pst, &store, "a1", &o, &mut |_| {}).unwrap();
        assert_eq!((done.messages, done.skipped, done.runs, done.blocked), (900, 0, 2, None), "{at}");
        assert_eq!(everything(&store, "a1"), everything(&whole, "a1"), "{at}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A write to the account's data file failing at the disk while the import runs (one from the
/// window, on a drive that filled for a moment): the store opens the file again at its last
/// durable commit, so the batches the import wrote without waiting for the disk since may be
/// gone. The import's next write sees it and stops as a pause does, writing nothing more: no
/// checkpoint names what went, nothing is counted as left out, and gone on with it's the import
/// that never stopped. Going on regardless, it named sources that were gone.
#[test]
fn a_data_file_opened_again_after_a_failed_write_stops_the_import() {
    let (dir, pst) = data_file("resume-reopened", 300, false);
    let whole = store_at(&dir, "whole");
    import_with(&pst, &whole, "a1", &opts(2, None), &mut |_| {}).unwrap();
    let roomy = || Some(Arc::new(Filling::new(usize::MAX, 0)) as Arc<dyn crate::space::Space>);
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    let big: Vec<u8> = (0..64 << 20)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    let elsewhere = mailcraft_model::MessageKey { account: "a1".into(), folder: "Elsewhere".into(), validity: 1, uid: 1 };
    for at in [200, 450] {
        let store = store_at(&dir, &format!("part-{at}"));
        let o = ImportOptions { batch_bytes: 3000, space: roomy(), ..opts(2, None) };
        let mut failed = false;
        let part = import_with(&pst, &store, "a1", &o, &mut |p| {
            if p.read >= at && !failed {
                failed = true;
                store.fill_drive(true);
                assert!(store.write_body(&elsewhere, &big).is_err(), "{at}: the drive is full");
                store.fill_drive(false);
            }
        })
        .unwrap();
        assert!(failed, "{at}");
        assert_eq!(store.data_file_reopened("a1"), 1, "{at}");
        assert_eq!((part.stopped, part.skipped, part.blocked), (Some(StopHow::Pause), 0, None), "{at}");
        assert!(!part.skips.contains_key(&SkipReason::NotWritten), "{at}: {:?}", part.skips);
        assert!(part.messages < 900, "{at}: stopped part-way: {}", part.messages);
        if let Found::Some(cp) = resume::load(&store, "a1") {
            assert_eq!((cp.state, part.messages), (CheckState::Running, usize::try_from(cp.counts.imported).unwrap()), "{at}");
        }
        let o = ImportOptions { resume: true, space: roomy(), ..opts(2, None) };
        let done = import_with(&pst, &store, "a1", &o, &mut |_| {}).unwrap();
        assert_eq!((done.messages, done.skipped, done.blocked), (900, 0, None), "{at}");
        assert_eq!(everything(&store, "a1"), everything(&whole, "a1"), "{at}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
