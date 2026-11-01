//! How an import runs: asked to stop part-way, and what it measures of itself.

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
    // A store that doesn't compress keeps them as they are.
    let plain = Store::memory();
    let s = import(&pst, &plain, "t2", &mut |_| {}).unwrap().stats;
    assert_eq!(s.raw_bytes, s.kept_bytes);
    let _ = std::fs::remove_dir_all(&dir);
}
