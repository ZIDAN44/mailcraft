//! Issue #61: an account with calendars and contacts only, and no mail server. Its thread syncs
//! them with the CalDAV and CardDAV server and nothing else; what's for mail is answered at once.

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, DavSettings, MessageKey, SyncStatus};

use super::*;
use crate::fakes::fake_dav::{BOOK, CAL, PASSWORD, USER, serve};

fn until(pool: &Pool, what: &str, f: impl Fn(&Event) -> bool) -> Event {
    let began = Instant::now();
    let mut seen = Vec::new();
    while began.elapsed() < Duration::from_secs(20) {
        if let Some(e) = pool.wait(Duration::from_millis(20)) {
            if f(&e) {
                return e;
            }
            seen.push(e);
        }
    }
    panic!("timed out waiting for {what}; saw {seen:?}");
}

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
    let first = until(&pool, "the first sync", synced);
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
    let gone = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", gone.local_addr().unwrap().port());
    drop(gone);
    let mut pool = start("calendar-only-gone", &base, PASSWORD);
    let failed = until(&pool, "an error", |e| matches!(status(e), Some(SyncStatus::Error { .. })));
    let Some(SyncStatus::Error { error }) = status(&failed) else { panic!("not an error: {failed:?}") };
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
    let e = until(&pool, "the send refused", |e| matches!(e, Event::SendFailed { .. } | Event::Queued { .. } | Event::Sent { .. }));
    assert!(matches!(e, Event::SendFailed { id: 7, .. }), "{e:?}");
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), uid: 1, ..Default::default() };
    pool.submit(Job::FetchBody { key });
    until(&pool, "the body's answer", |e| matches!(e, Event::BodyFailed { .. } | Event::Body { .. }));
    pool.submit(Job::GetVacation { account: "a1".into() });
    until(&pool, "automatic replies' answer", |e| matches!(e, Event::Vacation { .. }));
    pool.submit(Job::Remove { account: "a1".into() });
}
