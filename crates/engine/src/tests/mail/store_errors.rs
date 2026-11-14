//! Engine tests: a store that fails to read or write loses no mail (a list that couldn't be read
//! was saved as empty, a source that couldn't be kept left a message without one).

use super::*;

use mailcraft_store::FolderState;

const SHORT_FILM: &str = "Projects/Short film";

/// The sample in a cache whose files can be made to fail.
fn failing_session() -> Session {
    let store = Store::failing();
    assert!(sample::seed_if_empty(&store, NOW).unwrap());
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(s.take_jobs().is_empty());
    s
}

fn store(s: &Session) -> &Store {
    s.store.as_ref().unwrap()
}

fn folder(s: &Session, path: &str) -> FolderState {
    store(s).load_folder(sample::SAMPLE_ACCOUNT, path).unwrap()
}

fn subjects(st: &FolderState) -> Vec<String> {
    let mut v: Vec<String> = st.messages.iter().map(|m| m.subject.clone()).collect();
    v.sort();
    v
}

/// Every message of a folder has its source.
fn all_kept(s: &Session, st: &FolderState) -> bool {
    st.messages.iter().all(|m| store(s).read_body(&m.key).is_some())
}

#[test]
fn a_local_move_that_cant_read_or_write_leaves_the_mail_where_it_was() {
    let mut s = failing_session();
    let inbox = folder(&s, "INBOX");
    let film = folder(&s, SHORT_FILM);
    assert!(!film.messages.is_empty() && all_kept(&s, &inbox) && all_kept(&s, &film));
    // A list, a source that can't be read; a source that can't be written.
    for (reads, writes) in [(Some("state.json"), None), (Some(".eml"), None), (None, Some(".eml"))] {
        let k = key_of(&s, "Character turnarounds");
        s.run("list.select", &k).unwrap();
        let undo = s.undo.len();
        store(&s).fail(reads, writes);
        let r = s.run("mail.move", &json!({"folder": SHORT_FILM}));
        store(&s).fail(None, None);
        // Said by the command (the CLI too), with nothing to undo.
        assert!(matches!(r, Err(CmdError::Failed(_))), "{reads:?} {writes:?}: {r:?}");
        assert_eq!(s.undo.len(), undo, "{reads:?} {writes:?}: no undo for a move that didn't happen");
        assert_eq!(subjects(&folder(&s, "INBOX")), subjects(&inbox), "{reads:?} {writes:?}: still in the Inbox");
        assert_eq!(subjects(&folder(&s, SHORT_FILM)), subjects(&film), "{reads:?} {writes:?}: the destination keeps its mail");
        assert!(all_kept(&s, &folder(&s, "INBOX")) && all_kept(&s, &folder(&s, SHORT_FILM)), "{reads:?} {writes:?}");
        s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
        assert!(s.list.iter().any(|m| m.subject == "Character turnarounds"), "{reads:?} {writes:?}: shown where it is");
    }
    // Working again, it moves.
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.move", &json!({"folder": SHORT_FILM})).unwrap();
    assert_eq!(folder(&s, SHORT_FILM).messages.len(), film.messages.len() + 1);
    assert!(all_kept(&s, &folder(&s, SHORT_FILM)));
}

/// The path a folder's list is written to ends with this.
fn list_of(folder: &str) -> String {
    format!("f{:016x}/state.json", mailcraft_store::fnv64(folder))
}

/// A local move whose copies are filed but whose sources can't be taken out (their list can't be
/// written) failed: the mail is in both folders, and there's nothing to undo. It said it was
/// moved, with an Undo that couldn't put it back.
#[test]
fn a_local_move_that_cant_take_the_mail_out_says_so() {
    let mut s = failing_session();
    let inbox = folder(&s, "INBOX");
    let film = folder(&s, SHORT_FILM);
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    let undo = s.undo.len();
    store(&s).fail(None, Some(&list_of("INBOX")));
    let r = s.run("mail.move", &json!({"folder": SHORT_FILM}));
    store(&s).fail(None, None);
    assert!(matches!(&r, Err(CmdError::Failed(e)) if e.contains(SHORT_FILM)), "{r:?}");
    assert_eq!(s.undo.len(), undo, "nothing to undo");
    assert_eq!(subjects(&folder(&s, "INBOX")), subjects(&inbox), "still where it was");
    assert_eq!(folder(&s, SHORT_FILM).messages.len(), film.messages.len() + 1, "and copied");
    assert!(s.list.iter().any(|m| m.subject == "Character turnarounds"), "shown where it still is");
}

/// The same into a folder whose name isn't ASCII: named as it reads, not as its path is kept
/// (`Caf&AOk-`).
#[test]
fn a_local_move_that_cant_take_the_mail_out_names_the_folder_as_it_reads() {
    let mut s = failing_session();
    s.run("folder.create", &json!({"name": "Café"})).unwrap();
    let to = mailcraft_model::utf7::encode("Café");
    assert_ne!(to, "Café");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    store(&s).fail(None, Some(&list_of("INBOX")));
    let r = s.run("mail.move", &json!({"folder": to}));
    store(&s).fail(None, None);
    assert!(matches!(&r, Err(CmdError::Failed(e)) if e.contains("copied to Café,") && !e.contains(&to)), "{r:?}");
}

/// Mail moved to a server whose copy is there, but that can't be deleted here, is said to be in
/// both places.
#[test]
fn mail_moved_to_a_server_that_cant_be_deleted_here_says_so() {
    let mut s = failing_session();
    let server = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let trash =
        mailcraft_model::Folder { account: server.clone(), path: "Trash".into(), role: FolderRole::Trash, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: server.clone(), folders: vec![trash] });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let inbox = folder(&s, "INBOX");
    let k = key_of(&s, "Character turnarounds");
    let key: mailcraft_model::MessageKey = s.list.iter().find(|m| m.subject == "Character turnarounds").unwrap().key.clone();
    s.run("list.select", &k).unwrap();
    s.run("mail.move", &json!({"folder": "Trash", "account": server})).unwrap();
    s.take_jobs();
    store(&s).fail(None, Some(&list_of("INBOX")));
    s.apply(Event::CopiesFiled { account: server.clone(), folder: "Trash".into(), done: vec![key], failed: vec![], moved: true });
    store(&s).fail(None, None);
    assert!(
        s.notices.iter().any(|n| n.error && n.text.starts_with("The mail was copied to Trash, but couldn't be deleted where it was")),
        "{:?}",
        s.notices
    );
    assert!(!s.notices.iter().any(|n| n.text.contains("moved to Trash")), "not said to be moved: {:?}", s.notices);
    assert_eq!(subjects(&folder(&s, "INBOX")), subjects(&inbox), "still here");
}

#[test]
fn filing_locally_that_cant_read_or_write_keeps_the_draft_and_the_message() {
    let mut s = failing_session();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "Mix notes", "body": "First take\n"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    let drafts = folder(&s, "Drafts");
    let first = s.compose.as_ref().unwrap().draft.clone().unwrap();
    assert!(drafts.find(first.uid).is_some() && all_kept(&s, &drafts));
    let sent = folder(&s, "Sent");
    s.run("compose.update", &json!({"body": "Second take\n"})).unwrap();
    for (reads, writes) in [(Some("state.json"), None), (None, Some(".eml"))] {
        store(&s).fail(reads, writes);
        let saved = s.run("compose.saveDraft", &json!({}));
        let kept = s.keep_drafts();
        let sending = s.run("compose.send", &json!({}));
        store(&s).fail(None, None);
        assert!(saved.is_err() && kept == 0 && sending.is_err(), "{reads:?} {writes:?}: {saved:?} {kept} {sending:?}");
        // The earlier draft is still there, whole, and the message is still being written.
        assert_eq!(subjects(&folder(&s, "Drafts")), subjects(&drafts), "{reads:?} {writes:?}");
        assert!(all_kept(&s, &folder(&s, "Drafts")), "{reads:?} {writes:?}");
        assert_eq!(subjects(&folder(&s, "Sent")), subjects(&sent), "{reads:?} {writes:?}: nothing half-sent");
        let c = s.compose.as_ref().unwrap();
        assert_eq!((c.draft.as_ref(), c.body.as_str()), (Some(&first), "Second take\n"), "{reads:?} {writes:?}");
    }
    // Working again, it's saved and sent.
    s.run("compose.saveDraft", &json!({})).unwrap();
    assert!(folder(&s, "Drafts").find(first.uid).is_none(), "the earlier draft goes once the new one is kept");
    s.run("compose.send", &json!({})).unwrap();
    assert!(s.compose.is_none());
    assert_eq!(folder(&s, "Sent").messages.len(), sent.messages.len() + 1);
    assert!(folder(&s, "Drafts").messages.iter().all(|m| m.subject != "Mix notes"));
}

#[test]
fn calendars_and_contacts_that_cant_be_read_arent_written_over() {
    let cache = Store::failing();
    assert!(sample::seed_if_empty(&cache, NOW).unwrap());
    let before = cache.load_account_doc::<serde_json::Value>(sample::SAMPLE_ACCOUNT, mailcraft_model::pim::PIM_DOC).unwrap();
    cache.fail(Some(mailcraft_model::pim::PIM_DOC), None);
    let mut s = Session::open(Some(cache), mailcraft_model::dates::Clock::new(NOW, 0));
    store(&s).fail(None, None);
    assert!(s.notices.iter().any(|n| n.error), "said");
    s.run("contact.create", &json!({"name": "Mira Chen", "emails": "mira@colour.example"})).unwrap();
    let after = store(&s).load_account_doc::<serde_json::Value>(sample::SAMPLE_ACCOUNT, mailcraft_model::pim::PIM_DOC).unwrap();
    assert_eq!(after, before, "what couldn't be read isn't saved over");
    // Gone, the account is forgotten: one added later under its id saves its own.
    s.run("account.remove", &json!({"account": sample::SAMPLE_ACCOUNT})).unwrap();
    assert!(s.pim_unread.is_empty());
}

#[test]
fn a_damaged_source_moves_without_it() {
    let mut s = failing_session();
    let film = folder(&s, SHORT_FILM);
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    store(&s).fail(Some("!.eml"), None);
    s.run("mail.move", &json!({"folder": SHORT_FILM})).unwrap();
    store(&s).fail(None, None);
    assert_eq!(folder(&s, SHORT_FILM).messages.len(), film.messages.len() + 1, "moved, its unreadable source left behind");
    assert!(folder(&s, "INBOX").messages.iter().all(|m| m.subject != "Character turnarounds"));
}

#[test]
fn a_destination_out_of_numbers_takes_nothing_rather_than_two_under_one() {
    let mut s = failing_session();
    let mut film = folder(&s, SHORT_FILM);
    film.uid_next = u32::MAX;
    store(&s).save_folder(sample::SAMPLE_ACCOUNT, SHORT_FILM, &film).unwrap();
    let inbox = folder(&s, "INBOX");
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    assert!(s.run("mail.move", &json!({"folder": SHORT_FILM})).is_err());
    assert_eq!(subjects(&folder(&s, "INBOX")), subjects(&inbox));
    assert_eq!(subjects(&folder(&s, SHORT_FILM)), subjects(&film));
    // Nor is a draft filed there.
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    let mut drafts = folder(&s, "Drafts");
    drafts.uid_next = u32::MAX;
    store(&s).save_folder(sample::SAMPLE_ACCOUNT, "Drafts", &drafts).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "Mix notes", "body": "First take\n"})).unwrap();
    assert!(s.run("compose.saveDraft", &json!({})).is_err());
    assert_eq!(subjects(&folder(&s, "Drafts")), subjects(&drafts));
}

#[test]
fn rules_and_snoozes_that_cant_be_read_arent_written_over() {
    let cache = Store::failing();
    assert!(sample::seed_if_empty(&cache, NOW).unwrap());
    let rule = json!({"name": "Print", "conditions": [{"if": "from", "value": "printshop"}], "actions": [{"do": "markRead"}]});
    let mut s = Session::open(Some(cache.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("rules.add", &json!({"rule": rule})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.snooze", &json!({"until": NOW + 3600})).unwrap();
    let docs = || (cache.load_doc::<serde_json::Value>("rules.json").unwrap(), cache.load_doc::<serde_json::Value>("snoozes.json").unwrap());
    drop(s);
    for name in ["rules.json", "snoozes.json"] {
        let before = docs();
        cache.fail(Some(name), None);
        let mut s = Session::open(Some(cache.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
        cache.fail(None, None);
        assert!(s.notices.iter().any(|n| n.error), "{name}: said");
        let _ = s.run("rules.add", &json!({"rule": {"name": "Other", "conditions": [{"if": "subject", "value": "x"}], "actions": [{"do": "flag"}]}}));
        let k = json!({"key": s.list[0].key.to_id()});
        let _ = s.run("list.select", &k);
        let _ = s.run("mail.snooze", &json!({"until": NOW + 7200}));
        s.clock = mailcraft_model::dates::Clock::new(NOW + 4000, 0);
        s.tick();
        let after = docs();
        if name == "rules.json" {
            assert_eq!(after.0, before.0, "the rules aren't saved over");
        } else {
            assert_eq!(after.1, before.1, "the snoozes aren't saved over");
        }
    }
}

/// The Replied and Forwarded flags owed by replies waiting in the Outbox (`pending-marks.json`),
/// when they can't be read at start, aren't saved over by the next reply queued.
#[test]
fn owed_flags_that_cant_be_read_arent_written_over() {
    let cache = Store::failing();
    let mut s = Session::open(Some(cache.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let inbox =
        mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    let key = mailcraft_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid: 1 };
    let hello = mailcraft_model::Summary { key: key.clone(), subject: "Hello".into(), date: NOW - 60, ..Default::default() };
    cache.save_folder(&id, "INBOX", &FolderState { validity: 9, uid_next: 2, messages: vec![hello.into()], ..Default::default() }).unwrap();
    // A reply the network layer couldn't send: in the Outbox, its Replied flag owed (the
    // account's folders as its sync finds them).
    let queue_reply = |s: &mut Session| {
        s.apply(Event::Folders { account: id.clone(), folders: vec![inbox.clone()] });
        s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
        s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
        s.run("list.open", &json!({"key": key.to_id()})).unwrap();
        s.apply(Event::Body { key: key.clone(), raw: b"From: a@b.example\r\nSubject: Hello\r\nMessage-ID: <h@b>\r\n\r\nHi\r\n".to_vec() });
        s.run("mail.reply", &json!({})).unwrap();
        s.run("compose.send", &json!({})).unwrap();
        let Some(Job::Send { id: send_id, .. }) = s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) else { panic!("not sent") };
        s.apply(Event::Queued { account: id.clone(), id: send_id, error: "offline".into(), kind: Default::default() });
    };
    queue_reply(&mut s);
    let owed = || cache.load_doc::<Value>("pending-marks.json").unwrap();
    let before = owed();
    assert_eq!(before.as_array().map(Vec::len), Some(1), "{before}");
    drop(s);
    cache.fail(Some("pending-marks.json"), None);
    // (Later, so the reply has an id of its own.)
    let mut s = Session::open(Some(cache.clone()), mailcraft_model::dates::Clock::new(NOW + 60, 0));
    cache.fail(None, None);
    // Said, as rules and snoozed mail that couldn't be read are (it was only logged).
    assert!(s.notices.iter().any(|n| n.error && n.text.contains("Outbox")), "{:?}", s.notices);
    s.take_jobs();
    queue_reply(&mut s);
    assert_eq!(owed(), before, "the owed flags aren't saved over");
}

/// An Outbox message the server refused for good says it isn't tried again; one that failed for
/// now says it isn't sent yet (both read "Not sent yet").
#[test]
fn an_outbox_message_refused_for_good_doesnt_read_as_one_to_be_tried_again() {
    let mut s = failing_session();
    let item = |id: u64, refused: bool| mailcraft_store::OutboxItem {
        id,
        from: "me@example.com".into(),
        recipients: vec!["ana@example.com".into()],
        queued: NOW + i64::try_from(id).unwrap(),
        error: Some("550 no such user here".into()),
        refused,
        ..Default::default()
    };
    let raw = |subject: &str| format!("From: me@example.com\r\nTo: ana@example.com\r\nSubject: {subject}\r\n\r\nhi\r\n").into_bytes();
    store(&s).outbox_add(sample::SAMPLE_ACCOUNT, &item(81, true), &raw("Refused")).unwrap();
    store(&s).outbox_add(sample::SAMPLE_ACCOUNT, &item(82, false), &raw("Later")).unwrap();
    s.run("folder.outbox", &json!({})).unwrap();
    let preview = |subject: &str| s.list.iter().find(|m| m.subject == subject).map(|m| m.preview.clone()).unwrap();
    assert!(
        preview("Refused").starts_with("Not sent, and not tried again") && preview("Refused").ends_with("550 no such user here"),
        "{}",
        preview("Refused")
    );
    assert_eq!(preview("Later"), "Not sent yet: 550 no such user here");
}

/// An Outbox message whose files can't be taken out stays, and that's said: deleted, it went all
/// the same; taken back to be changed, it went twice (as it was, and as changed).
#[test]
fn an_outbox_message_that_cant_be_taken_out_says_so() {
    let mut s = failing_session();
    let item = mailcraft_store::OutboxItem {
        id: 71,
        from: "me@example.com".into(),
        recipients: vec!["ana@example.com".into()],
        queued: NOW,
        ..Default::default()
    };
    let raw = b"From: me@example.com\r\nTo: ana@example.com\r\nSubject: Waiting\r\n\r\nhi\r\n";
    store(&s).outbox_add(sample::SAMPLE_ACCOUNT, &item, raw).unwrap();
    s.run("folder.outbox", &json!({})).unwrap();
    let key = s.list[0].key.clone();
    store(&s).fail(None, Some("outbox/71.json|outbox/71.eml|outbox/71.gone"));
    let r = s.run("outbox.edit", &json!({"key": key.to_id()}));
    assert!(matches!(r, Err(CmdError::Failed(_))), "{r:?}");
    assert!(s.compose.is_none(), "not opened to be sent again");
    s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    let r = s.run("mail.delete", &json!({}));
    assert!(matches!(r, Err(CmdError::Failed(_))), "{r:?}");
    assert_eq!(store(&s).outbox(sample::SAMPLE_ACCOUNT).len(), 1);
    assert_eq!(s.outbox.len(), 1, "still shown");
    // Its files held, but a mark beside them can be written: out of the Outbox.
    store(&s).fail(None, Some("outbox/71.json|outbox/71.eml"));
    s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    store(&s).fail(None, None);
    assert!(store(&s).outbox(sample::SAMPLE_ACCOUNT).is_empty() && s.outbox.is_empty());
}
