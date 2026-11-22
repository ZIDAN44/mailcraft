//! Issue #140: a colleague's calendar shared for its free/busy time alone shows those times,
//! asked for while the calendar shows them, kept a few minutes, said when they can't be had.

use mailcraft_model::ical::ShowAs;
use mailcraft_model::pim::free_busy::{Busy, FreeBusyAnswer};
use mailcraft_model::pim::{CalendarInfo, shared_calendar_id};

use super::*;
use crate::{AgendaItem, FreeBusyState, Module};

pub(super) const ANA: &str = "ana@contoso.example";

/// The sample session with Ana's calendar shared for its times alone, on the week of
/// 2026-11-02 (a Monday), the calendar not on screen yet.
pub(super) fn with_ana(name: &str) -> (Session, String, String) {
    let mut s = sample_session(name);
    let account = s.accounts[0].config.id.clone();
    let id = shared_calendar_id(ANA);
    s.pim_mut(&account).calendars.push(CalendarInfo { id: id.clone(), name: ANA.into(), read_only: true, free_busy: true, ..Default::default() });
    s.cal_day = Some(day("2026-11-04"));
    s.cal_mode = crate::CalMode::Week;
    s.take_jobs();
    (s, account, id)
}

pub(super) fn day(d: &str) -> i64 {
    at(&format!("{d}T00:00:00Z")) / 86_400
}

/// `2026-11-03T09:00:00Z` in Unix seconds.
pub(super) fn at(t: &str) -> i64 {
    let n = |a: usize, b: usize| t[a..b].parse::<u32>().unwrap();
    mailcraft_model::dates::days_from_civil(i64::from(n(0, 4)), n(5, 7), n(8, 10)) * 86_400 + i64::from(n(11, 13) * 3600 + n(14, 16) * 60)
}

/// What the session asked for: (account, people, from, to).
fn asked(s: &mut Session) -> Vec<(String, Vec<String>, i64, i64)> {
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::FreeBusy { account, people, from, to } => Some((account, people, from, to)),
            _ => None,
        })
        .collect()
}

pub(super) fn busy(start: &str, end: &str, show_as: ShowAs, subject: &str) -> Busy {
    Busy { start: at(start), end: at(end), show_as, subject: subject.into(), ..Default::default() }
}

fn answer(s: &mut Session, account: &str, (from, to): (i64, i64), busy: Vec<Busy>, error: Option<&str>) {
    let answers = vec![FreeBusyAnswer { email: ANA.into(), busy, error: error.map(str::to_string), work: None }];
    s.apply(Event::FreeBusy { account: account.into(), from, to, answers });
}

fn ana_items(s: &Session) -> Vec<AgendaItem> {
    let (d0, d1) = s.cal_days();
    s.agenda(s.local_midnight(d0), s.local_midnight(d1)).into_iter().filter(|i| i.busy_only).collect()
}

#[test]
fn times_are_asked_for_while_the_calendar_shows_them() {
    let (mut s, account, id) = with_ana("free-busy-asked");
    // Mail on screen: nothing asked.
    s.tick();
    assert!(asked(&mut s).is_empty());
    assert_eq!(s.free_busy_state(&account, &id), Some(FreeBusyState::Asking));

    // The calendar: Ana's times, from the Monday a week before the week shown to a week past it.
    s.module = Module::Calendar;
    s.tick();
    let jobs = asked(&mut s);
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    let (acc, people, from, to) = jobs[0].clone();
    assert_eq!((acc.as_str(), people), (account.as_str(), vec![ANA.to_string()]));
    assert_eq!((from, to), (at("2026-10-26T00:00:00Z"), at("2026-11-16T00:00:00Z")));
    // Once, while the answer is awaited.
    s.tick();
    assert!(asked(&mut s).is_empty());

    // Her times: drawn among the events, opening nothing; free time isn't sent.
    answer(
        &mut s,
        &account,
        (from, to),
        vec![
            busy("2026-11-03T09:00:00Z", "2026-11-03T10:30:00Z", ShowAs::Busy, "Budget review"),
            busy("2026-11-05T00:00:00Z", "2026-11-06T00:00:00Z", ShowAs::OutOfOffice, ""),
            Busy { private: true, ..busy("2026-11-06T14:00:00Z", "2026-11-06T15:00:00Z", ShowAs::Tentative, "Doctor") },
        ],
        None,
    );
    assert_eq!(s.free_busy_state(&account, &id), Some(FreeBusyState::Shown));
    let items = ana_items(&s);
    assert_eq!(items.len(), 3, "{items:?}");
    assert_eq!((items[0].summary.as_str(), items[0].calendar.as_str(), items[0].all_day), ("", id.as_str(), true), "the day away first");
    assert_eq!(items[0].show_as, ShowAs::OutOfOffice);
    assert_eq!((items[1].summary.as_str(), items[1].start, items[1].all_day), ("Budget review", at("2026-11-03T09:00:00Z"), false));
    assert_eq!((items[2].summary.as_str(), items[2].privacy, items[2].status.as_str()), ("", mailcraft_model::ical::Privacy::Private, "TENTATIVE"));
    // Not an event of anyone's here: nothing to change or remind of.
    assert!(s.run("event.delete", &json!({"uid": items[1].uid})).is_err());
    assert!(s.due_reminders(at("2026-11-03T09:00:00Z")).iter().all(|r| r.summary != "Budget review"));
    let listed = s.run("calendar.events", &json!({})).unwrap();
    assert!(listed.as_array().unwrap().iter().any(|e| e["summary"] == "Budget review" && e["busyOnly"] == true));

    // Next week is in what's had already; asked again after five minutes.
    s.run("calendar.goto", &json!({"step": 1})).unwrap();
    s.tick();
    assert!(asked(&mut s).is_empty());
    s.clock.now += 299;
    s.tick();
    assert!(asked(&mut s).is_empty());
    s.clock.now += 2;
    s.tick();
    assert_eq!(asked(&mut s).len(), 1);
}

#[test]
fn a_week_far_off_is_asked_for_at_once_and_a_failure_is_said_and_tried_again() {
    let (mut s, account, id) = with_ana("free-busy-failed");
    s.module = Module::Calendar;
    s.tick();
    let (_, _, from, to) = asked(&mut s).remove(0);
    answer(&mut s, &account, (from, to), vec![busy("2026-11-03T09:00:00Z", "2026-11-03T10:00:00Z", ShowAs::Busy, "")], None);

    // Two months on: asked at once, the old times shown until the new come.
    s.run("calendar.goto", &json!({"date": "2027-01-06"})).unwrap();
    s.tick();
    let jobs = asked(&mut s);
    assert_eq!(jobs.len(), 1);
    let (_, _, from2, to2) = jobs[0].clone();
    assert!(from2 <= at("2027-01-04T00:00:00Z") && to2 >= at("2027-01-11T00:00:00Z"));

    // An answer to the window asked before, come late, doesn't end the wait for this one.
    answer(&mut s, &account, (from, to), Vec::new(), None);
    s.tick();
    assert!(asked(&mut s).is_empty());

    // Refused: said, nothing drawn, tried again a minute on (not every frame).
    answer(&mut s, &account, (from2, to2), Vec::new(), Some("doesn't share their free/busy time"));
    assert_eq!(s.free_busy_state(&account, &id), Some(FreeBusyState::Failed("doesn't share their free/busy time")));
    assert!(ana_items(&s).is_empty());
    s.clock.now += 59;
    s.tick();
    assert!(asked(&mut s).is_empty());
    s.clock.now += 2;
    s.tick();
    assert_eq!(asked(&mut s).len(), 1);

    // No answer at all (a job lost): asked again after two minutes.
    s.clock.now += 119;
    s.tick();
    assert!(asked(&mut s).is_empty());
    s.clock.now += 2;
    s.tick();
    assert_eq!(asked(&mut s).len(), 1);
}

#[test]
fn a_hidden_calendar_and_the_month_view_ask_no_more_than_needed() {
    let (mut s, account, id) = with_ana("free-busy-month");
    s.module = Module::Calendar;
    // Hidden: nothing asked, nothing drawn.
    s.run("calendar.show", &json!({"account": account, "calendar": id, "shown": false})).unwrap();
    s.tick();
    assert!(asked(&mut s).is_empty());
    s.run("calendar.show", &json!({"account": account, "calendar": id, "shown": true})).unwrap();

    // The month view: six weeks, asked as one window no longer than the server gives, and not
    // again once it's had (a window cut short must not look missing on every frame).
    s.run("calendar.view", &json!({"mode": "month"})).unwrap();
    s.tick();
    let (_, _, from, to) = asked(&mut s).remove(0);
    assert!(to - from <= 42 * 86_400 - 3600, "{}", to - from);
    let (d0, _) = s.cal_days();
    assert!(from <= s.local_midnight(d0));
    answer(&mut s, &account, (from, to), Vec::new(), None);
    for _ in 0..3 {
        s.clock.now += 10;
        s.tick();
        assert!(asked(&mut s).is_empty());
    }
}

/// A calendar of another kind is none of this: no state, no times asked for it.
#[test]
fn only_calendars_shared_for_their_times_are_asked_about() {
    let (mut s, account, id) = with_ana("free-busy-others");
    s.pim_mut(&account).calendars.iter_mut().for_each(|c| c.free_busy = false);
    s.module = Module::Calendar;
    s.tick();
    assert!(asked(&mut s).is_empty());
    assert_eq!(s.free_busy_state(&account, &id), None);
    assert_eq!(s.free_busy_state(&account, "nope"), None);
    // Times answered for someone not asked about show nowhere.
    answer(&mut s, &account, (0, 1), vec![busy("2026-11-03T09:00:00Z", "2026-11-03T10:00:00Z", ShowAs::Busy, "x")], None);
    assert!(ana_items(&s).is_empty());
}

/// Issue #141: the Schedule view shows a day (moving a day at a time), and asks for the times of
/// the colleagues shown on it as the other views do.
#[test]
fn the_schedule_view_shows_a_day_and_its_peoples_times() {
    let (mut s, account, _) = with_ana("free-busy-schedule");
    s.module = Module::Calendar;
    let r = s.run("calendar.view", &json!({"mode": "schedule"})).unwrap();
    assert_eq!(r["mode"], "schedule");
    assert_eq!(s.cal_days(), (day("2026-11-04"), day("2026-11-05")));
    s.run("calendar.goto", &json!({"step": 1})).unwrap();
    assert_eq!(s.cal_days(), (day("2026-11-05"), day("2026-11-06")));
    s.tick();
    let jobs = asked(&mut s);
    assert_eq!(jobs.len(), 1);
    let (acc, _, from, to) = jobs[0].clone();
    assert_eq!(acc, account);
    assert!(from <= at("2026-11-05T00:00:00Z") && to >= at("2026-11-06T00:00:00Z"));
    assert!(s.run("calendar.view", &json!({"mode": "timeline"})).is_err());
}

/// Issue #185: a colleague's work hours, as their server gave them with their times, shade
/// their calendar's; without them, the user's own do.
#[test]
fn a_colleagues_work_hours_come_with_their_times() {
    use mailcraft_model::pim::work_hours::{WorkHours, WorkPeriod, WorkZone};
    let (mut s, account, id) = with_ana("free-busy-work-hours");
    let wednesday = (at("2026-11-04T00:00:00Z"), at("2026-11-05T00:00:00Z"));
    // Not answered yet: the user's own, 08:00 to 17:00 here (UTC).
    assert_eq!(s.calendar_work_spans(&account, &id, wednesday.0, wednesday.1), [(at("2026-11-04T08:00:00Z"), at("2026-11-04T17:00:00Z"))]);
    // Ana works 09:00 to 17:00 an hour east of UTC.
    let work = WorkHours { periods: vec![WorkPeriod { start: 9 * 60, ..Default::default() }], zone: WorkZone { bias: -60, ..Default::default() } };
    let answers = vec![FreeBusyAnswer { email: ANA.into(), busy: Vec::new(), error: None, work: Some(work) }];
    s.apply(Event::FreeBusy { account: account.clone(), from: at("2026-10-26T00:00:00Z"), to: at("2026-11-16T00:00:00Z"), answers });
    assert_eq!(s.calendar_work_spans(&account, &id, wednesday.0, wednesday.1), [(at("2026-11-04T08:00:00Z"), at("2026-11-04T16:00:00Z"))]);
    // The user's own calendar keeps the user's.
    let own = s.pim[&account].calendars.iter().find(|c| !c.free_busy).map(|c| c.id.clone()).unwrap();
    assert_eq!(s.calendar_work_spans(&account, &own, wednesday.0, wednesday.1), [(at("2026-11-04T08:00:00Z"), at("2026-11-04T17:00:00Z"))]);
}

/// Issue #242: a colleague's free/busy time from an account whose CalDAV server schedules
/// meetings (RFC 6638: SOGo, Stalwart, Nextcloud), as from an Exchange one: their calendar opens,
/// alone or in a group, and their times are asked of the account's server while it's shown. One
/// that doesn't schedule (Radicale) can't, and says why.
#[test]
fn a_calendar_server_that_schedules_gives_colleagues_times() {
    let mut s = Session::open(Some(temp("free-busy-caldav")), mailcraft_model::dates::Clock::new(at("2026-11-04T12:00:00Z"), 0));
    let added = s.run("account.add", &json!({"email": "zoe@example.org", "username": "zoe", "password": "pw", "dav": "https://dav.example.org/"}));
    let account = added.unwrap()["account"].as_str().unwrap().to_string();
    let own = "https://dav.example.org/calendars/zoe/personal/".to_string();
    let synced = |s: &mut Session, schedules: bool| {
        let sync = mailcraft_model::pim::DavSync {
            caldav_home: Some("https://dav.example.org/calendars/zoe/".into()),
            calendars: Some(vec![CalendarInfo { id: own.clone(), name: "Personal".into(), ..Default::default() }]),
            auto_schedule: Some(schedules),
            checked: s.clock.now,
            ..Default::default()
        };
        s.apply(Event::DavSynced { account: account.clone(), sync: Box::new(sync) });
    };
    // Not known yet, or known not to schedule: refused, saying why.
    let refused = s.run("calendar.openShared", &json!({"email": ANA})).unwrap_err().to_string();
    assert!(refused.contains("schedules meetings"), "{refused}");
    synced(&mut s, false);
    assert!(!s.opens_others(&account));
    assert!(s.run("calendar.openShared", &json!({"email": ANA})).is_err());

    // A server that schedules: her calendar opens, of her times alone; nothing of hers to read.
    synced(&mut s, true);
    assert!(s.opens_others(&account));
    s.take_jobs();
    let r = s.run("calendar.openShared", &json!({"email": "Ana@Contoso.example"})).unwrap();
    let id = shared_calendar_id(ANA);
    assert_eq!((r["account"].as_str(), r["calendar"].as_str()), (Some(account.as_str()), Some(id.as_str())));
    let cal = s.pim[&account].calendars.iter().find(|c| c.id == id).cloned().unwrap();
    assert!(cal.free_busy && cal.read_only && cal.foreign && cal.name == ANA, "{cal:?}");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::ReadShared { .. } | Job::SyncDav { .. })));

    // On screen: asked of this account's server.
    s.cal_day = Some(day("2026-11-04"));
    s.module = Module::Calendar;
    s.tick();
    let jobs = asked(&mut s);
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    assert_eq!((jobs[0].0.as_str(), jobs[0].1.clone()), (account.as_str(), vec![ANA.to_string()]));

    // The next sync knows nothing of her: her calendar stays.
    synced(&mut s, true);
    assert!(s.pim[&account].calendars.iter().any(|c| c.id == id));

    // A group: each one's times, Ana's calendar not opened twice.
    s.run("calendar.groupCreate", &json!({"name": "Team", "members": "ana@contoso.example, ben@contoso.example"})).unwrap();
    let ids: Vec<&str> = s.pim[&account].calendars.iter().filter(|c| c.free_busy).map(|c| c.id.as_str()).collect();
    let ben = shared_calendar_id("ben@contoso.example");
    assert_eq!(ids, [id.as_str(), ben.as_str()]);
    s.run("calendar.groupDelete", &json!({"name": "Team"})).unwrap();
    assert!(s.pim[&account].calendars.iter().all(|c| !c.free_busy), "closed with the group");
}
