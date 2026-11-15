//! A series moved: where its taken-out and changed occurrences go (issue #100).

use super::*;

/// 18:10 in Berlin on a day of November 2026 (UTC+1).
fn berlin(day: u32, h: i64, m: i64) -> i64 {
    days_from_civil(2026, 11, day) * 86_400 + (h - 1) * 3600 + m * 60
}

fn daily(start: i64) -> CalEvent {
    CalEvent { uid: "s".into(), start, end: start + 1800, tzid: "Europe/Berlin".into(), rrule: Some("FREQ=DAILY".into()), ..Default::default() }
}

/// The report's series: daily at 18:10, Wednesday's taken out, then the series moved to 18:20.
/// The occurrence taken out moves with it, so it stays out; the others are where they were.
#[test]
fn a_series_moved_later_takes_its_occurrences_along() {
    let old = daily(berlin(16, 18, 10));
    let new = daily(berlin(16, 18, 20));
    assert_eq!(moved_occurrences(&old, &new, &[berlin(18, 18, 10), berlin(20, 18, 10)]), [Some(berlin(18, 18, 20)), Some(berlin(20, 18, 20))]);
    // Nothing moved: the times stay as they are, one that's no occurrence too.
    assert_eq!(moved_occurrences(&old, &old.clone(), &[berlin(18, 18, 10), 5]), [Some(berlin(18, 18, 10)), Some(5)]);
}

/// Moved to another day: the third occurrence is still the third, across the change from summer
/// time (an hour that a shift by the start's difference would get wrong).
#[test]
fn a_series_moved_to_another_day_keeps_each_occurrence_its_number_across_summer_time() {
    // Weekly on Saturdays at 09:00 in Berlin from 10 October 2026, moved to Sundays; summer time
    // (UTC+2) ends early on Sunday 25 October.
    let nine = |d: u32| days_from_civil(2026, 10, d) * 86_400 + if d < 25 { 7 } else { 8 } * 3600;
    let old = CalEvent { rrule: Some("FREQ=WEEKLY".into()), ..daily(nine(10)) };
    let new = CalEvent { rrule: Some("FREQ=WEEKLY".into()), ..daily(nine(11)) };
    let moved = moved_occurrences(&old, &new, &[nine(24), nine(17)]);
    assert_eq!(moved, [Some(nine(25)), Some(nine(18))]);
    assert_ne!(moved[0], Some(nine(24) + 86_400), "a day later by the clock is 25 hours later");
}

/// A series that now ends sooner (its count lowered): an occurrence past its end has nowhere to
/// go.
#[test]
fn an_occurrence_past_the_new_end_has_none() {
    let old = CalEvent { rrule: Some("FREQ=DAILY;COUNT=5".into()), ..daily(berlin(16, 18, 10)) };
    let new = CalEvent { rrule: Some("FREQ=DAILY;COUNT=3".into()), ..old.clone() };
    assert_eq!(moved_occurrences(&old, &new, &[berlin(17, 18, 10), berlin(19, 18, 10)]), [Some(berlin(17, 18, 10)), None]);
}

/// A series begun long ago, without an end: what it repeats is looked at up to its limit, and an
/// occurrence past that moves as the start does.
#[test]
fn a_long_series_is_followed_to_its_limit() {
    let old = CalEvent { tzid: String::new(), ..daily(0) };
    let new = CalEvent { start: 600, end: 2400, ..old.clone() };
    let far = (MAX_STARTS as i64 + 10) * 86_400;
    assert_eq!(moved_occurrences(&old, &new, &[86_400, far]), [Some(86_400 + 600), Some(far + 600)]);
}
