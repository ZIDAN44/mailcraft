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
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
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

/// The messages in `folder` on the fake server, as MailCraft wrote them ([`as_written`]).
fn in_folder(st: &Arc<Mutex<State>>, folder: &str) -> Vec<Vec<u8>> {
    st.lock().unwrap().mailbox(folder).map(|b| b.msgs.iter().map(|m| as_written(&m.raw)).collect()).unwrap_or_default()
}

/// A Trash folder (SPECIAL-USE `\Trash`), with nothing in it yet, on `st`.
fn with_trash(st: &Arc<Mutex<State>>) {
    st.lock().unwrap().boxes.push(Mailbox {
        name: "Trash".into(),
        attr: r"\Trash",
        validity: 700,
        uid_next: 1,
        msgs: Vec::new(),
        expunged: Vec::new(),
    });
}

/// A draft saved again on a phone, which kept its Message-ID and built the rest anew.
const PHONE: &[u8] =
    b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <own@studio.example>\r\n\r\nto finish the plans, edited on the phone\r\n";

/// A draft MailCraft saved, the phone's copy of it ([`PHONE`]) and another message a search for
/// its Message-ID finds put in Drafts, and the draft saved again (kept on this computer as
/// MailCraft closed, and its save got there too) on a server with `caps`: what's in Drafts and
/// in Trash after, as written, and the draft.
fn own_copy_saved_again(name: &str, caps: &'static str, stamp: Option<&'static str>) -> (Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<u8>) {
    let st = State::new();
    st.lock().unwrap().caps = caps;
    st.lock().unwrap().stamp_appends = stamp;
    with_drafts(&st);
    with_trash(&st);
    let (mut pool, store, _log) = online(&st, name);
    let own =
        b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <own@studio.example>\r\n\r\nto finish the plans for the winter show\r\n".to_vec();
    save_draft(&mut pool, &own, "own@studio.example");
    put_in_drafts(&st, PHONE.to_vec());
    put_in_drafts(
        &st,
        b"From: zoe@studio.example\r\nSubject: Notes\r\nMessage-ID: <other@studio.example> (was <own@studio.example>)\r\n\r\nx\r\n".to_vec(),
    );
    save_draft(&mut pool, &own, "own@studio.example");
    let (drafts, trash) = (in_folder(&st, "Drafts"), in_folder(&st, "Trash"));
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
    (drafts, trash, own)
}

/// Found in review: the copies of a draft found by its Message-ID were removed, whatever they
/// were: the draft saved again on a phone under the same Message-ID, or another message a
/// server's search matched by the words of its Message-ID. Only MailCraft's own copy, as it
/// filed it, is replaced; the others stay.
#[test]
fn only_mailcrafts_own_copy_of_a_draft_is_replaced() {
    let (drafts, trash, own) = own_copy_saved_again("draft-own-copy", "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA", None);
    assert_eq!(drafts.iter().filter(|r| **r == own).count(), 1, "MailCraft's copy once");
    assert!(drafts.iter().any(|r| r == PHONE), "the phone's copy stays");
    assert_eq!(drafts.len(), 3, "the other message stays");
    assert!(trash.is_empty(), "removed for good, as UIDPLUS can");
}

/// Gap 7: a server that adds a header to what it's given keeps MailCraft's copy at another size
/// than sent, and the copy was known by its size: the draft filed again was in Drafts twice.
/// It's known by the mark on it, which the server keeps.
#[test]
fn a_copy_kept_at_another_size_is_replaced() {
    let stamp = "X-Scanned: clean";
    let (drafts, _, own) = own_copy_saved_again("draft-stamped", "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA", Some(stamp));
    let stamped = [stamp.as_bytes(), b"\r\n", &own].concat();
    assert_eq!(drafts.iter().filter(|r| **r == stamped).count(), 1, "MailCraft's copy once");
    assert!(drafts.iter().any(|r| r == PHONE), "the phone's copy stays");
    assert_eq!(drafts.len(), 3, "the other message stays");
}

/// The messages in `folder` on the fake server not marked deleted (those MailCraft shows), as
/// written.
fn shown_in(st: &Arc<Mutex<State>>, folder: &str) -> Vec<Vec<u8>> {
    st.lock()
        .unwrap()
        .mailbox(folder)
        .map(|b| b.msgs.iter().filter(|m| !m.flags.iter().any(|f| f == r"\Deleted")).map(|m| as_written(&m.raw)).collect())
        .unwrap_or_default()
}

/// Another program's message in Drafts, marked deleted and not yet expunged.
const DELETED_ELSEWHERE: &[u8] = b"From: zoe@studio.example\r\nSubject: Old\r\nMessage-ID: <gone@studio.example>\r\n\r\nx\r\n";

/// Gap 7: without UIDPLUS, a plain `EXPUNGE` removes whatever else is marked deleted in the
/// folder, so MailCraft's own copy was left, and the draft was in Drafts twice. It's marked
/// deleted, which MailCraft doesn't show, for the folder's next expunge; another program's
/// message marked deleted stays until then, and nothing goes to Trash (found in review: moved
/// there, every save's older version went).
#[test]
fn without_uidplus_the_own_copy_is_marked_deleted() {
    // Each pass in a folder of its own: the last one's threads, still ending, can't write into it.
    for (pass, caps) in ["IMAP4rev1 MOVE CONDSTORE ENABLE QUOTA", "IMAP4rev1 CONDSTORE ENABLE QUOTA"].into_iter().enumerate() {
        let st = State::new();
        st.lock().unwrap().caps = caps;
        with_drafts(&st);
        with_trash(&st);
        put_in_drafts(&st, DELETED_ELSEWHERE.to_vec());
        if let Some(m) = st.lock().unwrap().mailbox("Drafts").and_then(|b| b.msgs.last_mut()) {
            m.flags.push(r"\Deleted".into());
        }
        let (mut pool, store, _log) = online(&st, &format!("draft-no-uidplus-{pass}"));
        let own = b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <own@studio.example>\r\n\r\nto finish\r\n".to_vec();
        save_draft(&mut pool, &own, "own@studio.example");
        put_in_drafts(&st, PHONE.to_vec());
        save_draft(&mut pool, &own, "own@studio.example");
        let shown = shown_in(&st, "Drafts");
        assert_eq!(shown.iter().filter(|r| **r == own).count(), 1, "{caps}: MailCraft's copy shown once");
        assert!(shown.iter().any(|r| r == PHONE), "{caps}: the phone's copy stays");
        assert!(in_folder(&st, "Drafts").iter().any(|r| r == DELETED_ELSEWHERE), "{caps}: another's deleted message stays");
        assert!(in_folder(&st, "Trash").is_empty(), "{caps}: nothing in Trash");
        let log = st.lock().unwrap().log.clone();
        assert!(!log.iter().any(|c| ["EXPUNGE", "UID EXPUNGE", "UID MOVE", "UID COPY"].iter().any(|v| c.starts_with(v))), "{caps}: {log:?}");
        pool.submit(Job::Remove { account: "a1".into() });
        let _ = std::fs::remove_dir_all(store.root());
    }
}

/// Found in review: every copy carries MailCraft's mark, so at each save the copy it replaces was
/// taken for an earlier own copy too: on a server with MOVE but no UIDPLUS it went to Trash (each
/// version saved there), and with UIDPLUS it was looked for and removed twice. A save replaces
/// its copy alone, and looks for no other.
#[test]
fn each_save_replaces_its_copy_alone() {
    // Each pass in a folder of its own: the last one's threads, still ending, can't write into it.
    for (pass, caps) in ["IMAP4rev1 MOVE CONDSTORE ENABLE QUOTA", "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA"].into_iter().enumerate() {
        let st = State::new();
        st.lock().unwrap().caps = caps;
        with_drafts(&st);
        with_trash(&st);
        let (mut pool, store, _log) = online(&st, &format!("draft-versions-{pass}"));
        let mut replaces = None;
        let mut last = Vec::new();
        for n in 1..=3 {
            last = format!("From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <v@studio.example>\r\n\r\nversion {n}\r\n").into_bytes();
            pool.submit(Job::SaveDraft { account: "a1".into(), raw: last.clone(), folder: "Drafts".into(), replaces: replaces.take() });
            until(&pool, "draft saved", |e| matches!(e, Event::DraftSaved { message_id, error: None, .. } if message_id == "v@studio.example"));
            let uid = st.lock().unwrap().mailbox("Drafts").and_then(|b| b.msgs.last().map(|m| m.uid));
            replaces = uid.map(|uid| MessageKey { account: "a1".into(), folder: "Drafts".into(), validity: 600, uid });
        }
        assert_eq!(shown_in(&st, "Drafts"), [last], "{caps}: the last version, once");
        assert!(in_folder(&st, "Trash").is_empty(), "{caps}: nothing in Trash");
        let log = st.lock().unwrap().log.clone();
        let searches = log.iter().filter(|c| c.starts_with("UID SEARCH") && c.contains("Message-ID")).count();
        assert_eq!(searches, 0, "{caps}: none looked for (the first save has none filed before it): {log:?}");
        let expunges = log.iter().filter(|c| c.starts_with("UID EXPUNGE")).count();
        assert_eq!(expunges, if caps.contains("UIDPLUS") { 2 } else { 0 }, "{caps}: each replaced copy once: {log:?}");
        pool.submit(Job::Remove { account: "a1".into() });
        let _ = std::fs::remove_dir_all(store.root());
    }
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
    let drafts = |st: &Arc<Mutex<State>>| in_folder(st, "Drafts");
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
