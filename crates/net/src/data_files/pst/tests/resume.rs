//! An import that stops part-way goes on from its checkpoint: paused, cancelled, cut off as if
//! the process ended, or with the reader failing on a row, it comes out as the import that never
//! stopped. A file that changed since, or went, isn't gone on with; one Outlook only opened is.

use std::path::Path;

use mailcraft_model::job::{Blocked, SkipReason, StopHow};
use mailcraft_model::pim::{PIM_DOC, Pim};
use mailcraft_store::Store;
use pst_rw::{UnicodePstBatchMessage, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};

use super::super::item::Item;
use super::super::lanes::tests::Fault;
use super::super::out::Out;
use super::super::resume::{self, CHECKPOINT_DOC, CheckState, Checkpoint, Found};
use super::super::{ImportOptions, Imported, Stop, import_with};
use super::pipeline::{data_file, everything};

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

/// Cut off as if the process ended (after a checkpoint, and batches past it), the import goes on
/// from its checkpoint: what was written past it is written again under the same numbers, and it
/// comes out as the import that never stopped.
#[test]
fn cut_off_and_gone_on_with_is_the_same_import() {
    let (dir, pst) = data_file("resume-cut", 300, false);
    let whole = store_at(&dir, "whole");
    import_with(&pst, &whole, "a1", &opts(2, None), &mut |_| {}).unwrap();
    for (n, at) in [150u64, 450, 899].into_iter().enumerate() {
        let name = format!("cut{n}");
        {
            let store = store_at(&dir, &name);
            assert!(import_with(&pst, &store, "a1", &opts(2, Some(Fault::CutOffAfter(at))), &mut |_| {}).is_err());
        }
        let store = store_at(&dir, &name);
        // Cut off before its first checkpoint, it starts again from the beginning.
        match resume::load(&store, "a1") {
            Found::Some(cp) => assert_eq!(cp.state, CheckState::Running, "cut off at {at}"),
            _ => assert!(at < 300, "a folder done is in a checkpoint"),
        }
        let (done, _) = go_on(&pst, &store, None);
        assert_eq!((done.messages, done.skipped), (900, 0), "cut off at {at}");
        assert_eq!(everything(&store, "a1"), everything(&whole, "a1"), "cut off at {at}");
    }
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
    let to = [UnicodePstRecipient { name: "Zoe", email: "zoe@studio.example", recipient_type: UnicodePstRecipientType::To }];
    let more = UnicodePstMessage {
        subject: "One more",
        sender_name: "Ana",
        sender_email: "ana@soundlab.example",
        recipients: &to,
        body: "late",
        html_body: None,
        message_id: "<more@run.example>",
        delivery_time: (1_800_000_000 + 11_644_473_600) * 10_000_000,
    };
    Writer::append_many_in_folder_with_attachments(&changed, &["Inbox"], &[UnicodePstBatchMessage { message: more, attachments: &[] }]).unwrap();
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
    assert!(super::super::checkpoint_from_bytes(&good));
    // The fuzz target's seed is one an import would go on from.
    let seed = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fuzz/seeds/import_checkpoint/paused.json")).unwrap();
    assert!(super::super::checkpoint_from_bytes(&seed));
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
        assert!(!super::super::checkpoint_from_bytes(bad));
    }
}
