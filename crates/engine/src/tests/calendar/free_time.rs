//! Issue #186: the next (or previous) time everyone whose calendar is shown is free, for a length
//! chosen, inside everyone's work hours, as Outlook's AutoPick finds one.

use mailcraft_model::ical::ShowAs;
use mailcraft_model::pim::free_busy::FreeBusyAnswer;
use mailcraft_model::pim::work_hours::{WorkHours, WorkPeriod, WorkZone};

use super::free_busy::{ANA, at, busy, day, with_ana};
use super::*;
use crate::Module;

/// The sample session with its own events taken out, Ana's calendar shown beside it.
fn quiet(name: &str) -> (Session, String, String) {
    let (mut s, account, id) = with_ana(name);
    for p in s.pim.values_mut() {
        p.events.clear();
    }
    (s, account, id)
}

/// What a search found: its start and end, as `2026-11-03T09:00:00Z`.
fn found(r: &serde_json::Value) -> (String, String) {
    assert_eq!(r["found"], true, "{r}");
    let iso = |t: i64| {
        let (y, m, d) = mailcraft_model::dates::civil_from_days(t.div_euclid(86_400));
        let s = t.rem_euclid(86_400);
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:00Z", s / 3600, s % 3600 / 60)
    };
    (iso(r["start"].as_i64().unwrap()), iso(r["end"].as_i64().unwrap()))
}

fn event(s: &mut Session, summary: &str, start: &str, end: &str, show_as: &str) {
    s.run("event.create", &json!({"summary": summary, "start": start, "end": end, "showAs": show_as})).unwrap();
}

#[test]
fn the_next_time_everyone_is_free_skips_whats_taken_and_the_hours_off() {
    let (mut s, account, id) = quiet("free-time-own");
    // Ana's calendar hidden: the user's own alone.
    s.run("calendar.show", &json!({"account": account, "calendar": id, "shown": false})).unwrap();
    s.run("calendar.goto", &json!({"date": "2026-11-02"})).unwrap();
    event(&mut s, "Planning", "2026-11-02T08:00", "2026-11-02T12:00", "busy");
    event(&mut s, "Maybe", "2026-11-02T12:30", "2026-11-02T17:00", "tentative");
    event(&mut s, "Lunch walk", "2026-11-03T08:00", "2026-11-03T09:00", "free");

    // Half an hour: Monday's gap at noon.
    let r = s.run("calendar.findFree", &json!({"minutes": 30})).unwrap();
    assert_eq!(found(&r), ("2026-11-02T12:00:00Z".into(), "2026-11-02T12:30:00Z".into()));
    assert_eq!(s.free_slot(), Some((at("2026-11-02T12:00:00Z"), at("2026-11-02T12:30:00Z"))));
    // Next: the tentative afternoon is taken; Tuesday at 08:00 (a free event leaves it free).
    let r = s.run("calendar.findFree", &json!({})).unwrap();
    assert_eq!(found(&r).0, "2026-11-03T08:00:00Z");
    assert_eq!(found(&s.run("calendar.findFree", &json!({})).unwrap()).0, "2026-11-03T08:30:00Z");
    // Back, and back again.
    assert_eq!(found(&s.run("calendar.findFree", &json!({"back": true})).unwrap()).0, "2026-11-03T08:00:00Z");
    assert_eq!(found(&s.run("calendar.findFree", &json!({"back": true})).unwrap()).0, "2026-11-02T12:00:00Z");
    // Before Monday: Friday's last half hour, the weekend not worked.
    assert_eq!(found(&s.run("calendar.findFree", &json!({"back": true})).unwrap()).0, "2026-10-30T16:30:00Z");
    assert_eq!(s.cal_day, Some(day("2026-10-30")), "the calendar goes to it");

    // An hour, from Monday: none on Monday, Tuesday at 08:00.
    s.run("calendar.goto", &json!({"date": "2026-11-02"})).unwrap();
    assert_eq!(s.free_slot(), None, "moved by hand: looked for from the days shown");
    assert_eq!(found(&s.run("calendar.findFree", &json!({"minutes": 60})).unwrap()).0, "2026-11-03T08:00:00Z");
    assert_eq!(s.prefs.free_time_minutes, 60, "the length kept");

    // Never before now, and a length checked.
    s.run("calendar.goto", &json!({"date": "2026-10-01"})).unwrap();
    let r = s.run("calendar.findFree", &json!({})).unwrap();
    assert!(at(&found(&r).0) >= s.clock.now, "{r}");
    for bad in [json!({"minutes": 5}), json!({"minutes": 2000})] {
        assert!(s.run("calendar.findFree", &bad).is_err(), "{bad}");
    }
}

#[test]
fn a_colleagues_times_and_hours_count_too() {
    let (mut s, account, _) = quiet("free-time-ana");
    s.module = Module::Calendar;
    s.tick();
    // Her times not come yet: nothing to look through.
    assert!(s.run("calendar.findFree", &json!({"minutes": 60})).is_err());
    // Monday taken from 08:00 to 10:00 (tentatively: taken too), and she works 09:00 to 17:00
    // an hour east (08:00 to 16:00 here).
    let work = WorkHours { periods: vec![WorkPeriod { start: 9 * 60, ..Default::default() }], zone: WorkZone { bias: -60, ..Default::default() } };
    let answers = vec![FreeBusyAnswer {
        email: ANA.into(),
        busy: vec![busy("2026-11-02T08:00:00Z", "2026-11-02T10:00:00Z", ShowAs::Tentative, "")],
        error: None,
        work: Some(work),
    }];
    s.apply(Event::FreeBusy { account: account.clone(), from: at("2026-10-26T00:00:00Z"), to: at("2026-11-16T00:00:00Z"), answers });
    s.run("calendar.goto", &json!({"date": "2026-11-02"})).unwrap();
    event(&mut s, "Review", "2026-11-02T10:00", "2026-11-02T15:30", "busy");
    // Monday: 15:30 to 16:00 is the only half hour both are free and at work; an hour isn't.
    assert_eq!(found(&s.run("calendar.findFree", &json!({"minutes": 30})).unwrap()).0, "2026-11-02T15:30:00Z");
    s.run("calendar.goto", &json!({"date": "2026-11-02"})).unwrap();
    let r = s.run("calendar.findFree", &json!({"minutes": 60})).unwrap();
    assert_eq!(found(&r), ("2026-11-03T08:00:00Z".into(), "2026-11-03T09:00:00Z".into()));
    assert_eq!(r["unknown"], json!([]));

    // Eight hours, which her day (08:00 to 16:00) holds and the user's own (08:00 to 17:00)
    // with it: none in her times, which end on the 16th, and the calendar goes there.
    event(&mut s, "Every day", "2026-11-03T12:00", "2026-11-03T13:00", "busy");
    s.run("event.create", &json!({"summary": "Each day", "start": "2026-11-04T12:00", "end": "2026-11-04T13:00", "repeat": "daily"})).unwrap();
    s.run("calendar.goto", &json!({"date": "2026-11-02"})).unwrap();
    let r = s.run("calendar.findFree", &json!({"minutes": 480})).unwrap();
    assert_eq!(r["found"], false, "{r}");
    assert_eq!(r["to"], "2026-11-16");
    assert_eq!(s.cal_day, Some(day("2026-11-16")), "on to where her times end, for them to be asked for");
    assert!(s.notices.iter().any(|n| n.text == crate::session::pim::NONE_YET), "{:?}", s.notices);
}

/// A colleague whose times can't be had counts as free, and that's said.
#[test]
fn someone_whose_times_cant_be_had_counts_as_free() {
    let (mut s, account, _) = quiet("free-time-unknown");
    let answers = vec![FreeBusyAnswer { email: ANA.into(), error: Some("doesn't share their free/busy time".into()), ..Default::default() }];
    s.apply(Event::FreeBusy { account, from: at("2026-10-26T00:00:00Z"), to: at("2026-11-16T00:00:00Z"), answers });
    s.run("calendar.goto", &json!({"date": "2026-11-02"})).unwrap();
    let r = s.run("calendar.findFree", &json!({"minutes": 30})).unwrap();
    assert_eq!(found(&r).0, "2026-11-02T08:00:00Z");
    assert_eq!(r["unknown"], json!([ANA]));
    assert_eq!(s.notices.last().map(|n| n.text.as_str()), Some(crate::session::pim::UNKNOWN_FREE));
}

/// Issue #277: a meeting's form asks for its guests' times, their calendars not shown, kept
/// fresh while it's open; and finds the next time the user and they are all free: their times
/// taken and the user's own (but the meeting's, being moved). Not come yet, that's said.
#[test]
fn a_meetings_guests_times_are_asked_for_and_the_next_free_time_found() {
    let (mut s, account, id) = quiet("assistant");
    s.run("calendar.show", &json!({"account": account, "calendar": id, "shown": false})).unwrap();
    s.module = Module::Mail;
    s.take_jobs();
    let guests = format!("Ana <{ANA}>, riley@studio.example");
    let r = s.run("event.scheduling", &json!({"account": account, "attendees": guests, "date": "2026-11-02"})).unwrap();
    assert_eq!((r["guests"][0]["email"].as_str(), r["guests"][0]["pending"].as_bool()), (Some(ANA), Some(true)), "{r}");
    let jobs = s.take_jobs();
    let Some(Job::FreeBusy { people, from, to, .. }) = jobs.first().cloned() else { panic!("{jobs:?}") };
    assert_eq!(people, [ANA], "the user's own address isn't asked for");
    let next = |s: &mut Session, uid: Option<&str>| {
        let mut p = json!({"account": account, "attendees": format!("Ana <{ANA}>"), "start": "2026-11-02T08:00", "minutes": 60});
        if let Some(u) = uid {
            p["uid"] = json!(u);
        }
        s.run("event.nextFree", &p)
    };
    assert!(matches!(next(&mut s, None), Err(CmdError::Failed(e)) if e == crate::session::pim::NOT_COME));

    let answers = vec![FreeBusyAnswer {
        email: ANA.into(),
        busy: vec![busy("2026-11-02T08:00:00Z", "2026-11-02T10:00:00Z", ShowAs::Busy, "")],
        ..Default::default()
    }];
    s.apply(Event::FreeBusy { account: account.clone(), from, to, answers });
    let r = s.run("event.scheduling", &json!({"account": account, "attendees": format!("Ana <{ANA}>"), "date": "2026-11-02"})).unwrap();
    assert_eq!(r["guests"][0]["busy"][0]["start"], at("2026-11-02T08:00:00Z"), "{r}");
    assert!(s.take_jobs().is_empty(), "known: not asked again");
    let uid = s.run("event.create", &json!({"summary": "Planning", "start": "2026-11-02T10:00", "end": "2026-11-02T11:00"})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(found(&next(&mut s, None).unwrap()).0, "2026-11-02T11:00:00Z", "after Ana's and the user's own");
    assert_eq!(found(&next(&mut s, Some(&uid)).unwrap()).0, "2026-11-02T10:00:00Z", "the meeting moved: its own time free");
    // Kept fresh while the form is open: asked again once they're old.
    s.clock.now += 400;
    s.free_busy_due();
    assert!(matches!(s.take_jobs().as_slice(), [Job::FreeBusy { .. }]), "asked again");
}
