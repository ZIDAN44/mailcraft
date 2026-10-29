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
