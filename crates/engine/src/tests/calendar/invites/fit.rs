//! Engine tests: how an invitation fits the user's calendar (issue #194): what it clashes with,
//! what's right before or after it, and their events in the hours around it.

use mailcraft_model::ical::CalEvent;

use crate::Fit;

use super::*;
use super::{ZOE, deliver, invitation};

fn made(s: &mut Session, summary: &str, start: i64, end: i64, more: Value) -> String {
    let fmt = |t: i64| {
        let l = s.clock.local(t);
        format!("{}T{:02}:{:02}", mailcraft_model::dates::day_string(l.div_euclid(86_400)), l.rem_euclid(86_400) / 3600, l.rem_euclid(3600) / 60)
    };
    let mut p = json!({"summary": summary, "start": fmt(start), "end": fmt(end)});
    if let (Some(p), Value::Object(more)) = (p.as_object_mut(), more) {
        p.extend(more);
    }
    s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string()
}

/// An hour's meeting at a whole hour two days out.
fn at_the_hour() -> CalEvent {
    let start = NOW + 2 * 86_400;
    let start = start - start.rem_euclid(3600);
    CalEvent { start, end: start + 3600, ..invitation("crit@studio.example", "Storyboard crit", 0) }
}

#[test]
fn an_invitation_says_what_it_clashes_with_and_whats_beside_it() {
    let mut s = sample_session("invite-fit");
    let e = at_the_hour();
    // Nothing then: nothing clashes, nothing beside it.
    let day = s.invite_day(&e).unwrap();
    assert!(day.clashes().next().is_none() && day.beside().is_none(), "{:?}", day.items);
    assert!(day.from <= e.start - 7200 && day.to >= e.end + 7200, "two hours either side");

    // One ending as it starts: right before it.
    made(&mut s, "Sound mix", e.start - 3600, e.start, json!({}));
    let day = s.invite_day(&e).unwrap();
    assert!(matches!(day.beside(), Some((it, Fit::Before)) if it.summary == "Sound mix"), "{:?}", day.items);
    // One overlapping it: a clash, and then beside it no more counts.
    made(&mut s, "Grade review", e.start + 1800, e.end + 1800, json!({}));
    let day = s.invite_day(&e).unwrap();
    let clashes: Vec<&str> = day.clashes().map(|it| it.summary.as_str()).collect();
    assert_eq!(clashes, ["Grade review"]);
    assert!(day.beside().is_none(), "a clash says more");
    // Shown as free, or cancelled: shown, but no clash.
    made(&mut s, "Lunch walk", e.start, e.end, json!({"showAs": "free"}));
    let day = s.invite_day(&e).unwrap();
    assert!(day.items.iter().any(|(it, f)| it.summary == "Lunch walk" && *f == Fit::Apart));
    assert_eq!(day.clashes().count(), 1);
    // Its own copy in the calendar isn't one of them (it arrived tentative, issue #20).
    deliver(&mut s, 1, ZOE, "REQUEST", std::slice::from_ref(&e), NOW);
    let day = s.invite_day(&e).unwrap();
    assert!(!day.items.iter().any(|(it, _)| it.uid == e.uid), "{:?}", day.items);
    // An all-day invitation has no hours to show.
    assert!(s.invite_day(&CalEvent { all_day: true, ..e.clone() }).is_none());
}

#[test]
fn a_hidden_calendar_still_counts() {
    let mut s = sample_session("invite-fit-hidden");
    let e = at_the_hour();
    let uid = made(&mut s, "Edit session", e.start, e.end, json!({}));
    // The calendar it's in hidden from the view: still the user's time.
    let (account, calendar) = s.find_event(&uid).map(|(a, e)| (a.to_string(), e.calendar.clone())).unwrap();
    s.run("calendar.show", &json!({"account": account, "calendar": calendar, "shown": false})).unwrap();
    let day = s.invite_day(&e).unwrap();
    assert_eq!(day.clashes().map(|it| it.summary.as_str()).collect::<Vec<_>>(), ["Edit session"]);
    // A colleague's calendar shown beside the user's is theirs: their meeting then isn't a clash.
    let p = s.pim_mut(&account);
    p.calendars.push(mailcraft_model::pim::CalendarInfo { id: "ana".into(), name: "Ana".into(), foreign: true, ..Default::default() });
    p.events.push(mailcraft_model::pim::StoredEvent {
        calendar: "ana".into(),
        href: String::new(),
        etag: String::new(),
        event: CalEvent { uid: "anas@soundlab.example".into(), summary: "Ana's review".into(), start: e.start, end: e.end, ..Default::default() },
        dirty: false,
    });
    assert!(s.agenda(e.start, e.end).iter().any(|it| it.summary == "Ana's review"), "shown");
    let day = s.invite_day(&e).unwrap();
    assert_eq!(day.clashes().map(|it| it.summary.as_str()).collect::<Vec<_>>(), ["Edit session"], "not a colleague's");
}
