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
    done(s);
}
