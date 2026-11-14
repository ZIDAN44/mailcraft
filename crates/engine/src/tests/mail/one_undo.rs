//! One action over mail of several accounts (All Inboxes) is one Undo.

use mailcraft_model::{Flags, Folder, MessageKey, Summary};

use super::*;

/// Two accounts kept on this computer (a move there is done at once), each with one message in
/// its Inbox, shown together.
fn all_inboxes(name: &str) -> (Session, Vec<String>) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let store = s.store.clone().unwrap();
    let mut ids = Vec::new();
    for (n, email) in ["zoe@example.org", "ben@example.org"].into_iter().enumerate() {
        let id = s.run("account.add", &json!({"email": email, "local": true})).unwrap()["account"].as_str().unwrap().to_string();
        let raw = format!("From: Ana <ana@x.example>\r\nTo: {email}\r\nSubject: Note {n}\r\nMessage-ID: <{n}@x.example>\r\n\r\nHi.\r\n");
        let folder = |path: &str, role| Folder { account: id.clone(), path: path.into(), role, selectable: true, ..Default::default() };
        s.apply(Event::Folders { account: id.clone(), folders: vec![folder("INBOX", FolderRole::Inbox), folder("Trash", FolderRole::Trash)] });
        let key = MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid: 1 };
        let m: Summary = mailcraft_mime::summary_from_full(raw.as_bytes(), key.clone(), Flags::default(), 0, NOW);
        let st = mailcraft_store::FolderState { validity: 9, uid_next: 2, highest_modseq: 0, messages: vec![m.into()], window: 0 };
        store.save_folder(&id, "INBOX", &st).unwrap();
        store.write_body(&key, raw.as_bytes()).unwrap();
        s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
        ids.push(id);
    }
    s.run("folder.unified", &json!({"role": "inbox"})).unwrap();
    assert_eq!(s.list.len(), 2);
    s.take_jobs();
    (s, ids)
}

/// Marked read, put in a category, and deleted, over both accounts at once: each is one Undo
/// that puts both back (it took one Ctrl+Z per account, in no set order).
#[test]
fn one_action_over_several_accounts_is_one_undo() {
    let (mut s, ids) = all_inboxes("one-undo");
    let inbox = |s: &Session, id: &str| s.store.as_ref().unwrap().load_folder(id, "INBOX").unwrap().messages;
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.markRead", &json!({})).unwrap();
    assert_eq!(s.undo.len(), 1, "marked read");
    s.run("mail.categorize", &json!({"category": "Work"})).unwrap();
    assert_eq!(s.undo.len(), 2, "put in a category");
    s.run("edit.undo", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    for id in &ids {
        let m = &inbox(&s, id)[0];
        assert!(!m.flags.seen && m.flags.keywords.is_empty(), "{id}: both put back");
    }
    s.run("folder.unified", &json!({"role": "inbox"})).unwrap();
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert_eq!(s.undo.len(), 1, "deleted");
    assert!(ids.iter().all(|id| inbox(&s, id).is_empty()));
    s.run("edit.undo", &json!({})).unwrap();
    assert!(ids.iter().all(|id| inbox(&s, id).len() == 1), "both back");
    assert!(s.undo.is_empty());
}

/// Archived over two accounts, one of which has no Archive folder: refused before anything
/// moves. The first account's mail was archived (and the selection left on it), then the second
/// refused.
#[test]
fn a_move_one_account_cant_make_moves_nothing() {
    let (mut s, ids) = all_inboxes("move-checked-first");
    let folder = |path: &str, role| Folder { account: ids[0].clone(), path: path.into(), role, selectable: true, ..Default::default() };
    s.apply(Event::Folders {
        account: ids[0].clone(),
        folders: vec![folder("INBOX", FolderRole::Inbox), folder("Trash", FolderRole::Trash), folder("Archive", FolderRole::Archive)],
    });
    s.run("folder.unified", &json!({"role": "inbox"})).unwrap();
    s.run("list.selectAll", &json!({})).unwrap();
    let e = s.run("mail.archive", &json!({})).unwrap_err().to_string();
    assert!(e.contains("Archive"), "{e}");
    for id in &ids {
        assert_eq!(s.store.as_ref().unwrap().load_folder(id, "INBOX").unwrap().messages.len(), 1, "{id}: still in its Inbox");
    }
    assert!(s.undo.is_empty() && s.selection.len() == 2, "nothing done");
}

/// Six accounts on servers, each with one message in its Inbox, shown together.
fn six_servers(name: &str) -> (Session, Vec<String>) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let store = s.store.clone().unwrap();
    let mut ids = Vec::new();
    for n in 0..6 {
        let email = format!("user{n}@example.org");
        let id = s.run("account.add", &json!({"email": email, "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
        let folder = |path: &str, role| Folder { account: id.clone(), path: path.into(), role, selectable: true, ..Default::default() };
        s.apply(Event::Folders { account: id.clone(), folders: vec![folder("INBOX", FolderRole::Inbox), folder("Trash", FolderRole::Trash)] });
        let key = MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid: 1 };
        let m = Summary { key, subject: format!("Note {n}"), message_id: format!("{n}@x.example"), date: NOW - 60, ..Default::default() };
        let st = mailcraft_store::FolderState { validity: 9, uid_next: 2, highest_modseq: 0, messages: vec![m.into()], window: 0 };
        store.save_folder(&id, "INBOX", &st).unwrap();
        s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
        ids.push(id);
    }
    ids.sort();
    s.run("folder.unified", &json!({"role": "inbox"})).unwrap();
    assert_eq!(s.list.len(), 6);
    s.take_jobs();
    (s, ids)
}

/// One change over mail of several accounts goes to their servers in the accounts' order, each
/// account's changes together, whatever the change: it went in no set order for some of them.
#[test]
fn a_change_over_several_accounts_goes_in_their_order() {
    let (mut s, ids) = six_servers("accounts-order");
    let order = |s: &mut Session| -> Vec<String> {
        s.take_jobs().into_iter().filter_map(|j| if let Job::Apply { account, .. } = j { Some(account) } else { None }).collect()
    };
    for (id, params) in [
        ("mail.markRead", json!({})),
        ("mail.categorize", json!({"category": "Work"})),
        ("mail.flag", json!({})),
        ("mail.flag", json!({"complete": true})),
        ("mail.pin", json!({})),
    ] {
        s.run("list.selectAll", &json!({})).unwrap();
        s.run(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
        let mut accounts = order(&mut s);
        assert!(!accounts.is_empty(), "{id}: nothing sent");
        assert!(accounts.is_sorted(), "{id}: {accounts:?}");
        accounts.dedup();
        assert_eq!(accounts, ids, "{id}");
        s.run("edit.undo", &json!({})).unwrap();
        let mut back = order(&mut s);
        assert!(back.is_sorted(), "{id} undone: {back:?}");
        back.dedup();
        assert_eq!(back, ids, "{id} undone");
    }
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.delete", &json!({"permanent": true})).unwrap();
    let accounts = order(&mut s);
    assert_eq!(accounts, ids, "deleted");
}
