//! A recurring meeting moved as a whole (issue #100): its occurrences taken out stay out, its
//! changed ones stay the ones they were, and the guests hear of all of it.

use mailcraft_model::ical::{self, CalEvent};
use mailcraft_model::pim::{CalendarInfo, StoredEvent};

use super::*;

/// 18:10 in Berlin on a day of November 2026 (UTC+1).
fn berlin(day: u32, h: i64, m: i64) -> i64 {
    mailcraft_model::dates::days_from_civil(2026, 11, day) * 86_400 + (h - 1) * 3600 + m * 60
}

/// The report's series: the user's daily meeting at 18:10 in Berlin with a guest, in a calendar
/// on a server, its Wednesday (18 November) taken out and its Friday (20 November) changed
/// (`friday`: that occurrence's start and end). Thursday's (19 November) is an item of its own,
/// as Exchange keeps one.
fn series(name: &str, friday: (i64, i64)) -> Session {
    let mut s = sample_session(name);
    let me = s.account("sample").unwrap().config.email.clone();
    let p = s.pim_mut("sample");
    p.calendars.push(CalendarInfo { id: "CAL1".into(), name: "Work".into(), ..Default::default() });
    let master = CalEvent {
        uid: "daily".into(),
        summary: "Stand-up".into(),
        start: berlin(16, 18, 10),
        end: berlin(16, 18, 40),
        tzid: "Europe/Berlin".into(),
        rrule: Some("FREQ=DAILY;COUNT=10".into()),
        exdates: vec![berlin(18, 18, 10)],
        organizer: Some(ical::Attendee { email: me.clone(), ..Default::default() }),
        attendees: vec![
            ical::Attendee { email: me, partstat: "ACCEPTED".into(), ..Default::default() },
            ical::Attendee { email: "ana@soundlab.example".into(), partstat: "ACCEPTED".into(), ..Default::default() },
        ],
        ..Default::default()
    };
    let changed = |at: i64, (start, end): (i64, i64)| CalEvent {
        recurrence_id: Some(at),
        start,
        end,
        rrule: None,
        exdates: vec![],
        summary: "Stand-up (long)".into(),
        ..master.clone()
    };
    let item = |event: CalEvent, href: &str| StoredEvent { calendar: "CAL1".into(), href: href.into(), etag: "e".into(), event, dirty: false };
    p.events.extend([
        item(master.clone(), "S1"),
        item(changed(berlin(20, 18, 10), friday), "S1"),
        item(changed(berlin(19, 18, 10), (berlin(19, 18, 10), berlin(19, 18, 40))), "I2"),
    ]);
    s.take_jobs();
    s
}

fn parts(s: &Session) -> Vec<CalEvent> {
    s.pim["sample"].events.iter().filter(|e| e.event.uid == "daily").map(|e| e.event.clone()).collect()
}

/// The starts of the series' occurrences on a day of November.
fn on(s: &Session, day: u32) -> Vec<i64> {
    let events = parts(s);
    let from = berlin(day, 1, 0);
    ical::occurrences(&events, from, from + 86_400).into_iter().map(|o| o.start).collect()
}

/// The update the guests were sent (the sample account files it in Sent).
fn update(s: &Session) -> String {
    let (method, raw) = sent_calendar(s, "Sent", "Invitation: Stand-up");
    assert_eq!(method, "REQUEST");
    raw
}

/// The report: Wednesday taken out, the series moved ten minutes later. Wednesday stays out
/// (it came back, at the old time in the `EXDATE`), each other day has its one occurrence at
/// the new time, and Friday's change, at its occurrence's own time, moves with it.
#[test]
fn a_series_moved_keeps_its_occurrences_taken_out_and_changed() {
    let mut s = series("moving-later", (berlin(20, 18, 10), berlin(20, 18, 40)));
    s.run("event.update", &json!({"uid": "daily", "start": berlin(16, 18, 20)})).unwrap();
    assert_eq!(on(&s, 18), Vec::<i64>::new(), "Wednesday stays out");
    assert_eq!(on(&s, 17), [berlin(17, 18, 20)]);
    assert_eq!(on(&s, 20), [berlin(20, 18, 20)], "Friday's change, once, at the new time");
    let friday = parts(&s).into_iter().find(|e| e.recurrence_id == Some(berlin(20, 18, 20))).expect("Friday's change follows");
    assert_eq!((friday.summary.as_str(), friday.end), ("Stand-up (long)", berlin(20, 18, 50)));
    let master = parts(&s).into_iter().find(|e| e.recurrence_id.is_none()).unwrap();
    assert_eq!(master.exdates, [berlin(18, 18, 20)]);
    assert!(s.pim["sample"].events.iter().filter(|e| e.event.uid == "daily").all(|e| e.dirty), "the server learns of each");

    // The guests' update has the series as it is now, its changed occurrences with it: a
    // calendar takes the series as the message has it.
    let sent = update(&s);
    let unfolded = sent.replace("\r\n ", "");
    assert!(unfolded.contains("EXDATE;TZID=Europe/Berlin:20261118T182000"), "{sent}");
    assert_eq!(unfolded.matches("BEGIN:VEVENT").count(), 3, "{sent}");
    assert!(unfolded.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261120T182000"), "{sent}");
    assert!(unfolded.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261119T182000"), "{sent}");
}

/// An occurrence moved to a time of its own stays at that time: only which occurrence it
/// replaces follows the series.
#[test]
fn an_occurrence_moved_on_its_own_stays_where_it_was_put() {
    let mut s = series("moving-own-time", (berlin(20, 9, 0), berlin(20, 9, 30)));
    s.run("event.update", &json!({"uid": "daily", "start": berlin(16, 18, 20)})).unwrap();
    assert_eq!(on(&s, 20), [berlin(20, 9, 0)], "one that day, where it was put, not the series' too");
    let friday = parts(&s).into_iter().find(|e| e.summary == "Stand-up (long)" && e.start == berlin(20, 9, 0)).unwrap();
    assert_eq!(friday.recurrence_id, Some(berlin(20, 18, 20)));
}

/// A series shortened (its count lowered) by the move: the changed occurrence it no longer has
/// goes, and an item of its own is deleted on the server.
#[test]
fn a_changed_occurrence_the_series_no_longer_has_goes() {
    let mut s = series("moving-shorter", (berlin(20, 18, 10), berlin(20, 18, 40)));
    s.update_event("daily", |e| e.rrule = Some("FREQ=DAILY;COUNT=2".into())).unwrap();
    let left = parts(&s);
    assert_eq!(left.iter().filter_map(|e| e.recurrence_id).collect::<Vec<_>>(), Vec::<i64>::new(), "{left:?}");
    assert_eq!(s.pim["sample"].deleted.iter().map(|t| t.href.as_str()).collect::<Vec<_>>(), ["I2"]);
    assert!(left[0].exdates.is_empty(), "Wednesday is past the second");
}

/// Changing something else (its title) moves nothing.
#[test]
fn a_series_changed_otherwise_keeps_its_occurrences() {
    let mut s = series("moving-not", (berlin(20, 18, 10), berlin(20, 18, 40)));
    let before: Vec<Option<i64>> = parts(&s).iter().map(|e| e.recurrence_id).collect();
    s.run("event.update", &json!({"uid": "daily", "summary": "Daily stand-up"})).unwrap();
    assert_eq!(parts(&s).iter().map(|e| e.recurrence_id).collect::<Vec<_>>(), before);
    assert_eq!(parts(&s)[0].exdates, [berlin(18, 18, 10)]);
}
