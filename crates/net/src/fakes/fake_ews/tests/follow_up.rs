//! Gap 9: a message's follow-up on Exchange, as Outlook keeps it. A flag's due day set in
//! Outlook comes over with the flag, and one marked complete comes as complete; a day given here
//! goes to the message (as a date, and as the instant it starts where the user is), and one
//! taken off goes; Mark Complete is complete there too.

use std::sync::Arc;

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Event, FOLLOW_UP_COMPLETE, Flag, FollowUpDay, Job, Op, SyncStatus};
use mailcraft_store::Store;

use super::*;
use crate::waiting::{eventually, until};

fn day(y: i64, m: u32, d: u32) -> i64 {
    mailcraft_model::dates::days_from_civil(y, m, d)
}

#[test]
fn follow_up_days_and_completion_go_both_ways() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let budget = d.deliver(&inbox, &super::mail("Budget", "m1"), false, "2026-11-18T09:00:00Z");
    let done = d.deliver(&inbox, &super::mail("Done already", "m2"), false, "2026-11-17T09:00:00Z");
    // Outlook flagged one, due the 20th (its date, at midnight), and marked the other complete.
    for it in d.items.iter_mut() {
        if it.id == budget {
            (it.flagged, it.due) = (true, Some("2026-11-20T00:00:00Z".into()));
        } else if it.id == done {
            it.complete = true;
        }
    }
    let server = serve(d);
    let data = server.data.clone();
    let store = Store::open(crate::data_files::scratch::scratch("ews-follow-up")).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig { name: "Riley".into(), ..super::account("a1", &server.url) };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let kept =
        |subject: &str| store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == subject).map(|m| (**m).clone()).unwrap();
    let b = kept("Budget");
    assert!(b.flags.flagged && b.flags.follow_up_due == Some(day(2026, 11, 20)), "{:?}", b.flags);
    let c = kept("Done already");
    assert!(!c.flags.flagged && c.flags.has(FOLLOW_UP_COMPLETE), "{:?}", c.flags);

    // A day given here: the date, and when it starts an hour east of UTC.
    let next = FollowUpDay { day: day(2026, 11, 25), at: day(2026, 11, 25) * 86_400 - 3600 };
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFollowUp { keys: vec![b.key.clone()], due: Some(next) } });
    let on_server = |id: &str| data.lock().unwrap().items.iter().find(|i| i.id == id).cloned().unwrap();
    eventually("the day on the server", || on_server(&budget).due.as_deref() == Some("2026-11-25T00:00:00Z"));
    // Marked complete here: complete there.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![b.key.clone()], flag: Flag::Flagged, on: false } });
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetKeyword { keys: vec![b.key.clone()], keyword: FOLLOW_UP_COMPLETE.into(), on: true } });
    eventually("complete on the server", || {
        let i = on_server(&budget);
        i.complete && !i.flagged
    });
    // Flagged again with no day: the day taken off.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetKeyword { keys: vec![b.key.clone()], keyword: FOLLOW_UP_COMPLETE.into(), on: false } });
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![b.key.clone()], flag: Flag::Flagged, on: true } });
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFollowUp { keys: vec![b.key.clone()], due: None } });
    eventually("flagged with no day", || {
        let i = on_server(&budget);
        i.flagged && !i.complete && i.due.is_none()
    });
    // Changed in Outlook since: given a day, and the other one flagged again; then marked
    // complete. Each comes with the next sync (only the first sync's had them).
    {
        let mut d = data.lock().unwrap();
        for it in d.items.iter_mut() {
            if it.id == budget {
                it.due = Some("2026-11-27T00:00:00Z".into());
            } else if it.id == done {
                (it.flagged, it.complete) = (true, false);
            }
        }
    }
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    eventually("the day set in Outlook", || kept("Budget").flags.follow_up_due == Some(day(2026, 11, 27)));
    eventually("flagged again in Outlook", || {
        let c = kept("Done already");
        c.flags.flagged && !c.flags.has(FOLLOW_UP_COMPLETE)
    });
    for it in data.lock().unwrap().items.iter_mut().filter(|i| i.id == budget) {
        (it.flagged, it.complete) = (false, true);
    }
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    eventually("completed in Outlook", || {
        let b = kept("Budget");
        !b.flags.flagged && b.flags.has(FOLLOW_UP_COMPLETE)
    });
    drop(pool);
}

/// Gap 9: a flag's reminder, as Outlook keeps it on the message: one Outlook set (and snoozed)
/// comes over at its signal time; one set here goes there at both times; dismissed here, it's no
/// longer set there; dismissed in Outlook, the next sync has none.
#[test]
fn a_flags_reminder_goes_both_ways() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let call = d.deliver(&inbox, &super::mail("Call Ben", "m1"), false, "2026-11-18T09:00:00Z");
    for it in d.items.iter_mut().filter(|i| i.id == call) {
        it.flagged = true;
        it.reminder_set = true;
        it.reminder_time = Some("2026-11-20T08:00:00Z".into());
        it.reminder_signal = Some("2026-11-20T08:30:00Z".into());
    }
    let server = serve(d);
    let data = server.data.clone();
    let store = Store::open(crate::data_files::scratch::scratch("ews-flag-reminder")).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig { name: "Riley".into(), ..super::account("a1", &server.url) };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let kept = || store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "Call Ben").map(|m| (**m).clone()).unwrap();
    let at = |h: i64, m: i64| day(2026, 11, 20) * 86_400 + h * 3600 + m * 60;
    let c = kept();
    assert_eq!(c.flags.reminder, Some(at(8, 30)), "snoozed in Outlook: when it reminds next");

    let on_server = |id: &str| data.lock().unwrap().items.iter().find(|i| i.id == id).cloned().unwrap();
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetReminder { keys: vec![c.key.clone()], at: Some(at(16, 0)) } });
    eventually("the reminder on the server", || {
        let i = on_server(&call);
        i.reminder_set && i.reminder_signal.as_deref() == Some("2026-11-20T16:00:00Z") && i.reminder_time.as_deref() == Some("2026-11-20T16:00:00Z")
    });
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetReminder { keys: vec![c.key.clone()], at: None } });
    eventually("dismissed on the server", || {
        let i = on_server(&call);
        !i.reminder_set && i.reminder_signal.is_none() && i.flagged
    });

    // Set again there and read; then dismissed in Outlook: read as none.
    for it in data.lock().unwrap().items.iter_mut().filter(|i| i.id == call) {
        (it.reminder_set, it.reminder_signal) = (true, Some("2026-11-21T07:00:00Z".into()));
    }
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    eventually("set in Outlook", || kept().flags.reminder == Some(day(2026, 11, 21) * 86_400 + 7 * 3600));
    for it in data.lock().unwrap().items.iter_mut().filter(|i| i.id == call) {
        it.reminder_set = false;
    }
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    eventually("dismissed in Outlook", || kept().flags.reminder.is_none());
    drop(pool);
}
