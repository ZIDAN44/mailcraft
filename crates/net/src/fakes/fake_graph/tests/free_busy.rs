//! Colleagues' free/busy times on Exchange Online (issue #140): `getSchedule`, and a calendar
//! shared for those alone listed as that.

use mailcraft_model::ical::ShowAs;

use super::*;

fn at(s: &str) -> i64 {
    mailcraft_jmap::utc_date(s).unwrap()
}

fn item(status: &str, start: &str, end: &str, subject: Option<&str>) -> Value {
    let mut v = json!({"status": status, "start": {"dateTime": format!("{start}.0000000"), "timeZone": "UTC"},
                       "end": {"dateTime": format!("{end}.0000000"), "timeZone": "UTC"}, "isPrivate": false});
    if let Some(s) = subject {
        v["subject"] = json!(s);
        v["location"] = json!("Room 4");
    }
    v
}

#[test]
fn schedules_come_with_what_each_shares() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.schedules.push(("ana@contoso.example".into(), item("busy", "2026-11-03T09:00:00", "2026-11-03T10:30:00", Some("Budget review"))));
    d.schedules.push(("ana@contoso.example".into(), item("free", "2026-11-03T11:00:00", "2026-11-03T12:00:00", None)));
    d.schedules.push(("ben@contoso.example".into(), item("oof", "2026-11-04T00:00:00", "2026-11-05T00:00:00", None)));
    d.schedules.push(("ben@contoso.example".into(), item("busy", "2027-01-04T00:00:00", "2027-01-05T00:00:00", None)));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let people: Vec<String> = ["Ana@contoso.example", "ben@contoso.example", "nobody@contoso.example"].map(String::from).into();
    let from = at("2026-11-02T00:00:00Z");
    let answers = c.availability(&people, from, from + 7 * 86_400).unwrap();
    assert_eq!(
        answers.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(),
        ["Ana@contoso.example", "ben@contoso.example", "nobody@contoso.example"]
    );
    let ana = &answers[0];
    assert_eq!(ana.busy.len(), 1, "free time isn't time taken: {:?}", ana.busy);
    assert_eq!((ana.busy[0].start, ana.busy[0].end), (at("2026-11-03T09:00:00Z"), at("2026-11-03T10:30:00Z")));
    assert_eq!((ana.busy[0].subject.as_str(), ana.busy[0].location.as_str()), ("Budget review", "Room 4"));
    assert_eq!(answers[1].busy.iter().map(|b| b.show_as).collect::<Vec<_>>(), [ShowAs::OutOfOffice]);
    assert_eq!(answers[2].error.as_deref(), Some("not found in the organisation"));

    // Asked in UTC; a window past six weeks is cut, not refused.
    let long = c.availability(&people[..1], from, from + 365 * 86_400).unwrap();
    assert_eq!(long[0].error, None);
    let d = server.data.lock().unwrap();
    let ask = d.schedule_asks.last().unwrap();
    assert_eq!(ask["startTime"], json!({"dateTime": "2026-11-02T00:00:00", "timeZone": "UTC"}));
    let end = at(&format!("{}Z", ask["endTime"]["dateTime"].as_str().unwrap()));
    assert_eq!(end - from, crate::ews::AVAILABILITY_WINDOW_MAX);
    assert!(d.violations.is_empty(), "{:?}", d.violations);
}

/// A calendar asked for whose events can't be read (shared for its free/busy time alone) is
/// listed as that: read-only, no events of its own, no error.
#[test]
fn a_calendar_shared_for_its_times_alone_is_listed_as_that() {
    use mailcraft_model::pim::{Pim, shared_calendar_id};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.others.push(("ben@contoso.example".into(), false));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let pim = Pim { shared: vec!["ben@contoso.example".into(), "cleo@contoso.example".into()], ..Default::default() };
    let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let cals = sync.calendars.unwrap();
    let find = |e: &str| cals.iter().find(|x| x.id == shared_calendar_id(e)).unwrap();
    assert!(!find("ben@contoso.example").free_busy);
    let cleo = find("cleo@contoso.example");
    assert!(cleo.free_busy && cleo.read_only, "{cleo:?}");
}

/// Issue #185: a colleague's work hours come with their schedule, their zone by its Windows name.
#[test]
fn a_schedules_work_hours_come_with_it() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.schedules.push(("ana@contoso.example".into(), item("busy", "2026-11-03T09:00:00", "2026-11-03T10:30:00", None)));
    d.working_hours.push((
        "ana@contoso.example".into(),
        json!({"daysOfWeek": ["monday", "tuesday", "wednesday", "thursday", "friday"], "startTime": "08:30:00.0000000",
               "endTime": "17:00:00.0000000", "timeZone": {"name": "W. Europe Standard Time"}}),
    ));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let from = at("2026-11-02T00:00:00Z");
    let answers = c.availability(&["ana@contoso.example".to_string()], from, from + 7 * 86_400).unwrap();
    let work = answers[0].work.as_ref().expect("her work hours");
    assert_eq!((work.periods[0].days, work.periods[0].start, work.periods[0].end), (0b1_1111, 510, 1020));
    assert!(work.spans(from, from + 86_400).contains(&(at("2026-11-02T07:30:00Z"), at("2026-11-02T16:00:00Z"))), "an hour east in November");
}
