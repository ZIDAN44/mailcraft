//! The calendar's agenda over a lot of events.

use super::*;

/// The calendar's month view over a busy calendar, timed as a frame asks for it: the month's
/// agenda, the small months' busy days (each an agenda of its own) and the To-Do Bar's week.
/// 20,000 events, one every six hours from five years ago, each with two guests, 50 of them
/// weekly series. Run under the heavy lock:
/// `cargo test --release -p mailcraft-engine --lib measure_agenda_frame -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_agenda_frame() {
    use mailcraft_model::ical::{Attendee, CalEvent};
    use mailcraft_model::pim::{LOCAL, StoredEvent};
    let mut s = sample_session("measure-agenda");
    let guest = |email: &str| Attendee { email: email.into(), name: "Guest".into(), partstat: "ACCEPTED".into(), rsvp: true };
    let p = s.pim_mut("sample");
    for i in 0..20000_i64 {
        let start = NOW - 5 * 365 * 86_400 + i * 3 * 3600 * 2;
        let e = CalEvent {
            uid: format!("e{i}"),
            summary: format!("Review {i}"),
            location: "Room 4".into(),
            start,
            end: start + 3600,
            attendees: vec![guest("riley@studio.example"), guest(&format!("guest{i}@example.org"))],
            categories: vec!["Work".into()],
            rrule: (i % 400 == 0).then(|| "FREQ=WEEKLY".to_string()),
            ..Default::default()
        };
        p.events.push(StoredEvent { calendar: LOCAL.into(), event: e, ..Default::default() });
    }
    let day = 86_400;
    let (from, to) = (NOW - NOW.rem_euclid(day) - 3 * day, NOW - NOW.rem_euclid(day) + 39 * day);
    let frame = |s: &Session| s.agenda(from, to).len() + s.agenda(from - 42 * day, to).len() + s.agenda(NOW, NOW + 7 * day).len();
    let n = frame(&s);
    let t = std::time::Instant::now();
    for _ in 0..50 {
        assert_eq!(frame(&s), n);
    }
    println!("MEASURE agenda per frame ({n} items): {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 50.0);
    done(s);
}

/// A month's calendar of 200 daily series begun ten years ago, drawn: each was walked from its
/// first day, 3,650 of them, each time.
/// `cargo test --release -p mailcraft-engine --lib measure_old_series_frame -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_old_series_frame() {
    use mailcraft_model::ical::CalEvent;
    use mailcraft_model::pim::{LOCAL, StoredEvent};
    let mut s = sample_session("measure-old-series");
    let p = s.pim_mut("sample");
    for i in 0..200_i64 {
        let start = NOW - 10 * 365 * 86_400 + i * 300;
        let e = CalEvent {
            uid: format!("d{i}"),
            summary: format!("Standup {i}"),
            start,
            end: start + 900,
            rrule: Some("FREQ=DAILY".into()),
            ..Default::default()
        };
        p.events.push(StoredEvent { calendar: LOCAL.into(), event: e, ..Default::default() });
    }
    let day = 86_400;
    let from = NOW - NOW.rem_euclid(day) - 3 * day;
    let n = s.agenda(from, from + 42 * day).len();
    let t = std::time::Instant::now();
    for _ in 0..20 {
        assert_eq!(s.agenda(from, from + 42 * day).len(), n);
    }
    println!("MEASURE old series per frame ({n} items): {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
    done(s);
}
