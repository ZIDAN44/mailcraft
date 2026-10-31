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
