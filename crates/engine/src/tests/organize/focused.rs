//! Engine tests: the Focused Inbox while mail keeps arriving.

use super::*;

/// Mail arriving in a Focused Inbox doesn't have the address book counted again each time (that
/// read every folder of every account, on each sync of the Inbox on screen): the one counted
/// last sorts it until it's ten minutes old. Suggestions, which want everyone, count it again.
#[test]
fn the_address_book_isnt_counted_again_for_each_sync() {
    let mut s = sample_session("focused-book");
    s.run("view.focused", &json!({"on": true})).unwrap();
    let focused: Vec<String> = s.list.iter().map(|m| m.subject.clone()).collect();
    let built = s.contacts_built;
    for _ in 0..10 {
        s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    }
    assert_eq!(s.contacts_built, built, "not counted again");
    assert_eq!(s.list.iter().map(|m| m.subject.clone()).collect::<Vec<_>>(), focused, "sorted as before");
    // Ten minutes on, the next sync counts it again, once.
    s.set_clock(NOW + 601, 0);
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    assert_eq!(s.contacts_built, built + 1);
    // A suggestion counts what's there now.
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    assert!(!s.suggest_contacts("ana", 5).is_empty());
    assert_eq!(s.contacts_built, built + 2);
}

/// Issue #199: with the Focused Inbox on, only mail that goes to Focused is announced, as
/// Outlook does: a newsletter coming into Other comes in quietly, a colleague's message is said.
/// Asked to, Other's mail is announced too; with the Focused Inbox off, all of it is.
#[test]
fn only_focused_mail_is_announced() {
    let (mut s, id) = server_account("focused-notify", json!({"email": "zoe@example.org", "password": "pw"}), &[("INBOX", FolderRole::Inbox)]);
    let store = s.store.clone().unwrap();
    let m = |uid: u32, from: &str, bulk: bool| mailcraft_model::Summary {
        key: mailcraft_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid },
        subject: format!("m{uid}"),
        from: mailcraft_model::EmailAddress::new("", from),
        date: NOW - 10,
        bulk,
        ..Default::default()
    };
    let mut kept: Vec<mailcraft_model::Summary> = Vec::new();
    let mut arrive = |s: &mut Session, more: Vec<mailcraft_model::Summary>| -> Vec<String> {
        kept.extend(more);
        let st = mailcraft_store::FolderState {
            validity: 9,
            uid_next: 99,
            highest_modseq: 0,
            messages: kept.iter().rev().cloned().map(Into::into).collect(),
            window: 0,
        };
        store.save_folder(&id, "INBOX", &st).unwrap();
        s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
        s.take_new_mail().into_iter().map(|m| m.subject).collect()
    };
    arrive(&mut s, vec![m(1, "old@x.example", false)]);
    s.run("view.focused", &json!({"on": true})).unwrap();
    // A newsletter and a colleague's message: only the colleague's is said.
    assert_eq!(arrive(&mut s, vec![m(2, "news@shop.example", true), m(3, "ana@x.example", false)]), ["m3"]);
    // Asked to, Other's too.
    s.run("view.notifyOther", &json!({"on": true})).unwrap();
    assert_eq!(arrive(&mut s, vec![m(4, "news@shop.example", true)]), ["m4"]);
    s.run("view.notifyOther", &json!({"on": false})).unwrap();
    // A sender always put in Focused is said, newsletter or not.
    s.prefs.focus_overrides.insert("news@shop.example".into(), true);
    assert_eq!(arrive(&mut s, vec![m(5, "news@shop.example", true)]), ["m5"]);
    s.prefs.focus_overrides.clear();
    // The Focused Inbox off: all of it.
    s.run("view.focused", &json!({"on": false})).unwrap();
    assert_eq!(arrive(&mut s, vec![m(6, "news@shop.example", true)]), ["m6"]);
    // Kept with the settings.
    s.run("view.notifyOther", &json!({"on": true})).unwrap();
    let kept: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert!(kept.notify_other);
}

/// A new-mail notification clicked shows its message where it is (issue #273): from another
/// folder, its folder opens; on the Focused tab, Other (where it is); a filter hiding it is
/// lifted; and it's shown. One no longer there is said, the view as it was.
#[test]
fn a_message_is_shown_where_it_is() {
    let mut s = sample_session("reveal");
    s.run("view.focused", &json!({"on": true})).unwrap();
    s.run("list.focusTab", &json!({"value": "other"})).unwrap();
    let other = s.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.clone()).expect("in Other");
    // Read, so the Unread filter hides it.
    s.run("list.select", &json!({"key": other.to_id()})).unwrap();
    s.run("mail.markRead", &json!({})).unwrap();
    s.run("list.focusTab", &json!({"value": "focused"})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    s.run("list.filter", &json!({"value": "unread"})).unwrap();
    s.run("message.reveal", &json!({"key": other.to_id()})).unwrap();
    assert_eq!(s.filter, crate::session::ListFilter::All, "the filter lifted");
    assert!(matches!(&s.view, View::Folder { account, folder } if account == "sample" && folder == "INBOX"), "{:?}", s.view);
    assert!(s.focus_other, "on the Other tab, where it is");
    assert!(s.reading.as_ref().is_some_and(|r| r.key == other));
    assert_eq!(s.selection, std::slice::from_ref(&other));
    // One no longer there.
    let gone = mailcraft_model::MessageKey { uid: 99_999, ..other.clone() };
    assert!(s.run("message.reveal", &json!({"key": gone.to_id()})).is_err());
    let elsewhere = mailcraft_model::MessageKey { folder: "No such folder".into(), ..other };
    assert!(s.run("message.reveal", &json!({"key": elsewhere.to_id()})).is_err());
    assert!(s.run("message.reveal", &json!({})).is_err());
}
