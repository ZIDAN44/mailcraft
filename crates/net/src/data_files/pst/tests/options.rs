//! How an import runs: asked to stop part-way, what it measures of itself, and how it holds the
//! data file.

use mailcraft_model::job::StopHow;
use mailcraft_store::Store;
use outlook_pst::{UnicodePstBatchMessage, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};
use std::path::PathBuf;

use super::super::{ImportOptions, Stop, import, import_with};

/// One lane reads the items: what these tests count is one reader's.
fn one_lane() -> ImportOptions {
    ImportOptions { lanes: 1, ..ImportOptions::default() }
}

/// A data file of `n` messages in its Inbox, in a folder of its own.
fn data_file(name: &str, n: usize) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pst = dir.join("in.pst");
    let owned: Vec<(String, String)> = (0..n).map(|k| (format!("Report #{k}"), format!("<r{k}@run.example>"))).collect();
    let body = "Here is the week's plan, as agreed. ".repeat(40);
    let recipients =
        [UnicodePstRecipient { name: "Zoe", email: "zoe@studio.example", recipient_type: UnicodePstRecipientType::To, ..Default::default() }];
    let batch: Vec<UnicodePstBatchMessage<'_>> = owned
        .iter()
        .enumerate()
        .map(|(k, o)| UnicodePstBatchMessage {
            message: UnicodePstMessage {
                subject: &o.0,
                sender_name: "Ana",
                sender_email: "ana@soundlab.example",
                recipients: &recipients,
                body: &body,
                html_body: None,
                message_id: &o.1,
                delivery_time: (1_700_000_000 + k as i64 * 60 + 11_644_473_600) * 10_000_000,
                message_class: "IPM.Note",
                properties: &[],
            },
            attachments: &[],
        })
        .collect();
    Writer::create_in_folder_with_attachments(&pst, &["Inbox"], &batch[0].message, batch[0].attachments).unwrap();
    Writer::append_many_in_folder_with_attachments(&pst, &["Inbox"], &batch[1..]).unwrap();
    (dir, pst)
}

/// An import asked to stop reads no further, and keeps what it read: written, listed and counted,
/// as when a file ends early. Asked before it starts, it brings in nothing but an Inbox to open.
#[test]
fn an_import_asked_to_stop_keeps_what_it_read() {
    let (dir, pst) = data_file("stop", 700);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let stop = Stop::default();
    let opts = ImportOptions { stop: stop.clone(), ..ImportOptions::default() };
    // Asked at the first word of progress (200 messages in).
    let done = import_with(&pst, &store, "s1", &opts, &mut |_| stop.raise(StopHow::Cancel)).unwrap();
    assert_eq!((done.messages, done.stopped), (200, Some(StopHow::Cancel)), "stopped at the next item, and says so");
    assert_eq!(done.stopped_in.as_deref(), Some("INBOX"), "part-way through the Inbox");
    let inbox = done.folders.iter().find(|f| f.path == "INBOX").unwrap();
    let st = store.load_folder("s1", "INBOX").unwrap();
    assert_eq!((st.messages.len(), inbox.total), (200, 200), "what was read is listed and counted");
    assert!(st.messages.iter().all(|m| store.read_body(&m.key).is_some()), "and its sources written");

    let done = import_with(&pst, &store, "s2", &opts, &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.stopped), (0, Some(StopHow::Cancel)));
    assert_eq!(done.folders.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["INBOX"]);

    // Not asked: the whole file, as `import` brings it in.
    let all = import(&pst, &store, "s3", &mut |_| {}).unwrap();
    assert_eq!((all.messages, all.stopped), (700, None));
    // Asked only once the last item is in: nothing was cut short.
    let late = Stop::default();
    let opts = ImportOptions { stop: late.clone(), ..ImportOptions::default() };
    let small = data_file("stop-late", 3).1;
    let done = import_with(&small, &store, "s4", &opts, &mut |_| {}).unwrap();
    late.raise(StopHow::Cancel);
    assert_eq!((done.messages, done.stopped), (3, None));
    let _ = std::fs::remove_dir_all(small.parent().unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Paused, an import writes what it read but leaves the folder it was part-way through out of
/// the list (it goes on with it later, and nobody can change it meanwhile); thrown away, it
/// writes nothing more at all, not even a folder.
#[test]
fn a_pause_hides_the_folder_part_way_and_a_discard_writes_nothing_more() {
    let (dir, pst) = data_file("pause", 700);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let stop = Stop::default();
    let opts = ImportOptions { stop: stop.clone(), lanes: 1, ..ImportOptions::default() };
    let done = import_with(&pst, &store, "p1", &opts, &mut |_| stop.raise(StopHow::Pause)).unwrap();
    assert_eq!((done.stopped, done.stopped_in.as_deref()), (Some(StopHow::Pause), Some("INBOX")));
    let listed: Vec<String> = store.load_folders("p1").unwrap().into_iter().map(|f| f.path).collect();
    assert_eq!(listed, ["Deleted Items"], "the folder finished is, the Inbox part-way isn't, nor one made up");
    assert_eq!(done.folders.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["Deleted Items"]);
    assert_eq!(done.messages, 200, "what was read is written all the same");

    let stop = Stop::default();
    let opts = ImportOptions { stop: stop.clone(), lanes: 1, ..ImportOptions::default() };
    let done = import_with(&pst, &store, "p2", &opts, &mut |_| stop.raise(StopHow::Discard)).unwrap();
    assert_eq!(done.stopped, Some(StopHow::Discard));
    assert!(done.folders.is_empty());
    let listed: Vec<String> = store.load_folders("p2").unwrap().into_iter().map(|f| f.path).collect();
    assert_eq!(listed, ["Deleted Items"], "only what was in before it was thrown away (the data file goes with it)");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file looked at before it's imported is planned then: what it holds is told, and its import,
/// handed that plan, doesn't walk the folders again (a fault in planning its Inbox costs it
/// nothing), and brings in what an import planning for itself does.
#[test]
fn a_file_planned_when_looked_at_is_imported_without_planning_again() {
    let (dir, pst) = data_file("planned", 30);
    let planned = super::super::inspect(&pst).unwrap();
    assert_eq!((planned.messages, planned.folders, planned.pim_items), (30, 2, 0), "{planned:?}");
    assert!(planned.bytes > 0);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let fault = Some(super::super::Fault::PlanAt(1));
    let opts = ImportOptions { lanes: 1, fault, planned: Some(std::sync::Arc::new(planned)), ..ImportOptions::default() };
    let done = import_with(&pst, &store, "w1", &opts, &mut |_| {}).unwrap();
    assert_eq!(done.messages, 30);
    assert_eq!(store.load_folder("w1", "INBOX").unwrap().messages.len(), 30);
    // Planning for itself, the same fault loses the Inbox: the plan above was what spared it.
    let opts = ImportOptions { lanes: 1, fault, ..ImportOptions::default() };
    let replanned = import_with(&pst, &store, "w2", &opts, &mut |_| {}).unwrap();
    assert_eq!(replanned.messages, 0);

    let junk = dir.join("junk.pst");
    std::fs::write(&junk, b"not a data file at all").unwrap();
    assert!(super::super::inspect(&junk).is_err());
    assert!(super::super::inspect(&dir.join("gone.pst")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stop only ever goes further:MailCraft closing (a pause) while an account is removed (a
/// discard) leaves the import thrown away.
#[test]
fn a_stop_is_never_lowered() {
    let stop = Stop::default();
    assert_eq!(stop.how(), None);
    stop.raise(StopHow::Pause);
    assert_eq!(stop.how(), Some(StopHow::Pause));
    stop.raise(StopHow::Discard);
    stop.raise(StopHow::Pause);
    stop.raise(StopHow::Cancel);
    assert_eq!(stop.how(), Some(StopHow::Discard));
    let shared = stop.clone();
    assert!(shared.raised());
}

/// An import says where its time went, how much it held and how much it wrote: the writer's
/// stages and its waiting fit in its whole, as each of the other threads' stages does, its
/// sources shrank as the data file keeps them, and what it held at once is counted as rebuilt
/// (here one batch held them all, a megabyte compressed to far less).
#[test]
fn an_import_counts_where_its_time_went() {
    let (dir, pst) = data_file("stats", 600);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let done = import_with(&pst, &store, "t1", &one_lane(), &mut |_| {}).unwrap();
    let s = done.stats;
    assert_eq!((done.messages, s.lanes), (600, 1));
    assert!(s.write_ms + s.commit_ms + s.writer_idle_ms <= s.total_ms, "{s:?}");
    assert!([s.plan_ms, s.read_ms, s.build_ms, s.pack_ms].iter().all(|t| *t <= s.total_ms), "{s:?}");
    assert!(s.total_ms > 0, "{s:?}");
    assert!(s.raw_bytes > 600 * 1400, "every source counted: {s:?}");
    assert!(s.kept_bytes > 0 && s.kept_bytes < s.raw_bytes / 2, "compressed: {s:?}");
    assert_eq!(s.peak_budget_bytes, s.raw_bytes, "{s:?}");
    // The file read through the cache: every byte read at most once by each of the two handles
    // (the planner's and the lane's), far fewer reads than pages.
    let len = std::fs::metadata(&pst).unwrap().len();
    assert!(s.read_bytes > 0 && s.read_bytes <= 2 * len, "{s:?}");
    assert!(s.reads > 0 && s.reads < len / 512, "{s:?}");
    // The folder handle's pages are counted too, and are a few: it reads no items.
    assert!(s.walk_cache_bytes > 0 && s.walk_cache_bytes < 64 * 1024, "{s:?}");
    // A store that doesn't compress keeps them as they are.
    let plain = Store::memory();
    let s = import(&pst, &plain, "t2", &mut |_| {}).unwrap().stats;
    assert_eq!(s.raw_bytes, s.kept_bytes);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What every folder holds, by subject and date: the same whichever way the file was read.
fn contents(store: &Store, account: &str) -> Vec<(String, Vec<(String, i64)>)> {
    store
        .load_folders(account)
        .unwrap()
        .iter()
        .map(|f| {
            let mut list: Vec<(String, i64)> =
                store.load_folder(account, &f.path).unwrap().messages.iter().map(|m| (m.subject.clone(), m.date)).collect();
            list.sort();
            (f.path.clone(), list)
        })
        .collect()
}

/// The handle that reads items opened again every 3 items (and every one) loses nothing and
/// brings nothing in twice; and what was read of the file stays read, so a new handle doesn't
/// read it again: every byte of this small file is read once, however often it's reopened.
#[test]
fn reopening_the_reader_loses_nothing_and_rereads_nothing() {
    let (dir, pst) = data_file("recycle", 250);
    let store = Store::memory();
    let plain = import_with(&pst, &store, "r0", &one_lane(), &mut |_| {}).unwrap();
    assert_eq!((plain.messages, plain.stats.reopens), (250, 0), "a small file needs no reopening");
    // Each of the two handles (the planner's and the lane's) reads it at most once.
    let len = 2 * std::fs::metadata(&pst).unwrap().len();
    assert!(plain.stats.read_bytes > 0 && plain.stats.read_bytes <= len, "{:?}", plain.stats);
    for (account, every) in [("r3", 3), ("r1", 1)] {
        let opts = ImportOptions { recycle: every, ..one_lane() };
        let done = import_with(&pst, &store, account, &opts, &mut |_| {}).unwrap();
        assert_eq!((done.messages, done.skipped), (250, 0));
        assert_eq!(done.stats.reopens as usize, 250usize.div_ceil(every) - 1, "{:?}", done.stats);
        assert_eq!(contents(&store, account), contents(&store, "r0"));
        assert!(done.stats.read_bytes <= len, "read again after reopening: {:?}", done.stats);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// What sets the reopening off in the app is the pages a handle has read, not a count of items:
/// let hold only a few KiB, the handle that reads items is opened again as it reads, and the
/// import is the same as with no reopening, every byte of the file still read at most once.
#[test]
fn the_reader_is_reopened_once_its_pages_hold_too_much() {
    let (dir, pst) = data_file("page-limit", 250);
    let store = Store::memory();
    let plain = import_with(&pst, &store, "p0", &one_lane(), &mut |_| {}).unwrap();
    assert_eq!(plain.stats.reopens, 0);
    let len = 2 * std::fs::metadata(&pst).unwrap().len();
    let opts = ImportOptions { handle_cache_bytes: 8 * 1024, ..one_lane() };
    let done = import_with(&pst, &store, "p1", &opts, &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.skipped), (250, 0));
    assert!(done.stats.reopens > 0, "the pages read were counted: {:?}", done.stats);
    assert!(done.stats.reopens < 250, "and not every item set it off: {:?}", done.stats);
    assert_eq!(contents(&store, "p1"), contents(&store, "p0"));
    assert!(done.stats.read_bytes <= len, "{:?}", done.stats);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A data file that can't be written to imports: nothing asks to write it. (outlook-pst's own
/// `open` already went on without write access; this keeps the read-only opening honest.)
#[test]
fn a_read_only_data_file_imports() {
    let (dir, pst) = data_file("read-only", 5);
    let mut perms = std::fs::metadata(&pst).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&pst, perms.clone()).unwrap();
    let done = import(&pst, &Store::memory(), "o1", &mut |_| {});
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&pst, perms).unwrap();
    assert_eq!(done.map(|d| d.messages), Ok(5));
    let _ = std::fs::remove_dir_all(&dir);
}

/// While it's read, no other program may open the data file to write (Windows' sharing
/// violation, error 32), and the import leaves it as it was.
#[cfg(windows)]
#[test]
fn nothing_writes_the_data_file_while_it_is_read() {
    let (dir, pst) = data_file("shared", 450);
    let before = std::fs::metadata(&pst).unwrap().modified().unwrap();
    let mut tried = Vec::new();
    let done = import(&pst, &Store::memory(), "w1", &mut |_| {
        tried.push(std::fs::OpenOptions::new().write(true).open(&pst).map_err(|e| e.raw_os_error()).err());
        // Reading it is fine.
        assert!(std::fs::File::open(&pst).is_ok());
    })
    .unwrap();
    assert_eq!(done.messages, 450);
    assert!(!tried.is_empty() && tried.iter().all(|e| *e == Some(Some(32))), "{tried:?}");
    assert_eq!(std::fs::metadata(&pst).unwrap().modified().unwrap(), before);
    // Once it's done, the file is let go.
    assert!(std::fs::OpenOptions::new().write(true).open(&pst).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The import's data file has the bulk cache while it writes, and the usual one after, however
/// the import ends: finished, or failed part-way (a file that isn't a data file at all).
#[test]
fn an_import_has_the_bulk_cache_only_while_it_writes() {
    let (dir, pst) = data_file("bulk-cache", 300);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let mut during = Vec::new();
    let done = import_with(&pst, &store, "b1", &one_lane(), &mut |_| during.push(store.data_file_cache("b1"))).unwrap();
    assert_eq!(done.messages, 300);
    assert!(during.contains(&Some(mailcraft_store::BULK_CACHE)), "written with the bulk cache: {during:?}");
    assert_eq!(store.data_file_cache("b1"), None, "let go of as the import ended");
    assert_eq!(store.load_folder("b1", "INBOX").unwrap().messages.len(), 300);
    assert_eq!(store.data_file_cache("b1"), Some(mailcraft_store::CACHE));

    let junk = dir.join("junk.pst");
    std::fs::write(&junk, b"!BDN not really a data file at all, just some bytes".repeat(20)).unwrap();
    store
        .write_body(&mailcraft_model::MessageKey { account: "b2".into(), folder: "INBOX".into(), validity: 1, uid: 1 }, b"Subject: x\r\n\r\ny")
        .unwrap();
    assert!(import_with(&junk, &store, "b2", &one_lane(), &mut |_| {}).is_err());
    assert_ne!(store.data_file_cache("b2"), Some(mailcraft_store::BULK_CACHE), "a failed import lets go of it too");
    let _ = std::fs::remove_dir_all(&dir);
}
