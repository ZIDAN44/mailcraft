//! Graph accounts end to end, against the in-memory server.

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Event, Flag, Job, Op, SyncStatus};

use super::*;

mod directory;

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

/// Gap #7: searching Microsoft Graph (`$search`, and `$filter` for the read state) for mail
/// past the window.
#[test]
fn graph_is_searched_past_the_window() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
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
    let account = AccountConfig { id: "a1".into(), email: "me@contoso.example".into(), graph: Some(server.base.clone()), ..Default::default() };
    let data = server.data.clone();
    crate::fake::search_the_server_past_the_window("graph-search", account, TOKEN, &|n, read| {
        let mut d = data.lock().unwrap();
        let id = ids[n as usize - 1].clone();
        if read {
            d.messages.iter_mut().filter(|m| m.id == id).for_each(|m| m.read = true);
        } else {
            d.messages.retain(|m| m.id != id);
        }
    });
    let calls = server.data.lock().unwrap().calls.clone();
    assert!(calls.iter().any(|c| c.contains("$search=%22tag3%22")), "{calls:?}");
    assert!(calls.iter().any(|c| c.contains("$filter=isRead%20eq%20false")), "{calls:?}");
}

fn mail(subject: &str, id: &str) -> Vec<u8> {
    format!("From: Zoe Park <zoe@contoso.example>\r\nTo: me@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nHello about {subject}.\r\n")
        .into_bytes()
}

/// Someone else's calendar: editable when they let this account edit it (events added, changed
/// and deleted there, not in this mailbox), read-only when they don't.
#[test]
fn other_peoples_calendars_are_written_where_allowed() {
    use mailcraft_model::ical::CalEvent;
    use mailcraft_model::pim::{Pim, StoredEvent, Tombstone, shared_calendar_id};
    let mut d = Data { others: vec![("ana@contoso.example".into(), true), ("ben@contoso.example".into(), false)], ..Default::default() };
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    let at = |h: u32| json!({"dateTime": format!("2026-10-20T{h:02}:00:00.0000000")});
    for (id, cal, subject) in [
        ("A1", "user:ana@contoso.example", "Ana's review"),
        ("A2", "user:ana@contoso.example", "Ana's lunch"),
        ("B1", "user:ben@contoso.example", "Ben's call"),
    ] {
        d.events.push(json!({"id": id, "calendar": cal, "iCalUId": id.to_lowercase(), "subject": subject, "start": at(9), "end": at(10)}));
    }
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let (ana, ben) = (shared_calendar_id("ana@contoso.example"), shared_calendar_id("ben@contoso.example"));
    let mut pim = Pim { shared: vec!["ana@contoso.example".into(), "ben@contoso.example".into()], ..Default::default() };
    let first = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    let cals = first.calendars.clone().unwrap();
    let ro = |id: &str| cals.iter().find(|x| x.id == id).map(|x| x.read_only);
    assert_eq!((ro(&ana), ro(&ben)), (Some(false), Some(true)), "Ana lets this account edit, Ben doesn't");
    assert_eq!(first.events.iter().filter(|e| e.calendar == ana).count(), 2);
    // Here: a new event in Ana's calendar, one of hers changed, the other deleted.
    let review = first.events.iter().find(|e| e.event.summary == "Ana's review").cloned().unwrap();
    let lunch = first.events.iter().find(|e| e.event.summary == "Ana's lunch").cloned().unwrap();
    let mut changed = review.clone();
    changed.event.summary = "Ana's review (moved)".into();
    changed.dirty = true;
    pim.events = vec![
        changed,
        StoredEvent {
            calendar: ana.clone(),
            event: CalEvent { uid: "new-1".into(), summary: "Poster check".into(), start: 1_792_000_000, end: 1_792_003_600, ..Default::default() },
            dirty: true,
            ..Default::default()
        },
    ];
    pim.deleted = vec![Tombstone { href: lunch.href.clone(), etag: String::new(), calendar: ana.clone() }];
    let second = crate::graph_pim::sync(&c, &pim, 1_792_000_100).unwrap();
    assert!(second.errors.is_empty(), "{:?}", second.errors);
    let d = server.data.lock().unwrap();
    let in_ana = |subject: &str| d.events.iter().any(|e| e["subject"] == subject && e["calendar"] == "user:ana@contoso.example");
    assert!(in_ana("Ana's review (moved)") && in_ana("Poster check"), "changed and added in Ana's calendar");
    assert!(!d.events.iter().any(|e| e["id"] == "A2"), "deleted from Ana's calendar");
    assert!(d.events.iter().all(|e| e["calendar"] != "CAL1"), "nothing landed in this mailbox's own calendar");
    assert!(second.pushed_events.iter().all(|e| e.calendar == ana), "they stay in Ana's calendar here");
    assert_eq!(second.deleted, [lunch.href]);
}

#[test]
fn exchange_calendars_and_contacts_sync_both_ways() {
    use mailcraft_model::ical::CalEvent;
    use mailcraft_model::pim::{LOCAL, Pim, StoredCard, StoredEvent, Tombstone};
    use mailcraft_model::vcard::{Card, Typed};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.events.push(json!({"id": "OLD", "calendar": "CAL1", "iCalUId": "old", "subject": "Gone", "start": {"dateTime": "2026-10-01T09:00:00.0000000"}, "end": {"dateTime": "2026-10-01T10:00:00.0000000"}}));
    d.events.push(json!({"id": "E1", "calendar": "CAL1", "iCalUId": "standup", "subject": "Stand-up", "changeKey": "x",
        "start": {"dateTime": "2026-10-05T09:00:00.0000000"}, "end": {"dateTime": "2026-10-05T09:15:00.0000000"},
        "recurrence": {"pattern": {"type": "weekly", "interval": 1, "daysOfWeek": ["monday", "wednesday", "friday"]}, "range": {"type": "noEnd"}}}));
    d.contacts.push(json!({"id": "C1", "displayName": "Ana Lima", "emailAddresses": [{"address": "ana@contoso.example"}]}));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    // Here: a new event and person, and a deleted event.
    let mut pim = Pim::default();
    pim.events.push(StoredEvent {
        calendar: LOCAL.into(),
        event: CalEvent { uid: "local-1".into(), summary: "Poster review".into(), start: 1_792_000_000, end: 1_792_003_600, ..Default::default() },
        dirty: true,
        ..Default::default()
    });
    pim.cards.push(StoredCard {
        book: LOCAL.into(),
        card: Card {
            uid: "p1".into(),
            name: "Ben Ito".into(),
            emails: vec![Typed { value: "ben@contoso.example".into(), kind: String::new() }],
            ..Default::default()
        },
        dirty: true,
        ..Default::default()
    });
    pim.deleted.push(Tombstone { href: "OLD".into(), etag: String::new(), ..Default::default() });
    let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    {
        let d = server.data.lock().unwrap();
        assert!(!d.events.iter().any(|e| e["id"] == "OLD"), "deleted on the server");
        assert!(d.events.iter().any(|e| e["subject"] == "Poster review" && e["calendar"] == "CAL1"), "new event in the default calendar");
        assert!(d.contacts.iter().any(|c| c["displayName"] == "Ben Ito"));
    }
    assert_eq!(sync.deleted, ["OLD"]);
    assert_eq!(sync.pushed_events[0].calendar, "CAL1");
    assert!(sync.pushed_cards[0].href.starts_with("contact:"));
    // Read back: the recurring stand-up with its rule, both people.
    let standup = sync.events.iter().find(|e| e.event.summary == "Stand-up").unwrap();
    assert_eq!(standup.event.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE,FR"));
    assert_eq!(standup.event.end - standup.event.start, 900);
    assert_eq!(sync.events.len(), 2);
    assert_eq!(sync.cards.len(), 2);
    assert_eq!(sync.calendars.as_ref().map(Vec::len), Some(1));
    assert!(server.data.lock().unwrap().calls.iter().all(|c| !c.contains("contacts/p1")), "a local uid is never sent as an id");
}

/// A Microsoft account with no mailbox (signed in with a Gmail address) said "Connecting…" for
/// good: each refusal was retried at once. It says why and waits until asked.
#[test]
fn a_microsoft_account_without_a_mailbox_says_so_and_stops() {
    let server = serve(Data { no_mailbox: true, ..Default::default() });
    let store = crate::fake::tests_store("graph-nomailbox");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig { id: "a1".into(), email: "me@gmail.example".into(), graph: Some(server.base.clone()), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(TOKEN.into())) });
    let seen = until(&pool, "the error", |e| matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. }));
    let error = seen.iter().find_map(|e| match e {
        Event::Status { status: SyncStatus::Error { error }, .. } => Some(error.clone()),
        _ => None,
    });
    assert!(error.as_deref().is_some_and(|e| e.contains("no Outlook.com or Microsoft 365 mailbox")), "{error:?}");
    std::thread::sleep(Duration::from_millis(1500));
    let tries = server.data.lock().unwrap().calls.len();
    assert!(tries <= 2, "asked once, not over and over: {tries} requests");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Two regressions together. Batched GETs lacked the ImmutableId preference, so their ids
/// didn't match the listing's and a third sync re-downloaded everything; and 429 entries
/// inside a batch were treated as absent. Here the server caps batches at 4 (then 2) and
/// refuses the first request outright.
#[test]
fn graph_batched_details_keep_stable_ids_and_retry_when_busy() {
    let mut d = Data { at_once: Some(4), throttle: 1, ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"));
    for n in 1..=30 {
        let raw = format!("From: Zoe <zoe@studio.example>\r\nTo: me@x.example\r\nSubject: Note {n}\r\nMessage-ID: <n{n}@x>\r\n\r\nHi.\r\n");
        d.deliver(&inbox, raw.as_bytes(), true, &format!("2024-05-{:02}T09:00:00Z", n % 28 + 1));
    }
    let server = serve(d);
    let store = crate::fake::tests_store("graph-batches");
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let mut st = crate::jmap::State::default();
    crate::graph::list_folders(&c, &mut st, "a1").unwrap();
    let opts = crate::sync::SyncOptions { window: 1000, prefetch: 0, ..Default::default() };
    let first = crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!((first.new, store.load_folder("a1", "INBOX").unwrap().messages.len()), (30, 30), "all 30 arrived");
    // Ten more: below the threshold, so they're fetched individually through $batch.
    {
        let mut d = server.data.lock().unwrap();
        let inbox = d.resolve("inbox").unwrap();
        for n in 31..=40 {
            let raw = format!("From: Zoe <zoe@studio.example>\r\nTo: me@x.example\r\nSubject: Note {n}\r\nMessage-ID: <n{n}@x>\r\n\r\nHi.\r\n");
            d.deliver(&inbox, raw.as_bytes(), true, "2024-06-01T09:00:00Z");
        }
        d.at_once = Some(2);
    }
    let more = crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!((more.new, store.load_folder("a1", "INBOX").unwrap().messages.len()), (10, 40));
    assert!(server.data.lock().unwrap().calls.iter().any(|c| c.starts_with("POST /$batch")), "used $batch");
    server.data.lock().unwrap().calls.clear();
    let again = crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let calls = server.data.lock().unwrap().calls.clone();
    assert_eq!((again.new, again.removed), (0, 0), "{calls:?}");
    assert!(!calls.iter().any(|c| c.contains("/me/messages/")), "no per-message GETs on a quiet sync: {calls:?}");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Graph files a sent message's copy a moment after taking it: Sent Items showed it only at the
/// next poll (minutes) or Send / Receive. It's read again soon after.
#[test]
fn a_sent_message_shows_in_sent_items_soon_after() {
    let mut d = Data { file_late: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"));
    d.folder("Sent Items", None, Some("sentitems"));
    let server = serve(d);
    let store = crate::fake::tests_store("graph-sentsoon");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig { id: "a1".into(), email: "me@contoso.example".into(), graph: Some(server.base.clone()), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(TOKEN.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 1,
        raw: mail("Poster", "s1"),
        from: "me@contoso.example".into(),
        recipients: vec!["zoe@contoso.example".into()],
        sent_folder: Some("Sent Items".into()),
    });
    until(&pool, "sent", |e| matches!(e, Event::Sent { .. }));
    eventually("the copy in Sent Items", || store.load_folder("a1", "Sent Items").is_ok_and(|f| f.messages.iter().any(|m| m.subject == "Poster")));
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn an_exchange_account_syncs_changes_and_sends_through_graph() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    d.folder("Drafts", None, Some("drafts"));
    let sent = d.folder("Sent Items", None, Some("sentitems"));
    let trash = d.folder("Deleted Items", None, Some("deleteditems"));
    d.folder("Outbox", None, Some("outbox"));
    let projects = d.folder("Projects", None, None);
    d.folder("2024", Some(&projects), None);
    d.replies = json!({"status": "disabled", "internalReplyMessage": ""});
    let first = d.deliver(&inbox, &mail("Storyboard", "m1"), true, "2024-05-06T09:00:00Z");
    let second = d.deliver(&inbox, &mail("Sound mix", "m2"), false, "2024-05-06T10:00:00Z");
    d.deliver(&projects, &mail("Plan", "m3"), true, "2024-05-01T10:00:00Z");
    let server = serve(d);
    let data = server.data.clone();

    let store = crate::fake::tests_store("graph-e2e");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        name: "Me".into(),
        email: "me@contoso.example".into(),
        username: "me@contoso.example".into(),
        graph: Some(server.base.clone()),
        ..Default::default()
    };
    assert!(!account.is_local() && account.is_api());

    // A token Microsoft refuses means signing in again.
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("expired".into())) });
    until(&pool, "sign in again", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));

    pool.submit(Job::Start { account: account.clone(), password: Some(Secret(TOKEN.into())) });
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
    assert_eq!(
        paths,
        ["Deleted Items", "Drafts", "INBOX", "Projects", "Projects/2024", "Sent Items"],
        "pages followed, children listed, the Outbox left out"
    );
    let role = |p: &str| folders.iter().find(|f| f.path == p).unwrap().role;
    assert_eq!((role("Sent Items"), role("Deleted Items")), (mailcraft_model::FolderRole::Sent, mailcraft_model::FolderRole::Trash));
    let inbox_folder = folders.iter().find(|f| f.path == "INBOX").unwrap();
    assert_eq!((inbox_folder.total, inbox_folder.unread), (2, 1));
    let st = store.load_folder("a1", "INBOX").unwrap();
    let subjects: Vec<&str> = st.messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, ["Sound mix", "Storyboard"], "newest first");
    assert!(st.messages[1].flags.seen && !st.messages[0].flags.seen);
    assert_eq!(st.messages[0].from.email, "zoe@contoso.example");
    assert!(store.has_body(&st.messages[0].key), "the newest sources are downloaded");

    // Open a message whose source isn't cached.
    let key = st.messages[0].key.clone();
    store.remove_body(&key);
    pool.submit(Job::FetchBody { key: key.clone() });
    until(&pool, "body", |e| matches!(e, Event::Body { key: k, raw } if *k == key && raw.starts_with(b"From:")));

    // Read, flag, move: the same message keeps its number.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
    eventually("read on the server", || data.lock().unwrap().message(&second).unwrap().read);
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Flagged, on: true } });
    eventually("flagged on the server", || data.lock().unwrap().message(&second).unwrap().flagged);
    pool.submit(Job::Apply { account: "a1".into(), op: Op::Move { keys: vec![key.clone()], to: "Projects".into() } });
    eventually("moved on the server", || data.lock().unwrap().message(&second).unwrap().folder == projects);
    until(&pool, "the move shows in Projects", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Projects"));
    assert!(
        store.load_folder("a1", "Projects").unwrap().messages.iter().any(|m| m.subject == "Sound mix" && m.key.uid == key.uid && m.flags.flagged)
    );

    // Folders: create inside Projects, rename and move it to the top, delete it.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Projects/Caf&AOk-".into() } });
    eventually("created", || data.lock().unwrap().by_name("Café").is_some_and(|f| f.parent == projects));
    until(&pool, "folder list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Projects/Caf&AOk-")));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Projects/Caf&AOk-".into(), to: "Clients".into() } });
    eventually("renamed", || data.lock().unwrap().by_name("Clients").is_some_and(|f| f.parent == ROOT));
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
        external: Some("Out of office".into()),
        outside: "none".into(),
    };
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
    until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
    assert_eq!(data.lock().unwrap().replies["scheduledEndDateTime"]["dateTime"], "2026-10-20T23:59:59");
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: mailcraft_model::job::Vacation { text: "refuse".into(), ..away.clone() } });
    until(&pool, "a refused change", |e| matches!(e, Event::Vacation { vacation: Some(v), error: Some(_), .. } if *v == away));

    // Empty Deleted Items; delete for good.
    for i in 0..3 {
        data.lock().unwrap().deliver(&trash, &mail(&format!("Old {i}"), &format!("t{i}")), true, "2024-01-01T00:00:00Z");
    }
    pool.submit(Job::Apply { account: "a1".into(), op: Op::EmptyFolder { path: "Deleted Items".into() } });
    eventually("emptied", || !data.lock().unwrap().messages.iter().any(|m| m.folder == trash));
    let gone = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "Storyboard").unwrap().key.clone();
    pool.submit(Job::Apply { account: "a1".into(), op: Op::Expunge { keys: vec![gone] } });
    eventually("deleted for good", || data.lock().unwrap().message(&first).is_none());

    // Send: the Bcc goes in the MIME for Exchange; Sent Items has the copy.
    let raw = b"From: me@contoso.example\r\nTo: zoe@contoso.example\r\nSubject: e2e\r\nMessage-ID: <e2e@x>\r\n\r\nhi\r\n".to_vec();
    let send = |id: u64, to: Vec<String>| Job::Send {
        account: "a1".into(),
        id,
        raw: raw.clone(),
        from: "me@contoso.example".into(),
        recipients: to,
        sent_folder: Some("Sent Items".into()),
    };
    pool.submit(send(7, vec!["zoe@contoso.example".into(), "boss@contoso.example".into()]));
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
    until(&pool, "the copy in Sent", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Sent Items"));
    let out = String::from_utf8(data.lock().unwrap().sent.last().unwrap().clone()).unwrap();
    assert!(out.starts_with("Bcc: boss@contoso.example\r\nFrom:"), "{out}");
    assert!(data.lock().unwrap().messages.iter().any(|m| m.folder == sent));
    pool.submit(send(8, vec!["nobody@contoso.example".into()]));
    until(&pool, "refused", |e| matches!(e, Event::SendFailed { id: 8, error, .. } if error.contains("no such recipient")));

    // A draft replaces its earlier copy.
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft one", "d1"), folder: "Drafts".into(), replaces: None });
    until(&pool, "draft", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Drafts"));
    let draft = store.load_folder("a1", "Drafts").unwrap().messages.iter().find(|m| m.subject == "Draft one").unwrap().key.clone();
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft two", "d2"), folder: "Drafts".into(), replaces: Some(draft) });
    eventually("one draft left", || data.lock().unwrap().messages.iter().filter(|m| m.draft).count() == 1);
    assert!(data.lock().unwrap().calls.iter().any(|c| c.starts_with("POST /$batch")), "small requests go in batches");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// A message whose source the server won't give doesn't stop the folder: the others
/// download, and the folder syncs.
#[test]
fn one_unreadable_message_holds_up_nothing() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    let mail =
        |s: &str| format!("From: Zoe <zoe@studio.example>\r\nTo: me@x.example\r\nSubject: {s}\r\nMessage-ID: <{s}@x>\r\n\r\nHello.\r\n").into_bytes();
    d.deliver(&inbox, &mail("one"), false, "2026-10-01T09:00:00Z");
    let bad = d.deliver(&inbox, &mail("two"), false, "2026-10-01T10:00:00Z");
    d.deliver(&inbox, &mail("three"), false, "2026-10-01T11:00:00Z");
    d.unreadable.push(bad);
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let store = crate::fake::tests_store("graph-unreadable");
    let mut st = crate::jmap::State::default();
    crate::graph::list_folders(&c, &mut st, "a1").unwrap();
    crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &crate::sync::SyncOptions::default()).unwrap();
    let fs = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(fs.messages.len(), 3);
    let with_body: Vec<&str> = fs.messages.iter().filter(|m| store.has_body(&m.key)).map(|m| m.subject.as_str()).collect();
    assert_eq!(with_body, ["three", "one"], "the others downloaded");
    let _ = std::fs::remove_dir_all(store.root());
}

/// An OAuth token Graph refuses (it ran out while the computer slept, say) is renewed from the
/// refresh token and the work done again; only a refused refresh asks to sign in again.
#[test]
fn a_refused_token_is_renewed_once_before_asking_to_sign_in() {
    // The token endpoint's three answers, in order: t1 at sign-in, t2 on the first renewal, and
    // a refusal of the refresh token on the second.
    let answers = [
        ("200 OK", r#"{"access_token":"t1","expires_in":3600}"#),
        ("200 OK", r#"{"access_token":"t2","expires_in":3600}"#),
        ("400 Bad Request", r#"{"error":"invalid_grant"}"#),
    ];
    let answers = answers
        .iter()
        .map(|(status, json)| {
            let mut reply = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n", json.len()).into_bytes();
            reply.extend_from_slice(json.as_bytes());
            reply
        })
        .collect();
    let (oauth_base, token_requests) = crate::http::tests::serve(answers);
    let mut p = crate::oauth::tests::local(&format!("{oauth_base}/token"));
    p.id = "test-graph-renew".into();
    crate::oauth::TEST_PROVIDERS.lock().unwrap().push(p);

    let mut d = Data { token: Some("t1".into()), ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"));
    d.deliver(&inbox, &mail("Before", "m1"), true, "2024-05-06T09:00:00Z");
    let server = serve(d);
    let store = crate::fake::tests_store("graph-renew");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@contoso.example".into(),
        graph: Some(server.base.clone()),
        oauth: Some("test-graph-renew".into()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("r1".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));

    // The server stops taking t1: the next sync gets t2 and goes on, without asking anything.
    {
        let mut d = server.data.lock().unwrap();
        d.token = Some("t2".into());
        d.deliver(&inbox, &mail("After", "m2"), false, "2024-05-06T10:00:00Z");
    }
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    let has = |s: &str| store.load_folder("a1", "INBOX").is_ok_and(|f| f.messages.iter().any(|m| m.subject == s));
    let end = Instant::now() + Duration::from_secs(20);
    while !has("After") {
        assert!(Instant::now() < end, "never synced with the renewed token");
        if let Some(Event::Status { status: SyncStatus::NeedsPassword { error }, .. }) = pool.wait(Duration::from_millis(100)) {
            panic!("asked to sign in again: {error}");
        }
    }

    // Refused again, and the refresh token is refused too: now it's signing in again.
    server.data.lock().unwrap().token = Some("t3".into());
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "sign in again", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));
    let seen = token_requests.join().unwrap();
    assert!(seen.iter().all(|r| r.contains("grant_type=refresh_token&refresh_token=r1")), "{seen:?}");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #24: a message with files comes as its text first (its files named and sized, no
/// bytes), then whole, downloaded on the account's second connection.
#[test]
fn a_message_with_files_comes_as_its_text_first() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    let big = "A".repeat(3 << 20);
    let raw = format!(
        "From: Sam <sam@contoso.example>\r\nTo: me@contoso.example\r\nSubject: Rushes\r\nMessage-ID: <r1@x>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"B\"\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\nThe notes, to read now.\r\n\
         --B\r\nContent-Type: video/quicktime; name=\"reel.mov\"\r\nContent-Disposition: attachment; filename=\"reel.mov\"\r\n\r\n{big}\r\n--B--\r\n"
    );
    d.deliver(&inbox, raw.as_bytes(), false, "2024-05-06T10:00:00Z");
    let server = serve(d);
    let store = crate::fake::tests_store("graph-text-first");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig { id: "a1".into(), email: "me@contoso.example".into(), graph: Some(server.base.clone()), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(TOKEN.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let st = store.load_folder("a1", "INBOX").unwrap();
    let m = st.messages.first().unwrap();
    assert!(m.has_attachments && !store.has_body(&m.key), "not downloaded ahead");
    let key = m.key.clone();
    pool.submit(Job::FetchBody { key: key.clone() });
    let seen = until(&pool, "the whole of it", |e| matches!(e, Event::Body { key: k, raw } if *k == key && !mailcraft_mime::is_text_first(raw)));
    let first = seen.iter().find_map(|e| match e {
        Event::Body { key: k, raw } if *k == key => Some(raw.clone()),
        _ => None,
    });
    let first = first.unwrap();
    assert!(mailcraft_mime::is_text_first(&first), "its text first");
    let p = mailcraft_mime::parse(&first).unwrap();
    assert!(p.text.contains("The notes, to read now."), "{}", p.text);
    assert_eq!(p.attachments.iter().map(|a| (a.name.as_str(), a.size)).collect::<Vec<_>>(), [("reel.mov", big.len())]);
    assert!(first.len() < 4096, "no bytes of the file: {}", first.len());
    assert!(store.has_body(&key), "the whole of it kept");
    let calls = server.data.lock().unwrap().calls.clone();
    assert!(calls.iter().any(|c| c.contains("$expand=attachments")) && calls.iter().any(|c| c.ends_with("$value")), "{calls:?}");
}
