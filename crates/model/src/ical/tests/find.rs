//! Issue #33: the calendar's search box, read and matched against events.

use super::*;
use crate::dates::days_from_civil;

fn board() -> CalEvent {
    CalEvent {
        uid: "b".into(),
        summary: "Board meeting".into(),
        location: "Room 2.14, Main building".into(),
        description: "Budget for 2027 and the new studio".into(),
        organizer: Some(Attendee { email: "ana@studio.example".into(), name: "Ana Lima".into(), ..Default::default() }),
        attendees: vec![Attendee { email: "jun.park@studio.example".into(), name: "Jun Park".into(), ..Default::default() }],
        categories: vec!["Red category".into(), "Clients".into()],
        ..Default::default()
    }
}

fn hit(q: &str) -> bool {
    EventQuery::parse(q).matches(&board())
}

#[test]
fn words_and_fields_find_an_event() {
    // Anywhere, as word starts.
    for q in ["board", "BOARD meet", "budg", "2.14", "jun", "park@studio", "clients", "studio.example"] {
        assert!(hit(q), "{q}");
    }
    for q in ["oard", "dinner", "board dinner"] {
        assert!(!hit(q), "{q}");
    }
    // Each field where it looks.
    for (q, yes) in [
        ("location:2.14", true),
        ("where:main", true),
        ("location:board", false),
        ("subject:board", true),
        ("title:budget", false),
        ("notes:budget", true),
        ("organizer:ana", true),
        ("from:lima", true),
        ("from:jun", false),
        ("attendee:jun", true),
        ("to:ana", false),
        ("with:park", true),
        ("category:red", true),
        ("category:blue", false),
        ("organizer: ana", true),
    ] {
        assert_eq!(hit(q), yes, "{q}");
    }
    // A list is any of them; a phrase as written; `-` turns a term round.
    assert!(hit("organizer:zoe,ana") && !hit("organizer:zoe,ben"));
    assert!(hit("\"board meeting\"") && !hit("\"meeting board\""));
    assert!(hit("-dinner") && !hit("-board"));
    assert!(hit("location:\"room 2.14\""));
}

#[test]
fn dates_and_kinds_narrow_it() {
    let q = EventQuery::parse("board after:2026-10-01 before:2026/11/01");
    assert_eq!(q.after, Some(days_from_civil(2026, 10, 1)));
    assert_eq!(q.before, Some(days_from_civil(2026, 11, 1)));
    assert!(q.matches(&board()), "the dates pick occurrences, not events");
    // Turned round: not after is before.
    let q = EventQuery::parse("-after:20261001");
    assert_eq!((q.after, q.before), (None, Some(days_from_civil(2026, 10, 1))));
    // Two of the same: the narrower.
    let q = EventQuery::parse("after:2026-01-01 after:2026-06-01 before:2027-01-01 before:2026-12-01");
    assert_eq!((q.after, q.before), (Some(days_from_civil(2026, 6, 1)), Some(days_from_civil(2026, 12, 1))));
    // A date that isn't one is looked for as words.
    assert!(!EventQuery::parse("after:soon").matches(&board()));
    assert!(EventQuery::parse("after:2026-13-45").after.is_none());
    let mut private = board();
    assert!(!EventQuery::parse("is:private").matches(&private));
    set_privacy(&mut private, Privacy::Confidential);
    assert!(EventQuery::parse("is:private").matches(&private));
    assert!(EventQuery::parse("-is:private").matches(&board()));
    let series = CalEvent { rrule: Some("FREQ=WEEKLY".into()), ..board() };
    assert!(EventQuery::parse("is:recurring board").matches(&series) && !EventQuery::parse("is:recurring").matches(&board()));
}

#[test]
fn odd_queries_dont_break_it() {
    assert!(EventQuery::parse("").is_empty() && EventQuery::parse("   ").is_empty());
    assert!(!EventQuery::parse("after:2026-10-01").is_empty());
    for q in ["\"", "\"\"", ":", "-", "--", "location:", "location:,,,", "is:", "-is:", "after:", "@", "\u{0}", "📅", "ü:ö", "a:b:c"] {
        let parsed = EventQuery::parse(q);
        let _ = parsed.matches(&board());
        let _ = parsed.matches(&CalEvent::default());
    }
    assert!(EventQuery::parse("@").matches(&board()), "punctuation alone is looked for as written");
    // Huge: a few dozen terms at most, long notes looked through only so far.
    let long = "word ".repeat(10_000);
    assert!(EventQuery::parse(&long).terms.len() <= 64);
    let mut big = board();
    big.description = format!("{}needle", "x ".repeat(100_000));
    assert!(!EventQuery::parse("needle").matches(&big), "past what's looked through");
    big.description = "needle ".to_string() + &"x ".repeat(100_000);
    assert!(EventQuery::parse("needle").matches(&big));
}

/// Issue #25 with #33: an online meeting only its server (Teams' join link) or a `CONFERENCE`
/// line names is found by its link and its name, in the place.
#[test]
fn an_online_meeting_outside_the_place_and_notes_is_found() {
    let teams = CalEvent { summary: "Sync".into(), join_url: "https://teams.microsoft.com/l/meetup-join/19%3a".into(), ..Default::default() };
    let room = CalEvent {
        summary: "Seminar".into(),
        extra: vec!["CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=\"Lecture hall\":https://meet.example.edu/rooms/abc-def-ghi".into()],
        ..Default::default()
    };
    let m = |q: &str, e: &CalEvent| EventQuery::parse(q).matches(e);
    assert!(m("teams.microsoft.com", &teams) && m("location:teams", &teams));
    for q in ["meet.example.edu", "location:\"rooms/abc-def-ghi\"", "where:lecture", "https://meet.example.edu/rooms/abc-def-ghi"] {
        assert!(m(q, &room), "{q}");
    }
    assert!(!m("notes:meet.example.edu", &room), "not in its notes");
}

/// A three-word search over a big calendar: each event's text built once, not once per term.
#[test]
#[ignore = "a measurement"]
fn measure_a_search_over_a_big_calendar() {
    let notes = "Agenda: budget, hiring and the studio move. ".repeat(40);
    let events: Vec<CalEvent> = (0..20_000)
        .map(|i| CalEvent {
            uid: format!("e{i}"),
            summary: format!("Meeting {i}"),
            description: notes.clone(),
            attendees: (0..5)
                .map(|k| Attendee { email: format!("p{k}@studio.example"), name: format!("Person {k}"), ..Default::default() })
                .collect(),
            ..board()
        })
        .collect();
    let q = EventQuery::parse("budget studio zebra");
    let t = std::time::Instant::now();
    let mut found = 0;
    for _ in 0..5 {
        found = events.iter().filter(|e| q.matches(e)).count();
    }
    println!("MEASURE three words over 20,000 events: {:.1} ms ({found} found)", t.elapsed().as_secs_f64() * 1000.0 / 5.0);
}
