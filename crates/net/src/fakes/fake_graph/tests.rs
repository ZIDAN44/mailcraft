//! Graph accounts end to end, against the in-memory server.

use std::time::{Duration, Instant};

use mailcraft_model::job::{Secret, Vacation};
use mailcraft_model::{AccountConfig, Event, Job, SyncStatus};

use super::*;
use crate::fakes::exchange_e2e::{Exchange, OnServer, an_exchange_account_end_to_end};
use crate::waiting::{eventually, until};

mod checks;
mod directory;
mod follow_up;
mod free_busy;
mod large_folder;
mod renewal;

/// HTTP answers from a token endpoint, one per connection, in order: (status, JSON); an empty
/// status, the connection closed with no answer.
fn token_answers(answers: &[(&str, &str)]) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let replies = answers
        .iter()
        .map(|(status, json)| {
            if status.is_empty() {
                return Vec::new();
            }
            let mut r = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n", json.len()).into_bytes();
            r.extend_from_slice(json.as_bytes());
            r
        })
        .collect();
    crate::http::tests::serve(replies)
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
    let account = account("a1", &server.base);
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

/// Found in review: a search of the server, and a message downloaded, wrote the cache of an
/// account removed meanwhile, and its files came back. Over Graph (and EWS, whose code it shares)
/// neither writes anything of it.
#[test]
fn a_search_or_a_download_writes_nothing_for_an_account_removed_meanwhile() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    for n in 1..=6 {
        d.deliver(&inbox, &mail(&format!("Message {n}"), &format!("r{n}")), n > 3, &format!("2024-05-{n:02}T09:00:00Z"));
    }
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let store = crate::fake::store("graph-removed");
    let mut st = crate::jmap::State::default();
    crate::graph::list_folders(&c, &mut st, "a1").unwrap();
    crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &crate::sync::SyncOptions { window: 2, prefetch: 0, ..Default::default() })
        .unwrap();
    let fs = store.load_folder("a1", "INBOX").unwrap();
    let key = fs.messages[0].key.clone();
    // As the worker started, and as a search read the folder; then removed, the server asked.
    let seen = store.removals("a1");
    store.remove_account("a1").unwrap();
    let mut came = (*fs.messages[1]).clone();
    came.key.uid = 999;
    assert!(crate::exchange::window::found(&mut st, &store, ("a1", seen), "INBOX", fs, &[999], vec![came]).is_err());
    assert!(crate::graph::fetch_body(&c, &st, &store, &key, seen).is_err());
    assert!(!store.has_body(&key) && !store.root().join("a1").exists(), "the account's files came back");
    // Not removed since: kept.
    let seen = store.removals("a1");
    crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &crate::sync::SyncOptions { window: 2, prefetch: 0, ..Default::default() })
        .unwrap();
    let unread = crate::sync::word_query("is:unread", None).unwrap();
    assert_eq!(crate::graph::find_on_server(&c, &mut st, &store, ("a1", seen), "INBOX", &unread, 500).unwrap().1, 3);
    assert!(crate::graph::fetch_body(&c, &st, &store, &key, seen).is_ok() && store.has_body(&key));
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
    let store = crate::fake::store("graph-nomailbox");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig { id: "a1".into(), email: "me@gmail.example".into(), graph: Some(server.base.clone()), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(TOKEN.into())) });
    let seen = until(&pool, "the error", |e| matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. }));
    let error = seen.iter().find_map(|e| match e {
        Event::Status { status: SyncStatus::Error { error, .. }, .. } => Some(error.clone()),
        _ => None,
    });
    assert!(error.as_deref().is_some_and(|e| e.contains("no Outlook.com or Microsoft 365 mailbox")), "{error:?}");
    // Once its thread waits, nothing more is asked of the server until something wakes it.
    eventually("the account's thread waiting", || pool.idle("a1"));
    let tries = server.data.lock().unwrap().calls.len();
    assert!(tries <= 2, "asked once, not over and over: {tries} requests");
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
    let store = crate::fake::store("graph-batches");
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
}

/// Graph files a sent message's copy a moment after taking it: Sent Items showed it only at the
/// next poll (minutes) or Send / Receive. It's read again soon after.
#[test]
fn a_sent_message_shows_in_sent_items_soon_after() {
    let mut d = Data { file_late: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"));
    d.folder("Sent Items", None, Some("sentitems"));
    let server = serve(d);
    let store = crate::fake::store("graph-sentsoon");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = account("a1", &server.base);
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
}

/// The story every Exchange account is checked by, over Graph (small requests going in batches).
#[test]
fn an_exchange_account_syncs_changes_and_sends_through_graph() {
    let d = Data { replies: json!({"status": "disabled", "internalReplyMessage": ""}), ..Default::default() };
    let server = serve(d);
    let account = AccountConfig { name: "Me".into(), username: "me@contoso.example".into(), ..account("a1", &server.base) };
    let away = Vacation {
        enabled: true,
        subject: String::new(),
        text: "Back on Monday".into(),
        from: Some("2026-10-15".into()),
        until: Some("2026-10-20".into()),
        external: Some("Out of office".into()),
        outside: "none".into(),
        ..Default::default()
    };
    // A token Microsoft refuses means signing in again.
    an_exchange_account_end_to_end("graph-e2e", account, "expired", TOKEN, &[away], &server);
}

impl Exchange for Server {
    const TOP: &'static str = ROOT;
    const MOVES_KEEP_IDS: bool = true;
    const NO_SUCH_RECIPIENT: &'static str = "no such recipient";
    fn folder(&self, name: &str, parent: Option<&str>, known: Option<&str>) -> String {
        self.data.lock().unwrap().folder(name, parent, known)
    }
    fn deliver(&self, folder: &str, raw: &[u8], read: bool, at: &str) -> String {
        self.data.lock().unwrap().deliver(folder, raw, read, at)
    }
    fn unreadable(&self, id: &str) {
        self.data.lock().unwrap().unreadable.push(id.into());
    }
    fn message(&self, id: &str) -> Option<OnServer> {
        let d = self.data.lock().unwrap();
        d.message(id).map(|m| OnServer { read: m.read, flagged: m.flagged, folder: m.folder.clone() })
    }
    fn holds(&self, folder: &str, words: &str) -> bool {
        self.data.lock().unwrap().messages.iter().any(|m| m.folder == folder && String::from_utf8_lossy(&m.raw).contains(words))
    }
    fn count(&self, folder: &str) -> usize {
        self.data.lock().unwrap().messages.iter().filter(|m| m.folder == folder).count()
    }
    fn parent(&self, name: &str) -> Option<String> {
        self.data.lock().unwrap().by_name(name).map(|f| f.parent.clone())
    }
    fn drafts(&self) -> usize {
        self.data.lock().unwrap().messages.iter().filter(|m| m.draft).count()
    }
    fn last_sent(&self) -> Option<Vec<u8>> {
        self.data.lock().unwrap().sent.last().cloned()
    }
    fn replies_are(&self, _: &Vacation) {
        assert_eq!(self.data.lock().unwrap().replies["scheduledEndDateTime"]["dateTime"], "2026-10-20T23:59:59");
    }
    fn at_the_end(&self) {
        assert!(self.data.lock().unwrap().calls.iter().any(|c| c.starts_with("POST /$batch")), "small requests go in batches");
    }
}

/// A message whose source the server won't give doesn't stop the folder, over Graph.
#[test]
fn one_unreadable_message_holds_up_nothing() {
    let server = serve(Data::default());
    crate::fakes::exchange_e2e::one_unreadable_message_holds_up_nothing("graph-unreadable", &server, |store| {
        let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
        let mut st = crate::jmap::State::default();
        crate::graph::list_folders(&c, &mut st, "a1").unwrap();
        crate::graph::sync_folder(&c, &mut st, store, "a1", "INBOX", &crate::sync::SyncOptions::default()).unwrap();
    });
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
    let store = crate::fake::store("graph-text-first");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = account("a1", &server.base);
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

/// Issue #32 against the fake Graph: a private, tentative event is created with Graph's
/// `sensitivity` and `showAs`, read back so, and a change of both is sent and kept.
#[test]
fn private_and_show_as_go_to_graph_and_back() {
    use mailcraft_model::ical::{CalEvent, Privacy, ShowAs, privacy, set_privacy, set_show_as, show_as};
    use mailcraft_model::pim::{LOCAL, Pim, StoredEvent};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let mut e = CalEvent { uid: "doc-1".into(), summary: "Doctor".into(), start: 1_792_000_000, end: 1_792_003_600, ..Default::default() };
    set_privacy(&mut e, Privacy::Private);
    set_show_as(&mut e, ShowAs::Tentative);
    let pim = Pim { events: vec![StoredEvent { calendar: LOCAL.into(), event: e, dirty: true, ..Default::default() }], ..Default::default() };
    let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    {
        let d = server.data.lock().unwrap();
        let on = d.events.iter().find(|e| e["subject"] == "Doctor").unwrap();
        assert_eq!((on["sensitivity"].as_str(), on["showAs"].as_str()), (Some("private"), Some("tentative")));
    }
    let doc = sync.events.iter().find(|x| x.event.summary == "Doctor").unwrap().clone();
    assert_eq!((privacy(&doc.event), show_as(&doc.event)), (Privacy::Private, ShowAs::Tentative));
    let mut changed = doc;
    set_privacy(&mut changed.event, Privacy::Public);
    set_show_as(&mut changed.event, ShowAs::WorkingElsewhere);
    changed.dirty = true;
    let sync = crate::graph_pim::sync(&c, &Pim { events: vec![changed], ..Default::default() }, 1_792_000_100).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let d = server.data.lock().unwrap();
    let on = d.events.iter().find(|e| e["subject"] == "Doctor").unwrap();
    assert_eq!((on["sensitivity"].as_str(), on["showAs"].as_str()), (Some("normal"), Some("workingElsewhere")));
    let back = sync.events.iter().find(|x| x.event.summary == "Doctor").unwrap();
    assert_eq!((privacy(&back.event), show_as(&back.event)), (Privacy::Public, ShowAs::WorkingElsewhere));
}

/// Issues #25 and #32 at once against the fake Graph: a private, out-of-office event with a
/// room's meeting goes up with all three and comes back with all three, the meeting found in
/// its place and notes once.
#[test]
fn private_show_as_and_a_meeting_together_through_graph() {
    use mailcraft_model::ical::{CalEvent, MeetingLabels, MeetingRoom, Privacy, ShowAs, privacy, set_privacy, set_show_as, show_as};
    use mailcraft_model::pim::{LOCAL, Pim, StoredEvent};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let room = MeetingRoom::new("BBB", "https://meet.example.edu/rooms/abc-def-ghi", "123-456-789", "").unwrap();
    let mut e = CalEvent {
        uid: "all-1".into(),
        summary: "Doctor".into(),
        description: "Bring the card".into(),
        start: 1_792_000_000,
        end: 1_792_003_600,
        ..Default::default()
    };
    set_privacy(&mut e, Privacy::Confidential);
    set_show_as(&mut e, ShowAs::OutOfOffice);
    e.add_online_meeting(&room, &MeetingLabels::default());
    let pim = Pim { events: vec![StoredEvent { calendar: LOCAL.into(), event: e, dirty: true, ..Default::default() }], ..Default::default() };
    let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    {
        let d = server.data.lock().unwrap();
        let on = d.events.iter().find(|e| e["subject"] == "Doctor").unwrap();
        assert_eq!((on["sensitivity"].as_str(), on["showAs"].as_str()), (Some("confidential"), Some("oof")));
        assert_eq!(on["location"]["displayName"], room.url.as_str());
    }
    let back = &sync.events.iter().find(|x| x.event.summary == "Doctor").unwrap().event;
    assert_eq!((privacy(back), show_as(back)), (Privacy::Confidential, ShowAs::OutOfOffice));
    assert_eq!(back.online_meeting().as_deref(), Some(room.url.as_str()));
    assert_eq!(back.description.matches("---- Online meeting").count(), 1, "{}", back.description);
}

/// Issue #40 against the fake Graph: a reminder dismissed here is dismissed on the event; one
/// snoozed on an occurrence of a series is snoozed on that occurrence (found among the series'
/// instances) until its time; one whose event is gone is dropped. Each is said answered.
#[test]
fn reminders_answered_here_are_answered_on_graph() {
    use mailcraft_model::pim::{Pim, ReminderAnswer};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.events.push(serde_json::json!({"id": "EV-one", "calendar": "CAL1", "subject": "Dentist"}));
    d.events.push(serde_json::json!({"id": "EV-series", "calendar": "CAL1", "subject": "Stand-up"}));
    let at = |t: &str| serde_json::json!({"dateTime": t, "timeZone": "UTC"});
    d.instances.push((
        "EV-series".into(),
        vec![
            serde_json::json!({"id": "EV-series-3", "start": at("2026-11-03T09:00:00.0000000")}),
            serde_json::json!({"id": "EV-series-4", "start": at("2026-11-04T09:00:00.0000000")}),
        ],
    ));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let nov4 = 1_793_782_800; // 2026-11-04 09:00 UTC
    let answer = |uid: &str, href: &str, series: bool, snooze_until: Option<i64>| ReminderAnswer {
        calendar: "CAL1".into(),
        href: href.into(),
        uid: uid.into(),
        occurrence: nov4,
        series,
        snooze_until,
    };
    let pim = Pim {
        reminder_answers: vec![
            answer("one", "EV-one", false, None),
            answer("series", "EV-series", true, Some(nov4 - 300)),
            answer("gone", "EV-gone", false, None),
        ],
        ..Default::default()
    };
    let sync = crate::graph_pim::sync(&c, &pim, nov4 - 900).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.answered, [("one".to_string(), nov4), ("series".to_string(), nov4), ("gone".to_string(), nov4)]);
    let d = server.data.lock().unwrap();
    let done: Vec<(&str, &str)> = d.reminders.iter().map(|(a, id, _)| (a.as_str(), id.as_str())).collect();
    assert_eq!(done, [("dismissReminder", "EV-one"), ("snoozeReminder", "EV-series-4")]);
    assert_eq!(d.reminders[1].2["newReminderTime"], serde_json::json!({"dateTime": "2026-11-04T08:55:00", "timeZone": "UTC"}));
}

/// What's read is what was written, whatever the reply has in it: one line with an `&` and no
/// `<` came back `R&amp;D`, and grew by an `amp;` each time the replies were saved.
#[test]
fn automatic_replies_come_back_as_typed_however_often_they_are_saved() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    for typed in crate::exchange::REPLIES_AS_TYPED {
        let away = Vacation { enabled: true, text: typed.into(), external: Some(format!("Outside: {typed}")), ..Default::default() };
        c.set_vacation(&away).unwrap();
        let once = c.vacation().unwrap();
        assert_eq!(once, away, "read after the first save");
        c.set_vacation(&once).unwrap();
        assert_eq!(c.vacation().unwrap(), away, "read after the second save");
    }
    // A reply another program set, as a page: read as HTML.
    server.data.lock().unwrap().replies["internalReplyMessage"] = json!("<html><body><div>Back &amp; forth</div><div>R&amp;D</div></body></html>");
    assert_eq!(c.vacation().unwrap().text, "Back & forth\nR&D");
}

/// A delete Graph refuses (an event in someone's calendar this account may only read: 403) isn't
/// taken for done: it's said, and its tombstone stays to be asked again. It was dropped as if
/// the event were gone. One for something no longer there (404) is done.
#[test]
fn a_refused_delete_is_said_and_kept() {
    use mailcraft_model::pim::{Pim, Tombstone, shared_calendar_id};
    let mut d = Data { others: vec![("ben@contoso.example".into(), false)], ..Default::default() };
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    let at = |h: u32| json!({"dateTime": format!("2026-10-20T{h:02}:00:00.0000000")});
    d.events
        .push(json!({"id": "B1", "calendar": "user:ben@contoso.example", "iCalUId": "b1", "subject": "Ben's call", "start": at(9), "end": at(10)}));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let ben = shared_calendar_id("ben@contoso.example");
    let mut pim = Pim { shared: vec!["ben@contoso.example".into()], ..Default::default() };
    let first = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    let href = first.events.iter().find(|e| e.event.summary == "Ben's call").map(|e| e.href.clone()).unwrap();
    pim.deleted = vec![
        Tombstone { href: href.clone(), etag: String::new(), calendar: ben.clone() },
        Tombstone { href: "contact:NOT-THERE".into(), etag: String::new(), ..Default::default() },
    ];
    let second = crate::graph_pim::sync(&c, &pim, 1_792_000_100).unwrap();
    assert_eq!(second.deleted, ["contact:NOT-THERE"], "only the one that's gone");
    assert!(second.errors.iter().any(|e| e.contains("403")), "{:?}", second.errors);
    assert!(server.data.lock().unwrap().events.iter().any(|e| e["id"] == "B1"), "still on the server");
}

/// A delete of an id Graph can't read as one (a damaged one, or one kept from another kind of
/// account: 400, `ErrorInvalidIdMalformed`) can never be done: it's said once and its tombstone
/// goes, where it was refused, and said, at every sync. One with no id at all isn't asked. One
/// refused for want of rights (403) stays, as it may be allowed later.
#[test]
fn a_delete_of_an_id_that_isnt_one_is_said_once() {
    use mailcraft_model::pim::{Pim, Tombstone, shared_calendar_id};
    let mut d = Data { others: vec![("ben@contoso.example".into(), false)], ..Default::default() };
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    let at = |h: u32| json!({"dateTime": format!("2026-10-20T{h:02}:00:00.0000000")});
    d.events
        .push(json!({"id": "B1", "calendar": "user:ben@contoso.example", "iCalUId": "b1", "subject": "Ben's call", "start": at(9), "end": at(10)}));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let tombstone = |href: &str, calendar: &str| Tombstone { href: href.into(), etag: String::new(), calendar: calendar.into() };
    let mut pim = Pim { shared: vec!["ben@contoso.example".into()], ..Default::default() };
    pim.deleted = vec![
        tombstone("MALFORMED-1", ""),
        tombstone("contact:MALFORMED-2", ""),
        tombstone("", ""),
        tombstone("contact:", ""),
        tombstone("B1", &shared_calendar_id("ben@contoso.example")),
    ];
    let first = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert_eq!(first.deleted, ["MALFORMED-1", "contact:MALFORMED-2", "", "contact:"], "done with, as they can never be deleted");
    let said = |errors: &[String], what: &str| errors.iter().filter(|e| e.contains(what)).count();
    assert_eq!((said(&first.errors, "400"), said(&first.errors, "403")), (2, 1), "{:?}", first.errors);
    // The next sync has only the one that may yet be allowed.
    pim.deleted.retain(|t| !first.deleted.contains(&t.href));
    let second = crate::graph_pim::sync(&c, &pim, 1_792_000_100).unwrap();
    assert!(second.deleted.is_empty(), "{:?}", second.deleted);
    assert_eq!((said(&second.errors, "400"), said(&second.errors, "403")), (0, 1), "{:?}", second.errors);
    // Allowed since: it goes.
    server.data.lock().unwrap().others = vec![("ben@contoso.example".into(), true)];
    let third = crate::graph_pim::sync(&c, &pim, 1_792_000_200).unwrap();
    assert_eq!(third.deleted, ["B1"]);
    assert!(!server.data.lock().unwrap().events.iter().any(|e| e["id"] == "B1"));
}
