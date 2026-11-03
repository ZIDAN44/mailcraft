//! The import's threads: a planner, lanes that read and make items, and the writer that keeps
//! them in order. However many lanes read, the import is the same; a fault in any thread costs
//! what it was holding and no more, and never leaves the import waiting.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use mailcraft_model::{Folder, FolderRole};
use mailcraft_store::Store;
use outlook_pst::{
    UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType,
};

use super::super::lanes::MAX_PANICS;
use super::super::lanes::tests::Fault;
use super::super::out::Out;
use super::super::{ImportOptions, ImportSink, Imported, PlannedFolder, Progress, SkipReason, import_into, import_with};

/// A data file with mail in three folders, two nested: plain and HTML messages, some with a file
/// of 10-300 KiB or a picture the HTML shows, some with no message id. `big` gives every third
/// message a file of about a megabyte instead.
pub(super) fn data_file(name: &str, per_folder: usize, big: bool) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-lanes-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pst = dir.join("in.pst");
    let recipients = [UnicodePstRecipient { name: "Zoe", email: "zoe@studio.example", recipient_type: UnicodePstRecipientType::To }];
    let body = "Here is the week's plan, as agreed. ".repeat(20);
    let blobs: Vec<Vec<u8>> = (0..per_folder).map(|k| file_bytes(k, big)).collect();
    for (f, folder) in [&["Inbox"][..], &["Projects"], &["Projects", "2024"]].into_iter().enumerate() {
        let owned: Vec<(String, String)> = (0..per_folder)
            .map(|k| (format!("Report {f}-{k}"), if k % 7 == 3 { String::new() } else { format!("<r{f}-{k}@run.example>") }))
            .collect();
        let files: Vec<Vec<UnicodePstAttachment<'_>>> = (0..per_folder)
            .map(|k| match k % 3 {
                0 => vec![UnicodePstAttachment {
                    filename: "plan.bin",
                    mime_type: "application/octet-stream",
                    content_id: None,
                    data: &blobs[k],
                    ..Default::default()
                }],
                1 if k % 2 == 1 => {
                    vec![UnicodePstAttachment {
                        filename: "chart.png",
                        mime_type: "image/png",
                        content_id: Some("<c@run.example>"),
                        data: b"\x89PNG\r\n\x1a\nx",
                        ..Default::default()
                    }]
                }
                _ => Vec::new(),
            })
            .collect();
        let batch: Vec<UnicodePstBatchMessage<'_>> = owned
            .iter()
            .enumerate()
            .map(|(k, o)| UnicodePstBatchMessage {
                message: UnicodePstMessage {
                    subject: &o.0,
                    sender_name: "Ana",
                    sender_email: "ana@soundlab.example",
                    recipients: &recipients,
                    body: if k % 4 == 2 { "" } else { &body },
                    html_body: (k % 2 == 1).then_some("<p>The plan</p><img src=\"cid:c@run.example\">"),
                    message_id: &o.1,
                    delivery_time: (1_700_000_000 + (f * per_folder + k) as i64 * 60 + 11_644_473_600) * 10_000_000,
                    message_class: "IPM.Note",
                    properties: &[],
                },
                attachments: &files[k],
            })
            .collect();
        if f == 0 {
            Writer::create_in_folder_with_attachments(&pst, folder, &batch[0].message, batch[0].attachments).unwrap();
            Writer::append_many_in_folder_with_attachments(&pst, folder, &batch[1..]).unwrap();
        } else {
            Writer::append_many_in_folder_with_attachments(&pst, folder, &batch).unwrap();
        }
    }
    (dir, pst)
}

/// A file's bytes, of a size that differs from one message to the next, not all alike.
fn file_bytes(k: usize, big: bool) -> Vec<u8> {
    let len = if big { 900 * 1024 + k * 1024 } else { 10 * 1024 + (k * 7919) % (290 * 1024) };
    (0..len).map(|i| (i.wrapping_mul(31).wrapping_add(k) % 251) as u8).collect()
}

fn lanes(n: usize) -> ImportOptions {
    ImportOptions { lanes: n, ..ImportOptions::default() }
}

/// Everything an import left in the store: the folder list, every folder's list, every source.
pub(super) fn everything(store: &Store, account: &str) -> (Vec<Folder>, Vec<(String, mailcraft_store::FolderState)>, Vec<Vec<u8>>) {
    let folders = store.load_folders(account).unwrap();
    let mut lists = Vec::new();
    let mut sources = Vec::new();
    for f in &folders {
        let st = store.load_folder(account, &f.path).unwrap();
        for m in &st.messages {
            sources.push(store.read_body(&m.key).unwrap());
        }
        lists.push((f.path.clone(), st));
    }
    (folders, lists, sources)
}

/// `f` on a thread of its own, which must be done within a minute.
fn within_a_minute<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(60)).expect("the import came back")
}

pub(super) fn run(pst: &Path, store: &Store, account: &str, opts: &ImportOptions) -> Imported {
    import_with(pst, store, account, opts, &mut |_| {}).unwrap()
}

/// One lane or four, the import is byte for byte the same: the same folders, lists and sources.
/// Messages are numbered by the writer in the file's order, and an item with no message id gets
/// one from the file and its place in it, not from the order the lanes finished in.
#[test]
fn parallel_import_is_identical_to_sequential() {
    let (dir, pst) = data_file("same", 120, false);
    let (one, four) = (Store::memory(), Store::memory());
    let a = run(&pst, &one, "p1", &lanes(1));
    let b = run(&pst, &four, "p1", &lanes(4));
    assert_eq!((a.messages, a.skipped, b.messages, b.skipped), (360, 0, 360, 0));
    assert_eq!((a.stats.lanes, b.stats.lanes), (1, 4));
    assert_eq!(a.folders, b.folders);
    assert_eq!(everything(&one, "p1"), everything(&four, "p1"));
    // The same file imported again gives its id-less messages the same ids.
    let ids = |store: &Store| -> Vec<String> {
        everything(store, "p1")
            .2
            .iter()
            .filter_map(|raw| mailcraft_mime::parse(raw))
            .map(|p| p.message_id)
            .filter(|i| i.ends_with(".invalid"))
            .collect()
    };
    assert!(!ids(&one).is_empty());
    assert!(ids(&one).iter().all(|i| i.starts_with("pst.p1.")), "{:?}", ids(&one));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A budget of one byte: every item waits until it's the one the writer needs next, and the
/// import still finishes, the same as with room to spare.
#[test]
fn a_one_byte_budget_still_finishes() {
    let (dir, pst) = data_file("tiny", 40, false);
    let roomy = Store::memory();
    run(&pst, &roomy, "t1", &lanes(1));
    let store = Store::memory();
    let path = pst.clone();
    let tight = store.clone();
    let done = within_a_minute(move || run(&path, &tight, "t1", &ImportOptions { lanes: 4, budget_bytes: 1, ..ImportOptions::default() }));
    assert_eq!((done.messages, done.skipped), (120, 0));
    assert_eq!(everything(&store, "t1"), everything(&roomy, "t1"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Items of about a megabyte the file says are tiny: each is charged for what it is once read,
/// waiting for room as any other, and the import is the same.
#[test]
fn an_item_bigger_than_the_file_says_is_charged_for_what_it_is() {
    let (dir, pst) = data_file("understated", 12, true);
    let plain = Store::memory();
    run(&pst, &plain, "u1", &lanes(1));
    let store = Store::memory();
    let (path, s) = (pst.clone(), store.clone());
    let opts = ImportOptions { lanes: 4, budget_bytes: 4 << 20, fault: Some(Fault::Understate), ..ImportOptions::default() };
    let done = within_a_minute(move || run(&path, &s, "u1", &opts));
    assert_eq!((done.messages, done.skipped), (36, 0));
    assert!(done.stats.budget_wait_ms > 0 || done.stats.peak_budget_bytes > 0, "{:?}", done.stats);
    assert_eq!(everything(&store, "u1"), everything(&plain, "u1"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The reader panicking on one item costs that item: it's counted as unreadable, the lane reads
/// on with its handle opened again, and every other message comes in.
#[test]
fn a_lane_panic_skips_one_item() {
    let (dir, pst) = data_file("panic-one", 30, false);
    let store = Store::memory();
    let (path, s) = (pst.clone(), store.clone());
    let opts = ImportOptions { lanes: 4, fault: Some(Fault::PanicAt(17)), ..ImportOptions::default() };
    let done = within_a_minute(move || run(&path, &s, "x1", &opts));
    assert_eq!((done.messages, done.skipped), (89, 1));
    assert_eq!(done.skips.get(&SkipReason::MessageUnreadable), Some(&1));
    let listed: usize = done.folders.iter().map(|f| store.load_folder("x1", &f.path).unwrap().messages.len()).sum();
    assert_eq!(listed, 89);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A reader that panics on every item: after a run of them the file is taken as damaged and
/// reading stops, keeping what there is (the folders), and saying it stopped early.
#[test]
fn a_run_of_panics_stops_the_import() {
    let (dir, pst) = data_file("panic-all", 60, false);
    let store = Store::memory();
    let (path, s) = (pst.clone(), store.clone());
    let opts = ImportOptions { lanes: 4, fault: Some(Fault::PanicAlways), ..ImportOptions::default() };
    let done = within_a_minute(move || run(&path, &s, "x2", &opts));
    assert_eq!(done.messages, 0);
    assert!(done.skipped >= MAX_PANICS && done.skipped < 180, "stopped reading: {done:?}");
    assert_eq!(done.skips.get(&SkipReason::StoppedEarly), Some(&1));
    assert!(done.folders.iter().any(|f| f.role == FolderRole::Inbox));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The planner panicking part-way: the folders it handed out are kept, with the messages read
/// from them, listed and counted; the rest of the file isn't read.
#[test]
fn a_planner_panic_keeps_the_folders_read() {
    let (dir, pst) = data_file("planner", 50, false);
    let store = Store::memory();
    let (path, s) = (pst.clone(), store.clone());
    let opts = ImportOptions { lanes: 4, fault: Some(Fault::PlannerAfter(70)), ..ImportOptions::default() };
    let done = within_a_minute(move || run(&path, &s, "x3", &opts));
    assert_eq!(done.messages, 70, "{done:?}");
    let inbox = store.load_folder("x3", "INBOX").unwrap();
    assert_eq!(inbox.messages.len(), 50);
    let projects = done.folders.iter().find(|f| f.path == "Projects").unwrap();
    assert_eq!(projects.total, 20, "a folder left part-way still gets its list");
    assert_eq!(store.load_folders("x3").unwrap(), done.folders);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The reader failing on a folder as the file is planned, before any item is read: that folder
/// (and those under it) are left out and counted, and the ones planned before and after it come
/// in, rather than the import failing as a damaged file.
#[test]
fn a_panic_while_planning_keeps_the_other_folders() {
    let (dir, pst) = data_file("plan", 30, false);
    let store = Store::memory();
    let (path, s) = (pst.clone(), store.clone());
    let opts = ImportOptions { lanes: 4, fault: Some(Fault::PlanAt(2)), ..ImportOptions::default() };
    let done = within_a_minute(move || run(&path, &s, "x5", &opts));
    let paths: Vec<&str> = done.folders.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["Deleted Items", "INBOX"], "{done:?}");
    assert_eq!(done.messages, 30);
    assert_eq!(store.load_folder("x5", "INBOX").unwrap().messages.len(), 30);
    assert_eq!(done.skips.get(&SkipReason::FolderUnreadable), Some(&1), "{done:?}");
    assert_eq!(store.load_folders("x5").unwrap(), done.folders);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The writer panicking part-way: the import comes back (the lanes and the planner are told to
/// stop, and nothing waits on the writer), with what was written kept.
#[test]
fn a_writer_panic_comes_back() {
    let (dir, pst) = data_file("writer", 60, false);
    let store = Store::memory();
    let (path, s) = (pst.clone(), store.clone());
    let opts = ImportOptions { lanes: 4, budget_bytes: 64 * 1024, fault: Some(Fault::WriterAfter(30)), ..ImportOptions::default() };
    let done = within_a_minute(move || run(&path, &s, "x4", &opts));
    assert!(done.messages >= 30 && done.messages < 180, "{done:?}");
    let listed: usize = done.folders.iter().map(|f| store.load_folder("x4", &f.path).unwrap().messages.len()).sum();
    assert_eq!(listed, done.messages, "what was written is listed");
    let _ = std::fs::remove_dir_all(&dir);
}

/// What a sink is told.
#[derive(Default)]
struct Told {
    folders: Vec<Folder>,
    progress: Vec<Progress>,
    /// The folders planned, and how many folders were said done before them.
    planned: Vec<PlannedFolder>,
    planned_after: Option<usize>,
}

impl ImportSink for Told {
    fn planned(&mut self, folders: &[PlannedFolder]) {
        self.planned = folders.to_vec();
        self.planned_after = Some(self.folders.len());
    }

    fn progress(&mut self, p: Progress) {
        self.progress.push(p);
    }

    fn folder(&mut self, done: &Folder) {
        self.folders.push(done.clone());
    }
}

/// Each folder is said once, as it finishes, written and counted, a folder before the ones under
/// it; it's in the list on disk by then. The progress has the file's totals from the start.
#[test]
fn each_folder_is_said_once_as_it_finishes() {
    let (dir, pst) = data_file("deltas", 210, false);
    let store = Store::memory();
    let mut told = Told::default();
    let done = import_into(&pst, &store, "d1", &lanes(4), &mut told).unwrap();
    let paths: Vec<&str> = told.folders.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["Deleted Items", "INBOX", "Projects", "Projects/2024"]);
    assert!(told.folders.iter().skip(1).all(|f| f.total == 210), "{:?}", told.folders);
    assert_eq!(told.folders, done.folders);
    assert_eq!(store.load_folders("d1").unwrap(), done.folders);
    let first = told.progress.first().unwrap();
    assert_eq!(first.total, 630);
    assert!(first.bytes_total > 630 * 1000, "{first:?}");
    let last = told.progress.last().unwrap();
    assert_eq!((last.read, last.total), (600, 630));
    assert!(last.bytes_done <= last.bytes_total);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The folders a file holds are said before any comes in, each with the path it comes in under
/// and its items; going on after a pause, again, with the paths the folders done went to.
#[test]
fn the_folders_planned_are_said_first_with_their_paths() {
    let (dir, pst) = data_file("outline", 210, false);
    let store = Store::memory();
    let mut told = Told::default();
    let done = import_into(&pst, &store, "o1", &lanes(4), &mut told).unwrap();
    assert_eq!(told.planned_after, Some(0), "said before any folder");
    let mut planned: Vec<(&str, u64)> = told.planned.iter().map(|f| (f.path.as_str(), f.items)).collect();
    planned.sort_unstable();
    assert_eq!(planned, [("Deleted Items", 0), ("INBOX", 210), ("Projects", 210), ("Projects/2024", 210)]);
    assert_eq!(told.planned.iter().find(|f| f.path == "Projects/2024").map(|f| f.name.as_str()), Some("2024"));
    let mut made: Vec<&str> = done.folders.iter().map(|f| f.path.as_str()).collect();
    made.sort_unstable();
    assert_eq!(made, planned.iter().map(|p| p.0).collect::<Vec<_>>(), "as they came in");

    let store = Store::memory();
    let stop = super::super::Stop::default();
    let o = ImportOptions { stop: stop.clone(), ..lanes(2) };
    import_with(&pst, &store, "o1", &o, &mut |p| {
        if p.read >= 300 {
            stop.raise(mailcraft_model::job::StopHow::Pause);
        }
    })
    .unwrap();
    let mut again = Told::default();
    import_into(&pst, &store, "o1", &ImportOptions { resume: true, ..lanes(2) }, &mut again).unwrap();
    assert_eq!(again.planned, told.planned, "going on");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Folders finished close together go into the account's folder list in one write, not one
/// each: a file of thousands of folders doesn't cost a time that grows with their square.
#[test]
fn folders_finished_together_are_listed_in_one_write() {
    /// The length of the folder list on disk each time a folder is said.
    struct Lengths<'s>(&'s Store, Vec<usize>);
    impl ImportSink for Lengths<'_> {
        fn progress(&mut self, _: Progress) {}
        fn folder(&mut self, _: &Folder) {
            self.1.push(self.0.load_folders("f1").unwrap().len());
        }
    }
    let store = Store::memory();
    let mut sink = Lengths(&store, Vec::new());
    let began = std::time::Instant::now();
    let mut out = Out::new(&store, "f1", &ImportOptions::default(), &mut sink);
    for n in 0..500 {
        let path = out.add_folder(None, &format!("Folder {n}"));
        out.save_folder(&path);
    }
    let done = out.finish();
    let secs = usize::try_from(began.elapsed().as_secs()).unwrap();
    assert_eq!(done.last_save, None);
    assert_eq!(store.load_folders("f1").unwrap().len(), 501, "with an Inbox");
    let mut writes = sink.1.clone();
    assert_eq!(writes.len(), 501, "each said once");
    writes.dedup();
    assert!(writes.len() <= secs + 2, "{} writes in {secs} s", writes.len());
}
