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

const OWN: &str = "https://mail.mc31.test/SOGo/dav/bob@mc31.test/Calendar/personal/";
const ANNS: &str = "https://mail.mc31.test/SOGo/dav/bob@mc31.test/Calendar/ann_A_mc31_D_test_personal/";

/// A meeting ann organised with bob: SOGo puts bob's copy in his calendar, and ann's is in hers.
fn meeting(uid: &str, calendar: &str, partstat: &str) -> StoredEvent {
    let e = CalEvent {
        uid: uid.into(),
        summary: "Planning".into(),
        start: NOW + 86_400,
        end: NOW + 90_000,
        organizer: Some(ical::Attendee { email: "ann@mc31.test".into(), name: "Ann".into(), ..Default::default() }),
        attendees: vec![ical::Attendee { email: "riley@studio.example".into(), partstat: partstat.into(), rsvp: true, ..Default::default() }],
        ..Default::default()
    };
    StoredEvent { calendar: calendar.into(), href: format!("{calendar}{uid}.ics"), etag: "\"gcs00000000\"".into(), event: e, dirty: false }
}

/// bob's calendars as SOGo lists them: ann's (subscribed to, read-only) first, then his own.
fn synced(s: &mut Session, id: &str, synced: &[&str], events: Vec<StoredEvent>) {
    let sync = DavSync {
        caldav_home: Some("https://mail.mc31.test/SOGo/dav/bob@mc31.test/Calendar/".into()),
        calendars: Some(vec![
            CalendarInfo {
                id: ANNS.into(),
                name: "Personal Calendar (Ann)".into(),
                foreign: true,
                read_only: true,
                ctag: "1".into(),
                ..Default::default()
            },
            CalendarInfo { id: OWN.into(), name: "Personal Calendar".into(), ctag: "1".into(), ..Default::default() },
        ]),
        events,
        synced_calendars: synced.iter().map(|c| c.to_string()).collect(),
        unchanged_calendars: [ANNS, OWN].iter().filter(|c| !synced.contains(c)).map(|c| c.to_string()).collect(),
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.into(), sync: Box::new(sync) });
}

/// A real Mailcow: a meeting ann organised with bob is in his calendar (SOGo put it there) and
/// in hers, which he subscribed to in SOGo, under one uid. Read together, ann's copy took the
/// place of bob's, the one he answers in. His own is kept, and ann's shows only when his is gone.
#[test]
fn a_meeting_in_a_colleagues_calendar_too_keeps_the_users_own_copy() {
    let (mut s, id) = super::invite_placing::imap_session("sogo-two-copies");
    synced(&mut s, &id, &[ANNS, OWN], vec![meeting("m1", ANNS, "NEEDS-ACTION"), meeting("m1", OWN, "NEEDS-ACTION")]);
    let cals = |s: &Session| super::invite_placing::parts(s, &id, "m1").iter().map(|e| e.calendar.clone()).collect::<Vec<_>>();
    assert_eq!(cals(&s), [OWN], "bob's copy, not ann's");
    // ann's calendar read again (his unchanged): still his.
    synced(&mut s, &id, &[ANNS], vec![meeting("m1", ANNS, "NEEDS-ACTION")]);
    assert_eq!(cals(&s), [OWN]);
    // Answered from the calendar: in his own, sent to his own.
    s.run("event.respond", &json!({"uid": "m1", "answer": "accepted"})).unwrap();
    let mine = super::invite_placing::parts(&s, &id, "m1")[0].clone();
    assert!(mine.calendar == OWN && mine.dirty && mine.event.attendees[0].partstat == "ACCEPTED", "{mine:?}");
    // Gone from his calendar (he deleted it in SOGo's web pages): ann's shows.
    synced(&mut s, &id, &[ANNS, OWN], vec![meeting("m1", ANNS, "ACCEPTED")]);
    let still = super::invite_placing::parts(&s, &id, "m1");
    assert_eq!(still.len(), 1, "{still:?}");
}

/// An invitation from someone outside to ann and bob, already in ann's calendar that bob
/// subscribed to, arrives in bob's Inbox: it goes in his own calendar, as a new invitation, not
/// taken for an update of ann's copy (one he may only read, which the server refuses to store).
#[test]
fn an_invitation_already_in_a_colleagues_calendar_goes_in_the_users_own() {
    let (mut s, id) = super::invite_placing::imap_session("sogo-invited-too");
    synced(&mut s, &id, &[ANNS, OWN], vec![meeting("m2", ANNS, "ACCEPTED")]);
    assert_eq!(super::invite_placing::parts(&s, &id, "m2")[0].calendar, ANNS);
    let mut inv = meeting("m2", OWN, "NEEDS-ACTION").event;
    inv.organizer = Some(ical::Attendee { email: "zoe@studio.example".into(), name: "Zoe".into(), ..Default::default() });
    super::invite_placing::arrive(&mut s, &id, 1, "REQUEST", &[inv]);
    let parts = super::invite_placing::parts(&s, &id, "m2");
    assert_eq!(parts.len(), 1, "{parts:?}");
    assert!(parts[0].calendar == OWN && parts[0].dirty && parts[0].href.is_empty(), "{:?}", parts[0]);
    s.run("event.respond", &json!({"uid": "m2", "answer": "tentative"})).unwrap();
    assert!(super::invite_placing::parts(&s, &id, "m2").iter().all(|e| e.calendar == OWN));
    // ann's copy read again afterwards doesn't come back beside it.
    synced(&mut s, &id, &[ANNS], vec![meeting("m2", ANNS, "ACCEPTED")]);
    assert_eq!(super::invite_placing::parts(&s, &id, "m2").len(), 1);
}
