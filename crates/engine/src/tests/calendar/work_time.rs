//! Issue #185: the work time, the days the work week shows and the hours the calendar shades
//! outside of.

use mailcraft_model::dates::day_string;

use super::*;

/// The local day of `2026-10-24`.
fn day(d: &str) -> i64 {
    let n = |a: usize, b: usize| d[a..b].parse::<u32>().unwrap();
    mailcraft_model::dates::days_from_civil(i64::from(n(0, 4)), n(5, 7), n(8, 10))
}

fn listed(s: &Session) -> Vec<String> {
    s.cal_day_list().into_iter().map(day_string).collect()
}

#[test]
fn the_work_week_shows_the_days_worked() {
    let mut s = sample_session("work-time-days");
    // Saturday 2026-10-24.
    s.run("calendar.goto", &json!({"date": "2026-10-24"})).unwrap();
    s.run("calendar.view", &json!({"mode": "workWeek"})).unwrap();
    assert_eq!(listed(&s), ["2026-10-19", "2026-10-20", "2026-10-21", "2026-10-22", "2026-10-23"], "Monday to Friday to start with");

    // Sunday to Thursday: the week from the Sunday before.
    let r = s.run("calendar.workTime", &json!({"days": [6, 0, 1, 2, 3]})).unwrap();
    assert_eq!(r["days"], json!([0, 1, 2, 3, 6]));
    assert_eq!(listed(&s), ["2026-10-18", "2026-10-19", "2026-10-20", "2026-10-21", "2026-10-22"]);
    assert_eq!(s.cal_days(), (day("2026-10-18"), day("2026-10-23")));

    // Days apart: those alone, the range across them.
    s.run("calendar.workTime", &json!({"days": [0, 2, 4]})).unwrap();
    assert_eq!(listed(&s), ["2026-10-19", "2026-10-21", "2026-10-23"]);
    assert_eq!(s.cal_days(), (day("2026-10-19"), day("2026-10-24")));
    // What it shows is those days': not a Tuesday's event.
    let tuesday = s.local_midnight(day("2026-10-20")) + 10 * 3600;
    s.run("event.create", &json!({"summary": "On a Tuesday", "start": tuesday, "end": tuesday + 3600})).unwrap();
    let events = s.run("calendar.events", &json!({})).unwrap();
    assert!(!events.as_array().unwrap().iter().any(|e| e["summary"] == "On a Tuesday"), "{events}");
    let asked = s.run("calendar.events", &json!({"from": "2026-10-20", "to": "2026-10-20"})).unwrap();
    assert!(asked.as_array().unwrap().iter().any(|e| e["summary"] == "On a Tuesday"), "asked for: {asked}");
    s.run("calendar.goto", &json!({"step": 1})).unwrap();
    assert_eq!(listed(&s), ["2026-10-26", "2026-10-28", "2026-10-30"], "a week on");

    // The week view keeps all seven.
    s.run("calendar.view", &json!({"mode": "week"})).unwrap();
    assert_eq!(s.cal_day_list().len(), 7);
}

#[test]
fn the_work_hours_are_checked_and_kept() {
    let mut s = sample_session("work-time-hours");
    let r = s.run("calendar.workTime", &json!({"start": "09:30", "end": "18:00"})).unwrap();
    assert_eq!((r["start"].as_str(), r["end"].as_str()), (Some("09:30"), Some("18:00")));
    for bad in [
        json!({"days": []}),
        json!({"days": [7]}),
        json!({"days": "monday"}),
        json!({"start": "17:00", "end": "08:00"}),
        json!({"start": "25:00"}),
        json!({"end": "nine"}),
        json!({"start": "18:00"}),
    ] {
        assert!(s.run("calendar.workTime", &bad).is_err(), "{bad}");
    }
    let w = s.work_time();
    assert_eq!((w.start, w.end, w.days), (9 * 60 + 30, 18 * 60, mailcraft_model::pim::work_hours::MONDAY_TO_FRIDAY), "unchanged by those refused");
    let saved = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(saved["workTime"], json!({"days": 31, "start": 570, "end": 1080}));

    // The hours worked on a Monday, and none on the Saturday.
    let monday = s.local_midnight(day("2026-10-19"));
    assert_eq!(s.work_spans(monday, monday + 86_400), [(monday + 570 * 60, monday + 1080 * 60)]);
    let saturday = s.local_midnight(day("2026-10-24"));
    assert!(s.work_spans(saturday, saturday + 86_400).is_empty());
}
