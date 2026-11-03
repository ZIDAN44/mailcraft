//! Engine tests: an Outlook data file opened as a mailbox keeps MailCraft's copy beside it, goes
//! on with that copy when it's opened again, and is read again into it when it changed.

use std::path::{Path, PathBuf};

use super::*;

/// A session over a store of data files, and a folder with an Outlook file in it.
fn with_file(name: &str) -> (Session, PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("mailcraft-engine-beside-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let docs = dir.join("Documents");
    std::fs::create_dir_all(&docs).unwrap();
    let pst = docs.join("Archive.pst");
    std::fs::write(&pst, b"an Outlook file, as far as this test goes").unwrap();
    let store = Store::open_packed(dir.join("app"), dir.join("MailCraft Files")).unwrap();
    let s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    (s, dir, pst.to_string_lossy().into_owned())
}

/// What an import of `path` into `account` leaves, as the network side writes it: an Inbox with
/// a message, and its note, done.
fn imported(s: &mut Session, account: &str, path: &str) {
    let store = s.store.clone().unwrap();
    let inbox = mailcraft_model::Folder {
        account: account.into(),
        path: "INBOX".into(),
        role: FolderRole::Inbox,
        selectable: true,
        total: 1,
        ..Default::default()
    };
    store.save_folders(account, std::slice::from_ref(&inbox)).unwrap();
    let key = mailcraft_model::MessageKey { account: account.into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    store.write_body(&key, b"Subject: kept\r\n\r\nhello").unwrap();
    let mut st = store.load_folder(account, "INBOX").unwrap();
    st.validity = 1;
    st.messages.push(mailcraft_model::Summary { key, subject: "kept".into(), message_id: "kept@x".into(), ..Default::default() }.into());
    store.save_folder(account, "INBOX", &st).unwrap();
    let (size, modified) = mailcraft_store::file_stamp(Path::new(path)).unwrap();
    store.save_account_doc(account, "import.json", &json!({"state": "done", "source": {"path": path, "size": size, "modified": modified}})).unwrap();
    s.apply(Event::Folders { account: account.into(), folders: vec![inbox] });
    s.apply(Event::Imported { account: account.into(), messages: 1, skipped: 0, done: true, error: None, report: None });
}

#[test]
fn an_opened_file_keeps_its_copy_beside_it_and_goes_on_with_it() {
    let (mut s, dir, pst) = with_file("reopen");
    let copy = dir.join("Documents").join("Archive.mailcraft");
    let id = s.run("account.importPst", &json!({"path": pst})).unwrap()["account"].as_str().unwrap().to_string();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { resume: false, .. }]));
    assert_eq!(s.store.as_ref().unwrap().data_file(&id).as_deref(), Some(copy.as_path()), "its copy goes beside the file");
    imported(&mut s, &id, &pst);
    assert!(copy.is_file());

    // Removed: the copy stays beside the file. Opened again: gone on with, nothing read.
    s.run("account.remove", &json!({"account": id})).unwrap();
    s.take_jobs();
    assert!(copy.is_file());
    let opened = s.run("account.importPst", &json!({"path": pst})).unwrap();
    assert_eq!((opened["reopened"].as_bool(), opened["importing"].as_bool()), (Some(true), Some(false)), "{opened}");
    let again = opened["account"].as_str().unwrap().to_string();
    assert_eq!(again, id, "the account it was, its id free again");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::ImportPst { .. } | Job::RereadDataFile { .. })));
    assert_eq!(s.view, View::Folder { account: again.clone(), folder: "INBOX".into() });
    assert_eq!(s.account(&again).unwrap().folders.len(), 1);
    assert_eq!(s.data_source(&again).map(|r| r.stage), Some(mailcraft_model::job::ImportStage::Done));
    // Open as that account: read again into it when asked.
    assert!(s.run("account.importPst", &json!({"path": pst})).is_err(), "open already");
    let reread = s.run("account.importPst", &json!({"path": pst, "changes": "read"})).unwrap();
    assert_eq!(reread["rereading"].as_bool(), Some(true));
    assert!(matches!(s.take_jobs().as_slice(), [Job::RereadDataFile { account, .. }] if *account == again));
    assert!(s.import_state(&again).is_some_and(|st| st.live && st.rereading));
    assert!(s.run("account.importPst", &json!({"path": pst, "changes": "read"})).is_err(), "one at a time");
    // It adds what's new: the account stays, its mail too.
    let report = mailcraft_model::job::ImportReport { messages: 3, ..Default::default() };
    s.apply(Event::Imported { account: again.clone(), messages: 3, skipped: 0, done: true, error: None, report: Some(Box::new(report)) });
    assert!(s.import_state(&again).is_none() && s.account(&again).is_some());
    assert!(s.notices.last().is_some_and(|n| !n.error && n.text.contains("3 new messages")), "{:?}", s.notices.last());
    // One that fails, or stops, adds nothing and leaves the account as it was.
    s.run("account.importPst", &json!({"path": pst, "changes": "read"})).unwrap();
    s.apply(Event::Imported { account: again.clone(), messages: 0, skipped: 0, done: true, error: Some("the file is held".into()), report: None });
    assert!(s.account(&again).is_some() && copy.is_file(), "a failed read never takes the account");
    s.run("account.importPst", &json!({"path": pst, "changes": "read"})).unwrap();
    let paused = mailcraft_model::job::ImportProgress { phase: mailcraft_model::job::ImportPhase::Paused, ..Default::default() };
    s.apply(Event::ImportProgress { account: again.clone(), progress: paused });
    assert!(s.import_state(&again).is_none() && s.account(&again).is_some());
    // Removed while it reads: the account goes once it has stopped, its copy kept.
    s.take_jobs();
    s.run("account.importPst", &json!({"path": pst, "changes": "read"})).unwrap();
    s.take_jobs();
    s.run("account.remove", &json!({"account": again})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::StopImport { how: mailcraft_model::job::StopHow::Discard, .. }]));
    let report = mailcraft_model::job::ImportReport { stopped: Some(mailcraft_model::job::StopHow::Discard), ..Default::default() };
    s.apply(Event::Imported { account: again.clone(), messages: 0, skipped: 0, done: true, error: None, report: Some(Box::new(report)) });
    assert!(s.account(&again).is_none() && copy.is_file());

    // The file changed since: asked what to do; opened as it was, or read again into the copy.
    std::fs::write(&pst, b"an Outlook file, as far as this test goes, with more in it now").unwrap();
    let asked = s.run("account.importPst", &json!({"path": pst})).unwrap_err();
    assert!(asked.to_string().contains("changed"), "{asked}");
    assert!(s.run("account.importPst", &json!({"path": pst, "changes": "later"})).is_err());
    let kept = s.run("account.importPst", &json!({"path": pst, "changes": "keep"})).unwrap();
    let kept = kept["account"].as_str().unwrap().to_string();
    assert!(s.notices.last().is_some_and(|n| n.text.contains("opened as it was read")), "{:?}", s.notices.last());
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::RereadDataFile { .. })));
    s.run("account.remove", &json!({"account": kept})).unwrap();
    let read = s.run("account.importPst", &json!({"path": pst, "changes": "read"})).unwrap();
    let read = read["account"].as_str().unwrap().to_string();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::RereadDataFile { account, .. } if *account == read)));
    assert_eq!(s.account(&read).unwrap().folders.len(), 1, "its mail is there while it reads");
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Another account given the copy's id since: the copy is opened under a new one, its mail made
/// that account's.
#[test]
fn a_copy_whose_account_id_was_given_again_is_opened_as_a_new_one() {
    let (mut s, dir, pst) = with_file("new-id");
    let id = s.run("account.importPst", &json!({"path": pst})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    imported(&mut s, &id, &pst);
    s.run("account.remove", &json!({"account": id})).unwrap();
    // The id is another account's now.
    let other =
        s.run("account.add", &json!({"email": "zoe@studio.example", "password": "pw", "imapHost": "imap.example", "smtpHost": "smtp.example"}));
    let other = other.unwrap();
    assert_eq!(other.as_str().or_else(|| other["account"].as_str()), Some(id.as_str()));
    let opened = s.run("account.importPst", &json!({"path": pst})).unwrap();
    let again = opened["account"].as_str().unwrap().to_string();
    assert_ne!(again, id);
    let st = s.store.as_ref().unwrap().load_folder(&again, "INBOX").unwrap();
    assert_eq!(st.messages[0].key.account, again);
    assert!(s.account(&again).unwrap().folders.iter().all(|f| f.account == again));
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}
