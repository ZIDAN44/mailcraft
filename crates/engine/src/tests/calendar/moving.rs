//! Issue #192: an event dragged to another time or day moves, all of it, as long as it was; a
//! meeting the user organizes tells its guests; what can't be moved says why.

use super::*;

fn made(s: &mut Session, p: Value) -> String {
    s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string()
}

fn when(s: &Session, uid: &str) -> (i64, i64) {
    let e = &s.find_event(uid).unwrap().1.event;
    (e.start, e.end)
}

/// How many invitations to `subject` the sample account has sent (it files them in Sent).
fn invitations(s: &Session, subject: &str) -> usize {
    let sent = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    sent.messages.iter().filter(|m| m.subject == format!("Invitation: {subject}") || m.subject == format!("Updated: {subject}")).count()
}

#[test]
fn an_event_moves_to_another_time_and_day_as_long_as_it_was() {
    let mut s = sample_session("move-event");
    let uid = made(&mut s, json!({"summary": "Colour grade", "start": "2026-10-21T10:15", "end": "2026-10-21T11:45"}));
    let (start, end) = when(&s, &uid);
    let version = s.find_event(&uid).unwrap().1.event.sequence;
    // A day later, half an hour earlier.
    s.run("event.move", &json!({"uid": uid, "by": 86_400 - 1800})).unwrap();
    assert_eq!(when(&s, &uid), (start + 86_400 - 1800, end + 86_400 - 1800));
    assert!(s.find_event(&uid).unwrap().1.event.sequence > version, "a new version of it");
    // Back, earlier.
    s.run("event.move", &json!({"uid": uid, "by": -(86_400 - 1800)})).unwrap();
    assert_eq!(when(&s, &uid), (start, end));
    // Shown where it went.
    let shown = s.agenda(start - 3600, end + 3600);
    assert!(shown.iter().any(|i| i.uid == uid && i.start == start && i.end == end));
    // Not moved at all: nothing changes.
    let version = s.find_event(&uid).unwrap().1.event.sequence;
    s.run("event.move", &json!({"uid": uid, "by": 0})).unwrap();
    assert_eq!(s.find_event(&uid).unwrap().1.event.sequence, version);
    for bad in [
        json!({"uid": uid}),
        json!({"uid": uid, "by": "soon"}),
        json!({"uid": "nope", "by": 60}),
        json!({"by": 60}),
        json!({"uid": uid, "by": i64::MAX}),
    ] {
        assert!(s.run("event.move", &bad).is_err(), "{bad}");
    }
    assert_eq!(when(&s, &uid), (start, end));
}

#[test]
fn an_all_day_event_moves_by_whole_days() {
    let mut s = sample_session("move-all-day");
    let uid = made(&mut s, json!({"summary": "Festival", "start": "2026-10-21", "end": "2026-10-22", "allDay": true}));
    let (start, end) = when(&s, &uid);
    assert!(s.run("event.move", &json!({"uid": uid, "by": 3600})).is_err(), "not by an hour");
    s.run("event.move", &json!({"uid": uid, "by": -2 * 86_400})).unwrap();
    assert_eq!(when(&s, &uid), (start - 2 * 86_400, end - 2 * 86_400));
    assert!(s.find_event(&uid).unwrap().1.event.all_day);
}

#[test]
fn a_meeting_the_user_organizes_tells_its_guests() {
    let mut s = sample_session("move-meeting");
    let uid = made(&mut s, json!({"summary": "Trailer review", "start": "2026-10-21T15:00", "attendees": "Zoe Park <zoe@studio.example>"}));
    let before = invitations(&s, "Trailer review");
    assert_eq!(before, 1, "invited");
    let (start, _) = when(&s, &uid);
    s.run("event.move", &json!({"uid": uid, "by": 3600})).unwrap();
    assert_eq!(invitations(&s, "Trailer review"), 2, "the guests are told");
    // An update, said so (issue #235).
    let (method, raw) = sent_calendar(&s, "Sent", "Updated: Trailer review");
    assert_eq!(method, "REQUEST");
    assert!(raw.contains("zoe@studio.example"));
    assert_eq!(when(&s, &uid).0, start + 3600);
}

#[test]
fn what_cant_be_moved_says_why() {
    let mut s = sample_session("move-refused");
    // One that repeats, as a whole: its occurrences are dragged one at a time (issue #241).
    let weekly = made(&mut s, json!({"summary": "Stand-up", "start": "2026-10-21T09:00", "repeat": "weekly"}));
    let before = when(&s, &weekly);
    let e = s.run("event.move", &json!({"uid": weekly, "by": 3600})).unwrap_err();
    assert!(e.to_string().contains("repeating"), "{e}");
    assert_eq!(when(&s, &weekly), before);
    let items = s.agenda(before.0 - 3600, before.0 + 8 * 86_400);
    assert!(items.iter().filter(|i| i.uid == weekly).all(|i| i.recurring && s.movable(i)), "each occurrence dragged");
    // A meeting someone else organized.
    let mut invited = s.find_event(&weekly).unwrap().1.event.clone();
    invited.uid = "invited@soundlab.example".into();
    invited.rrule = None;
    invited.organizer = Some(mailcraft_model::ical::Attendee { email: "ana@soundlab.example".into(), name: "Ana".into(), ..Default::default() });
    invited.attendees = vec![mailcraft_model::ical::Attendee {
        email: "riley@studio.example".into(),
        name: "Riley".into(),
        partstat: "ACCEPTED".into(),
        rsvp: false,
        ..Default::default()
    }];
    let account = s.find_event(&weekly).unwrap().0.to_string();
    let calendar = s.find_event(&weekly).unwrap().1.calendar.clone();
    s.create_event(Some(&account), Some(&calendar), invited.clone()).unwrap();
    let e = s.run("event.move", &json!({"uid": invited.uid, "by": 3600})).unwrap_err();
    assert!(e.to_string().contains("organizer"), "{e}");
    // The user's own, alone: movable.
    let own = made(&mut s, json!({"summary": "Lunch", "start": "2026-10-22T12:00"}));
    let (start, end) = when(&s, &own);
    let items = s.agenda(start - 1, end + 1);
    let item = items.iter().find(|i| i.uid == own).unwrap();
    assert!(s.movable(item));
    // A task due that day isn't an event to move.
    assert!(!s.movable(&crate::AgendaItem { task: true, ..item.clone() }));
}
