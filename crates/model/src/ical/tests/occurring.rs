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
