//! The fake EWS server's own tests: mail, sending and sign-in.

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Event, Flag, Job, Op, SyncStatus};
use mailcraft_store::Store;

use super::*;

mod directory;
mod pim;

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
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

/// Gap #7: searching Exchange through EWS (`QueryString`, and a `Restriction` for the read
/// state) for mail past the window.
#[test]
fn ews_is_searched_past_the_window() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let mut ids = Vec::new();
    for n in 1..=25 {
        ids.push(
            d.deliver(
                &inbox,
                &format!(
                    "From: Zoe Park <zoe@x.example>\r\nTo: me@x.example\r\nSubject: Message {n}\r\nMessage-ID: <s{n}@x>\r\n\r\nBody tag{n}.\r\n"
                )
                .into_bytes(),
                n > 5,
                &format!("2024-05-{n:02}T09:00:00Z"),
            ),
        );
    }
    let server = serve(d);
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    let data = server.data.clone();
    crate::fake::search_the_server_past_the_window("ews-search", account, PASSWORD, &|n, read| {
        let mut d = data.lock().unwrap();
        let id = ids[n as usize - 1].clone();
        if read {
            d.items.iter_mut().filter(|m| m.id == id).for_each(|m| m.read = true);
        } else {
            d.items.retain(|m| m.id != id);
        }
    });
}

fn mail(subject: &str, id: &str) -> Vec<u8> {
    format!("From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nHello about {subject}.\r\n")
        .into_bytes()
}

#[test]
fn an_on_premises_exchange_account_works_through_ews() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Drafts", None, Some("drafts"), "IPF.Note");
    let sent = d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
    let trash = d.folder("Deleted Items", None, Some("deleteditems"), "IPF.Note");
    d.folder("Outbox", None, Some("outbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let projects = d.folder("Projects", None, None, "IPF.Note");
    d.folder("2024", Some(&projects), None, "IPF.Note");
    let first = d.deliver(&inbox, &mail("Storyboard", "m1"), true, "2024-05-06T09:00:00Z");
    let second = d.deliver(&inbox, &mail("Sound mix", "m2"), false, "2024-05-06T10:00:00Z");
    d.deliver(&projects, &mail("Plan", "m3"), true, "2024-05-01T10:00:00Z");
    let server = serve(d);
    let data = server.data.clone();

    let dir = std::env::temp_dir().join(format!("mailcraft-ews-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        name: "Riley".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    assert!(!account.is_local() && account.is_api());

    // A wrong password means signing in again.
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("wrong".into())) });
    until(&pool, "sign in again", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));

    pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
    let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let folders = seen
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::Folders { folders, .. } => Some(folders.clone()),
            _ => None,
        })
        .unwrap();
    let mut paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["Deleted Items", "Drafts", "INBOX", "Projects", "Projects/2024", "Sent Items"], "pages followed; no Outbox, no calendar");
    let inbox_folder = folders.iter().find(|f| f.path == "INBOX").unwrap();
    assert_eq!((inbox_folder.total, inbox_folder.unread), (2, 1));
    let st = store.load_folder("a1", "INBOX").unwrap();
    let subjects: Vec<&str> = st.messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, ["Sound mix", "Storyboard"], "newest first");
    assert!(st.messages[1].flags.seen && !st.messages[0].flags.seen);
    assert_eq!(st.messages[0].from.email, "zoe@contoso.example");
    assert!(store.has_body(&st.messages[0].key), "the newest sources are downloaded");
    assert!(data.lock().unwrap().handshakes <= 2, "NTLM signs in the connection once, not every request");

    // Open a message whose source isn't cached.
    let key = st.messages[0].key.clone();
    store.remove_body(&key);
    pool.submit(Job::FetchBody { key: key.clone() });
    until(&pool, "body", |e| matches!(e, Event::Body { key: k, raw } if *k == key && raw.starts_with(b"From:")));

    // Read, flag, move (Exchange gives the moved message a new id).
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
    eventually("read on the server", || data.lock().unwrap().items.iter().any(|i| i.id == second && i.read));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Flagged, on: true } });
    eventually("flagged on the server", || data.lock().unwrap().items.iter().any(|i| i.id == second && i.flagged));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::Move { keys: vec![key.clone()], to: "Projects".into() } });
    eventually("moved on the server", || {
        data.lock().unwrap().items.iter().any(|i| i.folder == projects && String::from_utf8_lossy(&i.raw).contains("Sound mix"))
    });
    until(&pool, "the move shows in Projects", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Projects"));
    eventually("in Projects here, flagged", || {
        store.load_folder("a1", "Projects").unwrap().messages.iter().any(|m| m.subject == "Sound mix" && m.flags.flagged)
    });

    // Folders: create, rename and move to the top, delete.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Projects/Caf&AOk-".into() } });
    eventually("created", || data.lock().unwrap().by_name("Café").is_some_and(|f| f.parent == projects));
    until(&pool, "folder list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Projects/Caf&AOk-")));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Projects/Caf&AOk-".into(), to: "Clients".into() } });
    eventually("renamed", || data.lock().unwrap().by_name("Clients").is_some_and(|f| f.parent == "ROOT"));
    until(&pool, "renamed in the list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Clients")));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteFolder { path: "Clients".into() } });
    eventually("deleted", || data.lock().unwrap().by_name("Clients").is_none());

    // Automatic replies.
    pool.submit(Job::GetVacation { account: "a1".into() });
    until(&pool, "replies read", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
    let away = mailcraft_model::job::Vacation {
        enabled: true,
        subject: String::new(),
        text: "Back on Monday".into(),
        from: Some("2026-10-15".into()),
        until: Some("2026-10-20".into()),
        ..Default::default()
    };
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
    until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
    assert_eq!(data.lock().unwrap().oof.0, "Scheduled");
    // Another reply for people outside the organisation, to the user's contacts only.
    let split = mailcraft_model::job::Vacation { external: Some("Out of office".into()), outside: "known".into(), ..away.clone() };
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: split.clone() });
    until(&pool, "two replies", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == split));
    let oof = data.lock().unwrap().oof.clone();
    assert_eq!((oof.4.as_str(), oof.5.as_str()), ("Out of office", "Known"));
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
    until(&pool, "one reply again", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: mailcraft_model::job::Vacation { text: "refuse".into(), ..away.clone() } });
    until(&pool, "a refused change", |e| matches!(e, Event::Vacation { vacation: Some(v), error: Some(_), .. } if *v == away));

    // Empty Deleted Items; delete for good.
    for i in 0..3 {
        data.lock().unwrap().deliver(&trash, &mail(&format!("Old {i}"), &format!("t{i}")), true, "2024-01-01T00:00:00Z");
    }
    pool.submit(Job::Apply { account: "a1".into(), op: Op::EmptyFolder { path: "Deleted Items".into() } });
    eventually("emptied", || !data.lock().unwrap().items.iter().any(|i| i.folder == trash));
    let gone = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "Storyboard").unwrap().key.clone();
    pool.submit(Job::Apply { account: "a1".into(), op: Op::Expunge { keys: vec![gone] } });
    eventually("deleted for good", || !data.lock().unwrap().items.iter().any(|i| i.id == first));

    // Send: the Bcc goes in the MIME for Exchange; Sent Items has the copy.
    let raw = b"From: riley@contoso.example\r\nTo: zoe@contoso.example\r\nSubject: e2e\r\nMessage-ID: <e2e@x>\r\n\r\nhi\r\n".to_vec();
    let send = |id: u64, to: Vec<String>| Job::Send {
        account: "a1".into(),
        id,
        raw: raw.clone(),
        from: "riley@contoso.example".into(),
        recipients: to,
        sent_folder: Some("Sent Items".into()),
    };
    pool.submit(send(7, vec!["zoe@contoso.example".into(), "boss@contoso.example".into()]));
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
    let out = String::from_utf8(data.lock().unwrap().sent.last().unwrap().clone()).unwrap();
    assert!(out.starts_with("Bcc: boss@contoso.example\r\nFrom:"), "{out}");
    assert!(data.lock().unwrap().items.iter().any(|i| i.folder == sent));
    pool.submit(send(8, vec!["nobody@contoso.example".into()]));
    until(&pool, "refused", |e| matches!(e, Event::SendFailed { id: 8, .. }));

    // A draft replaces its earlier copy.
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft one", "d1"), folder: "Drafts".into(), replaces: None });
    until(&pool, "draft", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Drafts"));
    let draft = store.load_folder("a1", "Drafts").unwrap().messages.iter().find(|m| m.subject == "Draft one").unwrap().key.clone();
    assert!(store.load_folder("a1", "Drafts").unwrap().messages.iter().any(|m| m.flags.draft));
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft two", "d2"), folder: "Drafts".into(), replaces: Some(draft) });
    eventually("one draft left", || data.lock().unwrap().items.iter().filter(|i| i.draft).count() == 1);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(dir);
}

/// A folder flagged hidden is not offered as mail, nor is anything nested in it at any
/// depth; its siblings and their subfolders still are.
#[test]
fn exchanges_own_folders_are_left_out_of_the_list() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let projects = d.folder("Projects", None, None, "IPF.Note");
    let working = d.folder("Working Set", None, None, "IPF.Note");
    let inner = d.folder("Inner", Some(&working), None, "IPF.Note");
    d.folder("Deeper", Some(&inner), None, "IPF.Note");
    d.folder("2024", Some(&projects), None, "IPF.Note");
    d.flagged_hidden.push(working);
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let mut names: Vec<String> = c.mailboxes().unwrap().into_iter().map(|m| m.name).collect();
    names.sort();
    assert_eq!(names, ["2024", "Inbox", "Projects"]);
}

/// Folder edits queued back to back (create, rename, delete, and a move into a new
/// folder): each must see the folder ids the previous one produced, within one replay.
#[test]
fn folder_edits_queued_together_each_find_their_folder() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("Storyboard", "m1"), false, "2024-05-06T09:00:00Z");
    let server = serve(d);
    let data = server.data.clone();
    let store = crate::fake::tests_store("ews-folder-burst");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    // All submitted at once, before the first reaches the server.
    for op in [
        Op::CreateFolder { path: "Drafts 2".into() },
        Op::RenameFolder { from: "Drafts 2".into(), to: "Clients".into() },
        Op::Move { keys: vec![key], to: "Clients".into() },
        Op::CreateFolder { path: "Old".into() },
        Op::DeleteFolder { path: "Old".into() },
    ] {
        pool.submit(Job::Apply { account: "a1".into(), op });
    }
    eventually("all made on the server", || {
        let d = data.lock().unwrap();
        let clients = d.by_name("Clients").map(|f| f.id.clone());
        clients.is_some_and(|c| d.items.iter().any(|i| i.folder == c))
            && d.by_name("Drafts 2").is_none()
            && d.by_name("Old").is_none()
            && store.load_ops("a1").unwrap().is_empty()
    });
    drop(pool);
}

/// Flag a message while a slow full sync runs, then shut the pool down: the flag is in the
/// cache and ops.json straight away, and the next session delivers it to the server.
#[test]
fn a_flag_set_mid_sync_is_not_lost() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Projects", None, None, "IPF.Note");
    d.folder("Archive 2024", None, None, "IPF.Note");
    let id = d.deliver(&inbox, &mail("Storyboard", "m1"), false, "2024-05-06T09:00:00Z");
    let server = serve(d);
    let data = server.data.clone();
    let dir = std::env::temp_dir().join(format!("mailcraft-ews-changes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let account = AccountConfig {
        id: "a1".into(),
        name: "Riley".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    let start = |store: &Store| {
        let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
        pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
        pool
    };
    let mut pool = start(&store);
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();

    // Slow the server down, start a full sync, flag the message during it.
    data.lock().unwrap().hold = Some(Duration::from_secs(2));
    pool.submit(Job::SyncAll { account: "a1".into() });
    std::thread::sleep(Duration::from_millis(300));
    let op = Op::SetFlag { keys: vec![key.clone()], flag: Flag::Flagged, on: true };
    pool.submit(Job::Apply { account: "a1".into(), op: op.clone() });
    let flagged = || store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.key == key && m.flags.flagged);
    assert!(flagged(), "cache updated immediately");
    assert_eq!(store.load_ops("a1").unwrap(), [op], "ops.json written immediately");
    // Shut down mid-sync; a fresh pool must push the flag and keep showing it.
    drop(pool);
    data.lock().unwrap().hold = None;
    std::thread::sleep(Duration::from_millis(2500));
    let pool = start(&store);
    until(&pool, "online again", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let on_server = || data.lock().unwrap().items.iter().any(|i| i.id == id && i.flagged);
    let end = Instant::now() + Duration::from_secs(20);
    while !on_server() || !store.load_ops("a1").unwrap().is_empty() {
        assert!(Instant::now() < end, "the flag never arrived on the server");
        let _ = pool.wait(Duration::from_millis(100));
    }
    assert!(flagged(), "the later syncs kept the flag locally");
    drop(pool);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Mail sent and automatic replies asked for while a big mailbox's first sync is still going
/// go at once.
#[test]
fn sending_and_automatic_replies_dont_wait_for_a_long_sync() {
    let mut d = Data { hold: Some(Duration::from_secs(4)), ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
    d.folder("Projects", None, None, "IPF.Note");
    d.folder("Archive 2024", None, None, "IPF.Note");
    d.deliver(&inbox, &mail("Storyboard", "m1"), false, "2024-05-06T09:00:00Z");
    let server = serve(d);
    let data = server.data.clone();
    let dir = std::env::temp_dir().join(format!("mailcraft-ews-busy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut pool = crate::worker::Pool::new(store, Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        name: "Riley".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    // The folders are listed; their messages take 4 s a folder from here.
    until(&pool, "folders", |e| matches!(e, Event::Folders { .. }));
    let raw = b"From: riley@contoso.example\r\nTo: zoe@contoso.example\r\nSubject: Quick answer\r\nMessage-ID: <q1@x>\r\n\r\nYes.\r\n".to_vec();
    let asked = Instant::now();
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 7,
        raw,
        from: "riley@contoso.example".into(),
        recipients: vec!["zoe@contoso.example".into()],
        sent_folder: Some("Sent Items".into()),
    });
    pool.submit(Job::GetVacation { account: "a1".into() });
    let (mut sent, mut replies) = (None, None);
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end && (sent.is_none() || replies.is_none()) {
        match pool.wait(Duration::from_millis(50)) {
            Some(Event::Sent { id: 7, .. }) => sent = Some(asked.elapsed()),
            Some(Event::SendFailed { error, .. }) => panic!("send failed: {error}"),
            Some(Event::Vacation { vacation: Some(_), .. }) => replies = Some(asked.elapsed()),
            _ => {}
        }
    }
    let (sent, replies) = (sent.expect("sent"), replies.expect("replies read"));
    assert!(sent < Duration::from_secs(3), "the message went in {sent:?}, behind the sync");
    assert!(replies < Duration::from_secs(3), "the replies came in {replies:?}, behind the sync");
    assert_eq!(data.lock().unwrap().sent.len(), 1, "sent once");
    // Several at once, and the replies changed meanwhile: each goes once, in order.
    for (id, subject) in [(8u64, "Second"), (9, "Third"), (10, "Fourth")] {
        pool.submit(Job::Send {
            account: "a1".into(),
            id,
            raw: format!("From: riley@contoso.example\r\nTo: zoe@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nx\r\n")
                .into_bytes(),
            from: "riley@contoso.example".into(),
            recipients: vec!["zoe@contoso.example".into()],
            sent_folder: Some("Sent Items".into()),
        });
    }
    let away = mailcraft_model::job::Vacation { enabled: true, text: "Away".into(), ..Default::default() };
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
    let asked = Instant::now();
    let mut done: Vec<u64> = Vec::new();
    let mut set = false;
    while asked.elapsed() < Duration::from_secs(30) && (done.len() < 3 || !set) {
        match pool.wait(Duration::from_millis(50)) {
            Some(Event::Sent { id, .. }) => done.push(id),
            Some(Event::SendFailed { error, .. }) => panic!("send failed: {error}"),
            Some(Event::Vacation { vacation: Some(v), error: None, .. }) if v == away => set = true,
            Some(Event::Vacation { error: Some(e), .. }) => panic!("replies: {e}"),
            _ => {}
        }
    }
    assert_eq!(done, [8, 9, 10]);
    assert!(set && asked.elapsed() < Duration::from_secs(4), "{:?}", asked.elapsed());
    let subjects: Vec<String> = data.lock().unwrap().sent.iter().map(|r| mailcraft_mime::parse(r).map(|p| p.subject).unwrap_or_default()).collect();
    assert_eq!(subjects, ["Quick answer", "Second", "Third", "Fourth"]);
    // The sync carries on, and finishes (each folder 4 s, Sent Items read again once however
    // many went).
    let end = Instant::now() + Duration::from_secs(60);
    while !matches!(pool.wait(Duration::from_millis(100)), Some(Event::Status { status: SyncStatus::Online { .. }, .. })) {
        assert!(Instant::now() < end, "the sync never finished");
    }
    assert_eq!(data.lock().unwrap().sent.len(), 4, "not sent again");
    // Removing the account stops both connections.
    pool.submit(Job::Remove { account: "a1".into() });
    pool.submit(Job::GetVacation { account: "a1".into() });
    until(&pool, "offline answer", |e| matches!(e, Event::Vacation { error: Some(_), .. }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn basic_sign_in_when_that_is_what_the_server_offers() {
    let mut d = Data { basic: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert_eq!(c.mailboxes().unwrap().len(), 1);
    assert!(matches!(crate::ews::Client::connect(&server.url, "x", "y", "e"), Err(crate::NetError::Auth(_))));
}

/// A message whose source the server won't give doesn't stop the folder: the others
/// download, and the folder syncs.
#[test]
fn one_unreadable_message_holds_up_nothing() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("one", "m1"), false, "2026-10-01T09:00:00Z");
    let bad = d.deliver(&inbox, &mail("two", "m2"), false, "2026-10-01T10:00:00Z");
    d.deliver(&inbox, &mail("three", "m3"), false, "2026-10-01T11:00:00Z");
    d.unreadable.push(bad);
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let store = crate::fake::tests_store("ews-unreadable");
    let mut st = crate::jmap::State::default();
    crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    crate::ews::sync_folder(&c, &mut st, &store, "a1", "INBOX", &crate::sync::SyncOptions::default()).unwrap();
    let fs = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(fs.messages.len(), 3);
    let with_body: Vec<&str> = fs.messages.iter().filter(|m| store.has_body(&m.key)).map(|m| m.subject.as_str()).collect();
    assert_eq!(with_body, ["three", "one"], "the others downloaded");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #1: a mailbox made in Dutch showed "Inbox", "Sent Items"… where Outlook shows the
/// names Exchange gave them ("Postvak IN", "Verzonden items"); they're kept now, and the
/// folders are still the account's Inbox, Sent Items… A folder of the user's own is as it was.
#[test]
fn special_folders_keep_the_names_exchange_gives_them() {
    let mut d = Data::default();
    d.folder("Postvak IN", None, Some("inbox"), "IPF.Note");
    d.folder("Verzonden items", None, Some("sentitems"), "IPF.Note");
    d.folder("Verwijderde items", None, Some("deleteditems"), "IPF.Note");
    d.folder("Afgehandeld", None, None, "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let folders = crate::ews::list_folders(&c, &mut crate::jmap::State::default(), "a1").unwrap();
    let named = |role: mailcraft_model::FolderRole| folders.iter().find(|f| f.role == role).map(|f| (f.path.clone(), f.display_name())).unwrap();
    assert_eq!(named(mailcraft_model::FolderRole::Inbox), ("INBOX".to_string(), "Postvak IN".to_string()));
    assert_eq!(named(mailcraft_model::FolderRole::Sent).1, "Verzonden items");
    assert_eq!(named(mailcraft_model::FolderRole::Trash).1, "Verwijderde items");
    let own = folders.iter().find(|f| f.path == "Afgehandeld").unwrap();
    assert_eq!((own.name.as_deref(), own.display_name().as_str()), (None, "Afgehandeld"));
}

/// For looking at the app against an Exchange server whose mailbox was made in Dutch, with
/// a shared mailbox mapped to it, on 127.0.0.1:8085 without TLS (`CONTOSO\riley`, password
/// `pw`): add the account with `"ews": "http://127.0.0.1:8085/EWS/Exchange.asmx"`.
/// Run: `cargo test -p mailcraft-net serve_ews_for_the_app -- --ignored`.
#[test]
#[ignore]
fn serve_ews_for_the_app() {
    let mut d = Data::default();
    let inbox = d.folder("Postvak IN", None, Some("inbox"), "IPF.Note");
    d.folder("Concepten", None, Some("drafts"), "IPF.Note");
    d.folder("Verzonden items", None, Some("sentitems"), "IPF.Note");
    d.folder("Verwijderde items", None, Some("deleteditems"), "IPF.Note");
    d.folder("Ongewenste e-mail", None, Some("junkemail"), "IPF.Note");
    d.folder("Postvak UIT", None, Some("outbox"), "IPF.Note");
    d.folder("Afgehandeld", None, None, "IPF.Note");
    d.deliver(&inbox, &mail("Weekly report", "o1"), false, "2026-10-19T08:00:00Z");
    d.deliver(&inbox, &mail("Re: Weekly report", "o2"), false, "2026-10-19T09:00:00Z");
    // `MAILCRAFT_FAKE_COUNT`: that many older ones in Afgehandeld ("Message 1" the oldest,
    // each body `needle-<n>`), for more mail than is kept (gap #7).
    let count: u32 = std::env::var("MAILCRAFT_FAKE_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let done = d.by_name("Afgehandeld").map(|f| f.id.clone()).unwrap_or_default();
    for n in 1..=count.min(20_000) {
        let raw = format!(
            "From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: Message {n}\r\nMessage-ID: <old{n}@x>\r\n\r\nOld body needle-{n}.\r\n"
        );
        let at = format!("2025-01-{:02}T{:02}:{:02}:00Z", 1 + n / 1440, n / 60 % 24, n % 60);
        d.deliver(&done, raw.as_bytes(), n % 7 != 0, &at);
    }
    let shared = d.shared_mailbox(SharedBox {
        address: "test@contoso.example".into(),
        name: "Test Grommunio".into(),
        automap: true,
        legacy: false,
        granted: true,
    });
    d.deliver(&shared, &mail("For the team", "s1"), false, "2026-10-19T10:00:00Z");
    // A Teams meeting today, its join link in the HTML body as Teams writes it, and the shared
    // mailbox's calendar with an event of its own.
    let agenda = d.folder("Agenda", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contactpersonen", None, Some("contacts"), "IPF.Contact");
    let teams = parse(
        "<t:CalendarItem><t:Subject>Weekly sync</t:Subject><t:Body BodyType=\"HTML\">&lt;html&gt;&lt;body&gt;&lt;p&gt;Agenda: the release.&lt;/p&gt;             &lt;div&gt;________________________________________________________________________________&lt;/div&gt;             &lt;div&gt;&lt;span style=\"font-size:24px\"&gt;Microsoft Teams &lt;a href=\"https://aka.ms/JoinTeamsMeeting\"&gt;Need help?&lt;/a&gt;&lt;/span&gt;&lt;/div&gt;             &lt;div&gt;&lt;a href=\"https://teams.microsoft.com/l/meetup-join/19%3ameeting_NjA3%40thread.v2/0?context=%7b%22Tid%22%3a%22c1%22%7d\"&gt;             &lt;span style=\"font-size:21px\"&gt;Join the meeting now&lt;/span&gt;&lt;/a&gt;&lt;/div&gt;&lt;div&gt;Meeting ID: 366 254 181 924&lt;/div&gt;             &lt;div&gt;Passcode: Ab3cD9&lt;/div&gt;&lt;/body&gt;&lt;/html&gt;</t:Body>             <t:Start>2026-10-20T13:00:00Z</t:Start><t:End>2026-10-20T13:30:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent>             <t:Location>Microsoft Teams Meeting</t:Location><t:UID>weekly-sync-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("E1".into(), agenda.clone(), teams.kids[0].clone()));
    let shared_cal = d.id("S");
    d.folders.push(Fo {
        id: shared_cal.clone(),
        name: "Agenda".into(),
        parent: String::new(),
        class: "IPF.Appointment".into(),
        known: Some("calendar".into()),
        owner: "test@contoso.example".into(),
    });
    let team = parse(
        "<t:CalendarItem><t:Subject>Team planning</t:Subject><t:Start>2026-10-20T09:00:00Z</t:Start><t:End>2026-10-20T10:00:00Z</t:End>             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>team-plan-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("T1".into(), shared_cal, team.kids[0].clone()));
    // Issue #1: a Google Calendar invitation, its body as Exchange gives it as text (each
    // link written `label<url>`), long links and all.
    let tok = "AKUaPmZ3cDk0bWtnZ2I5NjBhYmRlZjEyMzQ1Njc4OTBhYmNkZWYxMjM0NTY3ODkwYWJjZGVm";
    let eid = "NnQ5MWVxZzdhY2JkMWU0ZXJnaGtnOHFwdmEgc3R1ZGlvQGNvbnRvc28uZXhhbXBsZQ";
    let mut invite = String::from(
        "Join with Google Meet – You have been invited by ana@studio.example to attend an event named Design review on Wednesday 21 Oct 2026 14:00 – 15:00 (Eastern European Time).\n\n\
         <https://meet.google.com/abc-defg-hij>\nJoin with Google Meet\n\nMeeting link\nmeet.google.com/abc-defg-hij<https://meet.google.com/abc-defg-hij?hs=224>\n\
         Join by phone\n(US) +1 555-0100<tel:+1-555-0100%3B618021784%23>\nPIN: 618 021 784#\n\nMore phone numbers<https://tel.meet/abc-defg-hij?pin=6180217840000&hs=1>\n\n\n\
         When\nWednesday 21 Oct 2026 14:00 – 15:00 (Eastern European Time)\nGuests\nAna <mailto:ana@studio.example>\nRiley <mailto:riley@contoso.example>\nView all guest info<https://calendar.google.com/calendar/event?action=VIEW&eid=",
    );
    invite.push_str(&format!("{eid}&tok={tok}&ctz=Europe%2FAmsterdam&hl=en&es=0>\n\nReply for riley@contoso.example\n"));
    for (label, rst) in [("Yes", 1), ("No", 2), ("Maybe", 3)] {
        invite.push_str(&format!(
            "{label}\n\n<https://calendar.google.com/calendar/event?action=RESPOND&eid={eid}&rst={rst}&tok={tok}&ctz=Europe%2FAmsterdam&hl=en&es=0>\n"
        ));
    }
    invite.push_str(&format!(
        "More options\n\n<https://calendar.google.com/calendar/event?action=VIEW&eid={eid}&tok={tok}&ctz=Europe%2FAmsterdam&hl=en&es=0>\n\n\n\n\
         Sent by Google Calendar<https://calendar.google.com/calendar/>\n\nYou are receiving this email because you are an attendee on the event.\n\n\
         Forwarding this invitation could allow any recipient to send a response to the organizer, be added to the guest list, invite others regardless of their own invitation status, or modify your RSVP. Learn more<https://support.google.com/calendar/answer/37135#forwarding>\n"
    ));
    let xml = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let google = parse(&format!(
        "<t:CalendarItem><t:Subject>Invitation: Design review @ Wed 21 Oct 2026 14:00 - 15:00 (EET)</t:Subject><t:Body BodyType=\"Text\">{}</t:Body>\
         <t:Start>2026-10-21T12:00:00Z</t:Start><t:End>2026-10-21T13:00:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent>\
         <t:Location>https://meet.google.com/abc-defg-hij</t:Location><t:UID>google-invite-1</t:UID></t:CalendarItem>",
        xml(&invite)
    ));
    d.pim.push(("G1".into(), agenda, google.kids[0].clone()));
    // Issue #9: the organisation's directory, where someone new (Dana Whitfield) starts 45
    // seconds after the server does, for a search that finds her without a restart.
    for (n, a) in [("Alice Moreno", "alice.moreno@contoso.example"), ("Alicia Keyes", "alicia@contoso.example"), ("Bob Stone", "bob@contoso.example")]
    {
        d.directory.push((n.into(), a.into()));
    }
    let server = serve_at(d, "127.0.0.1:8085");
    std::thread::sleep(std::time::Duration::from_secs(45));
    server.data.lock().unwrap().directory.push(("Dana Whitfield".into(), "dana@contoso.example".into()));
    loop {
        std::thread::park();
    }
}
