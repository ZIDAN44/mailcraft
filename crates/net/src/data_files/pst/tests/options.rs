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
    assert_eq!(done.messages, 200, "stopped at the next item");
    let inbox = done.folders.iter().find(|f| f.path == "INBOX").unwrap();
    let st = store.load_folder("s1", "INBOX").unwrap();
    assert_eq!((st.messages.len(), inbox.total), (200, 200), "what was read is listed and counted");
    assert!(st.messages.iter().all(|m| store.read_body(&m.key).is_some()), "and its sources written");

    let done = import_with(&pst, &store, "s2", &opts, &mut |_| {}).unwrap();
    assert_eq!(done.messages, 0);
    assert_eq!(done.folders.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["INBOX"]);

    // Not asked: the whole file, as `import` brings it in.
    let all = import(&pst, &store, "s3", &mut |_| {}).unwrap();
    assert_eq!(all.messages, 700);
    let _ = std::fs::remove_dir_all(&dir);
}
