//! An event's several reminders (issue #224): Google's calendar gives an event up to five
//! alarms ("an hour before" and "2 minutes before"); each is read, kept as written until the
//! reminders are changed, and set.

use super::*;

/// As Google's calendar writes one: two alarms, the second an email one (still a reminder here).
fn google() -> CalEvent {
    let text = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:g1\r\nDTSTART:20261123T100000Z\r\nDTEND:20261123T110000Z\r\n\
                SUMMARY:Standup\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:This is an event reminder\r\nTRIGGER:-P0DT0H2M0S\r\n\
                END:VALARM\r\nBEGIN:VALARM\r\nACTION:EMAIL\r\nDESCRIPTION:This is an event reminder\r\nSUMMARY:Alarm notification\r\n\
                ATTENDEE:mailto:me@example.org\r\nTRIGGER:-P0DT1H0M0S\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    parse(text, 0).events.into_iter().next().unwrap_or_default()
}

#[test]
fn every_alarm_is_a_reminder_the_earliest_first() {
    let e = google();
    assert_eq!(reminders_minutes(&e), [60, 2]);
    assert_eq!(reminder_minutes(&e), Some(2), "the first alarm, as before");
    // Twice the same, one relative to the end, one that can't be read: each once, the others not.
    let lines = [
        "BEGIN:VALARM",
        "TRIGGER:-PT15M",
        "END:VALARM",
        "BEGIN:VALARM",
        "TRIGGER:-PT15M",
        "END:VALARM",
        "BEGIN:VALARM",
        "TRIGGER;RELATED=END:-PT5M",
        "END:VALARM",
        "BEGIN:VALARM",
        "TRIGGER:soon",
        "END:VALARM",
    ];
    let odd = CalEvent { start: 1_800_000_000, extra: lines.iter().map(|l| l.to_string()).collect(), ..Default::default() };
    assert_eq!(reminders_minutes(&odd), [15]);
    assert!(reminders_minutes(&CalEvent::default()).is_empty());
    // Many: held to the most read.
    let many: Vec<String> = (1..=40).flat_map(|m| ["BEGIN:VALARM".to_string(), format!("TRIGGER:-PT{m}M"), "END:VALARM".to_string()]).collect();
    let many = CalEvent { extra: many, ..Default::default() };
    assert_eq!(reminders_minutes(&many).len(), REMINDERS_MAX);
    assert_eq!(reminders_minutes(&many).first(), Some(&40));
}

#[test]
fn reminders_set_and_kept_as_written_when_the_same() {
    let mut e = google();
    let before = e.extra.clone();
    // The same ones, in any order and twice over: its alarms stay as written (an email one too).
    set_reminders(&mut e, &[2, 60, 60]);
    assert_eq!(e.extra, before);
    // Changed: one display alarm for each, the earliest first; then none.
    set_reminders(&mut e, &[15, 1440, 15]);
    assert_eq!(reminders_minutes(&e), [1440, 15]);
    assert_eq!(e.extra.iter().filter(|l| *l == "BEGIN:VALARM").count(), 2);
    assert!(!e.extra.iter().any(|l| l.contains("EMAIL")));
    set_reminders(&mut e, &[-5, i64::MAX]);
    assert_eq!(reminders_minutes(&e), [REMINDER_MAX_MINUTES, 0], "held to what a reminder can be");
    set_reminders(&mut e, &[]);
    assert!(reminders_minutes(&e).is_empty() && reminder_minutes(&e).is_none());
    // Through a write and a read.
    set_reminders(&mut e, &[60, 2]);
    let back = parse(&write(&[e], None, 1_800_000_000), 0).events;
    assert_eq!(back.first().map(reminders_minutes), Some(vec![60, 2]));
}
