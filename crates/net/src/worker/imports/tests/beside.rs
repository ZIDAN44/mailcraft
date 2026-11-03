//! An Outlook data file opened as a mailbox: its copy is made beside it, MailCraft's changes go
//! there, and the file itself is never written, opened, changed in MailCraft and closed, or read
//! again once it has changed.

use mailcraft_model::{Flag, Op};

use super::*;

/// The file's bytes and when it last changed.
fn as_it_is(pst: &str) -> (Vec<u8>, std::time::SystemTime) {
    (std::fs::read(pst).unwrap(), std::fs::metadata(pst).unwrap().modified().unwrap())
}

/// A message added to the file's Inbox, as Outlook would add one.
fn add_to_file(pst: &str, subject: &str, id: &str) {
    let recipients = [UnicodePstRecipient { name: "Zoe", email: "zoe@studio.example", recipient_type: UnicodePstRecipientType::To }];
    let message = UnicodePstMessage {
        subject,
        sender_name: "Ana",
        sender_email: "ana@soundlab.example",
        recipients: &recipients,
        body: "Later.",
        html_body: None,
        message_id: id,
        delivery_time: (1_800_000_000 + 11_644_473_600) * 10_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    Writer::append_many_in_folder_with_attachments(Path::new(pst), &["Inbox"], &[UnicodePstBatchMessage { message, attachments: &[] }]).unwrap();
}

fn ids(store: &Store, account: &str, folder: &str) -> Vec<String> {
    let mut v: Vec<String> = store.load_folder(account, folder).unwrap().messages.iter().map(|m| m.message_id.clone()).collect();
    v.sort();
    v
}

#[test]
fn an_opened_file_is_never_written_and_its_copy_beside_it_keeps_the_changes() {
    let (dir, pst, store, mut p) = setup("beside", 3, 2, "b1", 0);
    // Two with no Message-ID of their own: the import makes one up, naming the file's tag, which
    // the next write to the file changes.
    add_to_file(&pst, "No id, kept", "");
    add_to_file(&pst, "No id, deleted", "");
    let before = as_it_is(&pst);
    store.save_accounts(&[mailcraft_model::AccountConfig { id: "b1".into(), email: "in.pst".into(), ..Default::default() }]).unwrap();
    let copy = dir.join("in.mailcraft");
    assert_eq!(store.place_beside("b1", Path::new(&pst)).unwrap().path, copy);

    // Opened: the copy is made beside it, nothing with the other data files.
    p.submit(Job::ImportPst { account: "b1".into(), path: pst.clone(), resume: false });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    assert!(matches!(seen.last(), Some(Event::Imported { messages: 7, error: None, .. })), "{seen:?}");
    assert_eq!(store.data_file("b1").as_deref(), Some(copy.as_path()));
    assert!(copy.is_file() && data_files(&dir).is_empty(), "{:?}", data_files(&dir));
    let made_up = |store: &Store| {
        let inbox = store.load_folder("b1", "INBOX").unwrap();
        let mut v: Vec<String> = inbox.messages.iter().filter(|m| m.message_id.ends_with("@mailcraft.invalid")).map(|m| m.subject.clone()).collect();
        v.sort();
        v
    };
    assert_eq!(made_up(&store), ["No id, deleted", "No id, kept"]);
    let real = |store: &Store| ids(store, "b1", "INBOX").into_iter().filter(|i| !i.ends_with("@mailcraft.invalid")).collect::<Vec<_>>();
    assert_eq!(real(&store), ["r0-0@run.example", "r0-1@run.example", "r0-2@run.example"]);

    // Changed in MailCraft: one read, two deleted.
    let inbox = store.load_folder("b1", "INBOX").unwrap();
    let key = |id: &str| inbox.messages.iter().find(|m| m.message_id == id || m.subject == id).unwrap().key.clone();
    store.apply_op("b1", &Op::SetFlag { keys: vec![key("r0-0@run.example")], flag: Flag::Seen, on: true }).unwrap();
    store.apply_op("b1", &Op::Expunge { keys: vec![key("r0-1@run.example"), key("No id, deleted")] }).unwrap();
    // Closed: the file is as it was, to the byte and the time it last changed.
    drop(p);
    drop(store);
    assert!(as_it_is(&pst) == before, "the data file opened was written");

    // Changed in Outlook since: told so, and read again into the same copy.
    add_to_file(&pst, "Report late", "<late@run.example>");
    let changed = as_it_is(&pst);
    let store = Store::open_packed(dir.join("app"), dir.join("files")).unwrap();
    let mut p = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let found = store.copy_beside(Path::new(&pst)).unwrap();
    assert_eq!(found.path, dir.join("in (2).mailcraft"), "the copy is the account's still");
    assert!(store.data_file("b1").is_some());
    p.submit(Job::RereadDataFile { account: "b1".into(), path: pst.clone() });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    let Some(Event::Imported { messages: 1, error: None, report: Some(report), .. }) = seen.last() else { panic!("{seen:?}") };
    assert_eq!((report.messages, report.folders), (1, 1));
    // What's new came in; what was done in MailCraft stayed done, the messages with no
    // Message-ID of their own neither doubled nor brought back.
    assert_eq!(real(&store), ["late@run.example", "r0-0@run.example", "r0-2@run.example"]);
    assert_eq!(made_up(&store), ["No id, kept"]);
    let inbox = store.load_folder("b1", "INBOX").unwrap();
    assert!(inbox.messages.iter().find(|m| m.message_id == "r0-0@run.example").unwrap().flags.seen);
    assert!(as_it_is(&pst) == changed, "the data file read again was written");
    assert!(!store.data_files_dir().unwrap().join(".reread-b1").exists(), "the file read again is gone");

    // Removed and opened again: the copy is found, as the file is now (unchanged since).
    assert_eq!(store.remove_account("b1").unwrap().as_deref(), Some(copy.as_path()));
    let found = store.copy_beside(Path::new(&pst)).unwrap();
    assert_eq!(found.path, copy);
    let f = found.found.unwrap();
    assert_eq!((f.state.as_str(), f.changed, f.account.as_str()), ("done", false, "b1"));
    drop(p);
    drop(store);
    assert!(as_it_is(&pst) == changed);
    let _ = std::fs::remove_dir_all(&dir);
}
