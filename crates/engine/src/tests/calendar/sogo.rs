//! Engine tests: what a real Mailcow's SOGo does that other calendar servers don't, found
//! against one (`apps/mailcraft-cli/tests/real_mailcow_*.rs`).

use mailcraft_model::ical::{self, CalEvent};
use mailcraft_model::pim::{CalendarInfo, DavSync, StoredEvent};

use super::*;

/// SOGo's copy of ann's private event for bob, who may see its date and time only: in a
/// calendar of ann's that bob may change (SOGo's "modify" role for public events), so not
/// read-only. It shows as a private appointment, busy, and can't be changed: saving it would
/// write SOGo's stand-in over ann's event.
#[test]
fn an_event_sogo_hid_shows_as_private_and_is_not_changed() {
    let mut s = Session::open(Some(temp("sogo-hidden")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "bob@mc31.test", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let theirs = "https://mail.mc31.test/SOGo/dav/bob@mc31.test/Calendar/ann_A_mc31_D_test_personal/".to_string();
    let data = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nDTSTAMP:20261031T080000Z\r\nDTSTART:20261104T090000Z\r\nDTEND:20261104T100000Z\r\n\
                SUMMARY:(Private event)\r\nUID:d620a8857101682c5479e45f\r\nX-SOGO-SECURE:YES\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let event: CalEvent = ical::parse(data, 0).events.into_iter().next().unwrap();
    let sync = DavSync {
        caldav_home: Some("https://mail.mc31.test/SOGo/dav/bob@mc31.test/Calendar/".into()),
        calendars: Some(vec![CalendarInfo {
            id: theirs.clone(),
            name: "Personal Calendar (Ann Tester)".into(),
            foreign: true,
            ctag: "1".into(),
            ..Default::default()
        }]),
        events: vec![StoredEvent { calendar: theirs.clone(), href: format!("{theirs}x.ics"), etag: "\"1\"".into(), event, dirty: false }],
        synced_calendars: vec![theirs.clone()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    let shown = s.run("calendar.events", &json!({"from": "2026-10-01", "to": "2026-12-31"})).unwrap();
    let e = shown.as_array().unwrap().iter().find(|e| e["uid"] == "d620a8857101682c5479e45f").cloned().unwrap();
    assert_eq!((e["summary"].as_str(), e["privacy"].as_str(), e["showAs"].as_str()), (Some(""), Some("private"), Some("busy")), "{e}");
    assert!(s.run("calendar.search", &json!({"query": "is:private"})).unwrap().as_array().is_some_and(|h| h.len() == 1));
    assert!(
        s.run("calendar.search", &json!({"query": "Private event"})).unwrap().as_array().is_some_and(Vec::is_empty),
        "SOGo's words aren't its title"
    );
    s.take_jobs();
    let r = s.run("event.update", &json!({"uid": "d620a8857101682c5479e45f", "summary": "Bob was here"}));
    assert!(r.is_err(), "{r:?}");
    let (_, kept) = s.find_event("d620a8857101682c5479e45f").unwrap();
    assert!(!kept.dirty && kept.event.summary.is_empty());
}
