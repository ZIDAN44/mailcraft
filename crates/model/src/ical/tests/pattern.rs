//! The event form's recurrence (issue #103): Outlook's choices as rules, and back.

use super::*;

/// 2026-11-19, a Thursday: the third of November, not the last.
fn thu() -> i64 {
    days_from_civil(2026, 11, 19)
}

/// Each of Outlook's choices (the issue's table) makes its rule, and reads back as itself.
#[test]
fn outlooks_choices_make_their_rules_and_read_back() {
    let day = thu();
    let weekly = |interval, days: &[u32]| Pattern { interval, days: days.to_vec(), ..Pattern::new(PatternFreq::Weekly) };
    for (p, rule) in [
        (Pattern { interval: 2, ..Pattern::new(PatternFreq::Daily) }, "FREQ=DAILY;INTERVAL=2"),
        (weekly(1, &[0, 1, 2, 3, 4]), "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
        (weekly(3, &[1, 2]), "FREQ=WEEKLY;INTERVAL=3;BYDAY=TU,WE"),
        (Pattern { interval: 2, ..Pattern::new(PatternFreq::Monthly) }, "FREQ=MONTHLY;INTERVAL=2"),
        (Pattern { nth: Some(3), ..Pattern::new(PatternFreq::Monthly) }, "FREQ=MONTHLY;BYDAY=3TH"),
        (Pattern::new(PatternFreq::Yearly), "FREQ=YEARLY"),
        (Pattern { nth: Some(3), ..Pattern::new(PatternFreq::Yearly) }, "FREQ=YEARLY;BYMONTH=11;BYDAY=3TH"),
        (Pattern { end: PatternEnd::Count(10), ..Pattern::new(PatternFreq::Daily) }, "FREQ=DAILY;COUNT=10"),
        (Pattern { end: PatternEnd::Until(days_from_civil(2026, 12, 31)), ..Pattern::new(PatternFreq::Daily) }, "FREQ=DAILY;UNTIL=20261231"),
    ] {
        assert_eq!(p.rule(day), rule);
        assert_eq!(Pattern::read(rule, day), Some(p), "{rule}");
    }
    // What the start says isn't said again: a weekly one on its weekday, a monthly or yearly one
    // on its day (as written elsewhere, it reads the same).
    assert_eq!(Pattern::new(PatternFreq::Weekly).rule(day), "FREQ=WEEKLY");
    assert_eq!(weekly(1, &[3]).rule(day), "FREQ=WEEKLY");
    assert_eq!(Pattern::read("FREQ=MONTHLY;BYMONTHDAY=19", day).map(|p| p.rule(day)).as_deref(), Some("FREQ=MONTHLY"));
    assert_eq!(Pattern::read("FREQ=YEARLY;BYMONTH=11;BYMONTHDAY=19", day).map(|p| p.rule(day)).as_deref(), Some("FREQ=YEARLY"));
}

/// The last Friday, and the plain rules the list always had, read too; an `UNTIL` in UTC reads
/// as its day.
#[test]
fn rules_from_elsewhere_read_when_the_form_can_show_them() {
    let last_friday = days_from_civil(2026, 11, 27);
    assert!(last_of_month(last_friday) && !last_of_month(thu()));
    assert_eq!(Pattern::read("FREQ=MONTHLY;BYDAY=-1FR", last_friday).map(|p| p.nth), Some(Some(-1)));
    assert_eq!(Pattern::read("FREQ=WEEKLY", thu()), Some(Pattern::new(PatternFreq::Weekly)));
    let until = Pattern::read("FREQ=DAILY;UNTIL=20261231T225959Z;WKST=MO", thu()).unwrap();
    assert_eq!(until.end, PatternEnd::Until(days_from_civil(2026, 12, 31)));
}

/// A rule saying more than the form shows, or something of another day, is kept as it is.
#[test]
fn rules_the_form_cant_show_are_kept() {
    for rule in [
        "FREQ=HOURLY",
        "FREQ=DAILY;BYHOUR=9,17",
        "FREQ=MONTHLY;BYMONTHDAY=1,15",
        "FREQ=MONTHLY;BYMONTHDAY=3",
        "FREQ=MONTHLY;BYDAY=2TH",
        "FREQ=MONTHLY;BYDAY=3FR",
        "FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=19",
        "FREQ=WEEKLY;BYDAY=XX",
        "FREQ=DAILY;COUNT=0",
        "FREQ=DAILY;BYSETPOS=1",
        "INTERVAL=2",
    ] {
        assert_eq!(Pattern::read(rule, thu()), None, "{rule}");
    }
}

/// An end date is the end of that day where the event is, in UTC, as RFC 5545 asks of a timed
/// event; an all-day one keeps the date.
#[test]
fn an_end_date_is_the_end_of_that_day_where_the_event_is() {
    let mut e =
        CalEvent { tzid: "Europe/Berlin".into(), start: thu() * 86_400, rrule: Some("FREQ=DAILY;UNTIL=20261231".into()), ..Default::default() };
    until_where_it_is(&mut e);
    assert_eq!(e.rrule.as_deref(), Some("FREQ=DAILY;UNTIL=20261231T225959Z"));
    let last = starts(&e, i64::MAX).last().copied().unwrap();
    assert_eq!(civil_from_days(last.div_euclid(86_400)), (2026, 12, 31));
    let mut all_day = CalEvent { all_day: true, rrule: Some("FREQ=DAILY;UNTIL=20261231".into()), ..Default::default() };
    until_where_it_is(&mut all_day);
    assert_eq!(all_day.rrule.as_deref(), Some("FREQ=DAILY;UNTIL=20261231"));
}
