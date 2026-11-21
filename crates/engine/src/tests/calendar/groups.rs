//! Issue #140: calendar groups, as Outlook's: people whose calendars are opened, shown and
//! hidden together under a name, made from addresses as a recipient box takes them.

use mailcraft_model::pim::shared_calendar_id;

use super::*;

/// A session with one Exchange account (Zoe's), its jobs taken.
fn exchange(name: &str) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    (s, id)
}

#[test]
fn a_group_opens_its_peoples_calendars_and_shows_them_together() {
    let (mut s, id) = exchange("cal-groups");
    // As typed in a recipient box: names, commas and semicolons, one's own address left out.
    let r = s.run(
        "calendar.groupCreate",
        &json!({"name": " Design team ", "members": "Ana <Ana@contoso.example>; ben@contoso.example, zoe@contoso.example"}),
    );
    assert_eq!(r.unwrap()["group"], "Design team");
    let p = &s.pim[&id];
    assert_eq!(p.groups.len(), 1);
    assert_eq!(p.groups[0].members, ["ana@contoso.example", "ben@contoso.example"]);
    assert_eq!(p.shared, ["ana@contoso.example", "ben@contoso.example"], "their calendars opened");
    // Read from the server, theirs alone, not with the whole calendar pass (issue #208).
    let jobs = s.take_jobs();
    assert!(
        jobs.iter().any(|j| matches!(j, Job::ReadShared { people, .. } if people == &["ana@contoso.example", "ben@contoso.example"])),
        "{jobs:?}"
    );
    assert!(!jobs.iter().any(|j| matches!(j, Job::SyncDav { .. })), "{jobs:?}");
    // To a group of that name (whatever its case): added, once.
    s.run("calendar.groupCreate", &json!({"name": "design TEAM", "members": "cleo@contoso.example; ana@contoso.example"})).unwrap();
    assert_eq!(s.pim[&id].groups[0].members, ["ana@contoso.example", "ben@contoso.example", "cleo@contoso.example"]);
    assert_eq!(s.pim[&id].groups.len(), 1);
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::ReadShared { people, .. } if people == &["cleo@contoso.example"])), "the one new");

    // Hidden and shown together.
    s.run("calendar.groupShow", &json!({"name": "Design team", "shown": false})).unwrap();
    for e in ["ana@contoso.example", "ben@contoso.example", "cleo@contoso.example"] {
        assert!(!s.calendar_shown(&id, &shared_calendar_id(e)), "{e}");
    }
    assert_eq!(s.calendar_groups(&id).first().map(|(g, shown)| (g.name.as_str(), *shown)), Some(("Design team", false)));
    s.run("calendar.groupShow", &json!({"name": "Design team", "shown": true})).unwrap();
    assert!(s.calendar_groups(&id)[0].1);

    // One's calendar closed: out of the group; another group keeps its own people.
    s.run("calendar.groupCreate", &json!({"name": "Leads", "members": "ben@contoso.example"})).unwrap();
    s.run("calendar.closeShared", &json!({"email": "cleo@contoso.example"})).unwrap();
    assert_eq!(s.pim[&id].groups[0].members, ["ana@contoso.example", "ben@contoso.example"]);
    // Deleted: its people's calendars close, but Ben's, in Leads.
    let r = s.run("calendar.groupDelete", &json!({"name": "Design team"})).unwrap();
    assert_eq!(r["closed"], json!(["ana@contoso.example"]));
    assert_eq!(s.pim[&id].shared, ["ben@contoso.example"]);
    assert_eq!(s.pim[&id].groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Leads"]);
}

#[test]
fn a_group_needs_a_name_people_and_an_exchange_account() {
    let (mut s, _) = exchange("cal-groups-refused");
    for (name, members) in
        [("", "ana@contoso.example"), ("  ", "ana@contoso.example"), ("Team", ""), ("Team", "zoe@contoso.example"), ("Team", "not an address")]
    {
        assert!(s.run("calendar.groupCreate", &json!({"name": name, "members": members})).is_err(), "{name:?} {members:?}");
    }
    let many: Vec<String> = (0..51).map(|n| format!("p{n}@contoso.example")).collect();
    assert!(s.run("calendar.groupCreate", &json!({"name": "All", "members": many.join("; ")})).is_err(), "51 people");
    assert!(s.run("calendar.groupDelete", &json!({"name": "None such"})).is_err());
    assert!(s.run("calendar.groupShow", &json!({"name": "None such"})).is_err());
    let mut sample = sample_session("cal-groups-sample");
    assert!(sample.run("calendar.groupCreate", &json!({"name": "Team", "members": "ana@soundlab.example"})).is_err(), "not an Exchange account");
}

/// From the address book, several calendars open at once (as a recipient box holds them); one
/// refused doesn't stop the rest.
#[test]
fn several_calendars_open_at_once() {
    let (mut s, id) = exchange("cal-open-several");
    let r = s.run("calendar.openShared", &json!({"email": "Ana Silva <ana@contoso.example>; ben@contoso.example; zoe@contoso.example"})).unwrap();
    assert_eq!(r["opened"].as_array().map(Vec::len), Some(2));
    assert_eq!(r["refused"].as_array().map(Vec::len), Some(1), "one's own");
    assert_eq!(s.pim[&id].shared, ["ana@contoso.example", "ben@contoso.example"]);
    assert!(s.run("calendar.openShared", &json!({"email": "zoe@contoso.example; zoe@contoso.example"})).is_err(), "none opened: why");
}

/// Issue #208: the calendars of people just opened, read on their own, are taken in at once with
/// their events, the rest left as it is; a whole sync begun before they were opened doesn't take
/// them away again; one closed meanwhile isn't brought back.
#[test]
fn people_just_opened_come_on_their_own() {
    use mailcraft_model::ical::CalEvent;
    use mailcraft_model::pim::{CalendarInfo, DavSync, StoredEvent};
    let (mut s, id) = exchange("cal-people-only");
    let own = DavSync {
        calendars: Some(vec![CalendarInfo { id: "ews:calendar".into(), name: "Calendar".into(), ..Default::default() }]),
        synced_calendars: vec!["ews:calendar".into()],
        events: vec![StoredEvent {
            calendar: "ews:calendar".into(),
            href: "E1".into(),
            event: CalEvent { uid: "mine".into(), summary: "Mine".into(), start: NOW, end: NOW + 3600, ..Default::default() },
            ..Default::default()
        }],
        ..Default::default()
    };
    s.apply_dav(&id, &own);
    s.run("calendar.groupCreate", &json!({"name": "Team", "members": "ana@contoso.example; ben@contoso.example"})).unwrap();
    s.take_jobs();
    let (ana, ben) = (shared_calendar_id("ana@contoso.example"), shared_calendar_id("ben@contoso.example"));
    let theirs = |cal: &str, uid: &str| StoredEvent {
        calendar: cal.into(),
        href: uid.into(),
        event: CalEvent { uid: uid.into(), summary: uid.into(), start: NOW, end: NOW + 3600, ..Default::default() },
        ..Default::default()
    };
    let people = DavSync {
        people_only: true,
        calendars: Some(vec![
            CalendarInfo { id: ana.clone(), name: "ana@contoso.example".into(), read_only: true, ..Default::default() },
            CalendarInfo { id: ben.clone(), name: "ben@contoso.example".into(), read_only: true, free_busy: true, ..Default::default() },
        ]),
        synced_calendars: vec![ana.clone(), ben.clone()],
        events: vec![theirs(&ana, "ana-review")],
        ..Default::default()
    };
    s.apply_dav(&id, &people);
    let names = |s: &Session| {
        let mut n: Vec<String> = s.agenda(NOW - 10, NOW + 10).into_iter().map(|i| i.summary).collect();
        n.sort();
        n
    };
    assert_eq!(names(&s), ["Mine", "ana-review"], "theirs in, the user's own as it was");
    assert!(s.pim[&id].calendars.iter().any(|c| c.id == ben && c.free_busy), "Ben's free/busy calendar listed");
    // The whole pass begun before they were opened: it knows only the user's calendar.
    s.apply_dav(&id, &own);
    assert_eq!(names(&s), ["Mine", "ana-review"], "not taken away");
    assert!(s.pim[&id].calendars.iter().any(|c| c.id == ana));
    // Ana closed while hers was read: not brought back.
    s.run("calendar.closeShared", &json!({"email": "ana@contoso.example"})).unwrap();
    s.apply_dav(&id, &people);
    assert!(!s.pim[&id].calendars.iter().any(|c| c.id == ana));
    assert_eq!(names(&s), ["Mine"]);
}
