//! The Sent Items folder is made in its place and read after every other, so its items can be
//! given the ids the replies read before them answer (`out/sent_ids.rs`): the folders come out as
//! they would, it's read even when the walk failed before it, and an import stopped before or
//! while it's read goes on to the same import.

use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mailcraft_model::Folder;
use mailcraft_model::job::{SkipReason, StopHow};
use mailcraft_store::{FolderState, Store};

use super::super::item::Item;
use super::super::lanes::tests::Fault;
use super::super::lanes::{Ctx, Job, Outcome, work};
use super::super::order::Step;
use super::super::out::Out;
use super::super::out::sent_ids::SentIds;
use super::super::resume::{self, CheckState, Found};
use super::super::resume::{Checkpoint, Current};
use super::super::{ImportOptions, ImportSink, Imported, Progress, Stop, import_into, import_with};
use super::pipeline::{data_file, everything};

/// The file's "Projects" folder taken for its Sent Items: a folder with one under it, between
/// two others.
const SENT: &str = "Projects";

fn opts(lanes: usize, sent: bool, fault: Option<Fault>) -> ImportOptions {
    ImportOptions { lanes, fault, sent_folder: sent.then_some(SENT), ..ImportOptions::default() }
}

fn store_at(dir: &Path, name: &str) -> Store {
    Store::open_packed(dir.join(name).join("app"), dir.join(name).join("Files")).unwrap()
}

/// What an import left, in an order that doesn't depend on when its folders were listed.
fn kept(store: &Store) -> (Vec<Folder>, Vec<(String, FolderState)>, Vec<Vec<u8>>) {
    let (mut folders, mut lists, mut sources) = everything(store, "a1");
    folders.sort_by(|a, b| a.path.cmp(&b.path));
    lists.sort_by(|a, b| a.0.cmp(&b.0));
    sources.sort();
    (folders, lists, sources)
}

/// The folders an import says it finished, in order.
#[derive(Default)]
struct Said(Vec<String>);

impl ImportSink for Said {
    fn progress(&mut self, _: Progress) {}
    fn folder(&mut self, done: &Folder) {
        self.0.push(done.path.clone());
    }
}

/// With a Sent Items folder, the import is the one without: the same folders under the same
/// names (the one under it too), lists, numbers and sources, one lane or four; only its folder is
/// finished last.
#[test]
fn the_sent_folder_is_read_last_and_comes_out_the_same() {
    let (dir, pst) = data_file("sent-last", 60, false);
    let plain = store_at(&dir, "plain");
    let all = import_with(&pst, &plain, "a1", &opts(2, false, None), &mut |_| {}).unwrap();
    for lanes in [1, 4] {
        let store = store_at(&dir, &format!("sent{lanes}"));
        let mut said = Said::default();
        let done = import_into(&pst, &store, "a1", &opts(lanes, true, None), &mut said).unwrap();
        assert_eq!((done.messages, done.skipped), (180, 0));
        assert_eq!(said.0.last().map(String::as_str), Some(SENT), "{:?}", said.0);
        assert!(said.0.iter().any(|p| p == "Projects/2024"), "{:?}", said.0);
        assert_eq!(kept(&store), kept(&plain), "{lanes} lanes");
        let mut paths: Vec<&str> = done.folders.iter().map(|f| f.path.as_str()).collect();
        let mut before: Vec<&str> = all.folders.iter().map(|f| f.path.as_str()).collect();
        paths.sort_unstable();
        before.sort_unstable();
        assert_eq!(paths, before);
        let cp = match resume::load(&store, "a1") {
            Found::Some(cp) => cp,
            _ => panic!("no checkpoint"),
        };
        assert_eq!((cp.state, cp.deferred.is_none()), (CheckState::Done, true));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The reader failing on a folder after the Sent Items folder was put off: it's read all the
/// same, after the failure, and the import says it stopped early.
#[test]
fn a_walk_that_fails_after_sent_was_seen_still_imports_sent() {
    let (dir, pst) = data_file("sent-walk", 50, false);
    let store = Store::memory();
    // Inbox's 50, then 20 of the folder under the Sent Items one, then the walk fails.
    let done = import_with(&pst, &store, "a1", &opts(4, true, Some(Fault::PlannerAfter(70))), &mut |_| {}).unwrap();
    assert_eq!(done.messages, 120, "{done:?}");
    assert_eq!(done.skips.get(&SkipReason::StoppedEarly), Some(&1));
    assert_eq!(store.load_folder("a1", SENT).unwrap().messages.len(), 50, "the Sent Items folder whole");
    assert_eq!(store.load_folder("a1", "Projects/2024").unwrap().messages.len(), 20);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An import into `store`'s `a1` stopped as `how` once `at` items are read.
fn stopped_at(pst: &Path, store: &Store, at: usize, how: StopHow) -> Imported {
    let stop = Stop::default();
    let o = ImportOptions { stop: stop.clone(), ..opts(2, true, None) };
    import_with(pst, store, "a1", &o, &mut |p| {
        if p.read >= at {
            stop.raise(how);
        }
    })
    .unwrap()
}

fn go_on(pst: &Path, store: &Store) -> Imported {
    import_with(pst, store, "a1", &ImportOptions { resume: true, ..opts(2, true, None) }, &mut |_| {}).unwrap()
}

/// Paused or cancelled before the Sent Items folder is read (it's made, and noted in the
/// checkpoint as put off) or while it's read, and gone on with, the import is the one that never
/// stopped; so is one cut off as if the process ended, between checkpoints.
#[test]
fn stopped_around_the_sent_folder_and_gone_on_with_is_the_same_import() {
    let (dir, pst) = data_file("sent-resume", 300, false);
    let whole = store_at(&dir, "whole");
    import_with(&pst, &whole, "a1", &opts(2, true, None), &mut |_| {}).unwrap();
    let cases = [(400, StopHow::Pause, true), (400, StopHow::Cancel, true), (800, StopHow::Pause, false), (800, StopHow::Cancel, false)];
    for (n, (at, how, before)) in cases.into_iter().enumerate() {
        let store = store_at(&dir, &format!("part{n}"));
        let part = stopped_at(&pst, &store, at, how);
        assert_eq!(part.stopped, Some(how));
        let cp = match resume::load(&store, "a1") {
            Found::Some(cp) => cp,
            _ => panic!("no checkpoint"),
        };
        assert_eq!(cp.deferred.is_some(), before, "{at} {how:?}: put off");
        assert_eq!(cp.current.as_ref().map(|c| c.path == SENT), Some(!before), "{at} {how:?}: part-way");
        if how == StopHow::Pause {
            assert!(part.folders.iter().all(|f| f.path != SENT), "{at}: not listed until it's read");
        }
        let done = go_on(&pst, &store);
        assert_eq!((done.messages, done.skipped, done.runs), (900, 0, 2), "{at} {how:?}");
        assert_eq!(kept(&store), kept(&whole), "{at} {how:?}");
    }
    for (n, at) in [250u64, 350, 450, 650, 850].into_iter().enumerate() {
        let name = format!("cut{n}");
        {
            let store = store_at(&dir, &name);
            let o =
                ImportOptions { batch_bytes: 3000, checkpoint_every: Some(Duration::from_millis(15)), ..opts(2, true, Some(Fault::CutOffAfter(at))) };
            assert!(import_with(&pst, &store, "a1", &o, &mut |_| {}).is_err());
        }
        let store = store_at(&dir, &name);
        let done = go_on(&pst, &store);
        assert_eq!((done.messages, done.skipped), (900, 0), "cut off at {at}");
        assert_eq!(kept(&store), kept(&whole), "cut off at {at}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A conversation index: its header (GUID `g`) and `blocks` replies.
fn index(g: u8, blocks: &[u8]) -> Vec<u8> {
    let mut i = vec![1, 0, 0, 0, 0, 0];
    i.extend([g; 16]);
    for b in blocks {
        i.extend([*b; 5]);
    }
    i
}

/// The messages lanes make of `items` (read by node, from 1), each a job of the Sent Items folder
/// when `sent` is given; the replies they are and the ids given, as the writer gets them.
fn made(items: &[Item], sent: Option<&Arc<SentIds>>) -> Vec<(Vec<u8>, Option<(u64, String)>, Option<String>)> {
    let opts = ImportOptions::default();
    let ctx = Ctx::looking(Path::new("sent.pst"), &opts);
    let (jobs_tx, jobs_rx) = mpsc::sync_channel(items.len());
    let job = |n: usize| Job {
        seq: n as u64,
        node: n as u32 + 1,
        size: 1,
        charge: 1,
        folder: 0,
        pim: false,
        folder_node: 9,
        row: n as u64,
        sent: sent.cloned(),
    };
    for n in 1..items.len() {
        jobs_tx.send(job(n)).unwrap();
    }
    drop(jobs_tx);
    let (tx, rx) = mpsc::channel();
    let read = |node: u32| {
        let it = &items[node as usize - 1];
        Ok(Item {
            class: it.class.clone(),
            message_id: it.message_id.clone(),
            in_reply_to: it.in_reply_to.clone(),
            thread_index: it.thread_index.clone(),
            ..Item::default()
        })
    };
    work(&ctx, job(0), &Mutex::new(jobs_rx), &tx, &read, &|| {});
    drop(tx);
    let mut out: Vec<_> = rx.into_iter().collect();
    out.sort_by_key(|d| d.seq);
    out.into_iter()
        .map(|d| match d.step {
            Step::Item { outcome: Outcome::Built(b), .. } => (b.prepared.source().unwrap(), b.reply.clone(), b.given.as_ref().map(|g| g.id.clone())),
            _ => panic!("built"),
        })
        .collect()
}

/// Lanes note each reply's parent and the id it answers before the Sent Items folder; in it,
/// they give an item with no id the one its replies answer, in its source.
#[test]
fn lanes_note_replies_and_give_sent_items_their_ids() {
    let note = |id: &str, irt: &str, ix: Vec<u8>| Item {
        class: "IPM.Note".into(),
        message_id: id.into(),
        in_reply_to: irt.into(),
        thread_index: ix,
        ..Item::default()
    };
    let replies = [note("r1@ana.example", "a@studio.example", index(7, &[2])), note("r2@ana.example", "", index(8, &[2]))];
    let read = made(&replies, None);
    assert_eq!(read[0].1, Some((mailcraft_model::thread::index_keys(&index(7, &[])).own, "a@studio.example".into())));
    assert_eq!(read[1].1, None, "no In-Reply-To");
    let mut ids = SentIds::default();
    for (raw, reply, _) in read {
        ids.note(reply, &mailcraft_mime::parse(&raw).unwrap().message_id);
    }
    let frozen = Arc::new(ids.freeze());
    let sent = [note("", "", index(7, &[])), note("", "", index(8, &[])), note("own@studio.example", "", index(7, &[]))];
    let got = made(&sent, Some(&frozen));
    let id_of = |raw: &[u8]| mailcraft_mime::parse(raw).unwrap().message_id;
    assert_eq!((id_of(&got[0].0), got[0].2.as_deref()), ("a@studio.example".to_string(), Some("a@studio.example")));
    assert!(id_of(&got[1].0).ends_with("@mailcraft.invalid"), "nothing answers it");
    assert_eq!((id_of(&got[2].0), got[2].2.as_deref()), ("own@studio.example".to_string(), None), "an id of its own");
    assert!(got.iter().all(|g| g.1.is_none()), "replies in the Sent Items folder aren't noted");
}

/// Going on with a Sent Items folder put off after it was read in part (cut off while it was
/// read, gone on with, and cut off again before it was reached): its rows read are not read again.
#[test]
fn a_sent_folder_put_off_again_goes_on_from_its_row() {
    let store = Store::memory();
    let mut sink = |_| {};
    let mut out = Out::new(&store, "a1", &ImportOptions::default(), &mut sink);
    let sent = Current { node: 0x8042, path: "Sent Items".into(), next_row: 7, uid_next: 8, tail: None };
    let cp = Checkpoint { deferred: Some(sent), ..Checkpoint::default() };
    let skip = out.resume_from(cp);
    assert_eq!((skip.start(0x8042), skip.start(0x8062)), (7, 0));
}
