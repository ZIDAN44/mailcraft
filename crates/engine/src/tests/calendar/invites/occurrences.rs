//! Issue #241: one occurrence of a series changed, moved or answered on its own, as Outlook's
//! "this occurrence": kept as a changed occurrence of the same event, the series' others as they
//! are; the guests (or the organizer) told of that one alone.

use super::*;

/// The occurrences of `uid` from `from` for `weeks` weeks: (start, summary).
fn shown(s: &Session, uid: &str, from: i64, weeks: i64) -> Vec<(i64, String)> {
    s.agenda(from, from + weeks * WEEK).into_iter().filter(|a| a.uid == uid).map(|a| (a.start, a.summary)).collect()
}

/// A weekly stand-up the user organizes, with Zoe invited, in the sample's own calendar: its uid
/// and its first occurrence.
fn my_series(s: &mut Session) -> (String, i64) {
    let r = s
        .run(
            "event.create",
            &json!({"summary": "Stand-up", "start": "2026-10-21T09:00", "repeat": "weekly", "attendees": "Zoe Park <zoe@studio.example>"}),
        )
        .unwrap();
    let uid = r["uid"].as_str().unwrap().to_string();
    let first = s.find_event(&uid).unwrap().1.event.start;
    (uid, first)
}

#[test]
fn one_occurrence_is_changed_on_its_own() {
    let mut s = sample_session("occurrence-change");
    let (uid, first) = my_series(&mut s);
    let second = first + WEEK;
    s.run(
        "event.update",
        &json!({"uid": uid, "occurrence": second, "summary": "Stand-up (Room 2)", "location": "Room 2", "start": "2026-10-28T10:00", "end": "2026-10-28T10:30"}),
    )
    .unwrap();
    // The series as it was; that week alone changed, at its new time.
    let m = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((m.summary.as_str(), m.rrule.as_deref(), m.start), ("Stand-up", Some("FREQ=WEEKLY"), first));
    let weeks = shown(&s, &uid, first, 3);
    assert_eq!(weeks.len(), 3, "{weeks:?}");
    assert_eq!(weeks[0], (first, "Stand-up".to_string()));
    assert_eq!(weeks[1], (second + 3600, "Stand-up (Room 2)".to_string()));
    assert_eq!(weeks[2], (first + 2 * WEEK, "Stand-up".to_string()));
    let own = parts(&s, &uid).into_iter().find(|e| e.event.recurrence_id == Some(second)).expect("kept as a changed occurrence");
    assert!(own.event.rrule.is_none() && own.event.exdates.is_empty());
    assert_eq!(own.event.sequence, m.sequence + 1, "a new version of that occurrence");
    // Zoe is sent that occurrence alone: one event, naming it.
    let (method, raw) = sent_calendar(&s, "Sent", "Updated: Stand-up (Room 2)");
    assert_eq!(method, "REQUEST");
    let cal = mailcraft_model::ical::parse(&raw[raw.find("BEGIN:VCALENDAR").unwrap()..], 0);
    assert_eq!(cal.events.len(), 1, "{raw}");
    assert_eq!(cal.events[0].recurrence_id, Some(second));
    // Changed again, the same part; saved unchanged, nothing new and nobody told.
    s.run("event.update", &json!({"uid": uid, "occurrence": second, "location": "Room 3"})).unwrap();
    assert_eq!(parts(&s, &uid).len(), 2);
    let sent = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap().messages.len();
    s.run("event.update", &json!({"uid": uid, "occurrence": first + 2 * WEEK, "summary": "Stand-up"})).unwrap();
    assert_eq!(parts(&s, &uid).len(), 2, "nothing kept for an occurrence saved unchanged");
    assert_eq!(s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap().messages.len(), sent);
}

#[test]
fn one_occurrence_is_moved_and_the_series_keeps_it() {
    let mut s = sample_session("occurrence-move");
    let (uid, first) = my_series(&mut s);
    let third = first + 2 * WEEK;
    // Dragged a day on and an hour later: that occurrence alone.
    let items = s.agenda(third, third + 3600);
    let it = items.iter().find(|a| a.uid == uid).unwrap();
    assert!(s.movable(it), "a repeating event's occurrence is dragged");
    s.run("event.move", &json!({"uid": uid, "occurrence": third, "by": 86_400 + 3600})).unwrap();
    let weeks = shown(&s, &uid, first, 4);
    assert_eq!(weeks.iter().map(|w| w.0).collect::<Vec<_>>(), [first, first + WEEK, third + 86_400 + 3600, first + 3 * WEEK]);
    // Moved again from where it is now.
    s.run("event.move", &json!({"uid": uid, "occurrence": third, "by": -3600})).unwrap();
    assert!(shown(&s, &uid, first, 4).iter().any(|w| w.0 == third + 86_400));
    // The whole series still moves only in its form; moved there, the changed occurrence keeps
    // to its own time (issue #100).
    assert!(s.run("event.move", &json!({"uid": uid, "by": 3600})).is_err());
    s.run("event.update", &json!({"uid": uid, "start": "2026-10-21T08:00", "end": "2026-10-21T08:30"})).unwrap();
    let weeks = shown(&s, &uid, first - 3600, 4);
    assert!(weeks.iter().any(|w| w.0 == third + 86_400), "{weeks:?}");
    assert_eq!(weeks.len(), 4);
}

#[test]
fn only_an_occurrence_the_series_has_is_changed() {
    let mut s = sample_session("occurrence-refused");
    let (uid, first) = my_series(&mut s);
    for at in [first + 3600, first - WEEK, first + WEEK / 2] {
        let e = s.run("event.update", &json!({"uid": uid, "occurrence": at, "summary": "x"})).unwrap_err();
        assert!(e.to_string().contains("occurrence"), "{at}: {e}");
    }
    // One taken out (deleted) isn't there to change.
    s.run("event.delete", &json!({"uid": uid, "occurrence": first + WEEK})).unwrap();
    assert!(s.run("event.update", &json!({"uid": uid, "occurrence": first + WEEK, "summary": "x"})).is_err());
    assert!(s.run("event.respond", &json!({"uid": uid, "occurrence": first + WEEK, "answer": "declined"})).is_err());
    assert_eq!(parts(&s, &uid).len(), 1, "nothing kept for the refused");
    // An event that doesn't repeat: the occurrence named is the event.
    let once = s.run("event.create", &json!({"summary": "Once", "start": "2026-10-22T09:00"})).unwrap()["uid"].as_str().unwrap().to_string();
    let at = s.find_event(&once).unwrap().1.event.start;
    s.run("event.update", &json!({"uid": once, "occurrence": at, "summary": "Once, renamed"})).unwrap();
    assert_eq!(s.find_event(&once).unwrap().1.event.summary, "Once, renamed");
    assert_eq!(parts(&s, &once).len(), 1);
}

/// A guest answers one occurrence from the calendar: their answer in that occurrence alone, kept
/// in the series' item for the server, the organizer told of it (a REPLY naming it); the
/// series' other occurrences keep the answer they had.
#[test]
fn one_occurrence_is_answered_on_its_own() {
    let mut s = sample_session("occurrence-answer");
    let master = weekly_series(&mut s);
    let uid = master.uid.clone();
    let week3 = master.start + 3 * WEEK;
    s.run("event.respond", &json!({"uid": uid, "occurrence": week3, "answer": "declined"})).unwrap();
    let mine = |e: &CalEvent| e.attendees.iter().find(|a| a.email == "riley@studio.example").map(|a| a.partstat.clone());
    let own = parts(&s, &uid).into_iter().find(|e| e.event.recurrence_id == Some(week3)).expect("kept as a changed occurrence");
    assert_eq!(mine(&own.event).as_deref(), Some("DECLINED"));
    assert!(own.dirty, "to the server");
    assert_eq!(own.event.start, week3, "at its time");
    assert_eq!(mine(&s.find_event(&uid).unwrap().1.event).as_deref(), Some("ACCEPTED"), "the series keeps its answer");
    let week1 = parts(&s, &uid).into_iter().find(|e| e.event.recurrence_id == Some(master.start + WEEK)).unwrap();
    assert_eq!(mine(&week1.event).as_deref(), Some("ACCEPTED"));
    // The organizer is told of that occurrence alone.
    let notices = &s.pim["sample"].notices;
    let reply = notices.iter().find(|n| n.method == "REPLY").map(|n| n.event.clone());
    let sent = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    let mailed = sent.messages.iter().find(|m| m.subject.starts_with("Declined")).and_then(|m| s.message_calendar(&m.key));
    let told = reply.or_else(|| mailed.and_then(|c| c.events.into_iter().next())).expect("a reply, to send or sent");
    assert_eq!(told.recurrence_id, Some(week3));
    // Accepted again: the same part.
    s.run("event.respond", &json!({"uid": uid, "occurrence": week3, "answer": "accepted"})).unwrap();
    assert_eq!(parts(&s, &uid).iter().filter(|e| e.event.recurrence_id == Some(week3)).count(), 1);
    assert!(s.run("event.respond", &json!({"uid": uid, "occurrence": week3, "answer": "maybe"})).is_err());
}
