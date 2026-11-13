//! Against a real Exchange Web Services server on this computer (grommunio in Docker; never a
//! real account), run by hand: what the in-memory server can't prove about a real one's answers.
//!
//! ```text
//! MAILCRAFT_REAL_EWS="http://127.0.0.1:18080/EWS/Exchange.asmx user@domain password" \
//!   cargo test -p mailcraft-net real_ews -- --ignored --nocapture
//! ```
//!
//! The mailbox's calendar holds `Budget review`, `Stand-up` (Monday, Wednesday and Friday),
//! `Company holiday` (all day) and `Design sync`; its contacts, Ana Silva.

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::pim::Pim;
use mailcraft_model::{AccountConfig, Event, Job, SyncStatus};

use crate::worker::{MemoryCredentials, Pool};

/// `url user password` from the environment, or `None` (the test is skipped).
fn server() -> Option<(String, String, String)> {
    let v = std::env::var("MAILCRAFT_REAL_EWS").ok()?;
    let mut parts = v.split_whitespace().map(str::to_string);
    Some((parts.next()?, parts.next()?, parts.next()?))
}

/// The calendar and the contacts a real server holds are read, each whole.
#[test]
#[ignore]
fn real_ews_calendar_and_contacts() {
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let c = crate::ews::Client::connect(&url, &user, &password, &user).unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    for e in &sync.events {
        eprintln!(
            "event {:?} {} {} all_day={} rrule={:?} loc={:?}",
            e.event.summary, e.event.start, e.event.end, e.event.all_day, e.event.rrule, e.event.location
        );
    }
    for k in &sync.cards {
        eprintln!("card {:?} {:?}", k.card.display(), k.card.emails.iter().map(|e| e.value.as_str()).collect::<Vec<_>>());
    }
    eprintln!("errors {:?}", sync.errors);
    eprintln!("calendars {:?}", sync.calendars.as_ref().map(|c| c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>()));
    let names: Vec<&str> = sync.events.iter().map(|e| e.event.summary.as_str()).collect();
    for want in ["Budget review", "Stand-up", "Company holiday", "Design sync"] {
        assert!(names.contains(&want), "{want} is missing: {names:?}");
    }
    let standup = sync.events.iter().find(|e| e.event.summary == "Stand-up").unwrap();
    assert_eq!(standup.event.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE,FR"));
    assert!(sync.events.iter().any(|e| e.event.summary == "Company holiday" && e.event.all_day));
    assert!(sync.cards.iter().any(|k| k.card.emails.iter().any(|e| e.value == "ana@ews.test")), "the contact is missing");
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
}

/// A mailbox with a dozen folders of mail to bring in has its calendar and contacts once the
/// Inbox is in, not after the last folder (issue #80: the calendar stayed empty meanwhile).
#[test]
#[ignore]
fn real_ews_calendar_comes_with_the_inbox() {
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let store = crate::fake::store("real-ews-first");
    let mut pool = Pool::new(store, std::sync::Arc::new(MemoryCredentials::default()));
    let account = AccountConfig { id: "x1".into(), email: user.clone(), username: user, ews: Some(url), ..Default::default() };
    let began = Instant::now();
    pool.submit(Job::Start { account, password: Some(Secret(password)) });
    let (mut folders, mut calendar, mut online) = (Vec::new(), None, None);
    let end = Instant::now() + Duration::from_secs(900);
    while Instant::now() < end && (calendar.is_none() || online.is_none()) {
        match pool.wait(Duration::from_millis(100)) {
            Some(Event::Status { status: SyncStatus::Syncing { folder }, .. }) if !folders.contains(&folder) => {
                eprintln!("{:>6.1} s  {folder}", began.elapsed().as_secs_f32());
                folders.push(folder);
            }
            Some(Event::DavSynced { sync, .. }) => {
                eprintln!(
                    "{:>6.1} s  calendar: {} events, {} contacts, after {} folders",
                    began.elapsed().as_secs_f32(),
                    sync.events.len(),
                    sync.cards.len(),
                    folders.len()
                );
                calendar.get_or_insert((began.elapsed(), folders.len(), sync.events.len()));
            }
            Some(Event::Status { status: SyncStatus::Online { .. }, .. }) => {
                eprintln!("{:>6.1} s  online, {} folders", began.elapsed().as_secs_f32(), folders.len());
                online.get_or_insert(began.elapsed());
            }
            Some(Event::Notice { message, .. }) => eprintln!("notice: {message}"),
            _ => {}
        }
    }
    let (at, after, events) = calendar.expect("the calendar was never read");
    let online = online.expect("the mail never finished");
    assert!(events >= 4, "{events} events");
    assert!(after <= 1, "the calendar waited for {after} folders");
    assert!(at < online, "the calendar came at {at:?}, the mail was done at {online:?}");
}
