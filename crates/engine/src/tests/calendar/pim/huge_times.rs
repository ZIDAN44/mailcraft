//! Times given to the calendar's commands as numbers, however large.

use super::*;

/// A start, an end or an occurrence given as a number past any calendar's (`i64::MAX` seconds),
/// or as such a number written out, is refused as a parameter, not an overflow: the sums made
/// from it panicked in a debug build (caught as "failed unexpectedly") and wrapped to a time long
/// past in a release one.
#[test]
fn times_past_any_calendar_are_refused_as_params() {
    let mut s = sample_session("huge-times");
    for huge in [i64::MAX, i64::MIN, i64::MAX - 3600] {
        let as_text = huge.to_string();
        for params in
            [json!({"summary": "x", "start": huge}), json!({"summary": "x", "start": NOW, "end": huge}), json!({"summary": "x", "start": as_text})]
        {
            let r = s.run("event.create", &params);
            assert!(matches!(r, Err(CmdError::Params(_))), "{params}: {r:?}");
        }
    }
    let uid = s.run("event.create", &json!({"summary": "Weekly", "start": "2026-11-20T10:00", "repeat": "weekly"})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let r = s.run("event.delete", &json!({"uid": uid, "occurrence": i64::MAX}));
    assert!(!matches!(&r, Err(CmdError::Failed(m)) if m.contains("unexpectedly")), "{r:?}");
    // Within the calendar's years, numbers still work.
    s.run("event.create", &json!({"summary": "y", "start": NOW, "end": NOW + 3600})).unwrap();
}

/// Times read back from the calendar's own file past any calendar's: an invitation held since
/// `i64::MIN` goes to the calendar here (it panicked on every tick), and an event kept ending at
/// `i64::MAX` is refused as too long when it's changed (its length wrapped round and passed).
#[test]
fn times_kept_past_any_calendar_dont_overflow() {
    let mut s = sample_session("huge-kept-times");
    let key = s.list[0].key.clone();
    s.pim_mut("sample").held.push(mailcraft_model::pim::HeldInvite { key, since: i64::MIN, ics: String::new(), from: String::new() });
    s.tick();
    assert!(s.pim["sample"].held.is_empty(), "long held: let go");
    let uid = s.run("event.create", &json!({"summary": "Long", "start": "2026-11-20T10:00"})).unwrap()["uid"].as_str().unwrap().to_string();
    let e = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).unwrap();
    (e.event.start, e.event.end) = (-1000, i64::MAX);
    let r = s.run("event.update", &json!({"uid": uid, "summary": "Longer"}));
    assert!(matches!(&r, Err(CmdError::Params(m)) if m.contains("a year at most")), "{r:?}");
    // An all-day one kept before any calendar's days: brought to its day's start without an
    // overflow (it panicked in a debug build, and wrapped to the far future in a release one).
    for (start, end) in [(i64::MIN, i64::MIN + 86_400), (i64::MIN + 1, i64::MIN)] {
        let e = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).unwrap();
        (e.event.start, e.event.end, e.event.all_day) = (start, end, true);
        let r = s.run("event.update", &json!({"uid": uid, "summary": "Early"}));
        assert!(!matches!(&r, Err(CmdError::Failed(m)) if m.contains("unexpectedly")), "{r:?}");
        let e = &s.find_event(&uid).unwrap().1.event;
        assert!(e.start < 0 && e.end < 0 && e.end >= e.start, "{e:?}");
    }
}
