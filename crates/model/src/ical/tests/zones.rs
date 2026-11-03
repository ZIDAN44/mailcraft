//! Events in named time zones: read, expanded at their wall-clock time (summer or winter),
//! written back with their zone; and rules that limit or pick among the days they make.

use super::*;
use crate::zone::Zone;

fn ny() -> Zone {
    Zone::named("America/New_York").unwrap()
}

fn cal(body: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n")
}

fn event(lines: &[&str]) -> String {
    format!("BEGIN:VEVENT\r\nUID:z1\r\n{}\r\nEND:VEVENT\r\n", lines.join("\r\n"))
}

fn starts_between(events: &[CalEvent], from: i64, to: i64) -> Vec<i64> {
    occurrences(events, from, to).iter().map(|o| o.start).collect()
}

/// The US rules, under a name no database knows: only the file's definition says what it is.
const STUDIO_EASTERN: &str = "BEGIN:VTIMEZONE\r\nTZID:Studio Eastern\r\n\
BEGIN:DAYLIGHT\r\nTZNAME:SEDT\r\nTZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\nDTSTART:19700308T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\r\nEND:DAYLIGHT\r\n\
BEGIN:STANDARD\r\nTZNAME:SEST\r\nTZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\nDTSTART:19701101T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\r\nEND:STANDARD\r\n\
END:VTIMEZONE\r\n";

/// 09:00 on Mondays in New York stays 09:00 there in July (13:00 UTC, not 14:00), with or
/// without the zone's definition in the file, and the event keeps its zone.
#[test]
fn a_weekly_meeting_keeps_its_wall_clock_time_in_summer() {
    let monday =
        event(&["DTSTART;TZID=America/New_York:20260105T090000", "DTEND;TZID=America/New_York:20260105T100000", "RRULE:FREQ=WEEKLY;BYDAY=MO"]);
    let e = parse(&cal(&monday), 0).events;
    assert_eq!(e[0].tzid, "America/New_York");
    assert_eq!(e[0].start, at(2026, 1, 5, 14, 0), "09:00 EST");
    assert_eq!(starts_between(&e, at(2026, 7, 6, 0, 0), at(2026, 7, 7, 0, 0)), [at(2026, 7, 6, 13, 0)], "09:00 EDT");
    assert_eq!(occurrences(&e, at(2026, 7, 6, 0, 0), at(2026, 7, 7, 0, 0))[0].end, at(2026, 7, 6, 14, 0));

    // A zone only the file defines.
    let custom = cal(&format!("{STUDIO_EASTERN}{}", monday.replace("America/New_York", "Studio Eastern")));
    let e = parse(&custom, 0).events;
    assert_eq!((e[0].tzid.as_str(), e[0].start), ("Studio Eastern", at(2026, 1, 5, 14, 0)));
    assert!(e[0].vtimezone.first().is_some_and(|l| l == "BEGIN:VTIMEZONE"), "{:?}", e[0].vtimezone);
    assert_eq!(starts_between(&e, at(2026, 7, 6, 0, 0), at(2026, 7, 7, 0, 0)), [at(2026, 7, 6, 13, 0)]);
    // Outlook's Windows names.
    let e = parse(&cal(&monday.replace("America/New_York", "Eastern Standard Time")), 0).events;
    assert_eq!(starts_between(&e, at(2026, 7, 6, 0, 0), at(2026, 7, 7, 0, 0)), [at(2026, 7, 6, 13, 0)]);
}

/// An occurrence deleted in the event's zone stays deleted, summer or winter; one moved in it
/// replaces the one it names.
#[test]
fn exceptions_match_in_the_events_zone() {
    let text = cal(&format!(
        "{}{}",
        event(&[
            "DTSTART;TZID=America/New_York:20260105T090000",
            "DTEND;TZID=America/New_York:20260105T100000",
            "RRULE:FREQ=WEEKLY;BYDAY=MO",
            "EXDATE;TZID=America/New_York:20260706T090000,20260112T090000",
        ]),
        event(&[
            "RECURRENCE-ID;TZID=America/New_York:20260713T090000",
            "DTSTART;TZID=America/New_York:20260713T110000",
            "DTEND;TZID=America/New_York:20260713T120000",
        ]),
    ));
    let e = parse(&text, 0).events;
    assert_eq!(starts_between(&e, at(2026, 7, 1, 0, 0), at(2026, 7, 21, 0, 0)), [at(2026, 7, 13, 15, 0), at(2026, 7, 20, 13, 0)]);
    assert_eq!(starts_between(&e, at(2026, 1, 5, 0, 0), at(2026, 1, 20, 0, 0)), [at(2026, 1, 5, 14, 0), at(2026, 1, 19, 14, 0)]);
}

/// Monday and Wednesday at 18:00 in Los Angeles are Tuesday and Thursday in UTC: the days are
/// the zone's.
#[test]
fn weekdays_are_the_events_own() {
    let e = parse(&cal(&event(&["DTSTART;TZID=America/Los_Angeles:20260105T180000", "DURATION:PT1H", "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=4"])), 0)
        .events;
    assert_eq!(starts(&e[0], i64::MAX), [at(2026, 1, 6, 2, 0), at(2026, 1, 8, 2, 0), at(2026, 1, 13, 2, 0), at(2026, 1, 15, 2, 0)]);
    let e = parse(&cal(&event(&["DTSTART;TZID=America/Los_Angeles:20260105T180000", "DURATION:PT1H", "RRULE:FREQ=WEEKLY;BYDAY=MO,WE"])), 0).events;
    assert_eq!(starts_between(&e, at(2026, 7, 6, 0, 0), at(2026, 7, 9, 0, 0)), [at(2026, 7, 7, 1, 0)], "Monday 18:00 PDT");
}

/// A date-only `UNTIL` is the end of that day where the event is; a UTC one is an instant.
#[test]
fn until_a_date_is_the_events_day() {
    let daily = |until: &str| {
        let e = parse(
            &cal(&event(&["DTSTART;TZID=America/Los_Angeles:20260105T180000", "DURATION:PT1H", &format!("RRULE:FREQ=DAILY;UNTIL={until}")])),
            0,
        )
        .events;
        starts(&e[0], i64::MAX).len()
    };
    assert_eq!(daily("20260107"), 3, "the 5th, 6th and 7th");
    assert_eq!(daily("20260108T015959Z"), 2);
    assert_eq!(daily("20260108T020000Z"), 3, "the last one is included");
}

/// A time that doesn't happen (the clocks go forward) is read an hour later, as RFC 5545 says;
/// a daily event over the change keeps its wall-clock time on either side.
#[test]
fn the_hour_that_doesnt_happen() {
    let e = parse(&cal(&event(&["DTSTART;TZID=America/New_York:20260307T023000", "DURATION:PT30M", "RRULE:FREQ=DAILY;COUNT=3"])), 0).events;
    assert_eq!(starts(&e[0], i64::MAX), [at(2026, 3, 7, 7, 30), at(2026, 3, 8, 7, 30), at(2026, 3, 9, 6, 30)]);
}

/// Times with no zone are the viewer's; a zone nobody defines falls back to the viewer's too.
#[test]
fn floating_and_unknown_zones_are_the_viewers() {
    let floating = cal(&event(&["DTSTART:20260706T090000", "DURATION:PT1H", "RRULE:FREQ=WEEKLY"]));
    let e = parse_in(&floating, ny()).events;
    assert_eq!(e[0].start, at(2026, 7, 6, 13, 0));
    assert_eq!(starts_between(&e, at(2026, 12, 28, 0, 0), at(2026, 12, 29, 0, 0)), [at(2026, 12, 28, 14, 0)], "09:00 EST in winter");
    let mars = cal(&event(&["DTSTART;TZID=Mars/Olympus:20260706T090000", "DURATION:PT1H"]));
    assert_eq!(parse_in(&mars, Zone::named("Europe/Berlin").unwrap()).events[0].start, at(2026, 7, 6, 7, 0));
}

/// Written back: in its zone, with the zone's definition (the file's own, or one made from the
/// database), once per zone; read again, the same occurrences.
#[test]
fn zones_round_trip() {
    let text = cal(&format!(
        "{}{}",
        event(&[
            "DTSTART;TZID=America/New_York:20260105T090000",
            "DTEND;TZID=America/New_York:20260105T100000",
            "RRULE:FREQ=WEEKLY;BYDAY=MO",
            "EXDATE;TZID=America/New_York:20260706T090000",
        ]),
        event(&["DTSTART;TZID=America/New_York:20260601T090000", "DURATION:PT1H"]).replace("UID:z1", "UID:z2"),
    ));
    let e = parse(&text, 0).events;
    let out = write(&e, None, at(2026, 1, 1, 0, 0));
    assert!(out.contains("DTSTART;TZID=America/New_York:20260105T090000\r\n"), "{out}");
    assert!(out.contains("DTEND;TZID=America/New_York:20260105T100000\r\n"), "{out}");
    assert!(out.contains("EXDATE;TZID=America/New_York:20260706T090000\r\n"), "{out}");
    assert_eq!(out.matches("BEGIN:VTIMEZONE").count(), 1, "{out}");
    assert!(out.contains("TZID:America/New_York\r\n") && out.contains("BEGIN:DAYLIGHT") && out.contains("TZOFFSETTO:-0400"), "{out}");
    let back = parse(&out, 0).events;
    assert_eq!((back[0].start, back[0].end, back[0].tzid.as_str()), (e[0].start, e[0].end, "America/New_York"));
    let window = (at(2026, 6, 1, 0, 0), at(2026, 8, 1, 0, 0));
    assert_eq!(starts_between(&back, window.0, window.1), starts_between(&e, window.0, window.1));
    // The zone made from the database works on its own, as the only definition a reader has.
    let renamed = out.replace("America/New_York", "Studio Eastern");
    assert_eq!(starts_between(&parse(&renamed, 0).events, window.0, window.1), starts_between(&e, window.0, window.1));

    // A zone the file defined goes back as it was.
    let custom =
        cal(&format!("{STUDIO_EASTERN}{}", event(&["DTSTART;TZID=Studio Eastern:20260105T090000", "DURATION:PT1H", "RRULE:FREQ=WEEKLY;BYDAY=MO"])));
    let e = parse(&custom, 0).events;
    let out = write(&e, None, 0);
    assert!(out.contains("DTSTART;TZID=Studio Eastern:20260105T090000\r\n") && out.contains("RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\r\n"), "{out}");
    assert_eq!(parse(&out, 0).events[0], e[0]);
    // All-day events stay dates; UTC ones stay UTC.
    let plain = CalEvent { uid: "u".into(), start: at(2026, 1, 5, 14, 0), end: at(2026, 1, 5, 15, 0), ..Default::default() };
    let out = write(std::slice::from_ref(&plain), None, 0);
    assert!(out.contains("DTSTART:20260105T140000Z") && !out.contains("VTIMEZONE"), "{out}");
}

/// `BYSETPOS` keeps only the listed places among the days a period's other rules produce.
#[test]
fn set_positions_pick_among_the_days() {
    let mk = |rrule: &str, start: i64| CalEvent { uid: "r".into(), start, end: start + 3600, rrule: Some(rrule.into()), ..Default::default() };
    // The month's final Tuesday-or-Thursday: 29 January (a Thursday), 26 February, 31 March.
    let e = mk("FREQ=MONTHLY;BYDAY=TU,TH;BYSETPOS=-1;COUNT=3", at(2026, 1, 29, 9, 0));
    assert_eq!(starts(&e, i64::MAX), [at(2026, 1, 29, 9, 0), at(2026, 2, 26, 9, 0), at(2026, 3, 31, 9, 0)]);
    // The 2nd and 3rd of the month's Mondays and Wednesdays together.
    let e = mk("FREQ=MONTHLY;BYDAY=MO,WE;BYSETPOS=3,2;COUNT=4", at(2026, 1, 7, 9, 0));
    assert_eq!(starts(&e, i64::MAX), [at(2026, 1, 7, 9, 0), at(2026, 1, 12, 9, 0), at(2026, 2, 4, 9, 0), at(2026, 2, 9, 9, 0)]);
    // Daily on weekdays: no weekends.
    let e = mk("FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR;COUNT=3", at(2026, 1, 2, 9, 0));
    assert_eq!(starts(&e, i64::MAX), [at(2026, 1, 2, 9, 0), at(2026, 1, 5, 9, 0), at(2026, 1, 6, 9, 0)]);
    // Friday the 13th: both must hold.
    let e = mk("FREQ=MONTHLY;BYMONTHDAY=13;BYDAY=FR;COUNT=3", at(2026, 2, 13, 9, 0));
    assert_eq!(starts(&e, i64::MAX), [at(2026, 2, 13, 9, 0), at(2026, 3, 13, 9, 0), at(2026, 11, 13, 9, 0)]);
    // Every Monday in January.
    let e = mk("FREQ=YEARLY;BYMONTH=1;BYDAY=MO;COUNT=5", at(2026, 1, 5, 9, 0));
    assert_eq!(
        starts(&e, i64::MAX),
        [at(2026, 1, 5, 9, 0), at(2026, 1, 12, 9, 0), at(2026, 1, 19, 9, 0), at(2026, 1, 26, 9, 0), at(2027, 1, 4, 9, 0)]
    );
    // Junk positions are passed over.
    for r in ["FREQ=MONTHLY;BYDAY=MO;BYSETPOS=0", "FREQ=MONTHLY;BYDAY=MO;BYSETPOS=-999,x,400", "FREQ=DAILY;BYSETPOS=1"] {
        assert!(starts(&mk(r, at(2026, 1, 5, 9, 0)), at(2027, 1, 1, 0, 0)).len() <= 366, "{r}");
    }
}

/// A zone as Outlook keeps one: its standard and summer offsets and when the clocks change, north
/// and south of the equator; one without summer time, and an event in no zone, have none.
#[test]
fn a_zone_as_outlook_keeps_one() {
    let at = |tzid: &str| CalEvent { tzid: tzid.into(), start: days_from_civil(2026, 6, 1) * 86_400, ..Default::default() };
    let change = |month, nth, weekday, hour: i64| Some(ZoneChange { month, nth, weekday, time: hour * 3600 });
    let ny = zone_rule(&at("America/New_York"), 2026).unwrap();
    // Second Sunday of March at 02:00, first Sunday of November at 02:00 (summer time's clock).
    assert_eq!(ny, ZoneRule { standard: -5 * 3600, daylight: -4 * 3600, to_daylight: change(3, 2, 0, 2), to_standard: change(11, 1, 0, 2) });
    // The last Sunday of March and of October.
    let berlin = zone_rule(&at("W. Europe Standard Time"), 2026).unwrap();
    assert_eq!(berlin, ZoneRule { standard: 3600, daylight: 7200, to_daylight: change(3, 5, 0, 2), to_standard: change(10, 5, 0, 3) });
    let sydney = zone_rule(&at("Australia/Sydney"), 2026).unwrap();
    assert_eq!(sydney, ZoneRule { standard: 10 * 3600, daylight: 11 * 3600, to_daylight: change(10, 1, 0, 2), to_standard: change(4, 1, 0, 3) });
    let tokyo = zone_rule(&at("Asia/Tokyo"), 2026).unwrap();
    assert_eq!(tokyo, ZoneRule { standard: 9 * 3600, daylight: 9 * 3600, to_daylight: None, to_standard: None });
    assert_eq!(zone_rule(&at(""), 2026), None);
    assert_eq!(zone_rule(&CalEvent { all_day: true, ..at("Asia/Tokyo") }, 2026), None);
    // Hostile years are clamped, not a panic.
    for year in [i64::MIN, i64::MAX, 0] {
        assert!(zone_rule(&at("America/New_York"), year).is_some());
    }
}

/// Hourly, minutely and secondly rules step from the start, on the days the rule allows; their
/// count and end hold, and a step of nothing (or a hostile one) ends.
#[test]
fn rules_within_a_day() {
    let start = days_from_civil(2026, 11, 6) * 86_400 + 22 * 3600; // a Friday, 22:00
    let e = |rule: &str| CalEvent { start, end: start + 600, rrule: Some(rule.into()), ..Default::default() };
    let hours = |rule: &str| starts(&e(rule), i64::MAX).iter().map(|t| (t - start) / 3600).collect::<Vec<_>>();
    assert_eq!(hours("FREQ=HOURLY;INTERVAL=2;COUNT=4"), [0, 2, 4, 6]);
    // Friday's and Monday's only: the weekend's hours are passed over, and not counted.
    assert_eq!(hours("FREQ=HOURLY;INTERVAL=12;BYDAY=MO,FR;COUNT=3"), [0, 60, 72]);
    assert_eq!(starts(&e("FREQ=MINUTELY;INTERVAL=15;UNTIL=20261106T224500Z"), i64::MAX).len(), 4);
    assert_eq!(starts(&e("FREQ=SECONDLY;COUNT=3"), i64::MAX), [start, start + 1, start + 2]);
    // No end: capped, and only up to `before`.
    assert_eq!(starts(&e("FREQ=HOURLY"), start + 5 * 3600).len(), 5);
    assert!(starts(&e("FREQ=SECONDLY"), i64::MAX).len() <= 50_000);
    // At its wall-clock days where it is: in New York, 22:00 Friday is Saturday in UTC.
    let ny = CalEvent { tzid: "America/New_York".into(), start: start + 5 * 3600, ..e("FREQ=HOURLY;BYDAY=FR;COUNT=2") };
    assert_eq!(starts(&ny, i64::MAX), [start + 5 * 3600, start + 6 * 3600]);
    let _ = starts(&CalEvent { start: i64::MAX - 10, ..e("FREQ=HOURLY") }, i64::MAX);
}
