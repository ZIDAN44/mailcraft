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
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
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
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
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

/// Issue #139 on a real server: a rule made here (its folder by its path, Junk Email) is on the
/// server and read back as made, its folder told by its path; turned off and renamed, then
/// deleted. Any rules already there are left as they are.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_inbox_rules() {
    use mailcraft_model::organize::{Condition, Rule, RuleAction, ServerRule, ServerRuleOp, ServerRules};
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let store = crate::fake::store("real-ews-rules");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = AccountConfig { id: "x1".into(), email: user.clone(), username: user, ews: Some(url), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(password)) });
    let wait = |pool: &mut Pool, what: &str, f: &dyn Fn(&Event) -> bool| -> Event {
        let end = Instant::now() + Duration::from_secs(300);
        while Instant::now() < end {
            if let Some(e) = pool.wait(Duration::from_millis(100))
                && f(&e)
            {
                return e;
            }
        }
        panic!("no {what}");
    };
    wait(&mut pool, "online", &|e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let junk = store.load_folders("x1").unwrap().into_iter().find(|f| f.role == mailcraft_model::FolderRole::Junk).expect("Junk Email").path;
    let rules = |pool: &mut Pool, job: Job| -> (Option<ServerRules>, Option<String>) {
        pool.submit(job);
        match wait(pool, "the rules", &|e| matches!(e, Event::ServerRules { .. })) {
            Event::ServerRules { rules, error, .. } => (rules, error),
            other => panic!("{other:?}"),
        }
    };
    let (before, error) = rules(&mut pool, Job::GetServerRules { account: "x1".into() });
    eprintln!("before: {before:?} {error:?}");
    let before = before.expect("the server's rules");
    let new = Rule {
        name: "MailCraft test".into(),
        enabled: true,
        account: None,
        conditions: vec![Condition::Subject("mailcraft-rule-test".into())],
        match_all: true,
        actions: vec![RuleAction::MoveTo(junk.clone())],
        stop: true,
    };
    let now = ServerRule {
        name: "MailCraft test".into(),
        priority: before.rules.len() as u32 + 1,
        enabled: true,
        rule: Some(new.clone()),
        ..Default::default()
    };
    let ops = vec![ServerRuleOp::Create { now }];
    let (after, error) = rules(&mut pool, Job::SetServerRules { account: "x1".into(), ops, drop_outlook_list: before.outlook_list });
    eprintln!("made: {after:?} {error:?}");
    assert_eq!(error, None);
    let made = after.unwrap().rules.into_iter().find(|r| r.name == "MailCraft test").expect("on the server");
    let back = made.rule.clone().expect("read back as MailCraft's own");
    assert_eq!((back.conditions, back.actions, back.stop), (new.conditions, new.actions, true), "its folder by its path");
    let now = ServerRule { name: "MailCraft test (off)".into(), enabled: false, ..made.clone() };
    let (after, error) =
        rules(&mut pool, Job::SetServerRules { account: "x1".into(), ops: vec![ServerRuleOp::Set { was: made, now }], drop_outlook_list: false });
    assert_eq!(error, None);
    let off = after.unwrap().rules.into_iter().find(|r| r.name == "MailCraft test (off)").expect("renamed");
    assert!(!off.enabled);
    let (after, error) =
        rules(&mut pool, Job::SetServerRules { account: "x1".into(), ops: vec![ServerRuleOp::Delete { was: off }], drop_outlook_list: false });
    assert_eq!(error, None);
    assert_eq!(after.unwrap().rules.len(), before.rules.len(), "gone again");
}

/// Free/busy times (issue #140) from a real availability service: an answer for each person
/// asked about, in order, whatever the server makes of them (what it can't give is said per
/// person, never a failure of the whole). Printed for a look.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_free_busy() {
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let c = crate::ews::Client::connect(&url, &user, &password, &user).unwrap();
    let domain = user.split('@').nth(1).unwrap_or("ews.test");
    let people = vec![user.clone(), format!("ana@{domain}"), format!("nobody-here@{domain}")];
    let now = 1_793_000_000;
    let answers = c.availability(&people, now - 7 * 86_400, now + 60 * 86_400).unwrap();
    for a in &answers {
        eprintln!("{}: {} busy, {:?}", a.email, a.busy.len(), a.error);
    }
    assert_eq!(answers.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), people.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(answers[2].error.is_some(), "nobody is there: {:?}", answers[2]);
}
