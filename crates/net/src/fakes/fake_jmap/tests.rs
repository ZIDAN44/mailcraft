use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Event, Flag, Job, Op, SyncStatus};
use mailcraft_store::Store;

use super::*;

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

fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < end, "never: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A JMAP folder kept no more than the server's `maxObjectsInGet` (100 here) whatever it was
/// to keep, so Show more showed nothing more: the newest are asked a page at a time.
#[test]
fn a_jmap_folder_keeps_more_than_one_get() {
    let mut d = Data::default();
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    for n in 1..=250 {
        d.deliver(
            &inbox,
            &mail(&format!("Message {n}"), &format!("p{n}")),
            &["$seen"],
            &format!("2024-05-{:02}T{:02}:{:02}:00Z", 1 + n / 60, n % 24, n % 60),
        );
    }
    let server = serve(d);
    let dir = std::env::temp_dir().join(format!("mailcraft-jmap-pages-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let c = crate::jmap::Client::connect(&server.base, USER, PASSWORD, false).unwrap();
    let mut st = crate::jmap::State::default();
    crate::jmap::list_folders(&c, &mut st, "a1").unwrap();
    let opts = |window| crate::sync::SyncOptions { window, prefetch: 0, ..Default::default() };
    let r = crate::jmap::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts(1000)).unwrap();
    assert_eq!((store.load_folder("a1", "INBOX").unwrap().messages.len(), r.total), (250, 250));
    crate::jmap::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts(150)).unwrap();
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 150, "a window of 150 keeps 150");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Gap #7: searching a JMAP server (`Email/query` with a filter) for mail past the window.
#[test]
fn a_jmap_server_is_searched_past_the_window() {
    let mut d = Data::default();
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    let mut ids = Vec::new();
    for n in 1..=25 {
        ids.push(
            d.deliver(
                &inbox,
                &format!(
                    "From: Zoe Park <zoe@x.example>\r\nTo: me@x.example\r\nSubject: Message {n}\r\nMessage-ID: <s{n}@x>\r\n\r\nBody tag{n}.\r\n"
                )
                .into_bytes(),
                if n <= 5 { &[] } else { &["$seen"] },
                &format!("2024-05-{n:02}T09:00:00Z"),
            ),
        );
    }
    let server = serve(d);
    let account = AccountConfig { id: "a1".into(), email: USER.into(), username: USER.into(), jmap: Some(server.base.clone()), ..Default::default() };
    let data = server.data.clone();
    crate::fake::search_the_server_past_the_window("jmap-search", account, PASSWORD, &|n, read| {
        let mut d = data.lock().unwrap();
        let id = ids[n as usize - 1].clone();
        if read {
            d.emails.iter_mut().filter(|e| e.id == id).for_each(|e| {
                e.keywords.insert("$seen".into());
            });
        } else {
            d.emails.retain(|e| e.id != id);
        }
    });
}

fn mail(subject: &str, id: &str) -> Vec<u8> {
    format!("From: Zoe Park <zoe@studio.example>\r\nTo: {USER}\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nHello about {subject}.\r\n")
        .into_bytes()
}

/// A rename, then a change naming the folder by its new name, with the folder listing in
/// between failing once (a busy server): the change waits for a fresh listing and is made,
/// not dropped as refused for want of the folder's id. (Seen once in CI: "never: deleted".)
#[test]
fn a_change_after_a_rename_outlives_a_failed_listing() {
    let mut d = Data::default();
    d.mailbox("Inbox", None, Some("inbox"));
    d.mailbox("Sent", None, Some("sent"));
    d.mailbox("Old name", None, None);
    let server = serve(d);
    let data = server.data.clone();
    let dir = std::env::temp_dir().join(format!("mailcraft-jmap-relist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig { id: "a1".into(), email: USER.into(), username: USER.into(), jmap: Some(server.base.clone()), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    // Both queued before the worker gets to them, the listing after the rename failing.
    data.lock().unwrap().fail_mailbox_get = 1;
    store.add_op("a1", Op::RenameFolder { from: "Old name".into(), to: "Clients".into() }).unwrap();
    pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteFolder { path: "Clients".into() } });
    eventually("renamed, then deleted", || {
        let d = data.lock().unwrap();
        d.by_name("Old name").is_none() && d.by_name("Clients").is_none()
    });
    assert_eq!(data.lock().unwrap().fail_mailbox_get, 0, "the listing did fail once");
    let mut failed = Vec::new();
    while let Some(e) = pool.wait(Duration::from_millis(200)) {
        if let Event::OpFailed { op, error, .. } = e {
            failed.push(format!("{op:?}: {error}"));
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_jmap_account_syncs_changes_and_sends() {
    let mut d = Data::default();
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    d.mailbox("Sent", None, Some("sent"));
    d.mailbox("Trash", None, Some("trash"));
    let projects = d.mailbox("Projects", None, None);
    let first = d.deliver(&inbox, &mail("Storyboard", "m1"), &["$seen"], "2024-05-06T09:00:00Z");
    let second = d.deliver(&inbox, &mail("Sound mix", "m2"), &[], "2024-05-06T10:00:00Z");
    d.deliver(&projects, &mail("Plan", "m3"), &["$seen", "$label1"], "2024-05-01T10:00:00Z");
    let server = serve(d);
    let data = server.data.clone();

    let dir = std::env::temp_dir().join(format!("mailcraft-jmap-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        name: "Me".into(),
        email: USER.into(),
        username: USER.into(),
        jmap: Some(server.base.clone()),
        ..Default::default()
    };
    assert!(!account.is_local());

    // A wrong password is reported, not retried.
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("nope".into())) });
    until(&pool, "password prompt", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));

    pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
    let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let folders = seen
        .iter()
        .find_map(|e| match e {
            Event::Folders { folders, .. } => Some(folders.clone()),
            _ => None,
        })
        .unwrap();
    let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["INBOX", "Sent", "Trash", "Projects"]);
    assert_eq!((folders[0].total, folders[0].unread), (2, 1), "counts come with the list");
    let st = store.load_folder("a1", "INBOX").unwrap();
    let subjects: Vec<&str> = st.messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, ["Sound mix", "Storyboard"], "newest first");
    assert!(st.messages[1].flags.seen && !st.messages[0].flags.seen);
    assert_eq!(st.messages[0].from.email, "zoe@studio.example");
    assert!(st.messages[0].preview.contains("Hello about"));
    assert!(store.has_body(&st.messages[0].key), "the newest sources are downloaded");
    let plan = store.load_folder("a1", "Projects").unwrap();
    assert!(plan.messages[0].flags.has("$label1"), "keywords become categories");

    // Open a message whose source isn't cached.
    let key = st.messages[0].key.clone();
    store.remove_body(&key);
    pool.submit(Job::FetchBody { key: key.clone() });
    until(&pool, "body", |e| matches!(e, Event::Body { key: k, raw } if *k == key && raw.starts_with(b"From:")));

    // Mark read, then move: the cache at once, the server right after.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
    eventually("the server has the flag", || data.lock().unwrap().email(&second).unwrap().keywords.contains("$seen"));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::Move { keys: vec![key.clone()], to: "Projects".into() } });
    eventually("moved on the server", || data.lock().unwrap().email(&second).unwrap().mailboxes == [projects.clone()].into());
    until(&pool, "the move shows in Projects", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Projects"));
    assert!(
        store.load_folder("a1", "Projects").unwrap().messages.iter().any(|m| m.subject == "Sound mix" && m.key.uid == key.uid),
        "same message, same number"
    );

    // Folders: create inside Projects, rename it, delete it.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Projects/Caf&AOk-".into() } });
    eventually("created", || data.lock().unwrap().by_name("Café").is_some_and(|m| m.parent.as_deref() == Some(projects.as_str())));
    until(&pool, "folder list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Projects/Caf&AOk-")));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Projects/Caf&AOk-".into(), to: "Clients".into() } });
    eventually("renamed", || data.lock().unwrap().by_name("Clients").is_some_and(|m| m.parent.is_none()));
    until(&pool, "renamed in the list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Clients")));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteFolder { path: "Clients".into() } });
    eventually("deleted", || data.lock().unwrap().by_name("Clients").is_none());

    // New mail arrives: the server pushes the change and the Inbox updates by itself (once
    // the worker is listening again after its last sync).
    eventually("listening for changes", || !data.lock().unwrap().listeners.is_empty());
    data.lock().unwrap().deliver(&inbox, &mail("Festival poster", "m4"), &[], "2024-05-07T08:00:00Z");
    until(&pool, "new mail, pushed", |e| {
        matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX")
            && store.load_folder("a1", "INBOX").unwrap().messages.first().is_some_and(|m| m.subject == "Festival poster")
    });

    // Mailbox settings: the space used, automatic replies, emptying Deleted Items.
    assert!(seen.iter().any(|e| matches!(e, Event::Quota { used, limit: 1_000_000, .. } if *used > 0)), "the quota comes with a full sync");
    // The addresses it sends from, as the server keeps them (issue #81).
    pool.submit(Job::FindIdentities { account: "a1".into() });
    let seen = until(&pool, "identities", |e| matches!(e, Event::IdentitiesFound { .. }));
    let Some(Event::IdentitiesFound { found, error: None, .. }) = seen.last() else { panic!("{:?}", seen.last()) };
    assert_eq!(found.iter().map(|i| (i.email.as_str(), i.name.as_str())).collect::<Vec<_>>(), [(USER, "Me")]);
    pool.submit(Job::GetVacation { account: "a1".into() });
    until(&pool, "replies read", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
    let away = mailcraft_model::job::Vacation {
        enabled: true,
        subject: "Away".into(),
        text: "Back on Monday".into(),
        from: Some("2026-10-15".into()),
        until: Some("2026-10-20".into()),
        ..Default::default()
    };
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
    until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
    assert_eq!(data.lock().unwrap().vacation["toDate"], "2026-10-20T23:59:59Z");
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: mailcraft_model::job::Vacation { text: "refuse".into(), ..away.clone() } });
    until(&pool, "a refused change", |e| matches!(e, Event::Vacation { vacation: Some(v), error: Some(_), .. } if *v == away));
    let trash = data.lock().unwrap().by_name("Trash").unwrap().id.clone();
    for i in 0..3 {
        data.lock().unwrap().deliver(&trash, &mail(&format!("Old {i}"), &format!("t{i}")), &[], "2024-01-01T00:00:00Z");
    }
    pool.submit(Job::Apply { account: "a1".into(), op: Op::EmptyFolder { path: "Trash".into() } });
    eventually("Trash emptied", || !data.lock().unwrap().emails.iter().any(|e| e.mailboxes.contains(&trash)));

    // Delete for good.
    let gone = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "Storyboard").unwrap().key.clone();
    pool.submit(Job::Apply { account: "a1".into(), op: Op::Expunge { keys: vec![gone] } });
    eventually("destroyed", || data.lock().unwrap().email(&first).is_none());

    // Send: stored in Sent, then submitted.
    let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: e2e\r\nMessage-ID: <e2e@x>\r\n\r\nhi\r\n".to_vec();
    let send = |id: u64, to: &str| Job::Send {
        account: "a1".into(),
        id,
        raw: raw.clone(),
        from: USER.into(),
        recipients: vec![to.into()],
        sent_folder: Some("Sent".into()),
    };
    pool.submit(send(7, "zoe@studio.example"));
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
    until(&pool, "the copy in Sent", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Sent"));
    {
        let d = data.lock().unwrap();
        let (email, from, rcpt) = d.submissions.last().unwrap().clone();
        assert_eq!((from.as_str(), rcpt), (USER, vec!["zoe@studio.example".to_string()]));
        assert!(d.email(&email).unwrap().keywords.contains("$seen"));
    }
    assert_eq!(store.load_folder("a1", "Sent").unwrap().messages[0].subject, "e2e");
    // Refused by the server: reported, and no copy left in Sent.
    let sent_before = data.lock().unwrap().emails.len();
    pool.submit(send(8, "nobody@studio.example"));
    until(&pool, "refused", |e| matches!(e, Event::SendFailed { id: 8, error, .. } if error.contains("no such recipient")));
    assert_eq!(data.lock().unwrap().emails.len(), sent_before);

    // A draft replaces its earlier copy.
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft one", "d1"), folder: "Sent".into(), replaces: None });
    until(&pool, "draft", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Sent"));
    let draft = store.load_folder("a1", "Sent").unwrap().messages.iter().find(|m| m.subject == "Draft one").unwrap().key.clone();
    assert!(store.load_folder("a1", "Sent").unwrap().find(draft.uid).unwrap().flags.draft);
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft two", "d2"), folder: "Sent".into(), replaces: Some(draft) });
    eventually("one draft left", || {
        let d = data.lock().unwrap();
        d.emails.iter().filter(|e| e.keywords.contains("$draft")).count() == 1
    });

    // The numbering survives a restart.
    let numbers = jmap_number(&store, &second);
    pool.submit(Job::Remove { account: "a1".into() });
    assert_eq!(numbers, Some(key.uid));
    assert!(data.lock().unwrap().calls.iter().any(|c| c == "EmailSubmission/set"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A JMAP account the server made with no identity to send from (Stalwart's new accounts):
/// one is made for the address, and the mail goes; the next sends use it.
#[test]
fn an_account_with_no_identity_gets_one_and_sends() {
    let mut d = Data { no_identity: true, ..Default::default() };
    d.mailbox("Inbox", None, Some("inbox"));
    d.mailbox("Sent", None, Some("sent"));
    let server = serve(d);
    let data = server.data.clone();
    let dir = std::env::temp_dir().join(format!("mailcraft-jmap-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        name: "Me".into(),
        email: USER.into(),
        username: USER.into(),
        jmap: Some(server.base.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: first\r\nMessage-ID: <id1@x>\r\n\r\nhi\r\n".to_vec();
    for id in [1, 2] {
        pool.submit(Job::Send {
            account: "a1".into(),
            id,
            raw: raw.clone(),
            from: USER.into(),
            recipients: vec!["zoe@studio.example".into()],
            sent_folder: Some("Sent".into()),
        });
        until(&pool, "sent", |e| matches!(e, Event::Sent { id: i, .. } if *i == id));
    }
    let d = data.lock().unwrap();
    assert_eq!(d.made_identities, [USER.to_string()], "made once, then used");
    assert_eq!(d.submissions.len(), 2);
    drop(d);
    drop(pool);
    let _ = std::fs::remove_dir_all(dir);
}

/// For trying the browser app by hand: a sample JMAP server at http://127.0.0.1:8775 (or
/// `MAILCRAFT_FAKE_JMAP_PORT`) for ten minutes, signing in as `me@studio.example` / `pw`.
/// `cargo test -p mailcraft-net serve_for_the_browser -- --ignored --nocapture`
#[test]
#[ignore]
fn serve_for_the_browser() {
    let mut d = Data::default();
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    d.mailbox("Drafts", None, Some("drafts"));
    d.mailbox("Sent", None, Some("sent"));
    d.mailbox("Trash", None, Some("trash"));
    let projects = d.mailbox("Projects", None, None);
    d.deliver(&inbox, &mail("Storyboard", "m1"), &["$seen"], "2026-10-14T09:00:00Z");
    d.deliver(&inbox, &mail("Sound mix", "m2"), &[], "2026-10-15T06:00:00Z");
    d.deliver(&projects, &mail("Plan", "m3"), &["$seen"], "2026-10-01T10:00:00Z");
    // `MAILCRAFT_FAKE_COUNT`: that many older ones in Projects ("Message 1" the oldest, each
    // body `needle-<n>`), for more mail than the page keeps (gap #7).
    let count: u32 = std::env::var("MAILCRAFT_FAKE_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    for n in 1..=count.min(20_000) {
        let raw = format!(
            "From: Zoe Park <zoe@studio.example>\r\nTo: {USER}\r\nSubject: Message {n}\r\nMessage-ID: <old{n}@x>\r\n\r\nOld body needle-{n}.\r\n"
        );
        let at = format!("2025-01-{:02}T{:02}:{:02}:00Z", 1 + n / 1440, n / 60 % 24, n % 60);
        d.deliver(&projects, raw.as_bytes(), &["$seen"], &at);
    }
    let port = std::env::var("MAILCRAFT_FAKE_JMAP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8775);
    let server = serve_at(d, port);
    println!("JMAP at {}/.well-known/jmap ({USER} / {PASSWORD})", server.base);
    // New mail now and then, to watch push bring it in.
    for n in 1..=10 {
        std::thread::sleep(Duration::from_secs(60));
        let subject = format!("Pushed message {n}");
        server.data.lock().unwrap().deliver(&inbox, &mail(&subject, &format!("push{n}")), &[], "2026-10-15T12:00:00Z");
        println!("delivered {subject}");
    }
}

fn jmap_number(store: &Store, id: &str) -> Option<u32> {
    let mut st = crate::jmap::State::load(store, "a1");
    let known = st.id(1).is_some() || st.id(2).is_some();
    known.then(|| st.number(id))
}
