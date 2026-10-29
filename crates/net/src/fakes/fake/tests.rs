//! Tests of IMAP sync and the worker pool against the in-memory server.

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Event, Flag, FolderRole, Job, MessageKey, Op, Security, ServerConfig, SyncStatus};

use super::*;
use crate::worker::apply_remote;

#[test]
fn wrong_password_is_an_auth_error() {
    let st = State::new();
    let mut c = Client::from_stream(Fake::connect(&st)).unwrap();
    assert!(matches!(c.login("me", "nope"), Err(crate::NetError::Auth(_))));
}

#[test]
fn lists_folders_with_roles() {
    let st = State::new();
    let mut c = login(&st);
    let f = c.list("a1").unwrap();
    let roles: Vec<(String, FolderRole)> = f.iter().map(|f| (f.display_name(), f.role)).collect();
    assert_eq!(
        roles,
        vec![
            ("Inbox".to_string(), FolderRole::Inbox),
            ("Sent Items".to_string(), FolderRole::Sent),
            ("Archive".to_string(), FolderRole::Archive),
            ("Café".to_string(), FolderRole::Other)
        ]
    );
}

#[test]
fn full_sync_cycle() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.add("INBOX", "first", &["\\Seen"]);
        s.add("INBOX", "second", &[]);
        s.add("INBOX", "third", &[]);
    }
    let store = store("cycle");
    let opts = SyncOptions::default();
    let mut c = login(&st);

    // First sync: everything arrives, sources prefetched, previews filled.
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!((r.new, r.removed, r.total, r.unread), (3, 0, 3, 2));
    let cache = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(cache.validity, 100);
    assert_eq!(cache.uid_next, 4);
    let subjects: Vec<&str> = cache.messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, vec!["third", "second", "first"]);
    assert_eq!(cache.messages[0].preview, "Body of third.");
    assert!(store.has_body(&cache.messages[0].key));

    // Nothing changed: CONDSTORE means no flag fetch at all.
    st.lock().unwrap().log.clear();
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert!(!r.touched());
    assert!(!st.lock().unwrap().log.iter().any(|l| l.contains("FLAGS")), "{:?}", st.lock().unwrap().log);

    // New mail, a flag change and a deletion on the server.
    {
        let mut s = st.lock().unwrap();
        s.add("INBOX", "fourth", &[]);
        s.modseq += 1;
        let ms = s.modseq;
        let b = s.mailbox("INBOX").unwrap();
        let m = b.msgs.iter_mut().find(|m| m.uid == 2).unwrap();
        m.flags.push("\\Flagged".into());
        m.modseq = ms;
        b.msgs.retain(|m| m.uid != 1);
    }
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!((r.new, r.changed, r.removed), (1, 1, 1));
    let cache = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(cache.messages.len(), 3);
    assert!(cache.find(2).unwrap().flags.flagged);
    assert!(cache.find(1).is_none());

    // UIDVALIDITY changes: the cache is rebuilt from scratch.
    st.lock().unwrap().mailbox("INBOX").unwrap().validity = 101;
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert!(r.reset);
    let cache = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(cache.validity, 101);
    assert!(cache.messages.iter().all(|m| m.key.validity == 101));
    assert_eq!(cache.messages.len(), 3);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn qresync_learns_expunges_and_flags_from_select() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE QRESYNC ENABLE";
        for i in 0..5 {
            s.add("INBOX", &format!("m{i}"), &[]);
        }
    }
    let store = store("qresync");
    let opts = SyncOptions::default();
    let mut c = login(&st);
    assert!(c.qresync());
    sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    {
        let mut s = st.lock().unwrap();
        s.expunge("INBOX", 2);
        s.expunge("INBOX", 4);
        s.set_flags("INBOX", 3, &["\\Seen", "\\Flagged"]);
        s.add("INBOX", "new", &[]);
        s.log.clear();
    }
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!((r.new, r.changed, r.removed), (1, 1, 2));
    let cache = store.load_folder("a1", "INBOX").unwrap();
    let uids: Vec<u32> = cache.messages.iter().map(|m| m.key.uid).collect();
    assert_eq!(uids, vec![6, 5, 3, 1]);
    assert!(cache.find(3).unwrap().flags.flagged);
    let log = st.lock().unwrap().log.clone();
    assert!(log.iter().any(|l| l.contains("(QRESYNC (100 ")), "{log:?}");
    // No flag fetch and no UID search over the cached range: the SELECT said it all.
    assert!(!log.iter().any(|l| l.contains("UID SEARCH UID 1:5") || l.contains("CHANGEDSINCE")), "{log:?}");

    // A new UIDVALIDITY still starts over.
    st.lock().unwrap().mailbox("INBOX").unwrap().validity = 101;
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert!(r.reset);
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 4);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn without_condstore_flags_are_compared() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1";
        s.add("INBOX", "a", &[]);
    }
    let store = store("nocs");
    let mut c = login(&st);
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
    st.lock().unwrap().mailbox("INBOX").unwrap().msgs[0].flags.push("\\Seen".into());
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
    assert_eq!(r.changed, 1);
    assert!(store.load_folder("a1", "INBOX").unwrap().messages[0].flags.seen);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn first_sync_takes_a_window() {
    let st = State::new();
    for i in 0..8 {
        st.lock().unwrap().add("INBOX", &format!("m{i}"), &[]);
    }
    let store = store("window");
    let mut c = login(&st);
    let opts = SyncOptions { window: 3, prefetch: 1, ..Default::default() };
    let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!(r.new, 3);
    let cache = store.load_folder("a1", "INBOX").unwrap();
    let uids: Vec<u32> = cache.messages.iter().map(|m| m.key.uid).collect();
    assert_eq!(uids, vec![8, 7, 6]);
    assert!(store.has_body(&cache.messages[0].key));
    assert!(!store.has_body(&cache.messages[1].key));
    // Later syncs only add what's new, not the older backlog.
    st.lock().unwrap().add("INBOX", "m8", &[]);
    assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap().new, 1);
    // More kept since (Options): the older mail comes in once, up to the new number.
    let more = SyncOptions { window: 6, ..opts };
    assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &more).unwrap().new, 2);
    let uids: Vec<u32> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.key.uid).collect();
    assert_eq!(uids, vec![9, 8, 7, 6, 5, 4]);
    assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &more).unwrap().new, 0, "once");
    let all = SyncOptions { window: usize::MAX, ..opts };
    assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &all).unwrap().new, 3, "all of it");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Permanent delete on a Gmail-style server: moved into Trash and expunged there. Other
/// servers get a plain EXPUNGE.
#[test]
fn gmail_permanent_delete_moves_to_trash_then_expunges() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE X-GM-EXT-1";
        for (name, attr, validity) in [("[Gmail]/All Mail", "\\All", 500), ("[Gmail]/Trash", "\\Trash", 600)] {
            s.boxes.push(Mailbox { name: name.into(), attr, validity, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() });
        }
        for subject in ["a", "b", "c", "d"] {
            s.add("INBOX", subject, &[]);
        }
        s.add("[Gmail]/Trash", "binned", &[]);
    }
    let mut c = login(&st);
    let key = |folder: &str, validity, uid| MessageKey { account: "a1".into(), folder: folder.into(), validity, uid };
    let count = |name: &str| st.lock().unwrap().mailbox(name).map_or(0, |b| b.msgs.len());
    // No Trash path given: plain EXPUNGE, which Gmail turns into an archive.
    apply_remote(&mut c, &Op::Expunge { keys: vec![key("INBOX", 100, 1)] }).unwrap();
    assert_eq!((count("INBOX"), count("[Gmail]/All Mail")), (3, 1));
    // With the Trash path: moved, then expunged; nothing left behind.
    let trash = Some("[Gmail]/Trash");
    crate::worker::apply_remote_with(&mut c, &Op::Expunge { keys: vec![key("INBOX", 100, 2), key("INBOX", 100, 3)] }, trash).unwrap();
    assert_eq!((count("INBOX"), count("[Gmail]/All Mail"), count("[Gmail]/Trash")), (1, 1, 1));
    // Keys already in Trash: expunged in place.
    crate::worker::apply_remote_with(&mut c, &Op::Expunge { keys: vec![key("[Gmail]/Trash", 600, 1)] }, trash).unwrap();
    assert_eq!((count("[Gmail]/All Mail"), count("[Gmail]/Trash")), (1, 0));
    // Non-Gmail server: the Trash path is ignored.
    st.lock().unwrap().caps = "IMAP4rev1 UIDPLUS MOVE";
    let mut c = login(&st);
    crate::worker::apply_remote_with(&mut c, &Op::Expunge { keys: vec![key("INBOX", 100, 4)] }, trash).unwrap();
    assert_eq!((count("INBOX"), count("[Gmail]/All Mail"), count("[Gmail]/Trash")), (0, 1, 0));
}

/// A move says where its messages went (COPYUID), with MOVE and without.
#[test]
fn a_move_says_the_new_uids() {
    let st = State::new();
    for s in ["a", "b", "c"] {
        st.lock().unwrap().add("INBOX", s, &[]);
    }
    let mut c = login(&st);
    c.select("INBOX", false).unwrap();
    assert_eq!(c.uid_move_to("1,3", "Archive").unwrap(), vec![1, 2]);
    st.lock().unwrap().caps = "IMAP4rev1 UIDPLUS";
    let mut c = login(&st);
    c.select("INBOX", false).unwrap();
    assert_eq!(c.uid_move_to("2", "Archive").unwrap(), vec![3]);
    assert_eq!(st.lock().unwrap().mailbox("INBOX").map(|b| b.msgs.len()), Some(0));
}

#[test]
fn remote_changes_flags_moves_deletes_and_appends() {
    let st = State::new();
    for s in ["a", "b", "c"] {
        st.lock().unwrap().add("INBOX", s, &[]);
    }
    let mut c = login(&st);
    let key = |uid| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid };
    apply_remote(&mut c, &Op::SetFlag { keys: vec![key(1), key(2)], flag: Flag::Seen, on: true }).unwrap();
    apply_remote(&mut c, &Op::Move { keys: vec![key(2)], to: "Archive".into() }).unwrap();
    apply_remote(&mut c, &Op::Expunge { keys: vec![key(3)] }).unwrap();
    // A stale key (old UIDVALIDITY) is skipped, not applied to whatever has that UID now.
    let mut stale = key(1);
    stale.validity = 99;
    apply_remote(&mut c, &Op::Expunge { keys: vec![stale] }).unwrap();
    let uid = c.append("Sent", "\\Seen", b"Subject: sent\r\n\r\nx\r\n").unwrap();
    assert_eq!(uid, Some(1));
    let mut s = st.lock().unwrap();
    let inbox: Vec<(u32, Vec<String>)> = s.mailbox("INBOX").unwrap().msgs.iter().map(|m| (m.uid, m.flags.clone())).collect();
    assert_eq!(inbox, vec![(1, vec!["\\Seen".to_string()])]);
    let arch = s.mailbox("Archive").unwrap();
    assert_eq!(arch.msgs.len(), 1);
    assert!(arch.msgs[0].flags.contains(&"\\Seen".to_string()));
    assert_eq!(s.mailbox("Sent").unwrap().msgs[0].raw, b"Subject: sent\r\n\r\nx\r\n");
}

/// For looking at the app against a mail server that acts as Outlook.com does, on
/// 127.0.0.1:1143 without TLS (user name anything, password `pw`, OAuth token `good-token`):
/// `MAILCRAFT_FAKE_REFUSE` refuses every password with its text ("Basic authentication is
/// disabled"); `MAILCRAFT_FAKE_CLOSED` (an address) takes that address's token but won't
/// open its mailbox. Run: `cargo test -p mailcraft-net serve_imap_for_the_app -- --ignored`.
#[test]
#[ignore]
fn serve_imap_for_the_app() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.refuse_passwords = std::env::var("MAILCRAFT_FAKE_REFUSE").ok().filter(|v| !v.is_empty());
        s.not_connected.extend(std::env::var("MAILCRAFT_FAKE_CLOSED").ok().filter(|v| !v.is_empty()));
        s.add("INBOX", "Welcome", &[]);
        // `MAILCRAFT_FAKE_COUNT`: that many more in the Inbox, oldest first ("Message 1" is
        // the oldest; each body says `needle-<n>`), for more mail than is kept (issue #4).
        let count: u32 = std::env::var("MAILCRAFT_FAKE_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
        for n in 1..=count.min(20_000) {
            let day = 1 + n / 400;
            s.add_raw(
                "INBOX",
                format!(
                    "From: Ana <ana@studio.example>\r\nTo: me@studio.example\r\nSubject: Message {n}\r\nDate: Mon, {day:02} Jun 2025 10:00:00 +0000\r\nMessage-ID: <m{n}@studio.example>\r\n\r\nBody of message {n}: needle-{n}.\r\n"
                )
                .into_bytes(),
            );
        }
        // `MAILCRAFT_FAKE_EML`: messages to put in the Inbox (paths, separated by `;`).
        for path in std::env::var("MAILCRAFT_FAKE_EML").unwrap_or_default().split(';').filter(|p| !p.is_empty()) {
            if let Ok(raw) = std::fs::read(path) {
                s.add_raw("INBOX", raw);
            }
        }
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:1143").unwrap();
    for sock in listener.incoming().flatten() {
        let st = st.clone();
        std::thread::spawn(move || {
            let mut fake = Fake::connect(&st);
            let mut sock = sock;
            let mut buf = [0u8; 8192];
            loop {
                let out: Vec<u8> = fake.out.drain(..).collect();
                if !out.is_empty() && sock.write_all(&out).is_err() {
                    return;
                }
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        let _ = fake.write(&buf[..n]);
                    }
                }
            }
        });
    }
}

#[test]
fn xoauth2_sign_in() {
    for caps in ["IMAP4rev1 AUTH=XOAUTH2 SASL-IR", "IMAP4rev1 AUTH=XOAUTH2"] {
        let st = State::new();
        st.lock().unwrap().caps = caps;
        let mut c = Client::from_stream(Fake::connect(&st)).unwrap();
        c.capability().unwrap();
        c.login_xoauth2("me@studio.example", GOOD_TOKEN).unwrap();
        assert!(c.list("a1").is_ok(), "signed in with {caps}");
        let mut c = Client::from_stream(Fake::connect(&st)).unwrap();
        c.capability().unwrap();
        assert!(matches!(c.login_xoauth2("me@studio.example", "expired"), Err(crate::NetError::Auth(_))), "{caps}");
    }
}

#[test]
fn keywords_round_trip_with_the_server() {
    let st = State::new();
    st.lock().unwrap().add("INBOX", "tagged", &[]);
    let store = store("keywords");
    let mut c = login(&st);
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid: 1 };
    apply_remote(&mut c, &Op::SetKeyword { keys: vec![key.clone()], keyword: "$label2".into(), on: true }).unwrap();
    apply_remote(&mut c, &Op::SetKeyword { keys: vec![key.clone()], keyword: "$Pinned".into(), on: true }).unwrap();
    // Not an atom: never sent.
    apply_remote(&mut c, &Op::SetKeyword { keys: vec![key], keyword: "two words".into(), on: true }).unwrap();
    assert!(!st.lock().unwrap().log.iter().any(|l| l.contains("two words")));
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
    let m = store.load_folder("a1", "INBOX").unwrap().messages[0].clone();
    assert!(m.flags.has("$label2") && m.flags.pinned(), "{:?}", m.flags);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn remote_folder_changes() {
    let st = State::new();
    st.lock().unwrap().add("Archive", "kept", &[]);
    let mut c = login(&st);
    apply_remote(&mut c, &Op::CreateFolder { path: "Clients".into() }).unwrap();
    // Creating an existing folder is not an error.
    apply_remote(&mut c, &Op::CreateFolder { path: "Clients".into() }).unwrap();
    c.select("Archive", false).unwrap();
    apply_remote(&mut c, &Op::RenameFolder { from: "Archive".into(), to: "Clients/Old".into() }).unwrap();
    assert!(c.selected().is_none(), "renaming the selected folder deselects it first");
    apply_remote(&mut c, &Op::DeleteFolder { path: "Projects/Caf&AOk-".into() }).unwrap();
    // Deleting a folder that's already gone is fine; a refused rename is an error.
    apply_remote(&mut c, &Op::DeleteFolder { path: "Nowhere".into() }).unwrap();
    assert!(matches!(apply_remote(&mut c, &Op::RenameFolder { from: "Nope".into(), to: "X".into() }), Err(crate::NetError::No(_))));
    let names: Vec<String> = c.list("a1").unwrap().into_iter().map(|f| f.path).collect();
    assert_eq!(names, vec!["INBOX", "Sent", "Clients/Old", "Clients"]);
    let s = st.lock().unwrap();
    assert_eq!(s.boxes.iter().find(|b| b.name == "Clients/Old").unwrap().msgs.len(), 1, "the mail moved with the folder");
    assert!(s.log.iter().any(|l| l.starts_with("SUBSCRIBE \"Clients\"")));
}

fn until(pool: &crate::worker::Pool, what: &str, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let end = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while Instant::now() < end {
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            let hit = f(&e);
            seen.push(e);
            if hit {
                return seen;
            }
        }
    }
    panic!("timed out waiting for {what}; saw {seen:?}");
}

#[test]
fn mailbox_settings_over_imap_and_managesieve() {
    let st = State::new();
    st.lock().unwrap().mailbox("Archive").unwrap().msgs.push(Msg {
        uid: 1,
        flags: Vec::new(),
        raw: b"Subject: old

x"
        .to_vec(),
        modseq: 1,
    });
    st.lock().unwrap().mailbox("Archive").unwrap().uid_next = 2;
    let imap_port = serve_tcp(&st);
    let (sieve_port, sieve) = crate::sieve::tests::serve(crate::sieve::tests::Fake {
        scripts: Vec::new(),
        active: None,
        sieve: "fileinto vacation date relational include",
        log: Vec::new(),
    });
    let store = store("mailbox");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.sieve_port = sieve_port;
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(seen.iter().any(|e| matches!(e, Event::Quota { limit: 1_048_576, used, .. } if *used >= 1024)), "GETQUOTAROOT, in bytes");

    pool.submit(Job::GetVacation { account: "a1".into() });
    until(&pool, "replies read", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
    let away = mailcraft_model::job::Vacation { enabled: true, subject: "Away".into(), text: "Back soon".into(), ..Default::default() };
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
    until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
    assert_eq!(sieve.lock().unwrap().active.as_deref(), Some(crate::sieve::SCRIPT));

    // Emptying a folder removes what isn't downloaded too.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::EmptyFolder { path: "Archive".into() } });
    let end = Instant::now() + Duration::from_secs(10);
    while !st.lock().unwrap().mailbox("Archive").unwrap().msgs.is_empty() {
        assert!(Instant::now() < end, "never emptied");
        std::thread::sleep(Duration::from_millis(50));
    }
    pool.submit(Job::Remove { account: "a1".into() });
    // An offline account answers at once.
    pool.submit(Job::GetVacation { account: "a1".into() });
    until(&pool, "offline answer", |e| matches!(e, Event::Vacation { vacation: None, error: Some(_), .. }));
    let _ = std::fs::remove_dir_all(store.root());
}

/// Send / Receive › Work Offline: connections close; changes, sent mail and drafts wait on
/// this computer; turned off, the account connects and the draft reaches the server.
#[test]
fn working_offline_keeps_drafts_and_mail_until_back() {
    let st = State::new();
    let imap_port = serve_tcp(&st);
    let store = store("offline");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    // Working offline before it starts: it never connects.
    pool.submit(Job::WorkOffline { on: true });
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "offline", |e| matches!(e, Event::Status { status: SyncStatus::Offline, .. }));
    let draft = b"From: me@studio.example
Subject: Kept

written offline
"
    .to_vec();
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: draft, folder: "INBOX".into(), replaces: None });
    until(&pool, "draft kept", |e| matches!(e, Event::Notice { message, .. } if message.contains("kept on this computer")));
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 7,
        raw: b"Subject: x

y"
        .to_vec(),
        from: "me@studio.example".into(),
        recipients: vec!["a@b.example".into()],
        sent_folder: None,
    });
    until(&pool, "queued", |e| matches!(e, Event::Queued { id: 7, .. }));
    pool.submit(Job::SyncAll { account: "a1".into() });
    assert!(st.lock().unwrap().mailbox("INBOX").unwrap().msgs.is_empty(), "nothing reached the server");
    // Back online: the kept draft goes up first.
    pool.submit(Job::WorkOffline { on: false });
    let end = Instant::now() + Duration::from_secs(20);
    while st.lock().unwrap().mailbox("INBOX").unwrap().msgs.is_empty() {
        assert!(Instant::now() < end, "the draft never reached the server");
        std::thread::sleep(Duration::from_millis(50));
    }
    let saved = st.lock().unwrap().mailbox("INBOX").unwrap().msgs[0].raw.clone();
    assert!(String::from_utf8_lossy(&saved).contains("written offline"));
    assert!(store.outbox("a1").iter().any(|i| i.id == 7), "sent mail waits in the Outbox");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

mod hostile;
mod idle;
mod searching;
mod sent;

/// Issue #4: an account kept online only syncs its folders and opens its mail as any other,
/// but none of it reaches the disk: no data file, nothing downloaded ahead of reading, and
/// once MailCraft closes, nothing of its mail is anywhere under the data folders. The other
/// account beside it keeps its mail as before (and shows the search finds what's kept).
#[test]
fn an_online_only_account_syncs_and_reads_with_nothing_on_disk() {
    fn on_disk(dir: &std::path::Path, needle: &[u8]) -> bool {
        let Ok(rd) = std::fs::read_dir(dir) else { return false };
        rd.flatten().any(|e| {
            let p = e.path();
            if p.is_dir() { on_disk(&p, needle) } else { std::fs::read(&p).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle)) }
        })
    }
    let mail = |subject: &str, body: &str| {
        format!("From: Ana <ana@studio.example>\r\nTo: me@studio.example\r\nSubject: {subject}\r\nMessage-ID: <{subject}@studio.example>\r\n\r\n{body}\r\n").into_bytes()
    };
    let (online, kept) = (State::new(), State::new());
    for i in 1..=3 {
        online.lock().unwrap().add_raw("INBOX", mail(&format!("ONLINE-SUBJECT-{i}"), &format!("ONLINE-BODY-{i}")));
        kept.lock().unwrap().add_raw("INBOX", mail(&format!("KEPT-SUBJECT-{i}"), &format!("KEPT-BODY-{i}")));
    }
    let t = std::env::temp_dir().join(format!("mailcraft-fake-online-only-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let store = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    let account = |id: &str, port: u16, online_only: bool| AccountConfig {
        id: id.into(),
        email: format!("{id}@studio.example"),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        online_only,
        ..Default::default()
    };
    let (a1, a2) = (account("a1", serve_tcp(&online), true), account("a2", serve_tcp(&kept), false));
    store.save_accounts(&[a1.clone(), a2.clone()]).unwrap();
    // As the engine does when it opens.
    store.set_online_only("a1", true).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: a1, password: Some(Secret("pw".into())) });
    pool.submit(Job::Start { account: a2, password: Some(Secret("pw".into())) });
    // A slow machine (FreeBSD's test VM) gets time; a timeout says what was still missing, and
    // what the accounts reported meanwhile.
    let end = Instant::now() + Duration::from_secs(60);
    let synced = |a: &str| store.load_folder(a, "INBOX").is_ok_and(|f| f.messages.len() == 3);
    let downloaded = || store.load_folder("a2", "INBOX").map(|f| f.messages.iter().filter(|m| store.has_body(&m.key)).count()).unwrap_or(0);
    let mut seen = Vec::new();
    while !(synced("a1") && synced("a2") && downloaded() == 3) {
        assert!(Instant::now() < end, "never synced: a1 {}, a2 {}, a2's bodies {}/3; events: {seen:?}", synced("a1"), synced("a2"), downloaded());
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            seen.push(e);
        }
    }
    let inbox = store.load_folder("a1", "INBOX").unwrap();
    assert!(inbox.messages.iter().any(|m| m.subject == "ONLINE-SUBJECT-2"));
    assert!(inbox.messages.iter().all(|m| !store.has_body(&m.key)), "nothing downloaded ahead of reading");
    // Opening one downloads it, into memory.
    let key = inbox.messages[0].key.clone();
    pool.submit(Job::FetchBody { key: key.clone() });
    let seen = until(&pool, "the opened message", |e| matches!(e, Event::Body { key: k, .. } if *k == key));
    assert!(seen.iter().any(|e| matches!(e, Event::Body { raw, .. } if String::from_utf8_lossy(raw).contains("ONLINE-BODY"))));
    assert!(store.has_body(&key));
    assert_eq!(store.data_file("a1"), None, "no data file");

    // MailCraft closes (its threads let the data files go).
    let kept_file = store.data_file("a2").unwrap();
    drop(pool);
    drop(store);
    let end = Instant::now() + Duration::from_secs(20);
    while std::fs::read(&kept_file).is_err() {
        assert!(Instant::now() < end, "the data file stayed open");
        std::thread::sleep(Duration::from_millis(50));
    }
    for i in 1..=3 {
        assert!(!on_disk(&t, format!("ONLINE-SUBJECT-{i}").as_bytes()) && !on_disk(&t, format!("ONLINE-BODY-{i}").as_bytes()), "{i}");
        assert!(on_disk(&t, format!("KEPT-SUBJECT-{i}").as_bytes()) && on_disk(&t, format!("KEPT-BODY-{i}").as_bytes()), "{i}");
    }
    let files: Vec<String> =
        std::fs::read_dir(t.join("MailCraft Files")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(files, ["a2@studio.example.mailcraft"], "only the other account has a data file");
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn outbox_holds_mail_until_smtp_is_back() {
    let st = State::new();
    let imap_port = serve_tcp(&st);
    // An SMTP port with nothing listening yet: sending fails at once (connection refused).
    let smtp_port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let store = store("outbox");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
        ..Default::default()
    };
    // Not started yet: a send goes straight to the Outbox.
    let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: queued\r\nMessage-ID: <q@x>\r\n\r\nhi\r\n".to_vec();
    let send = |id: u64| Job::Send {
        account: "a1".into(),
        id,
        raw: raw.clone(),
        from: "me@studio.example".into(),
        recipients: vec!["zoe@studio.example".into()],
        sent_folder: Some("Sent".into()),
    };
    pool.submit(send(1));
    until(&pool, "queued offline", |e| matches!(e, Event::Queued { id: 1, .. }));
    assert_eq!(store.outbox("a1").len(), 1);

    // Online, but the SMTP server is down: the next message joins the queue.
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool.submit(send(2));
    until(&pool, "queued on failure", |e| matches!(e, Event::Queued { id: 2, .. }));
    assert_eq!(store.outbox("a1").len(), 2);
    assert!(store.outbox("a1")[1].error.as_deref().is_some_and(|e| e.contains("network")), "{:?}", store.outbox("a1"));

    // A message held back for tomorrow (Delay Delivery) isn't sent with them.
    let later = mailcraft_store::OutboxItem {
        id: 3,
        from: "me@studio.example".into(),
        recipients: vec!["zoe@studio.example".into()],
        not_before: crate::worker::now() + 86_400,
        ..Default::default()
    };
    store.outbox_add("a1", &later, &raw).unwrap();

    // The server comes back: Send now delivers both, oldest first, and files copies.
    let (_, log1) = crate::smtp::tests::serve_once_on(std::net::TcpListener::bind(("127.0.0.1", smtp_port)).unwrap());
    pool.submit(Job::FlushOutbox { account: "a1".into() });
    until(&pool, "first sent", |e| matches!(e, Event::Sent { id: 1, .. }));
    assert!(log1.join().unwrap().contains("Subject: queued"));
    // serve_once takes one message; the second waits for the next server.
    let (_, log2) = crate::smtp::tests::serve_once_on(std::net::TcpListener::bind(("127.0.0.1", smtp_port)).unwrap());
    pool.submit(Job::FlushOutbox { account: "a1".into() });
    until(&pool, "second sent", |e| matches!(e, Event::Sent { id: 2, .. }));
    assert!(log2.join().unwrap().contains("RCPT TO:<zoe@studio.example>"));
    assert_eq!(store.outbox("a1").iter().map(|i| i.id).collect::<Vec<_>>(), vec![3], "the held-back one waits");
    let end = Instant::now() + Duration::from_secs(10);
    while st.lock().unwrap().mailbox("Sent").unwrap().msgs.len() < 2 {
        assert!(Instant::now() < end, "copies never reached Sent");
        std::thread::sleep(Duration::from_millis(50));
    }
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn worker_pool_end_to_end_over_tcp() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.add("INBOX", "hello", &[]);
        s.add("INBOX", "second", &["\\Seen"]);
    }
    let imap_port = serve_tcp(&st);
    let (smtp_port, smtp_log) = crate::smtp::tests::serve_once();
    let store = store("e2e");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        name: "Me".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
        ..Default::default()
    };

    // Wrong password first: the worker reports it and stops retrying.
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("nope".into())) });
    until(&pool, "password prompt", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));

    // Right password: folders, the Inbox, then online.
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("pw".into())) });
    let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(seen.iter().any(|e| matches!(e, Event::Folders { folders, .. } if folders.len() == 4)));
    assert!(seen.iter().any(|e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX")));
    let inbox = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(inbox.messages.len(), 2);

    // Open a message (already prefetched: answered from the cache).
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid: 1 };
    pool.submit(Job::FetchBody { key: key.clone() });
    until(&pool, "body", |e| matches!(e, Event::Body { key: k, raw } if *k == key && raw.starts_with(b"From:")));

    // Mark read: cache at once, server right after.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
    until(&pool, "flag change", |e| matches!(e, Event::FolderChanged { .. }));
    assert!(store.load_folder("a1", "INBOX").unwrap().find(1).unwrap().flags.seen);
    let end = Instant::now() + Duration::from_secs(10);
    while !st.lock().unwrap().mailbox("INBOX").unwrap().msgs[0].flags.iter().any(|f| f == "\\Seen") {
        assert!(Instant::now() < end, "the server never got the flag");
        std::thread::sleep(Duration::from_millis(50));
    }

    // Send: SMTP submission, then a copy filed in Sent.
    let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: e2e\r\nMessage-ID: <e2e@x>\r\n\r\nhi\r\n".to_vec();
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 7,
        raw,
        from: "me@studio.example".into(),
        recipients: vec!["zoe@studio.example".into()],
        sent_folder: Some("Sent".into()),
    });
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
    until(&pool, "sent copy", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Sent"));
    assert_eq!(st.lock().unwrap().mailbox("Sent").unwrap().msgs.len(), 1);
    assert!(smtp_log.join().unwrap().contains("Subject: e2e"));
    assert_eq!(store.load_folder("a1", "Sent").unwrap().messages[0].subject, "e2e");

    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #3: an outgoing server whose user name and password aren't the incoming server's.
/// Without its password kept, sending says so (and nothing goes as the IMAP user); once kept,
/// SMTP signs in with its own user name and password while IMAP keeps its own.
#[test]
fn the_outgoing_server_signs_in_with_its_own_user_name_and_password() {
    let st = State::new();
    let imap_port = serve_tcp(&st);
    let (smtp_port, smtp_log) = crate::smtp::tests::serve_once();
    let store = store("smtp-own-sign-in");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
        smtp_username: Some("relay-user".into()),
        ..Default::default()
    };
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let send = |id: u64| Job::Send {
        account: "a1".into(),
        id,
        raw: b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: relay\r\n\r\nhi\r\n".to_vec(),
        from: "me@studio.example".into(),
        recipients: vec!["zoe@studio.example".into()],
        sent_folder: None,
    };
    pool.submit(send(1));
    until(&pool, "no password", |e| match e {
        Event::SendFailed { id: 1, error, .. } | Event::Queued { id: 1, error, .. } => error.contains("outgoing server's password"),
        _ => false,
    });
    pool.submit(Job::SmtpPassword { account: account.clone(), password: Secret("smtp-secret".into()) });
    pool.submit(send(2));
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 2, .. }));
    let log = smtp_log.join().unwrap();
    let own = crate::imap::base64(b"\0relay-user\0smtp-secret");
    assert!(log.contains(&own), "SMTP signed in as relay-user: {log}");
    assert!(!log.contains(&crate::imap::base64(b"\0me@studio.example\0pw")), "not as the IMAP user");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}
