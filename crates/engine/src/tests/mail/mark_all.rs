//! Issue #197: Mark All as Read marks the folder, all of it: not a search's results or one tab
//! of Focused / Other alone, and on a server, the unread mail past what's kept here too.

use mailcraft_model::{Flags, Folder, MessageKey, Op, Summary};

use super::*;

fn unread_in(s: &mut Session, account: &str, folder: &str) -> usize {
    s.folder_state(account, folder).messages.iter().filter(|m| !m.flags.seen).count()
}

#[test]
fn the_folder_is_marked_read_all_of_it_whatever_is_shown() {
    let mut s = sample_session("mark-all-search");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let before = unread_in(&mut s, "sample", "INBOX");
    assert!(before >= 2, "the sample's Inbox has unread mail: {before}");
    // A search showing one of them: the folder is marked all the same.
    let subject = s.list.iter().find(|m| !m.flags.seen).unwrap().subject.clone();
    let word = subject.split_whitespace().max_by_key(|w| w.len()).unwrap().to_string();
    s.run("search.run", &json!({"query": word})).unwrap();
    assert!(s.list.len() < s.folder_state("sample", "INBOX").messages.len(), "a search shows less");
    let r = s.run("mail.markAllRead", &json!({})).unwrap();
    assert_eq!(r["count"].as_u64(), Some(before as u64));
    assert_eq!(unread_in(&mut s, "sample", "INBOX"), 0);
    assert_eq!(s.folder("sample", "INBOX").unwrap().unread, 0);
    // Kept that way.
    let kept = s.store.as_ref().unwrap().load_folder("sample", "INBOX").unwrap();
    assert!(kept.messages.iter().all(|m| m.flags.seen));
    // A local account's mail is all here: nothing for a server.
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Apply { op: Op::MarkFolderRead { .. }, .. })));
}

#[test]
fn a_folder_named_is_marked_where_it_is() {
    let mut s = sample_session("mark-all-named");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let projects = s.accounts[0].folders.iter().find(|f| f.path.starts_with("Projects")).map(|f| f.path.clone()).unwrap();
    let key = s.folder_state("sample", &projects).messages.first().map(|m| m.key.clone());
    if let Some(k) = key {
        s.apply_op("sample", Op::SetFlag { keys: vec![k], flag: mailcraft_model::Flag::Seen, on: false });
    }
    let inbox_unread = unread_in(&mut s, "sample", "INBOX");
    s.run("mail.markAllRead", &json!({"account": "sample", "folder": projects})).unwrap();
    assert_eq!(unread_in(&mut s, "sample", &projects), 0);
    assert_eq!(unread_in(&mut s, "sample", "INBOX"), inbox_unread, "the folder on view is left");
    assert!(matches!(&s.view, crate::View::Folder { folder, .. } if folder == "INBOX"), "not opened");
    for bad in [json!({"account": "sample"}), json!({"folder": "INBOX"}), json!({"account": "sample", "folder": "Nowhere"})] {
        assert!(s.run("mail.markAllRead", &bad).is_err(), "{bad}");
    }
}

/// On a server holding five unread where one is kept here: the one kept is marked as any is,
/// and the server marks the rest, the folder's count gone at once.
#[test]
fn a_servers_unread_mail_past_whats_kept_is_marked_there() {
    let (mut s, id) = server_account("mark-all-server", json!({"email": "zoe@example.org", "password": "pw"}), &[]);
    let inbox =
        Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, unread: 5, total: 9, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    let key = |uid| MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid };
    let mail = |uid: u32, seen: bool| -> Summary {
        let raw = format!("From: Ana <ana@x.example>\r\nSubject: Note {uid}\r\nMessage-ID: <{uid}@x>\r\n\r\nHi.\r\n");
        mailcraft_mime::summary_from_header(raw.as_bytes(), key(uid), Flags { seen, ..Default::default() }, 0, NOW)
    };
    let st = mailcraft_store::FolderState {
        validity: 9,
        uid_next: 3,
        highest_modseq: 0,
        messages: vec![mail(2, false).into(), mail(1, true).into()],
        window: 0,
    };
    s.store.as_ref().unwrap().save_folder(&id, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    s.take_jobs();
    let r = s.run("mail.markAllRead", &json!({})).unwrap();
    assert_eq!(r["count"].as_u64(), Some(5), "as many as the server has unread");
    let jobs = s.take_jobs();
    let applied: Vec<&Op> = jobs.iter().filter_map(|j| if let Job::Apply { op, .. } = j { Some(op) } else { None }).collect();
    assert!(applied.iter().any(|op| matches!(op, Op::SetFlag { keys, .. } if keys == &[key(2)])), "the one kept: {applied:?}");
    assert!(applied.iter().any(|op| matches!(op, Op::MarkFolderRead { path } if path == "INBOX")), "the rest on the server: {applied:?}");
    assert_eq!(s.folder(&id, "INBOX").unwrap().unread, 0);
    assert_eq!(unread_in(&mut s, &id, "INBOX"), 0);
    // Nothing unread left: nothing more for the server.
    s.run("mail.markAllRead", &json!({})).unwrap();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Apply { .. })));
}

/// Issue #248: Mark all as read clicked by mistake is undone (Ctrl+Z, or Undo on the top bar):
/// the mail it marked is unread again, the folder's count back, and mail already read stays so.
#[test]
fn mark_all_as_read_is_undone() {
    let mut s = sample_session("mark-all-undo");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let unread: Vec<MessageKey> = s.folder_state("sample", "INBOX").messages.iter().filter(|m| !m.flags.seen).map(|m| m.key.clone()).collect();
    assert!(unread.len() >= 2, "the sample's Inbox has unread mail");
    let read_before = s.folder_state("sample", "INBOX").messages.len() - unread.len();
    let count = s.folder("sample", "INBOX").unwrap().unread;
    s.run("mail.markAllRead", &json!({})).unwrap();
    assert_eq!(unread_in(&mut s, "sample", "INBOX"), 0);
    s.run("edit.undo", &json!({})).unwrap();
    let again: Vec<MessageKey> = s.folder_state("sample", "INBOX").messages.iter().filter(|m| !m.flags.seen).map(|m| m.key.clone()).collect();
    assert_eq!(again, unread);
    assert_eq!(s.folder_state("sample", "INBOX").messages.len() - again.len(), read_before);
    assert_eq!(s.folder("sample", "INBOX").unwrap().unread, count);
    // A folder named (its menu in the folder pane), the same.
    s.run("mail.markAllRead", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    assert_eq!(unread_in(&mut s, "sample", "INBOX"), 0);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(unread_in(&mut s, "sample", "INBOX"), unread.len());
    // Nothing unread: nothing to undo for it.
    s.run("mail.markAllRead", &json!({})).unwrap();
    s.run("mail.markAllRead", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(unread_in(&mut s, "sample", "INBOX"), unread.len(), "the click that marked something is the one undone");
}
