//! Engine tests: private events and Show as (issue #32), and the calendar's search (issue #33).

use mailcraft_model::ical::{Privacy, ShowAs, privacy, show_as};

use super::*;

fn search(s: &mut Session, p: Value) -> Vec<Value> {
    s.run("calendar.search", &p).unwrap().as_array().cloned().unwrap_or_default()
}

fn titles(list: &[Value]) -> Vec<String> {
    list.iter().filter_map(|e| e["summary"].as_str().map(str::to_string)).collect()
}

#[test]
fn private_and_show_as_are_set_kept_and_shown() {
    let mut s = sample_session("calendar-private");
    let uid = s
        .run(
            "event.create",
            &json!({"summary": "Doctor", "start": "2026-10-20T08:00", "end": "2026-10-20T09:00", "privacy": "private", "showAs": "oof"}),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((privacy(&e), show_as(&e)), (Privacy::Private, ShowAs::OutOfOffice));
    let day = s.run("calendar.events", &json!({"from": "2026-10-20", "to": "2026-10-20"})).unwrap();
    let doc = day.as_array().unwrap().iter().find(|x| x["uid"] == uid.as_str()).cloned().unwrap();
    assert_eq!((doc["privacy"].as_str(), doc["showAs"].as_str()), (Some("private"), Some("oof")));
    // Changing something else leaves both; "show time only" and free change them.
    s.run("event.update", &json!({"uid": uid, "location": "Clinic"})).unwrap();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((privacy(&e), show_as(&e)), (Privacy::Private, ShowAs::OutOfOffice));
    s.run("event.update", &json!({"uid": uid, "privacy": "confidential", "showAs": "free"})).unwrap();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((privacy(&e), show_as(&e)), (Privacy::Confidential, ShowAs::Free));
    let text = mailcraft_model::ical::write(std::slice::from_ref(&e), None, NOW);
    assert!(text.contains("CLASS:CONFIDENTIAL") && text.contains("TRANSP:TRANSPARENT") && text.contains("X-MICROSOFT-CDO-BUSYSTATUS:FREE"), "{text}");
    assert_eq!(text.matches("CLASS:").count(), 1);
    // Words that aren't one are refused, and change nothing.
    for bad in [json!({"uid": uid, "privacy": "secret"}), json!({"uid": uid, "showAs": "maybe"}), json!({"uid": uid, "privacy": ""})] {
        assert!(matches!(s.run("event.update", &bad), Err(CmdError::Params(_))), "{bad}");
    }
    assert_eq!(privacy(&s.find_event(&uid).unwrap().1.event), Privacy::Confidential);
    // A classification written elsewhere that MailCraft doesn't know is kept through an edit.
    let mut odd = s.find_event(&uid).unwrap().1.event.clone();
    odd.extra.retain(|l| !l.starts_with("CLASS"));
    odd.extra.insert(0, "CLASS:X-TEAM".into());
    s.update_event(&uid, |x| *x = odd).unwrap();
    s.run("event.update", &json!({"uid": uid, "summary": "Doctor (moved)", "privacy": "private"})).unwrap();
    assert!(s.find_event(&uid).unwrap().1.event.extra.iter().any(|l| l == "CLASS:X-TEAM"), "already private: kept as written");
    done(s);
}

#[test]
fn the_calendar_search_finds_events_in_every_calendar() {
    let mut s = sample_session("calendar-search");
    // Words from a subject, a place, a person; in both calendars (the personal and the studio's).
    assert_eq!(titles(&search(&mut s, json!({"query": "storyboard"}))), ["Storyboard review"]);
    assert_eq!(titles(&search(&mut s, json!({"query": "location:suite"}))), ["Edit session: act two"], "the studio calendar too");
    assert_eq!(titles(&search(&mut s, json!({"query": "attendee:sam"}))), ["Storyboard review"]);
    assert_eq!(titles(&search(&mut s, json!({"query": "organizer:ana"}))), ["Sound mix playback"]);
    assert_eq!(titles(&search(&mut s, json!({"query": "from:zoe@studio.example"}))), ["Storyboard review"]);
    assert_eq!(titles(&search(&mut s, json!({"query": "notes:scenes"}))), ["Storyboard review"]);
    let work = titles(&search(&mut s, json!({"query": "category:work"})));
    assert_eq!(work.len(), 2, "{work:?}");
    assert!(search(&mut s, json!({"query": "nothing-like-this"})).is_empty());
    // A recurring series is found once, at its next occurrence.
    let standup = search(&mut s, json!({"query": "stand-up"}));
    assert_eq!(standup.len(), 1, "{standup:?}");
    let next = standup[0]["start"].as_i64().unwrap();
    assert!(next > NOW && next - NOW < 3 * 86_400, "the next one: {next}");
    assert_eq!((standup[0]["recurring"].as_bool(), standup[0]["upcoming"].as_bool()), (Some(true), Some(true)));
    // Its occurrence opens it (named as the calendar names it).
    let week = s.run("calendar.events", &json!({"from": "2026-10-12", "to": "2026-10-25"})).unwrap();
    assert!(week.as_array().unwrap().iter().any(|e| e["uid"] == standup[0]["uid"] && e["occurrence"] == standup[0]["occurrence"]));
    // Past: the last one before now; with dates, the ones then.
    let past = search(&mut s, json!({"query": "stand-up", "when": "past"}));
    let last = past[0]["start"].as_i64().unwrap();
    assert!(last < NOW && NOW - last < 3 * 86_400 && !past[0]["upcoming"].as_bool().unwrap());
    let first = search(&mut s, json!({"query": "stand-up before:2026-10-01"}));
    assert_eq!(first.len(), 1);
    assert!(first[0]["start"].as_i64().unwrap() < crate::parse_local("2026-10-01", &s.clock).unwrap().0);
    let later = search(&mut s, json!({"query": "stand-up after:2026-12-01"}));
    assert!(later[0]["start"].as_i64().unwrap() >= crate::parse_local("2026-12-01", &s.clock).unwrap().0);
    assert!(search(&mut s, json!({"query": "stand-up after:2026-12-01", "when": "past"})).is_empty(), "nothing past after December");
    // Upcoming first (soonest first), then past (latest first).
    let all = search(&mut s, json!({"query": ""}));
    let upcoming: Vec<bool> = all.iter().map(|e| e["upcoming"].as_bool().unwrap()).collect();
    assert!(upcoming.windows(2).all(|w| w[0] || !w[1]), "{upcoming:?}");
    let starts: Vec<i64> = all.iter().filter(|e| e["upcoming"] == true).map(|e| e["start"].as_i64().unwrap()).collect();
    assert!(starts.windows(2).all(|w| w[0] <= w[1]));
    assert!(all.iter().all(|e| e["upcoming"] == true || e["end"].as_i64().unwrap() <= NOW));
    // Scope: one calendar (by name), or what's shown.
    assert_eq!(titles(&search(&mut s, json!({"query": "", "calendar": "Studio"}))).len(), 2);
    s.run("calendar.show", &json!({"account": "sample", "calendar": crate::sample::STUDIO_CALENDAR, "shown": false})).unwrap();
    assert!(search(&mut s, json!({"query": "location:suite"})).is_empty(), "a hidden calendar isn't searched");
    assert_eq!(titles(&search(&mut s, json!({"query": "suite", "calendar": crate::sample::STUDIO_CALENDAR}))), ["Edit session: act two"]);
    // Private ones.
    s.run("event.create", &json!({"summary": "Dentist", "start": "2026-10-21T08:00", "privacy": "private"})).unwrap();
    assert_eq!(titles(&search(&mut s, json!({"query": "is:private"}))), ["Lunch with Jun", "Dentist"], "the sample's lunch, then the new one");
    assert_eq!(titles(&search(&mut s, json!({"query": "-is:private lunch"}))), Vec::<String>::new());
    // Bad params are refused; a limit holds.
    assert!(matches!(s.run("calendar.search", &json!({"query": "x", "when": "soon"})), Err(CmdError::Params(_))));
    assert_eq!(search(&mut s, json!({"query": "", "limit": 2})).len(), 2);
    assert!(!search(&mut s, json!({"query": "", "limit": 0})).is_empty(), "at least one");
    done(s);
}

/// A changed occurrence of a series that matches when the series doesn't is found on its own,
/// and a cancelled one isn't; an all-day event is on its day; a series long over shows its last.
#[test]
fn changed_occurrences_all_day_and_finished_series() {
    use mailcraft_model::ical::CalEvent;
    let mut s = sample_session("calendar-search-odd");
    let day = crate::parse_local("2026-10-05", &s.clock).unwrap().0;
    let master = CalEvent {
        uid: "s1".into(),
        summary: "Weekly sync".into(),
        start: day + 9 * 3600,
        end: day + 10 * 3600,
        rrule: Some("FREQ=WEEKLY".into()),
        ..Default::default()
    };
    let moved = CalEvent {
        summary: "Sync with the board".into(),
        recurrence_id: Some(day + 9 * 3600 + 14 * 86_400),
        start: day + 14 * 86_400 + 15 * 3600,
        end: day + 14 * 86_400 + 16 * 3600,
        ..master.clone()
    };
    let gone = CalEvent { summary: "Board call (cancelled)".into(), status: "CANCELLED".into(), uid: "c1".into(), rrule: None, ..master.clone() };
    let over = CalEvent {
        uid: "o1".into(),
        summary: "Old course".into(),
        start: day - 400 * 86_400,
        end: day - 400 * 86_400 + 3600,
        rrule: Some("FREQ=DAILY;COUNT=5".into()),
        ..Default::default()
    };
    let all_day = CalEvent {
        uid: "a1".into(),
        summary: "Festival".into(),
        all_day: true,
        start: 1_792_800_000 / 86_400 * 86_400,
        end: 1_792_800_000 / 86_400 * 86_400 + 86_400,
        ..Default::default()
    };
    for e in [master, moved.clone(), gone, over, all_day.clone()] {
        let p = s.pim_mut("sample");
        p.events.push(mailcraft_model::pim::StoredEvent { calendar: mailcraft_model::pim::LOCAL.into(), event: e, ..Default::default() });
    }
    let board = search(&mut s, json!({"query": "board"}));
    assert_eq!(titles(&board), ["Sync with the board"], "the changed one on its own, not the cancelled one");
    assert_eq!(board[0]["start"].as_i64(), Some(moved.start));
    assert_eq!(board[0]["occurrence"].as_i64(), moved.recurrence_id);
    let sync = search(&mut s, json!({"query": "weekly"}));
    assert_eq!(sync.len(), 1, "the series once");
    let old = search(&mut s, json!({"query": "course"}));
    assert_eq!(old[0]["start"].as_i64(), Some(day - 396 * 86_400), "its last");
    assert!(search(&mut s, json!({"query": "course", "when": "upcoming"})).is_empty());
    let fest = search(&mut s, json!({"query": "festival"}));
    assert_eq!(fest[0]["start"].as_i64(), Some(s.clock.utc(all_day.start)), "from local midnight");
    assert_eq!(fest[0]["occurrence"].as_i64(), Some(all_day.start), "named as its rule names it");
    done(s);
}

/// A damaged cache (times at the ends of the range, a rule that never repeats, a calendar
/// with thousands of events) doesn't stop the search, nor make it slow.
#[test]
fn the_search_survives_a_hostile_cache() {
    use mailcraft_model::ical::CalEvent;
    use mailcraft_model::pim::{LOCAL, StoredEvent};
    let mut s = sample_session("calendar-search-hostile");
    let odd = [
        CalEvent { uid: "max".into(), summary: "edge max".into(), start: i64::MAX - 5, end: i64::MAX, ..Default::default() },
        CalEvent { uid: "min".into(), summary: "edge min".into(), start: i64::MIN, end: i64::MIN + 5, ..Default::default() },
        CalEvent {
            uid: "never".into(),
            summary: "edge never".into(),
            start: NOW,
            end: NOW + 60,
            rrule: Some("FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=31".into()),
            ..Default::default()
        },
        CalEvent {
            uid: "daily".into(),
            summary: "edge daily".into(),
            start: NOW - 30 * 365 * 86_400,
            end: NOW - 30 * 365 * 86_400 + 60,
            rrule: Some("FREQ=DAILY".into()),
            ..Default::default()
        },
        CalEvent { uid: "back".into(), summary: "edge backwards".into(), start: NOW, end: NOW - 3600, ..Default::default() },
        CalEvent { uid: "whole".into(), summary: "edge whole".into(), start: i64::MIN, end: i64::MAX, all_day: true, ..Default::default() },
        CalEvent {
            uid: "top".into(),
            summary: "edge top".into(),
            start: i64::MAX,
            end: i64::MAX,
            rrule: Some("FREQ=DAILY".into()),
            ..Default::default()
        },
    ];
    let p = s.pim_mut("sample");
    for e in odd {
        p.events.push(StoredEvent { calendar: LOCAL.into(), event: e, ..Default::default() });
    }
    for i in 0..20_000 {
        let e =
            CalEvent { uid: format!("bulk-{i}"), summary: format!("Bulk {i}"), start: NOW + i * 600, end: NOW + i * 600 + 300, ..Default::default() };
        p.events.push(StoredEvent { calendar: LOCAL.into(), event: e, ..Default::default() });
    }
    let started = std::time::Instant::now();
    for when in ["all", "upcoming", "past"] {
        let _ = search(&mut s, json!({"query": "edge", "when": when}));
        let _ = search(&mut s, json!({"query": "after:9999-12-31", "when": when}));
        let _ = search(&mut s, json!({"query": "before:0001-01-01", "when": when}));
    }
    let bulk = search(&mut s, json!({"query": "bulk", "limit": 1000}));
    assert_eq!(bulk.len(), 1000);
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    // What the calendar draws, around now and at the ends.
    let _ = s.agenda(NOW - 86_400 * 7, NOW + 86_400 * 7);
    let _ = s.agenda(i64::MIN, i64::MAX);
    done(s);
}

/// A date and time is its digits, seconds or not; what follows them makes it no time (found
/// closing gap 9: `13:30 PM`, typed in a form, was saved as half past one in the afternoon).
#[test]
fn a_time_with_words_after_it_is_no_time() {
    let clock = mailcraft_model::dates::Clock::new(NOW, 0);
    let at = |s: &str| crate::parse_local(s, &clock).map(|(t, _)| t);
    assert_eq!(at("2026-10-16T13:30"), at("2026-10-16 13:30:00"));
    assert!(at("2026-10-16T13:30").is_some() && at("2026-10-16T13:30:59").is_some());
    for junk in ["2026-10-16T13:30 PM", "2026-10-16T13:30pm", "2026-10-16T13:30:60", "2026-10-16T13:30:5", "2026-10-16T13:30Z"] {
        assert_eq!(at(junk), None, "{junk}");
    }
}
