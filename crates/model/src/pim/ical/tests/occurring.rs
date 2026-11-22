//! Events that happen once are told without their rule's machinery, as it would tell them.

use proptest::prelude::*;

use super::*;

/// The occurrences of `events` in `[from, to)` as [`occurrences`] finds them for a series: each
/// of the event's [`starts`], unless it ends by `from`, is skipped, or was moved.
fn by_starts(events: &[CalEvent], from: i64, to: i64) -> Vec<Occurrence> {
    let mut out = Vec::new();
    for (i, e) in events.iter().enumerate() {
        if e.recurrence_id.is_some() || e.status == "CANCELLED" {
            continue;
        }
        let len = e.end.saturating_sub(e.start).max(0);
        let moved = |s: i64| events.iter().any(|m| m.uid == e.uid && m.recurrence_id == Some(s));
        for s in starts(e, to).into_iter().take_while(|&s| s < to) {
            if s.saturating_add(len.max(1)) > from && !e.exdates.contains(&s) && !moved(s) {
                out.push(Occurrence { event: i, start: s, end: s.saturating_add(len) });
            }
        }
    }
    for (i, e) in events.iter().enumerate() {
        if e.recurrence_id.is_some() && e.status != "CANCELLED" && e.start < to && e.end.max(e.start.saturating_add(1)) > from {
            out.push(Occurrence { event: i, start: e.start, end: e.end });
        }
    }
    out.sort_by_key(|o| (o.start, o.event));
    out
}

fn time() -> impl Strategy<Value = i64> {
    prop_oneof![-50i64..50, Just(i64::MIN), Just(i64::MAX), Just(i64::MIN + 1), Just(i64::MAX - 1)]
}

fn event() -> impl Strategy<Value = CalEvent> {
    (0u8..3, time(), time(), any::<bool>(), proptest::option::of(time()), any::<bool>()).prop_map(|(uid, start, end, skipped, moved, cancelled)| {
        CalEvent {
            uid: format!("u{uid}"),
            start,
            end,
            exdates: if skipped { vec![start] } else { Vec::new() },
            recurrence_id: moved,
            status: if cancelled { "CANCELLED".into() } else { String::new() },
            ..Default::default()
        }
    })
}

proptest! {
    #[test]
    fn single_events_occur_as_a_rule_would_tell_them(events in proptest::collection::vec(event(), 0..8), from in time(), to in time()) {
        prop_assert_eq!(occurrences(&events, from, to), by_starts(&events, from, to));
    }
}

fn series() -> impl Strategy<Value = CalEvent> {
    let rules = proptest::sample::select(vec![
        "FREQ=DAILY",
        "FREQ=DAILY;INTERVAL=3",
        "FREQ=WEEKLY",
        "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH",
        "FREQ=MONTHLY",
        "FREQ=MONTHLY;BYMONTHDAY=31",
        "FREQ=MONTHLY;BYDAY=-1FR",
        "FREQ=YEARLY",
        "FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=29",
        "FREQ=DAILY;UNTIL=20300101T000000Z",
        "FREQ=WEEKLY;COUNT=40",
        "FREQ=HOURLY;INTERVAL=17",
        "FREQ=HOURLY;INTERVAL=11;BYDAY=SA",
    ]);
    let zones = proptest::sample::select(vec!["", "Pacific/Kiritimati", "America/New_York", "Pacific/Pago_Pago"]);
    // Begun up to 15 years before 2026, lasting up to two days, in a zone or none: asked about
    // up to 31 years of it, each within the 20,000 starts and 50,000 steps a walk from the first
    // goes to.
    (rules, 0i64..15 * 365, 0i64..2 * 86_400, any::<bool>(), zones).prop_map(|(rule, back, len, skipped, zone)| {
        let start = 1_767_225_600 - back * 86_400 + 9 * 3600;
        CalEvent {
            uid: "s".into(),
            start,
            end: start + len,
            rrule: Some(rule.to_string()),
            exdates: if skipped { vec![start + 7 * 86_400] } else { Vec::new() },
            tzid: zone.into(),
            ..Default::default()
        }
    })
}

proptest! {
    /// A series' occurrences in a window are those its rule tells from its first start, though
    /// they're found from near the window (each was walked to from the first, each frame).
    #[test]
    fn series_occur_as_their_rule_tells_them(e in series(), from_day in 0i64..20 * 365, days in 1i64..60) {
        let from = 1_767_225_600 - 16 * 365 * 86_400 + from_day * 86_400;
        let to = from + days * 86_400;
        prop_assert_eq!(occurrences(std::slice::from_ref(&e), from, to), by_starts(std::slice::from_ref(&e), from, to));
    }
}

/// A daily series begun 60 years ago shows this week: its starts were counted from its first,
/// and past 20,000 of them none came.
#[test]
fn a_long_series_still_occurs() {
    let now = 1_792_000_000;
    let start = now - 60 * 365 * 86_400;
    let e = CalEvent { uid: "old".into(), start, end: start + 1800, rrule: Some("FREQ=DAILY".into()), ..Default::default() };
    let this_week = occurrences(std::slice::from_ref(&e), now, now + 7 * 86_400);
    assert_eq!(this_week.len(), 7, "{this_week:?}");
    // And an hourly one begun ten years ago: past 50,000 steps it went no further.
    let start = now - 10 * 365 * 86_400;
    let e = CalEvent { uid: "hourly".into(), start, end: start + 600, rrule: Some("FREQ=HOURLY".into()), ..Default::default() };
    assert_eq!(occurrences(std::slice::from_ref(&e), now, now + 86_400).len(), 24);
}

/// A rule with a frequency RFC 5545 doesn't have (`FREQ=FORTNIGHTLY`, typed as a rule of its own)
/// is no rule: the event still occurs at its start, as one with no `FREQ` at all does, rather
/// than not at all.
#[test]
fn an_unknown_frequency_is_no_rule() {
    let start = at(2026, 10, 5, 9, 0);
    for rule in ["FREQ=FORTNIGHTLY", "FREQ=;COUNT=3", "COUNT=3", "FREQ=DAILYX;INTERVAL=2"] {
        let e = CalEvent { uid: "u".into(), start, end: start + 1800, rrule: Some(rule.into()), ..Default::default() };
        assert_eq!(starts(&e, i64::MAX), vec![start], "{rule}");
        assert_eq!(occurrences(std::slice::from_ref(&e), start - 86_400, start + 86_400).len(), 1, "{rule}");
    }
    let daily = CalEvent { uid: "d".into(), start, end: start + 1800, rrule: Some("freq=daily;count=3".into()), ..Default::default() };
    assert_eq!(starts(&daily, i64::MAX).len(), 3, "a known one in any case");
}

/// Issue #241: which times are a series' occurrences (to change one on its own), and each one's
/// number by its rule, as Exchange counts them (those taken out counted too).
#[test]
fn a_series_names_its_occurrences() {
    let start = 1_792_573_200; // a Wednesday, 09:00 UTC
    let week = 7 * 86_400;
    let mut e = CalEvent { uid: "w".into(), start, end: start + 1800, rrule: Some("FREQ=WEEKLY;COUNT=5".into()), ..Default::default() };
    assert!(is_occurrence(&e, start) && is_occurrence(&e, start + 4 * week));
    assert!(!is_occurrence(&e, start + 5 * week), "past its count");
    assert!(!is_occurrence(&e, start + 3600) && !is_occurrence(&e, start - week) && !is_occurrence(&e, start + week / 2));
    assert_eq!(occurrence_number(&e, start), Some(1));
    assert_eq!(occurrence_number(&e, start + 3 * week), Some(4));
    assert_eq!(occurrence_number(&e, start + 3600), None);
    // One taken out isn't there to change, yet still counts.
    e.exdates.push(start + week);
    assert!(!is_occurrence(&e, start + week));
    assert_eq!(occurrence_number(&e, start + 2 * week), Some(3));
    // An event that happens once: its own start.
    let once = CalEvent { rrule: None, exdates: Vec::new(), ..e };
    assert!(is_occurrence(&once, start) && !is_occurrence(&once, start + week));
}
