//! How an import runs: asked to stop part-way, what it measures of itself, and how it holds the
//! data file.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_store::Store;
use pst_rw::{UnicodePstBatchMessage, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};

use super::super::{ImportOptions, import, import_with};

/// A data file of `n` messages in its Inbox, in a folder of its own.
fn data_file(name: &str, n: usize) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pst = dir.join("in.pst");
    let owned: Vec<(String, String)> = (0..n).map(|k| (format!("Report #{k}"), format!("<r{k}@run.example>"))).collect();
    let body = "Here is the week's plan, as agreed. ".repeat(40);
    let recipients = [UnicodePstRecipient { name: "Zoe", email: "zoe@studio.example", recipient_type: UnicodePstRecipientType::To }];
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
    let cancel = Arc::new(AtomicBool::new(false));
    let opts = ImportOptions { cancel: Some(cancel.clone()), ..ImportOptions::default() };
    // Asked at the first word of progress (200 messages in).
    let done = import_with(&pst, &store, "s1", &opts, &mut |_| cancel.store(true, Ordering::Relaxed)).unwrap();
    assert_eq!((done.messages, done.cancelled), (200, true), "stopped at the next item, and says so");
    let inbox = done.folders.iter().find(|f| f.path == "INBOX").unwrap();
    let st = store.load_folder("s1", "INBOX").unwrap();
    assert_eq!((st.messages.len(), inbox.total), (200, 200), "what was read is listed and counted");
    assert!(st.messages.iter().all(|m| store.read_body(&m.key).is_some()), "and its sources written");

    let done = import_with(&pst, &store, "s2", &opts, &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.cancelled), (0, true));
    assert_eq!(done.folders.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["INBOX"]);

    // Not asked: the whole file, as `import` brings it in.
    let all = import(&pst, &store, "s3", &mut |_| {}).unwrap();
    assert_eq!((all.messages, all.cancelled), (700, false));
    // Asked only once the last item is in: nothing was cut short.
    let late = Arc::new(AtomicBool::new(false));
    let opts = ImportOptions { cancel: Some(late.clone()), ..ImportOptions::default() };
    let small = data_file("stop-late", 3).1;
    let done = import_with(&small, &store, "s4", &opts, &mut |_| {}).unwrap();
    late.store(true, Ordering::Relaxed);
    assert_eq!((done.messages, done.cancelled), (3, false));
    let _ = std::fs::remove_dir_all(small.parent().unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

/// An import says where its time went, how much it held and how much it wrote: its stages fit
/// in its whole (one lane, one thread), its sources shrank as the data file keeps them, and what
/// it held at once is counted as rebuilt (here one batch held them all, a megabyte compressed to
/// far less).
#[test]
fn an_import_counts_where_its_time_went() {
    let (dir, pst) = data_file("stats", 600);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let done = import(&pst, &store, "t1", &mut |_| {}).unwrap();
    let s = done.stats;
    assert_eq!((done.messages, s.lanes), (600, 1));
    let stages = [s.plan_ms, s.read_ms, s.build_ms, s.pack_ms, s.write_ms, s.commit_ms];
    assert!(stages.iter().sum::<u64>() <= s.total_ms, "{s:?}");
    assert!(s.total_ms > 0, "{s:?}");
    assert!(s.raw_bytes > 600 * 1400, "every source counted: {s:?}");
    assert!(s.kept_bytes > 0 && s.kept_bytes < s.raw_bytes / 2, "compressed: {s:?}");
    assert_eq!(s.peak_budget_bytes, s.raw_bytes, "{s:?}");
    // The file read through the cache: every byte read at most once, far fewer reads than pages.
    let len = std::fs::metadata(&pst).unwrap().len();
    assert!(s.read_bytes > 0 && s.read_bytes <= len, "{s:?}");
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
    let plain = import(&pst, &store, "r0", &mut |_| {}).unwrap();
    assert_eq!((plain.messages, plain.stats.reopens), (250, 0), "a small file needs no reopening");
    let len = std::fs::metadata(&pst).unwrap().len();
    assert!(plain.stats.read_bytes > 0 && plain.stats.read_bytes <= len, "{:?}", plain.stats);
    for (account, every) in [("r3", 3), ("r1", 1)] {
        let opts = ImportOptions { recycle: every, ..ImportOptions::default() };
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
    let plain = import(&pst, &store, "p0", &mut |_| {}).unwrap();
    assert_eq!(plain.stats.reopens, 0);
    let len = std::fs::metadata(&pst).unwrap().len();
    let opts = ImportOptions { handle_cache_bytes: 8 * 1024, ..ImportOptions::default() };
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
