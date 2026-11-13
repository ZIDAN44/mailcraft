//! Engine tests: mail moved or copied into a folder of an Outlook data file opened here
//! (issue #82), or of an account on a server, from another account or within one. Dragged
//! there, nothing happened.

use mailcraft_model::{Flags, Folder, MessageKey, Summary};

use super::*;

const FIRST: &[u8] = b"From: Ana <ana@x.example>\r\nTo: zoe@example.org\r\nSubject: First\r\nMessage-ID: <1@x.example>\r\nDate: Mon, 2 Nov 2026 10:00:00 +0000\r\n\r\nThe first one.\r\n";
const SECOND: &[u8] = b"From: Ben <ben@x.example>\r\nTo: zoe@example.org\r\nSubject: Second\r\nMessage-ID: <2@x.example>\r\nDate: Tue, 3 Nov 2026 10:00:00 +0000\r\n\r\nThe second one.\r\n";

fn folder(account: &str, path: &str, role: FolderRole) -> Folder {
    Folder { account: account.into(), path: path.into(), delimiter: Some('/'), role, selectable: true, ..Default::default() }
}

/// A session with an Outlook data file open (`INBOX`, `Kept`) and an account on a server whose
/// Inbox holds two messages: the first downloaded and flagged, the second only listed. Returns
/// the session, the data file's account, the server's, and the two messages' keys.
fn two_accounts(name: &str) -> (Session, String, String, [MessageKey; 2]) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let file = s.run("account.importPst", &json!({"path": r"C:\Mail\Archive.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    s.apply(Event::Folders {
        account: file.clone(),
        folders: vec![folder(&file, "INBOX", FolderRole::Inbox), folder(&file, "Kept", FolderRole::default())],
    });
    s.apply(Event::Imported { account: file.clone(), messages: 0, skipped: 0, done: true, error: None, report: None });
    let server = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.apply(Event::Folders {
        account: server.clone(),
        folders: vec![folder(&server, "INBOX", FolderRole::Inbox), folder(&server, "Trash", FolderRole::Trash)],
    });
    let key = |uid| MessageKey { account: server.clone(), folder: "INBOX".into(), validity: 9, uid };
    let store = s.store.clone().unwrap();
    let flagged = Flags { seen: true, flagged: true, keywords: vec!["work".into()], ..Default::default() };
    let first: Summary = mailcraft_mime::summary_from_full(FIRST, key(1), flagged, 0, NOW);
    let second: Summary = mailcraft_mime::summary_from_header(SECOND, key(2), Flags::default(), 0, NOW);
    let st = mailcraft_store::FolderState { validity: 9, uid_next: 3, highest_modseq: 0, messages: vec![second.into(), first.into()], window: 0 };
    store.save_folder(&server, "INBOX", &st).unwrap();
    store.write_body(&key(1), FIRST).unwrap();
    s.apply(Event::FolderChanged { account: server.clone(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": server, "folder": "INBOX"})).unwrap();
    assert_eq!(s.list.len(), 2);
    s.take_jobs();
    let keys = [key(1), key(2)];
    (s, file, server, keys)
}

/// What a folder holds: each message's subject, whether it's read, flagged, and its categories.
fn held(s: &Session, account: &str, folder: &str) -> Vec<(String, bool, bool, Vec<String>)> {
    let st = s.store.as_ref().unwrap().load_folder(account, folder).unwrap();
    st.messages.iter().map(|m| (m.subject.clone(), m.flags.seen, m.flags.flagged, m.flags.keywords.clone())).collect()
}

/// Moved into a data file's folder: the message is there, as it was marked and with its source,
/// and deleted on its server only then. One not downloaded yet is asked for, stays where it is
/// until it's here, and goes over when it comes.
#[test]
fn mail_moves_from_a_server_into_a_data_file() {
    let (mut s, file, server, [first, second]) = two_accounts("across-move");
    s.run("list.selectAll", &json!({})).unwrap();
    let r = s.run("mail.move", &json!({"folder": "Kept", "account": file})).unwrap();
    assert_eq!((r["moved"].as_u64(), r["waiting"].as_u64(), r["failed"].as_u64()), (Some(1), Some(1), Some(0)), "{r}");
    assert_eq!(held(&s, &file, "Kept"), [("First".to_string(), true, true, vec!["work".to_string()])]);
    let kept = s.store.as_ref().unwrap().load_folder(&file, "Kept").unwrap();
    let there = kept.messages[0].key.clone();
    assert_eq!(s.store.as_ref().unwrap().read_body(&there).as_deref(), Some(FIRST));
    assert_eq!(kept.messages[0].date, mailcraft_mime::summary_from_full(FIRST, there, Flags::default(), 0, 0).date, "its own date");
    // Deleted on the server, the other asked for; that one is still in the list.
    let jobs = s.take_jobs();
    assert!(
        jobs.iter().any(
            |j| matches!(j, Job::Apply { account, op: Op::Expunge { keys } } if account == &server && keys.as_slice() == std::slice::from_ref(&first))
        ),
        "{jobs:?}"
    );
    assert!(jobs.iter().any(|j| matches!(j, Job::FetchBody { key } if key == &second)), "{jobs:?}");
    assert_eq!(s.list.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Second"]);
    assert!(s.notices.iter().any(|n| !n.error && n.text.contains("not downloaded yet")), "{:?}", s.notices);

    // Asked again while it waits: not twice.
    s.run("list.selectAll", &json!({})).unwrap();
    let again = s.run("mail.move", &json!({"folder": "Kept", "account": file})).unwrap();
    assert_eq!((again["moved"].as_u64(), again["waiting"].as_u64()), (Some(0), Some(0)), "{again}");
    s.take_jobs();

    // Its source comes: filed, then deleted where it was.
    s.apply(Event::Body { key: second.clone(), raw: SECOND.to_vec() });
    let subjects: Vec<String> = held(&s, &file, "Kept").into_iter().map(|m| m.0).collect();
    assert_eq!(subjects, ["Second", "First"], "newest first");
    let jobs = s.take_jobs();
    assert!(
        jobs.iter().any(|j| matches!(j, Job::Apply { account, op: Op::Expunge { keys } } if account == &server && keys.as_slice() == std::slice::from_ref(&second))),
        "{jobs:?}"
    );
    assert!(s.list.is_empty() && s.transfers.is_empty());
    // It comes again (the reading pane asked too): nothing more.
    s.apply(Event::Body { key: second, raw: SECOND.to_vec() });
    assert_eq!(held(&s, &file, "Kept").len(), 2);
    done(s);
}

/// Copied: it's in both, nothing asked of the server but the source of one not here. One that
/// can't be downloaded stays where it is, and that's said.
#[test]
fn mail_copies_into_a_data_file_and_stays_where_it_was() {
    let (mut s, file, server, [_, second]) = two_accounts("across-copy");
    s.run("list.selectAll", &json!({})).unwrap();
    let r = s.run("mail.copy", &json!({"folder": "Kept", "account": file})).unwrap();
    assert_eq!((r["copied"].as_u64(), r["waiting"].as_u64()), (Some(1), Some(1)), "{r}");
    assert_eq!(held(&s, &file, "Kept").len(), 1);
    assert_eq!(s.list.len(), 2, "still where it was");
    let jobs = s.take_jobs();
    assert!(!jobs.iter().any(|j| matches!(j, Job::Apply { .. })), "{jobs:?}");
    s.apply(Event::BodyFailed { key: second, error: "the server is away".into() });
    assert!(s.transfers.is_empty());
    assert!(s.notices.iter().any(|n| n.error && n.text.contains("couldn't be copied") && n.text.contains("the server is away")), "{:?}", s.notices);
    assert_eq!((held(&s, &file, "Kept").len(), held(&s, &server, "INBOX").len()), (1, 2));

    // Within the data file: a copy in another of its folders, the first still in its place.
    s.run("folder.open", &json!({"account": file, "folder": "Kept"})).unwrap();
    s.run("list.selectAll", &json!({})).unwrap();
    let r = s.run("mail.copy", &json!({"folder": "INBOX"})).unwrap();
    assert_eq!(r["copied"].as_u64(), Some(1), "{r}");
    assert_eq!((held(&s, &file, "Kept").len(), held(&s, &file, "INBOX").len()), (1, 1));
    // And a move within it by its account's name is the move it always was.
    let r = s.run("mail.move", &json!({"folder": "INBOX", "account": file})).unwrap();
    assert_eq!(r["moved"].as_u64(), Some(1), "{r}");
    assert_eq!((held(&s, &file, "Kept").len(), held(&s, &file, "INBOX").len()), (0, 2));
    done(s);
}

/// Into a folder on a server: the network layer is asked to upload a copy, and what was moved
/// is deleted where it was only when it says the copy is there. One it couldn't upload stays,
/// and why is said. A folder or an account that isn't there: nothing moves.
#[test]
fn mail_goes_into_a_folder_on_a_server_once_its_copy_is_there() {
    let (mut s, file, server, [first, second]) = two_accounts("across-server");
    // Into the data file first, to have mail there: the downloaded one goes.
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.copy", &json!({"folder": "INBOX", "account": file})).unwrap();
    s.apply(Event::BodyFailed { key: second.clone(), error: "away".into() });
    s.take_jobs();
    s.run("folder.open", &json!({"account": file, "folder": "INBOX"})).unwrap();
    s.run("list.selectAll", &json!({})).unwrap();
    let there = s.list[0].key.clone();

    // Moved from the data file to the server's Trash: asked of the network layer, still here.
    let r = s.run("mail.move", &json!({"folder": "Trash", "account": server})).unwrap();
    assert_eq!((r["moved"].as_u64(), r["sending"].as_u64()), (Some(0), Some(1)), "{r}");
    let jobs = s.take_jobs();
    match jobs.as_slice() {
        [Job::FileCopies { account, folder, mail, moved: true }] => {
            assert_eq!((account, folder.as_str()), (&server, "Trash"));
            assert_eq!(mail.len(), 1);
            assert_eq!(mail[0].0, there);
            assert!(mail[0].1.seen && mail[0].1.flagged && mail[0].1.keywords == ["work"], "as it was marked: {:?}", mail[0].1);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(held(&s, &file, "INBOX").len(), 1, "not deleted before its copy is there");
    // Said meanwhile (issue #85): it's still where it was, and looked like nothing happened.
    assert!(s.notices.iter().any(|n| !n.error && n.text == "Moving 1 message to Trash…"), "{:?}", s.notices.last());
    // Asked again while it's on its way: not twice.
    let again = s.run("mail.move", &json!({"folder": "Trash", "account": server})).unwrap();
    assert_eq!(again["sending"].as_u64(), Some(0), "{again}");
    assert!(s.take_jobs().is_empty());
    // The server wouldn't take it: it stays, and that's said.
    s.apply(Event::CopiesFiled {
        account: server.clone(),
        folder: "Trash".into(),
        done: vec![],
        failed: vec![(there.clone(), "the mailbox is full".into())],
        moved: true,
    });
    assert_eq!(held(&s, &file, "INBOX").len(), 1);
    assert!(
        s.notices.iter().any(|n| n.error && n.text.contains("couldn't be moved to Trash") && n.text.contains("the mailbox is full")),
        "{:?}",
        s.notices
    );
    // Tried again, and it's there: deleted where it was.
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.move", &json!({"folder": "Trash", "account": server})).unwrap();
    s.take_jobs();
    s.apply(Event::CopiesFiled { account: server.clone(), folder: "Trash".into(), done: vec![there], failed: vec![], moved: true });
    assert!(s.notices.iter().any(|n| !n.error && n.text == "1 message moved to Trash"), "{:?}", s.notices.last());
    assert!(held(&s, &file, "INBOX").is_empty() && s.list.is_empty() && s.transfers.is_empty());

    // A copy within the server's account: uploaded too; the one not downloaded is asked for
    // first, and uploaded when it's here. Neither is deleted.
    s.run("folder.open", &json!({"account": server, "folder": "INBOX"})).unwrap();
    s.run("list.selectAll", &json!({})).unwrap();
    let r = s.run("mail.copy", &json!({"folder": "Trash"})).unwrap();
    assert_eq!((r["copied"].as_u64(), r["sending"].as_u64(), r["waiting"].as_u64()), (Some(0), Some(1), Some(1)), "{r}");
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::FileCopies { mail, moved: false, .. } if mail.len() == 1 && mail[0].0 == first)), "{jobs:?}");
    assert!(jobs.iter().any(|j| matches!(j, Job::FetchBody { key } if key == &second)), "{jobs:?}");
    s.store.as_ref().unwrap().write_body(&second, SECOND).unwrap();
    s.apply(Event::Body { key: second.clone(), raw: SECOND.to_vec() });
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::FileCopies { mail, moved: false, .. } if mail.len() == 1 && mail[0].0 == second)), "{jobs:?}");
    s.apply(Event::CopiesFiled { account: server.clone(), folder: "Trash".into(), done: vec![first, second], failed: vec![], moved: false });
    assert!(s.transfers.is_empty() && !s.take_jobs().iter().any(|j| matches!(j, Job::Apply { .. })));
    assert_eq!(held(&s, &server, "INBOX").len(), 2);

    assert!(matches!(s.run("mail.copy", &json!({"folder": "Nowhere", "account": file})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("mail.move", &json!({"folder": "INBOX", "account": "nobody"})), Err(CmdError::Params(_))));

    // The account removed while its messages are on their way: they're forgotten.
    s.run("mail.copy", &json!({"folder": "Trash"})).unwrap();
    assert_eq!(s.transfers.len(), 2);
    s.run("account.remove", &json!({"account": server})).unwrap();
    assert!(s.transfers.is_empty());
    done(s);
}

/// Found in review. A move the network layer never answered (its connection ended first) kept
/// the message "on its way": moved again, nothing happened, until MailCraft started again; and
/// one that waited for its download went when the message was merely opened, long after. Such
/// a one is forgotten after a while. Mail put into another account's Inbox didn't arrive
/// there: no rule runs on it.
#[test]
fn a_move_never_answered_is_forgotten_and_what_is_put_in_an_inbox_did_not_arrive() {
    let (mut s, file, server, [first, second]) = two_accounts("across-stale");
    // From the data file into the server's Inbox... first there has to be mail in the file.
    s.run("list.select", &json!({"key": first.to_id()})).unwrap();
    s.run("mail.copy", &json!({"folder": "Kept", "account": file})).unwrap();
    s.run("folder.open", &json!({"account": file, "folder": "Kept"})).unwrap();
    let kept = s.list.first().unwrap().key.clone();
    s.run("list.select", &json!({"key": kept.to_id()})).unwrap();
    let r = s.run("mail.copy", &json!({"folder": "INBOX", "account": server})).unwrap();
    assert_eq!(r["sending"], 1);
    assert!(s.moved_in.iter().any(|(a, f, id)| *a == server && f == "INBOX" && id.contains("1@x.example")), "put there by hand: {:?}", s.moved_in);
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::FileCopies { .. })));
    // Not answered yet, however long it takes (an upload waits behind the account's sync):
    // asked again, it's on its way, and isn't sent twice. Answered that it didn't go (its
    // connection stopped first): it can be sent again.
    let again = s.run("mail.copy", &json!({"folder": "INBOX", "account": server})).unwrap();
    assert_eq!(again["sending"], 0);
    s.clock.now += 31 * 60;
    let again = s.run("mail.copy", &json!({"folder": "INBOX", "account": server})).unwrap();
    assert_eq!(again["sending"], 0, "still on its way: not there twice");
    s.apply(Event::CopiesFiled {
        account: server.clone(),
        folder: "INBOX".into(),
        done: Vec::new(),
        failed: vec![(kept.clone(), "the account's connection stopped first".into())],
        moved: false,
    });
    let again = s.run("mail.copy", &json!({"folder": "INBOX", "account": server})).unwrap();
    assert_eq!(again["sending"], 1);
    s.take_jobs();

    // One waiting for its download, which never came: opened much later, it stays put.
    s.run("folder.open", &json!({"account": server, "folder": "INBOX"})).unwrap();
    s.run("list.select", &json!({"key": second.to_id()})).unwrap();
    let r = s.run("mail.move", &json!({"folder": "Kept", "account": file})).unwrap();
    assert_eq!(r["waiting"], 1);
    s.clock.now += 31 * 60;
    let store = s.store.clone().unwrap();
    store.write_body(&second, SECOND).unwrap();
    s.apply(Event::Body { key: second.clone(), raw: SECOND.to_vec() });
    assert!(store.load_folder(&server, "INBOX").unwrap().messages.iter().any(|m| m.key == second), "not moved after all that time");
    assert_eq!(held(&s, &file, "Kept").len(), 1);
    done(s);
}

/// Found in review. Moved to the folder it's in (Archive pressed in Archive), a message gave
/// way to the next one though nothing moved. And an Undo whose mail is gone from where it was
/// moved to (deleted for good since) was kept to be tried again, in the way of every Undo
/// after: it's dropped, and what was done before it is undone.
#[test]
fn nothing_moved_leaves_the_message_open_and_an_undo_that_cant_be_done_is_dropped() {
    let (mut s, _file, server, [first, _second]) = two_accounts("across-undo");
    s.run("list.open", &json!({"key": first.to_id()})).unwrap();
    let r = s.run("mail.move", &json!({"folder": "INBOX"})).unwrap();
    assert_eq!(r["moved"], 0);
    assert_eq!(s.selection.first(), Some(&first), "still the one open");

    // Flagged off, then moved to Trash; gone from Trash since.
    s.run("mail.flag", &json!({"on": false})).unwrap();
    s.run("mail.move", &json!({"folder": "Trash"})).unwrap();
    s.take_jobs();
    // Pressed at once, on a server account: the server hasn't moved it yet. Kept, to be tried
    // again (found in a second review: dropped, an Undo a second after a move was lost).
    let err = s.run("edit.undo", &json!({})).unwrap_err().to_string();
    assert!(err.contains("still being moved"), "{err}");
    s.clock.now += 600;
    let err = s.run("edit.undo", &json!({})).unwrap_err().to_string();
    assert!(err.contains("can't be put back"), "{err}");
    // The one before it is undone now.
    let undone = s.run("edit.undo", &json!({})).unwrap();
    assert_ne!(undone["undone"], json!("move"), "{undone}");
    let _ = server;
    done(s);
}
