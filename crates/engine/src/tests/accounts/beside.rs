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
/// a message, its note, done, and the note of what came in.
fn imported(s: &mut Session, account: &str, path: &str) {
    imported_as(s, account, path, "done", true);
}

/// [`imported`], its note saying how far it got (`state`), and with the note of what came in
/// only when `seen` (an import made before MailCraft noted it had none).
fn imported_as(s: &mut Session, account: &str, path: &str, state: &str, seen: bool) {
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
    store.save_account_doc(account, "import.json", &json!({"state": state, "source": {"path": path, "size": size, "modified": modified}})).unwrap();
    if seen {
        store.note_imported(account).unwrap();
    }
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

/// A copy with no note of what came in from its file (opened before MailCraft kept one): what's
/// new in the file can't be told from what was deleted here, so it isn't read in, whether the
/// file's account is open or the copy is found again; opened as it was, it is.
#[test]
fn a_copy_with_no_note_of_what_came_in_isnt_read_into_again() {
    let (mut s, dir, pst) = with_file("no-note");
    let id = s.run("account.importPst", &json!({"path": pst})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    imported_as(&mut s, &id, &pst, "done", false);
    assert!(!s.can_read_changes(&id));
    std::fs::write(&pst, b"an Outlook file, as far as this test goes, with more in it now").unwrap();
    let refused = s.run("account.importPst", &json!({"path": pst, "changes": "read"})).unwrap_err();
    assert!(refused.to_string().contains("no note"), "{refused}");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::RereadDataFile { .. })));
    s.run("account.remove", &json!({"account": id})).unwrap();
    s.take_jobs();
    let refused = s.run("account.importPst", &json!({"path": pst, "changes": "read"})).unwrap_err();
    assert!(refused.to_string().contains("no note"), "{refused}");
    assert!(s.accounts.is_empty(), "nothing opened");
    let kept = s.run("account.importPst", &json!({"path": pst, "changes": "keep"})).unwrap();
    assert_eq!(kept["reopened"].as_bool(), Some(true), "{kept}");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::RereadDataFile { .. })));
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy found beside the file whose import was cut off goes on; that run failing before
/// anything more came in (the file can't be read now) keeps the account and its copy, with what
/// was changed in MailCraft, and the import waits to go on.
#[test]
fn a_copy_gone_on_with_survives_a_run_that_fails() {
    let (mut s, dir, pst) = with_file("resume-fails");
    let copy = dir.join("Documents").join("Archive.mailcraft");
    let id = s.run("account.importPst", &json!({"path": pst})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    imported_as(&mut s, &id, &pst, "running", true);
    s.run("account.remove", &json!({"account": id})).unwrap();
    s.take_jobs();
    let opened = s.run("account.importPst", &json!({"path": pst})).unwrap();
    let again = opened["account"].as_str().unwrap().to_string();
    assert_eq!(opened["importing"].as_bool(), Some(true), "{opened}");
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { resume: true, .. }]));
    s.apply(Event::Imported { account: again.clone(), messages: 0, skipped: 0, done: true, error: Some("the file is held".into()), report: None });
    assert!(copy.is_file(), "the copy beside the file stays");
    assert!(s.account(&again).is_some_and(|a| a.folders.len() == 1), "the account stays, its mail too");
    assert!(s.import_state(&again).is_some_and(|st| !st.live), "it waits to go on");
    assert_eq!(s.data_source(&again).map(|r| r.stage), Some(mailcraft_model::job::ImportStage::Stopped));
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("couldn't go on")), "{:?}", s.notices.last());
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Another copy of a file open already is one of its own: a copy found beside the file, an
/// earlier account's, isn't taken for it.
#[test]
fn another_copy_is_a_new_one_beside_the_file() {
    let (mut s, dir, pst) = with_file("again");
    let docs = dir.join("Documents");
    let first = s.run("account.importPst", &json!({"path": pst})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    imported(&mut s, &first, &pst);
    let second = s.run("account.importPst", &json!({"path": pst, "again": true})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    imported(&mut s, &second, &pst);
    assert_eq!(s.store.as_ref().unwrap().data_file(&second), Some(docs.join("Archive (2).mailcraft")));
    // The second goes; the file changed since; another copy again is a new one, not that one.
    s.run("account.remove", &json!({"account": second})).unwrap();
    s.take_jobs();
    std::fs::write(&pst, b"an Outlook file, as far as this test goes, with more in it now").unwrap();
    let third = s.run("account.importPst", &json!({"path": pst, "again": true})).unwrap();
    assert_eq!(third["importing"].as_bool(), Some(true), "{third}");
    let third = third["account"].as_str().unwrap().to_string();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { resume: false, .. }]));
    assert_eq!(s.store.as_ref().unwrap().data_file(&third), Some(docs.join("Archive (3).mailcraft")));
    assert!(docs.join("Archive (2).mailcraft").is_file(), "the earlier copy is left as it was");
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A session on the MailCraft data folder `app` (and its data files) in `dir`.
fn on_folder(dir: &Path, app: &str) -> Session {
    let store = Store::open_packed(dir.join(app), dir.join(format!("{app} Files"))).unwrap();
    Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0))
}

/// Wait for the copy beside `pst` to be closed by a session dropped (its read marks' thread lets
/// go of its store a moment later), as seen from `s`. Never seen closed, the test fails here.
fn closed(s: &Session, pst: &str) {
    for _ in 0..400 {
        if s.store.as_ref().and_then(|st| st.copy_beside(Path::new(pst))).is_some_and(|b| b.found.is_some()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("the copy beside {pst} was never seen closed");
}

/// Whether `e` is the refusal of a copy the data folder at `folder` has.
fn refused_for(e: &CmdError, folder: &Path) -> bool {
    let e = e.to_string();
    e.contains("another MailCraft data folder") && e.contains(&*folder.to_string_lossy())
}

/// Opened in one data folder, then closed: another data folder (the CLI's, a `--data` run) is
/// refused the copy beside the file and told which folder has it, and takes it over only when
/// asked; the copy is then its own.
#[test]
fn a_copy_another_data_folder_has_is_refused_until_taken_over() {
    let (mut s, dir, pst) = with_file("owner");
    let id = s.run("account.importPst", &json!({"path": pst})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    imported(&mut s, &id, &pst);
    // Closed (MailCraft quit): the account and its copy stay that folder's.
    drop(s);

    let mut other = on_folder(&dir, "other");
    closed(&other, &pst);
    // The window looked at the file before the copy was there: what it knows is put right, so
    // the Open dialog says who has the copy and offers to take it over.
    other.data_file_inspected(&pst, Box::new(mailcraft_model::job::DataFileFacts { kind: "pst".into(), ..Default::default() }));
    let refused = other.run("account.importPst", &json!({"path": pst})).unwrap_err();
    let app = dir.join("app").to_string_lossy().into_owned();
    assert!(refused_for(&refused, &dir.join("app")), "{refused}");
    assert!(refused.to_string().contains("takeOver"), "{refused}");
    assert!(other.notices.last().is_some_and(|n| n.error && n.text.contains(&app)), "{:?}", other.notices.last());
    let facts = other.data_file_facts(&pst).unwrap();
    assert_eq!(facts.copy_held_by, app);
    assert_eq!(Path::new(&facts.copy), dir.join("Documents").join("Archive.mailcraft"));
    assert!(other.accounts.is_empty() && other.take_jobs().is_empty(), "nothing opened");
    // Taken over: opened as it was left, and this folder's from now on.
    let opened = other.run("account.importPst", &json!({"path": pst, "takeOver": true})).unwrap();
    assert_eq!((opened["reopened"].as_bool(), opened["importing"].as_bool()), (Some(true), Some(false)), "{opened}");
    let taken = opened["account"].as_str().unwrap().to_string();
    assert_eq!(other.account(&taken).unwrap().folders.len(), 1);
    let store = other.store.clone().unwrap();
    other.run("account.remove", &json!({"account": taken})).unwrap();
    // Removed there: no folder's now, so the first opens it again without asking.
    drop((other, store));
    let mut s = on_folder(&dir, "app");
    assert!(s.account(&id).is_some(), "the first folder still has its account");
    s.run("account.remove", &json!({"account": id})).unwrap();
    closed(&s, &pst);
    let opened = s.run("account.importPst", &json!({"path": pst})).unwrap();
    assert_eq!(opened["reopened"].as_bool(), Some(true), "{opened}");
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Taken over by another data folder, the copy is that folder's: the one that had it never opens
/// it again (its account there says why), and removing that account leaves the copy the other
/// folder's, so a third is still refused, told which has it.
#[test]
fn a_copy_taken_over_stays_taken_when_the_first_removes_its_account() {
    let (mut s, dir, pst) = with_file("taken-over");
    let id = s.run("account.importPst", &json!({"path": pst})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    imported(&mut s, &id, &pst);
    drop(s);
    let mut other = on_folder(&dir, "other");
    closed(&other, &pst);
    let opened = other.run("account.importPst", &json!({"path": pst, "takeOver": true})).unwrap();
    let taken = opened["account"].as_str().unwrap().to_string();
    drop(other);
    closed(&on_folder(&dir, "looking"), &pst);

    let copy = dir.join("Documents").join("Archive.mailcraft");
    let before = std::fs::read(&copy).unwrap();
    let mut s = on_folder(&dir, "app");
    assert!(s.account(&id).is_some(), "still listed in the first");
    let used = s.store.as_ref().unwrap().load_folders(&id).unwrap_err();
    assert!(used.to_string().contains("another MailCraft data folder"), "{used}");
    assert!(std::fs::read(&copy).unwrap() == before, "nothing of the copy written by the first");
    s.run("account.remove", &json!({"account": id})).unwrap();
    assert!(s.account(&id).is_none());
    drop(s);

    let mut third = on_folder(&dir, "third");
    closed(&third, &pst);
    let refused = third.run("account.importPst", &json!({"path": pst})).unwrap_err();
    assert!(refused_for(&refused, &dir.join("other")), "the second still has it: {refused}");
    drop(third);
    let other = on_folder(&dir, "other");
    assert_eq!(other.account(&taken).unwrap().folders.len(), 1, "the second's still");
    drop(other);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy made before copies said whose they are has no owner record, as one whose account was
/// removed has none (it's taken out): no folder's, it opens in any data folder as before.
#[test]
fn a_copy_with_no_owner_opens_anywhere() {
    let (mut s, dir, pst) = with_file("no-owner");
    let id = s.run("account.importPst", &json!({"path": pst})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    imported(&mut s, &id, &pst);
    s.run("account.remove", &json!({"account": id})).unwrap();
    drop(s);
    let mut other = on_folder(&dir, "other");
    closed(&other, &pst);
    let found = other.store.as_ref().and_then(|st| st.copy_beside(Path::new(&pst))).and_then(|b| b.found).unwrap();
    assert_eq!((found.owner.as_str(), found.held), ("", false));
    let opened = other.run("account.importPst", &json!({"path": pst})).unwrap();
    assert_eq!(opened["reopened"].as_bool(), Some(true), "{opened}");
    drop(other);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The data folder that had the copy is gone (or no longer names it): the copy opens elsewhere
/// without asking. A damaged id file is set aside on the way, never fatal.
#[test]
fn a_copy_whose_data_folder_is_gone_opens_elsewhere() {
    let (mut s, dir, pst) = with_file("owner-gone");
    let id = s.run("account.importPst", &json!({"path": pst})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    imported(&mut s, &id, &pst);
    drop(s);
    // Its list of data files no longer names the copy (the folder's other files are left).
    std::fs::write(dir.join("app").join("datafiles.json"), b"{}").unwrap();
    std::fs::create_dir_all(dir.join("other")).unwrap();
    std::fs::write(dir.join("other").join("datafolder.json"), b"\x00junk").unwrap();
    let mut other = on_folder(&dir, "other");
    closed(&other, &pst);
    let opened = other.run("account.importPst", &json!({"path": pst})).unwrap();
    assert_eq!(opened["reopened"].as_bool(), Some(true), "{opened}");
    assert_eq!(std::fs::read(dir.join("other").join("datafolder.json.damaged")).unwrap(), b"\x00junk");
    let taken = opened["account"].as_str().unwrap().to_string();
    other.run("account.remove", &json!({"account": taken})).unwrap();
    drop(other);
    // The other one's again, and closed; that folder removed: a third opens it as well.
    let mut other = on_folder(&dir, "other");
    closed(&other, &pst);
    other.run("account.importPst", &json!({"path": pst})).unwrap();
    drop(other);
    let mut third = on_folder(&dir, "third");
    closed(&third, &pst);
    let refused = third.run("account.importPst", &json!({"path": pst})).unwrap_err();
    assert!(refused_for(&refused, &dir.join("other")), "the other one has it: {refused}");
    std::fs::remove_dir_all(dir.join("other")).unwrap();
    let opened = third.run("account.importPst", &json!({"path": pst})).unwrap();
    assert_eq!(opened["reopened"].as_bool(), Some(true), "{opened}");
    drop(third);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The days an import's flagged messages are due come into the preferences as it ends, and as a
/// read of its file again ends: a day set here stays, junk is left out, and a day cleared here
/// isn't brought back by the next read.
#[test]
fn an_import_s_due_days_come_in_under_the_user_s() {
    use mailcraft_model::organize::FOLLOW_UP_DOC;
    use std::collections::BTreeMap;
    let (mut s, dir, pst) = with_file("due");
    let id = s.run("account.importPst", &json!({"path": pst})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    let store = s.store.clone().unwrap();
    s.prefs.follow_up.insert("mine@x".into(), 20_000);
    let days = BTreeMap::from([("kept@x".to_string(), 20_100), ("mine@x".to_string(), 20_200), ("junk@x".to_string(), i64::MAX)]);
    store.save_account_doc(&id, FOLLOW_UP_DOC, &days).unwrap();
    imported(&mut s, &id, &pst);
    let flagged = |message_id: &str| mailcraft_model::Summary {
        message_id: message_id.into(),
        flags: mailcraft_model::Flags { flagged: true, ..Default::default() },
        ..Default::default()
    };
    assert_eq!(s.follow_up_due(&flagged("kept@x")), Some(20_100));
    assert_eq!(s.follow_up_due(&flagged("mine@x")), Some(20_000), "the user's day stays");
    assert_eq!(s.follow_up_due(&flagged("junk@x")), None);
    let left: BTreeMap<String, i64> = store.load_account_doc(&id, FOLLOW_UP_DOC).unwrap();
    assert!(left.is_empty(), "taken: {left:?}");

    // Cleared here, then the file read again: what the read brought comes in, the cleared one
    // doesn't come back.
    s.prefs.follow_up.remove("kept@x");
    s.run("account.importPst", &json!({"path": pst, "changes": "read"})).unwrap();
    s.take_jobs();
    store.save_account_doc(&id, FOLLOW_UP_DOC, &BTreeMap::from([("new@x".to_string(), 20_300)])).unwrap();
    let report = mailcraft_model::job::ImportReport { messages: 1, ..Default::default() };
    s.apply(Event::Imported { account: id.clone(), messages: 1, skipped: 0, done: true, error: None, report: Some(Box::new(report)) });
    assert_eq!(s.follow_up_due(&flagged("new@x")), Some(20_300));
    assert_eq!(s.follow_up_due(&flagged("kept@x")), None);
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}
