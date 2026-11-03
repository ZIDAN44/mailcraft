//! Issue #40: a reminder dismissed or snoozed in MailCraft is told to the server, so the user's
//! other programs (Outlook) don't show it again; and one dismissed elsewhere isn't shown here.

use mailcraft_model::ical::{self, CalEvent};
use mailcraft_model::pim::{CalendarInfo, DavSync, ReminderAnswer, StoredEvent};

use super::*;

const CAL: &str = "cal-1";

/// An event with a 15-minute reminder, an hour after `NOW`; `daily` makes it a series.
fn with_reminder(uid: &str, href: &str, daily: bool) -> StoredEvent {
    let mut e = CalEvent {
        uid: uid.into(),
        summary: format!("Event {uid}"),
        start: NOW + 3600,
        end: NOW + 7200,
        rrule: daily.then(|| "FREQ=DAILY;COUNT=5".to_string()),
        dtstamp: NOW,
        ..Default::default()
    };
    ical::set_reminder(&mut e, Some(15));
    StoredEvent { calendar: CAL.into(), href: href.into(), etag: "1".into(), event: e, dirty: false }
}

/// A session with one account whose calendar holds `events`, as its server's sync gave them.
fn account_with(name: &str, events: Vec<StoredEvent>, exchange: bool) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "ana@studio.test", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    if exchange && let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == id) {
        a.config.graph = Some("https://graph.microsoft.com/v1.0".into());
    }
    let sync = DavSync {
        caldav_home: Some("graph".into()),
        calendars: Some(vec![CalendarInfo { id: CAL.into(), name: "Calendar".into(), ctag: "1".into(), ..Default::default() }]),
        events,
        synced_calendars: vec![CAL.into()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    s.take_jobs();
    (s, id)
}

fn due(s: &Session) -> Vec<String> {
    s.due_reminders(s.clock.now).into_iter().map(|r| r.uid).collect()
}

fn key_of(s: &Session, uid: &str) -> String {
    s.due_reminders(s.clock.now).into_iter().find(|r| r.uid == uid).map(|r| r.key).unwrap()
}

/// On an Exchange account each answer waits for the server's own action: a single event's by
/// its item, a series' occurrence by the series and its start; the sync that took them ends
/// their wait, and what's dismissed stays dismissed here meanwhile.
#[test]
fn an_answer_on_exchange_waits_for_the_server() {
    let (mut s, id) =
        account_with("reminder-exchange", vec![with_reminder("one", "AAMk-one", false), with_reminder("daily", "AAMk-daily", true)], true);
    s.clock.now = NOW + 3600 - 10 * 60;
    assert_eq!(due(&s).len(), 2, "{:?}", due(&s));
    let (one, daily) = (key_of(&s, "one"), key_of(&s, "daily"));
    s.run("reminders.dismiss", &json!({"key": one})).unwrap();
    s.run("reminders.snooze", &json!({"key": daily, "minutes": 5})).unwrap();
    assert!(due(&s).is_empty());
    let p = s.pim.get(&id).unwrap();
    assert_eq!(
        p.reminder_answers,
        [
            ReminderAnswer {
                calendar: CAL.into(),
                href: "AAMk-one".into(),
                uid: "one".into(),
                occurrence: NOW + 3600,
                series: false,
                snooze_until: None
            },
            ReminderAnswer {
                calendar: CAL.into(),
                href: "AAMk-daily".into(),
                uid: "daily".into(),
                occurrence: NOW + 3600,
                series: true,
                snooze_until: Some(s.clock.now + 300),
            },
        ]
    );
    assert!(p.events.iter().all(|e| !e.dirty), "the events themselves don't change");
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)), "the server is told at once");
    // The same occurrence answered again replaces its answer.
    s.clock.now += 300;
    let daily = key_of(&s, "daily");
    s.run("reminders.dismiss", &json!({"key": daily})).unwrap();
    let answers = &s.pim.get(&id).unwrap().reminder_answers;
    assert_eq!(answers.len(), 2);
    assert_eq!(answers.iter().find(|a| a.uid == "daily").and_then(|a| a.snooze_until), None);
    // The server took them.
    let sync = DavSync { answered: vec![("one".into(), NOW + 3600), ("daily".into(), NOW + 3600)], checked: NOW, ..Default::default() };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    assert!(s.pim.get(&id).unwrap().reminder_answers.is_empty());
    assert!(due(&s).is_empty(), "still dismissed here");
}

/// On a CalDAV account a dismissal is written into the event, as RFC 9074 says (its alarm
/// `ACKNOWLEDGED`), and the event goes to the server, unchanged otherwise; a snooze stays here.
/// An event another program dismissed that way isn't due here either.
#[test]
fn a_dismissal_on_caldav_acknowledges_the_alarm() {
    let (mut s, id) =
        account_with("reminder-caldav", vec![with_reminder("one", "/cal/one.ics", false), with_reminder("two", "/cal/two.ics", false)], false);
    s.clock.now = NOW + 3600 - 10 * 60;
    let (one, two) = (key_of(&s, "one"), key_of(&s, "two"));
    s.run("reminders.snooze", &json!({"key": two, "minutes": 5})).unwrap();
    assert!(!s.find_event("two").unwrap().1.dirty, "a snooze isn't the server's");
    s.run("reminders.dismiss", &json!({"key": one})).unwrap();
    let (_, e) = s.find_event("one").unwrap();
    assert!(e.dirty);
    assert_eq!(ical::acknowledged(&e.event), Some(s.clock.now));
    assert_eq!((e.event.sequence, ical::reminder_minutes(&e.event)), (0, Some(15)), "not a change to the meeting");
    let written = ical::write_for_server(std::slice::from_ref(&e.event), NOW);
    assert!(written.contains("BEGIN:VALARM") && written.contains("ACKNOWLEDGED:"), "{written}");
    assert!(s.pim.get(&id).unwrap().reminder_answers.is_empty());
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)));

    // Dismissed in another program: its copy comes acknowledged after the alarm, and isn't due.
    let mut elsewhere = with_reminder("three", "/cal/three.ics", false);
    ical::acknowledge(&mut elsewhere.event, NOW + 3600 - 14 * 60);
    let mut early = with_reminder("four", "/cal/four.ics", false);
    ical::acknowledge(&mut early.event, NOW - 86_400);
    let sync = DavSync {
        calendars: s.pim.get(&id).map(|p| p.calendars.clone()),
        events: vec![elsewhere, early],
        synced_calendars: vec![CAL.into()],
        kept: vec!["/cal/one.ics".into(), "/cal/two.ics".into()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id, sync: Box::new(sync) });
    let now_due = due(&s);
    assert!(!now_due.contains(&"three".to_string()), "{now_due:?}");
    assert!(now_due.contains(&"four".to_string()), "acknowledged before this alarm: still due ({now_due:?})");
}

/// Acknowledging replaces an alarm's earlier `ACKNOWLEDGED`, takes out snooze alarms, leaves the
/// rest of the event as written, and survives a damaged event.
#[test]
fn acknowledging_an_alarm_keeps_the_rest() {
    let data = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTAMP:20261031T080000Z\r\nDTSTART:20261104T090000Z\r\n\
                DTEND:20261104T100000Z\r\nSUMMARY:Planning\r\nX-KEPT:yes\r\nBEGIN:VALARM\r\nUID:a1\r\nACTION:DISPLAY\r\nTRIGGER:-PT10M\r\n\
                ACKNOWLEDGED:20261101T000000Z\r\nEND:VALARM\r\nBEGIN:VALARM\r\nUID:a2\r\nACTION:DISPLAY\r\nRELATED-TO;RELTYPE=SNOOZE:a1\r\n\
                TRIGGER;VALUE=DATE-TIME:20261104T085500Z\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let mut e = ical::parse(data, 0).events.into_iter().next().unwrap();
    assert_eq!(ical::acknowledged(&e).map(|t| t % 86_400), Some(0));
    ical::acknowledge(&mut e, NOW);
    let acks = e.extra.iter().filter(|l| l.starts_with("ACKNOWLEDGED")).count();
    assert_eq!(acks, 1, "{:?}", e.extra);
    assert_eq!(ical::acknowledged(&e), Some(NOW));
    assert!(e.extra.iter().any(|l| l == "X-KEPT:yes") && !e.extra.iter().any(|l| l.contains("SNOOZE")), "{:?}", e.extra);
    assert_eq!(ical::reminder_minutes(&e), Some(10));
    // An alarm never closed is kept as it was; no alarm, nothing to do.
    let mut broken = CalEvent { extra: vec!["BEGIN:VALARM".into(), "TRIGGER:-PT5M".into()], ..Default::default() };
    ical::acknowledge(&mut broken, NOW);
    assert_eq!(broken.extra, ["BEGIN:VALARM", "TRIGGER:-PT5M"]);
    let mut none = CalEvent::default();
    ical::acknowledge(&mut none, NOW);
    assert!(none.extra.is_empty() && ical::acknowledged(&none).is_none());
}
