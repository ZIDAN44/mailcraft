//! Issue #19: sent mail gets its copy on the server, wherever that server keeps Sent Items, and
//! one is made where it has none.

use super::*;

/// An account on `st`, started and online, with an SMTP server that takes one message.
fn online(st: &Arc<Mutex<State>>, name: &str) -> (crate::worker::Pool, Store, std::thread::JoinHandle<String>) {
    let imap_port = serve_tcp(st);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let smtp_port = listener.local_addr().unwrap().port();
    let (_, log) = crate::smtp::tests::serve_once_on(listener);
    let store = store(name);
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    (pool, store, log)
}

/// Send one message, as the engine does when the account knows no Sent folder.
fn send(pool: &mut crate::worker::Pool) {
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 1,
        raw: b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: Copy please\r\nMessage-ID: <c@x>\r\n\r\nhi\r\n".to_vec(),
        from: "me@studio.example".into(),
        recipients: vec!["zoe@studio.example".into()],
        sent_folder: None,
    });
    until(pool, "sent", |e| matches!(e, Event::Sent { id: 1, .. }));
}

/// The copy in `folder` on the server, marked read, waited for.
fn copy_in(st: &Arc<Mutex<State>>, folder: &str) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(m) = st.lock().unwrap().mailbox(folder).and_then(|b| b.msgs.first()) {
            assert!(String::from_utf8_lossy(&m.raw).contains("Subject: Copy please"));
            assert!(m.flags.iter().any(|f| f == "\\Seen"), "{:?}", m.flags);
            return;
        }
        assert!(Instant::now() < end, "no copy in {folder}: {:?}", st.lock().unwrap().boxes.iter().map(|b| b.name.clone()).collect::<Vec<_>>());
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A server with no Sent folder (GreenMail, a new account on many hosts): one is made, the copy
/// goes in it, and the account knows it as Sent Items. It was kept nowhere, and nothing said so.
#[test]
fn a_sent_folder_is_made_where_there_is_none() {
    let st = State::new();
    st.lock().unwrap().boxes.retain(|b| b.name != "Sent");
    let (mut pool, store, log) = online(&st, "sent-made");
    send(&mut pool);
    assert!(log.join().unwrap().contains("Subject: Copy please"));
    copy_in(&st, "Sent");
    let end = Instant::now() + Duration::from_secs(10);
    while !store.load_folders("a1").unwrap_or_default().iter().any(|f| f.path == "Sent" && f.role == FolderRole::Sent) {
        assert!(Instant::now() < end, "the account never knew it as Sent Items");
        std::thread::sleep(Duration::from_millis(50));
    }
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// A server that keeps every folder in the Inbox (cPanel, Courier) refuses `Sent`: `INBOX/Sent`
/// is made instead.
#[test]
fn a_sent_folder_is_made_in_the_inbox_where_folders_must_be() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.boxes.retain(|b| b.name == "INBOX");
        s.create_under = Some("INBOX/".into());
    }
    let (mut pool, store, _log) = online(&st, "sent-in-inbox");
    send(&mut pool);
    copy_in(&st, "INBOX/Sent");
    assert!(st.lock().unwrap().mailbox("Sent").is_none());
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Sent Items by another language's name, without the server saying so (`Gesendet`): that one
/// takes the copy, and no folder is made.
#[test]
fn a_sent_folder_in_another_language_takes_the_copy() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        if let Some(b) = s.mailbox("Sent") {
            b.name = "Gesendet".into();
            b.attr = "";
        }
    }
    let (mut pool, store, _log) = online(&st, "sent-gesendet");
    send(&mut pool);
    copy_in(&st, "Gesendet");
    assert!(st.lock().unwrap().mailbox("Sent").is_none(), "nothing made");
    assert!(!st.lock().unwrap().log.iter().any(|c| c.starts_with("CREATE")));
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Listed ahead of the server's flagged Sent (Dovecot lists cPanel's `INBOX.Sent` last): another
/// program's `Sent Items`, and a `Sent` of the user's own further down. The flagged one takes the
/// copy; the copies went in the first of those, where nobody looks.
#[test]
fn the_flagged_sent_folder_takes_the_copy_wherever_it_is_listed() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for (at, name) in (1..).zip(["Clients", "Clients/Sent", "Sent Items"]) {
            let b = Mailbox { name: name.into(), attr: "", validity: 500 + at, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() };
            s.boxes.insert(at as usize, b);
        }
    }
    let (mut pool, store, _log) = online(&st, "sent-flagged");
    let roles = store.load_folders("a1").unwrap_or_default();
    let sent: Vec<&str> = roles.iter().filter(|f| f.role == FolderRole::Sent).map(|f| f.path.as_str()).collect();
    assert_eq!(sent, ["Sent"]);
    send(&mut pool);
    copy_in(&st, "Sent");
    for other in ["Clients/Sent", "Sent Items"] {
        assert!(st.lock().unwrap().mailbox(other).is_some_and(|b| b.msgs.is_empty()), "{other} got it");
    }
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Found testing drafts on GreenMail started afresh: Sent and Drafts deleted on the server after
/// they were listed (by another client, say) refused every copy and draft filed in them, a draft
/// kept on this computer was refused again at every connection. The server's `NO [TRYCREATE]`
/// (RFC 3501 §6.3.11) says the folder may be made: it's made again, and the message goes in it.
#[test]
fn a_sent_or_drafts_folder_deleted_on_the_server_is_made_again() {
    let st = State::new();
    st.lock().unwrap().boxes.push(Mailbox {
        name: "Drafts".into(),
        attr: r"\Drafts",
        validity: 600,
        uid_next: 1,
        msgs: Vec::new(),
        expunged: Vec::new(),
    });
    let (mut pool, store, _log) = online(&st, "sent-gone");
    assert!(store.load_folders("a1").unwrap_or_default().iter().any(|f| f.path == "Drafts"), "listed");
    st.lock().unwrap().boxes.retain(|b| b.name != "Sent" && b.name != "Drafts");
    send(&mut pool);
    copy_in(&st, "Sent");
    let draft = b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <again@studio.example>\r\n\r\nto finish\r\n".to_vec();
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: draft, folder: "Drafts".into(), replaces: None });
    until(&pool, "draft saved", |e| matches!(e, Event::DraftSaved { message_id, error: None, .. } if message_id == "again@studio.example"));
    assert!(st.lock().unwrap().mailbox("Drafts").is_some_and(|b| b.msgs.len() == 1), "the draft is in Drafts again");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Sent and Drafts renamed on the server since they were listed (by another client), still the
/// account's by their role: the `NO [TRYCREATE]` for the old name has the folders listed again,
/// and the copy and the draft go in the renamed folders. No stray folder of the old name is made
/// (found in review).
#[test]
fn a_sent_or_drafts_folder_renamed_on_the_server_is_found_again() {
    let st = State::new();
    st.lock().unwrap().boxes.push(Mailbox {
        name: "Drafts".into(),
        attr: r"\Drafts",
        validity: 600,
        uid_next: 1,
        msgs: Vec::new(),
        expunged: Vec::new(),
    });
    let (mut pool, store, _log) = online(&st, "sent-renamed");
    for (from, to) in [("Sent", "Sent Messages"), ("Drafts", "Brouillons")] {
        if let Some(b) = st.lock().unwrap().boxes.iter_mut().find(|b| b.name == from) {
            b.name = to.into();
        }
    }
    send(&mut pool);
    copy_in(&st, "Sent Messages");
    let draft = b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <renamed@studio.example>\r\n\r\nto finish\r\n".to_vec();
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: draft, folder: "Drafts".into(), replaces: None });
    until(&pool, "draft saved", |e| matches!(e, Event::DraftSaved { message_id, error: None, .. } if message_id == "renamed@studio.example"));
    let mut s = st.lock().unwrap();
    assert!(s.mailbox("Brouillons").is_some_and(|b| b.msgs.len() == 1), "the draft is in the renamed Drafts");
    assert!(s.mailbox("Sent").is_none() && s.mailbox("Drafts").is_none(), "{:?}", s.boxes.iter().map(|b| b.name.clone()).collect::<Vec<_>>());
    drop(s);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// A draft kept on this computer as MailCraft closed, while its save was on its way and got there
/// too: sent up again at the next connection, it replaces the copy there (the same Message-ID),
/// so Drafts has it once (found in review: twice).
#[test]
fn a_draft_saved_twice_is_there_once() {
    let st = State::new();
    st.lock().unwrap().boxes.push(Mailbox {
        name: "Drafts".into(),
        attr: r"\Drafts",
        validity: 600,
        uid_next: 1,
        msgs: Vec::new(),
        expunged: Vec::new(),
    });
    let (mut pool, store, _log) = online(&st, "draft-twice");
    let draft = b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <twice@studio.example>\r\n\r\nto finish\r\n".to_vec();
    for _ in 0..2 {
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: draft.clone(), folder: "Drafts".into(), replaces: None });
        until(&pool, "draft saved", |e| matches!(e, Event::DraftSaved { message_id, error: None, .. } if message_id == "twice@studio.example"));
    }
    assert_eq!(st.lock().unwrap().mailbox("Drafts").map(|b| b.msgs.len()), Some(1), "once");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Drafts, with nothing in it yet, on `st`.
fn with_drafts(st: &Arc<Mutex<State>>) {
    st.lock().unwrap().boxes.push(Mailbox {
        name: "Drafts".into(),
        attr: r"\Drafts",
        validity: 600,
        uid_next: 1,
        msgs: Vec::new(),
        expunged: Vec::new(),
    });
}

/// A message put in Drafts by another program.
fn put_in_drafts(st: &Arc<Mutex<State>>, raw: Vec<u8>) {
    let mut s = st.lock().unwrap();
    if let Some(b) = s.mailbox("Drafts") {
        let uid = b.uid_next;
        b.uid_next += 1;
        b.msgs.push(Msg { uid, flags: vec![r"\Draft".into()], raw, modseq: 1 });
    }
}

/// Save `raw` as a draft, and wait for it to be said saved.
fn save_draft(pool: &mut crate::worker::Pool, raw: &[u8], id: &str) {
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: raw.to_vec(), folder: "Drafts".into(), replaces: None });
    until(pool, "draft saved", |e| matches!(e, Event::DraftSaved { message_id, error: None, .. } if message_id == id));
}

/// Found in review: the copies of a draft found by its Message-ID were removed, whatever they
/// were: the draft saved again on a phone under the same Message-ID, or another message a
/// server's search matched by the words of its Message-ID. Only MailCraft's own copy, as it
/// filed it, is replaced; the others stay.
#[test]
fn only_mailcrafts_own_copy_of_a_draft_is_replaced() {
    let st = State::new();
    with_drafts(&st);
    let (mut pool, store, _log) = online(&st, "draft-own-copy");
    let own = b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <own@studio.example>\r\n\r\nto finish the plans for the winter show, and the list of works for it\r\n".to_vec();
    save_draft(&mut pool, &own, "own@studio.example");
    // Edited on a phone, which kept its Message-ID.
    let phone =
        b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <own@studio.example>\r\n\r\nto finish the plans, edited on the phone\r\n".to_vec();
    put_in_drafts(&st, phone.clone());
    // Another message that a search for the Message-ID finds, as long as MailCraft's copy.
    let head = "From: zoe@studio.example\r\nSubject: Notes\r\nMessage-ID: <other@studio.example> (was <own@studio.example>)\r\n\r\n";
    let other = format!("{head}{}", "x".repeat(own.len() - head.len())).into_bytes();
    assert_eq!(other.len(), own.len());
    put_in_drafts(&st, other.clone());
    // Kept on this computer as MailCraft closed too, and saved again at the next connection.
    save_draft(&mut pool, &own, "own@studio.example");
    let raws: Vec<Vec<u8>> = st.lock().unwrap().mailbox("Drafts").map(|b| b.msgs.iter().map(|m| m.raw.clone()).collect()).unwrap_or_default();
    assert_eq!(raws.iter().filter(|r| **r == own).count(), 1, "MailCraft's copy once");
    assert!(raws.contains(&phone), "the phone's copy stays");
    assert!(raws.contains(&other), "the other message stays");
    assert_eq!(raws.len(), 3);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Without UIDPLUS a plain `EXPUNGE` removes everything marked deleted in the folder, another
/// program's too: no copy found by its Message-ID is removed there.
#[test]
fn no_copy_is_removed_without_uidplus() {
    let st = State::new();
    st.lock().unwrap().caps = "IMAP4rev1 MOVE CONDSTORE ENABLE QUOTA";
    with_drafts(&st);
    let (mut pool, store, _log) = online(&st, "draft-no-uidplus");
    let own = b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <plain@studio.example>\r\n\r\nto finish\r\n".to_vec();
    save_draft(&mut pool, &own, "plain@studio.example");
    save_draft(&mut pool, &own, "plain@studio.example");
    assert_eq!(st.lock().unwrap().mailbox("Drafts").map(|b| b.msgs.len()), Some(2), "both kept");
    assert!(!st.lock().unwrap().log.iter().any(|c| c.contains("EXPUNGE")), "{:?}", st.lock().unwrap().log);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Found in review: the drafts kept on this computer were taken only by a connection made for
/// a full sync. The next run's first job, a save of the same draft, connected by itself and went
/// first; the older draft kept as MailCraft closed went at the next connection, and replaced it
/// on the server. The kept drafts go first on every connection.
#[test]
fn a_draft_kept_on_close_goes_before_the_next_runs_save() {
    let st = State::new();
    with_drafts(&st);
    let (mut pool, store, _log) = online(&st, "kept-before-save");
    let first = b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <k@studio.example>\r\n\r\nsaved\r\n".to_vec();
    let closed = b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <k@studio.example>\r\n\r\nkept on close\r\n".to_vec();
    let next = b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <k@studio.example>\r\n\r\nwritten the next day\r\n".to_vec();
    save_draft(&mut pool, &first, "k@studio.example");
    pool.submit(Job::KeepDraft { account: "a1".into(), raw: closed.clone(), folder: "Drafts".into(), replaces: None });
    pool.shutdown();
    drop(pool);
    let port = serve_tcp(&st);
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: next.clone(), folder: "Drafts".into(), replaces: None });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let drafts = |st: &Arc<Mutex<State>>| -> Vec<Vec<u8>> {
        st.lock().unwrap().mailbox("Drafts").map(|b| b.msgs.iter().map(|m| m.raw.clone()).collect()).unwrap_or_default()
    };
    assert_eq!(drafts(&st), std::slice::from_ref(&next), "the newest only");
    // The account connected again later: nothing older left to go.
    pool.submit(Job::WorkOffline { on: true });
    pool.submit(Job::WorkOffline { on: false });
    until(&pool, "online again", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert_eq!(drafts(&st), [next], "the newest still");
    // In turn: the first save's copy replaced by the one kept on close, then that by the next.
    let appends = st.lock().unwrap().log.iter().filter(|c| c.contains("APPEND")).count();
    assert_eq!(appends, 3, "{:?}", st.lock().unwrap().log);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}
