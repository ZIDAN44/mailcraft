//! The lines an event keeps as written (`extra`) are capped inside its components too, and its
//! reminder is kept within them, where every reader of them looks.

use super::*;

fn one(lines: &str) -> CalEvent {
    let text = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261022T090000Z\r\nDTEND:20261022T100000Z\r\n{lines}END:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    parse(&text, 0).events.into_iter().next().unwrap_or_default()
}

const ALARM: &str = "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nACKNOWLEDGED:20261022T084600Z\r\nEND:VALARM\r\n";

/// A component of 100,000 lines, and one nested 10,000 deep: each went into the event's kept
/// lines whole, saved with the calendar and read through each time it was drawn.
#[test]
fn a_components_lines_are_capped() {
    let big = format!("BEGIN:X-BIG\r\n{}END:X-BIG\r\n", "X-LINE:junk\r\n".repeat(100_000));
    let e = one(&big);
    assert!(e.extra.len() <= 400, "{}", e.extra.len());
    assert_eq!(e.extra.first().map(String::as_str), Some("BEGIN:X-BIG"));
    assert_eq!(e.extra.last().map(String::as_str), Some("END:X-BIG"));
    let deep = format!("{}{}", "BEGIN:X-IN\r\n".repeat(10_000), "END:X-IN\r\n".repeat(10_000));
    let e = one(&deep);
    assert!(e.extra.len() <= 800, "{}", e.extra.len());
    let begun = e.extra.iter().filter(|l| l.starts_with("BEGIN:")).count();
    assert_eq!(begun, e.extra.iter().filter(|l| l.starts_with("END:")).count());
    // What comes after them is still read.
    let e = one(&format!("{big}{deep}SUMMARY:Board\r\n"));
    assert_eq!(e.summary, "Board");
}

/// An alarm after a long component, or after hundreds of properties, is still the event's
/// reminder: the reminder was read from the first 400 lines only, while dismissing it rewrote
/// every alarm.
#[test]
fn an_alarm_after_many_lines_is_the_reminder() {
    let big = format!("BEGIN:X-BIG\r\n{}END:X-BIG\r\n", "X-LINE:junk\r\n".repeat(500));
    for before in [big, "X-PROP:1\r\n".repeat(300)] {
        let e = one(&format!("{before}{ALARM}"));
        assert_eq!(reminder_minutes(&e), Some(15), "{}", e.extra.len());
        assert_eq!(acknowledged(&e), Some(1_792_658_760));
        let mut set = e.clone();
        set_reminder(&mut set, Some(30));
        assert_eq!(reminder_minutes(&set), Some(30));
    }
}

/// A to-do's components were capped where they began, but those nested in a kept one weren't.
#[test]
fn a_todos_nested_components_are_capped() {
    let deep = format!("BEGIN:X-OUT\r\n{}{}END:X-OUT\r\n", "BEGIN:X-IN\r\n".repeat(10_000), "END:X-IN\r\n".repeat(10_000));
    let text = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:t\r\nSUMMARY:Report\r\n{deep}END:VTODO\r\nEND:VCALENDAR\r\n");
    let tasks = parse(&text, 0).tasks;
    let t = tasks.first().expect("the to-do");
    assert!(t.extra.len() <= 400, "{}", t.extra.len());
    let begun = t.extra.iter().filter(|l| l.starts_with("BEGIN:")).count();
    assert_eq!(begun, t.extra.iter().filter(|l| l.starts_with("END:")).count());
    assert_eq!(t.title, "Report");
}
