//! Issue #61: an account with calendars and contacts only, and no mail server. Its thread syncs
//! them with the CalDAV and CardDAV server and nothing else; what's for mail is answered at once.

use std::time::Duration;

use mailcraft_model::ical::ShowAs;
use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, DavSettings, MessageKey, SyncStatus};

use super::*;
use crate::fakes::fake_dav::{BOOK, CAL, PASSWORD, USER, serve};

fn synced(e: &Event) -> bool {
    matches!(e, Event::DavSynced { .. })
}

fn status(e: &Event) -> Option<&SyncStatus> {
    match e {
        Event::Status { status, .. } if !matches!(status, SyncStatus::Connecting) => Some(status),
        _ => None,
    }
}

/// A calendar-only account at `server` (its root, as the user would give it).
fn account(server: &str) -> AccountConfig {
    let dav = DavSettings { caldav: server.into(), carddav: server.into(), username: None };
    AccountConfig { id: "a1".into(), email: USER.into(), username: USER.into(), dav: Some(dav), ..Default::default() }
}

fn start(name: &str, server: &str, password: &str) -> Pool {
    let store = crate::fake::store(name);
    let mut pool = Pool::new(store, std::sync::Arc::new(MemoryCredentials::default()));
    let account = account(server);
    assert!(account.is_calendar_only() && account.is_local() && account.has_server());
    pool.submit(Job::Start { account, password: Some(Secret(password.into())) });
    pool
}

fn requests(data: &std::sync::Arc<std::sync::Mutex<crate::fakes::fake_dav::Data>>) -> usize {
    data.lock().unwrap().log.len()
}

/// From the server's bare address: its calendars and contacts found and read, the account said
/// to be up to date, and a change asked for sent at once.
#[test]
fn syncs_calendars_and_contacts_from_the_bare_server_address() {
    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        let event = mailcraft_model::ical::CalEvent {
            uid: "e1".into(),
            summary: "Dentist".into(),
            start: 1_800_000_000,
            end: 1_800_003_600,
            ..Default::default()
        };
        d.put(&format!("{CAL}e1.ics"), &mailcraft_model::ical::write(&[event], None, 0));
        let card = mailcraft_model::vcard::Card { uid: "c1".into(), name: "Zoe Park".into(), ..Default::default() };
        d.put(&format!("{BOOK}c1.vcf"), &mailcraft_model::vcard::write(&card));
    }
    let mut pool = start("calendar-only-sync", &server.base, PASSWORD);
    let first = until(&pool, "the first sync", synced).pop().unwrap();
    let Event::DavSynced { sync, .. } = &first else { panic!("not a sync: {first:?}") };
    assert_eq!(sync.caldav_home.as_deref(), Some(format!("{}/dav/cal/", server.base).as_str()));
    assert_eq!(sync.carddav_home.as_deref(), Some(format!("{}/dav/card/", server.base).as_str()));
    assert_eq!(sync.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Dentist"]);
    assert_eq!(sync.cards.iter().map(|c| c.card.display()).collect::<Vec<_>>(), ["Zoe Park"]);
    until(&pool, "online", |e| matches!(status(e), Some(SyncStatus::Online { .. })));
    // A change made here: synced again now, not in a quarter of an hour.
    pool.submit(Job::SyncDav { account: "a1".into() });
    until(&pool, "the sync asked for", synced);
    pool.submit(Job::Remove { account: "a1".into() });
}

/// A refused password says so, and isn't tried again for every change made meanwhile: only Send /
/// Receive (or a new password) tries again.
#[test]
fn a_wrong_password_waits_for_send_receive() {
    let server = serve();
    let mut pool = start("calendar-only-password", &server.base, "wrong");
    until(&pool, "the password asked for", |e| matches!(status(e), Some(SyncStatus::NeedsPassword { .. })));
    let tried = requests(&server.data);
    pool.submit(Job::SyncDav { account: "a1".into() });
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(requests(&server.data), tried, "tried again for a change: {:?}", server.data.lock().unwrap().log);
    pool.submit(Job::SyncAll { account: "a1".into() });
    until(&pool, "asked again", |e| matches!(status(e), Some(SyncStatus::NeedsPassword { .. })));
    assert!(requests(&server.data) > tried);
    pool.submit(Job::Remove { account: "a1".into() });
}

/// A server that can't be reached is an error the account shows, not a crash or a hang, and it's
/// tried again later.
#[test]
fn a_server_gone_is_an_error_and_tried_again() {
    // A port of the test's own, every connection to it dropped unanswered. Not one let go of:
    // another test's server can be given it meanwhile (found under load: the "gone" server
    // answered, and there was no second error).
    let gone = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", gone.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for conn in gone.incoming() {
            drop(conn);
        }
    });
    let mut pool = start("calendar-only-gone", &base, PASSWORD);
    let failed = until(&pool, "an error", |e| matches!(status(e), Some(SyncStatus::Error { .. }))).pop().unwrap();
    let Some(SyncStatus::Error { error, .. }) = status(&failed) else { panic!("not an error: {failed:?}") };
    assert!(!error.is_empty());
    // Tried again after the backoff (five seconds), without being asked.
    until(&pool, "a second try", |e| matches!(status(e), Some(SyncStatus::Error { .. })));
    pool.submit(Job::Remove { account: "a1".into() });
}

/// What's for mail gets its answer at once from the pool (it has no mail server): nothing waits
/// on its thread, and nothing is queued to be sent for good.
#[test]
fn mail_jobs_are_answered_without_a_mail_server() {
    let server = serve();
    let mut pool = start("calendar-only-mail", &server.base, PASSWORD);
    until(&pool, "the first sync", synced);
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 7,
        raw: b"Subject: hi\r\n\r\nhi".to_vec(),
        from: USER.into(),
        recipients: vec!["you@example.org".into()],
        sent_folder: None,
    });
    let e = until(&pool, "the send refused", |e| matches!(e, Event::SendFailed { .. } | Event::Queued { .. } | Event::Sent { .. })).pop().unwrap();
    assert!(matches!(e, Event::SendFailed { id: 7, .. }), "{e:?}");
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), uid: 1, ..Default::default() };
    pool.submit(Job::FetchBody { key });
    until(&pool, "the body's answer", |e| matches!(e, Event::BodyFailed { .. } | Event::Body { .. }));
    pool.submit(Job::GetVacation { account: "a1".into() });
    until(&pool, "automatic replies' answer", |e| matches!(e, Event::Vacation { .. }));
    pool.submit(Job::Remove { account: "a1".into() });
}

/// Found on a real Baïkal (sabre/dav), whose sign-in is Digest unless set otherwise: every request
/// was refused ("the calendar server refused the user name or password"), as only Basic was sent.
/// Digest is answered, its nonce gone stale is answered again, and a wrong password still asks.
#[test]
fn a_server_taking_digest_only_is_signed_in_to() {
    let server = serve();
    server.data.lock().unwrap().digest = Some("n1".into());
    {
        let mut d = server.data.lock().unwrap();
        let event = mailcraft_model::ical::CalEvent {
            uid: "e1".into(),
            summary: "Dentist".into(),
            start: 1_800_000_000,
            end: 1_800_003_600,
            ..Default::default()
        };
        d.put(&format!("{CAL}e1.ics"), &mailcraft_model::ical::write(&[event], None, 0));
    }
    let mut pool = start("calendar-only-digest", &server.base, PASSWORD);
    let first = until(&pool, "the first sync", synced).pop().unwrap();
    let Event::DavSynced { sync, .. } = &first else { panic!("not a sync: {first:?}") };
    assert_eq!(sync.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Dentist"]);
    // Answered at once after the first refusal, not refused first every time.
    let refused = |s: &crate::fakes::fake_dav::Server| s.data.lock().unwrap().refused;
    let before = refused(&server);
    pool.submit(Job::SyncDav { account: "a1".into() });
    until(&pool, "a second sync", synced);
    assert_eq!(refused(&server), before, "{:?}", server.data.lock().unwrap().log);
    // The nonce goes stale: answered again with the new one, and the sync goes on.
    server.data.lock().unwrap().digest = Some("n2".into());
    pool.submit(Job::SyncDav { account: "a1".into() });
    until(&pool, "a sync after the nonce went stale", synced);
    assert_eq!(refused(&server), before + 1, "refused once, as stale");
    pool.submit(Job::Remove { account: "a1".into() });
    let mut wrong = start("calendar-only-digest-wrong", &server.base, "wrong");
    until(&wrong, "the password asked for", |e| matches!(status(e), Some(SyncStatus::NeedsPassword { .. })));
    wrong.submit(Job::Remove { account: "a1".into() });
}

/// Where the account's calendars are, kept as the engine keeps it after a sync.
fn save_home(pool: &Pool, base: &str) {
    let pim = mailcraft_model::pim::Pim { caldav_home: Some(format!("{base}/dav/cal/")), ..Default::default() };
    pool.store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
}

/// Issue #242: colleagues' free/busy times from a server that schedules (RFC 6638), asked of the
/// user's outbox as the user, an answer each; from one that doesn't, why there are none. Before,
/// the pool answered "not connected" for an account with no mail server, and any account but an
/// Exchange one said only Exchange could.
#[test]
fn colleagues_times_come_from_a_server_that_schedules() {
    let ask = |pool: &mut Pool| {
        let people = vec!["ana@studio.example".to_string(), "nobody@studio.example".into()];
        pool.submit(Job::FreeBusy { account: "a1".into(), people, from: 1_800_000_000, to: 1_800_604_800 });
        let e = until(pool, "the times", |e| matches!(e, Event::FreeBusy { .. })).pop().unwrap();
        let Event::FreeBusy { answers, .. } = e else { panic!("not times: {e:?}") };
        answers
    };
    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        d.schedules = true;
        let lines = "FREEBUSY;FBTYPE=BUSY:20270115T080000Z/20270115T090000Z\r\nFREEBUSY;FBTYPE=BUSY-TENTATIVE:20270115T100000Z/PT30M\r\n";
        d.free_busy.insert("ana@studio.example".into(), lines.into());
    }
    let mut pool = start("calendar-only-free-busy", &server.base, PASSWORD);
    until(&pool, "the first sync", synced);
    // Not known yet (the engine keeps where the calendars are, once a sync found them): said.
    assert!(ask(&mut pool).iter().all(|a| a.error.as_deref().is_some_and(|e| e.contains("isn't known"))));
    save_home(&pool, &server.base);
    let answers = ask(&mut pool);
    assert_eq!(answers.len(), 2, "{answers:?}");
    let ana: Vec<(i64, i64, ShowAs)> = answers[0].busy.iter().map(|b| (b.start, b.end, b.show_as)).collect();
    assert_eq!(answers[0].error, None, "{answers:?}");
    assert_eq!(ana, [(1_800_000_000, 1_800_003_600, ShowAs::Busy), (1_800_007_200, 1_800_009_000, ShowAs::Tentative)]);
    assert_eq!(answers[1].error.as_deref(), Some("Invalid calendar user"));
    {
        let d = server.data.lock().unwrap();
        let (originator, recipient, body) = d.queries.first().cloned().unwrap();
        assert_eq!(originator, format!("mailto:{USER}"));
        assert_eq!(recipient, "mailto:ana@studio.example, mailto:nobody@studio.example");
        assert!(body.contains(&format!("ORGANIZER:mailto:{USER}\r\n")) && body.contains("DTSTART:20270115T080000Z\r\n"), "{body}");
    }
    pool.submit(Job::Remove { account: "a1".into() });

    // A server that doesn't schedule: said for each, and nothing POSTed.
    let server = serve();
    let mut pool = start("calendar-only-free-busy-none", &server.base, PASSWORD);
    until(&pool, "the first sync", synced);
    save_home(&pool, &server.base);
    let answers = ask(&mut pool);
    assert!(answers.iter().all(|a| a.busy.is_empty() && a.error.as_deref().is_some_and(|e| e.contains("doesn't give"))), "{answers:?}");
    assert!(server.data.lock().unwrap().queries.is_empty());
    pool.submit(Job::Remove { account: "a1".into() });
}
