//! Gap 9: a message's follow-up on Microsoft 365, as Outlook keeps it: a flag's day and reminder
//! set in Outlook come over with the flag, and with each sync after when changed there; a day
//! and a reminder set here go to the message, and a reminder dismissed here is no longer set.

use mailcraft_model::{FOLLOW_UP_COMPLETE, FollowUpDay, Op};
use mailcraft_store::Store;

use super::*;

const SET: &str = "Boolean {00062008-0000-0000-C000-000000000046} Id 0x8503";
const SIGNAL: &str = "SystemTime {00062008-0000-0000-C000-000000000046} Id 0x8560";

fn day(y: i64, m: u32, d: u32) -> i64 {
    mailcraft_model::dates::days_from_civil(y, m, d)
}

#[test]
fn a_flags_day_and_reminder_go_both_ways() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    let id = d.deliver(&inbox, &mail("Call Ben", "m1"), false, "2026-11-18T09:00:00Z");
    // Outlook flagged it, due the 20th (its midnight an hour east of UTC), to remind at 08:30.
    for m in d.messages.iter_mut().filter(|m| m.id == id) {
        m.flagged = true;
        m.due = Some("2026-11-19T23:00:00.0000000".into());
        m.props = vec![(SET.into(), "true".into()), (SIGNAL.into(), "2026-11-20T08:30:00Z".into())];
    }
    let server = serve(d);
    let store = Store::open(crate::data_files::scratch::scratch("graph-follow-up")).unwrap();
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let mut st = crate::jmap::State::default();
    crate::graph::list_folders(&c, &mut st, "a1").unwrap();
    let sync = |st: &mut crate::jmap::State| crate::graph::sync_folder(&c, st, &store, "a1", "INBOX", &crate::sync::SyncOptions::default()).unwrap();
    let kept =
        |store: &Store| store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "Call Ben").map(|m| (**m).clone()).unwrap();
    let at = |d: i64, h: i64, m: i64| d * 86_400 + h * 3600 + m * 60;
    sync(&mut st);
    let m = kept(&store);
    assert!(m.flags.flagged, "{:?}", m.flags);
    assert_eq!((m.flags.follow_up_due, m.flags.reminder), (Some(day(2026, 11, 20)), Some(at(day(2026, 11, 20), 8, 30))));

    // Set here: a day and a reminder; then the reminder dismissed.
    let next = FollowUpDay { day: day(2026, 11, 25), at: day(2026, 11, 25) * 86_400 - 3600 };
    crate::graph::apply_remote(&c, &st, &Op::SetFollowUp { keys: vec![m.key.clone()], due: Some(next) }).unwrap();
    crate::graph::apply_remote(&c, &st, &Op::SetReminder { keys: vec![m.key.clone()], at: Some(at(day(2026, 11, 25), 9, 0)) }).unwrap();
    let on_server = || server.data.lock().unwrap().message(&id).cloned().unwrap();
    let s = on_server();
    assert_eq!(s.due.as_deref(), Some("2026-11-24T23:00:00"));
    assert!(s.props.contains(&(SET.into(), "true".into())) && s.props.contains(&(SIGNAL.into(), "2026-11-25T09:00:00Z".into())), "{:?}", s.props);
    sync(&mut st);
    assert_eq!(kept(&store).flags.reminder, Some(at(day(2026, 11, 25), 9, 0)), "read back");
    crate::graph::apply_remote(&c, &st, &Op::SetReminder { keys: vec![m.key.clone()], at: None }).unwrap();
    assert!(on_server().props.contains(&(SET.into(), "false".into())));
    sync(&mut st);
    assert_eq!(kept(&store).flags.reminder, None, "dismissed");

    // Changed in Outlook since: snoozed to the 26th, then marked complete.
    for m in server.data.lock().unwrap().messages.iter_mut().filter(|m| m.id == id) {
        m.props = vec![(SET.into(), "true".into()), (SIGNAL.into(), "2026-11-26T07:00:00Z".into())];
    }
    sync(&mut st);
    let m = kept(&store);
    assert_eq!((m.flags.follow_up_due, m.flags.reminder), (Some(day(2026, 11, 25)), Some(at(day(2026, 11, 26), 7, 0))));
    for m in server.data.lock().unwrap().messages.iter_mut().filter(|m| m.id == id) {
        (m.flagged, m.complete) = (false, true);
        m.props = vec![(SET.into(), "false".into())];
    }
    sync(&mut st);
    let m = kept(&store);
    assert!(!m.flags.flagged && m.flags.has(FOLLOW_UP_COMPLETE) && m.flags.reminder.is_none(), "{:?}", m.flags);
}
