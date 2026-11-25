//! Issue #279: an event moved here to another calendar of the same server, end to end against the
//! in-memory server: moved there (a WebDAV MOVE, which tells no guest anything, RFC 6638
//! §3.2.3.4); on a server that moves nothing, an event stored there and deleted where it was,
//! and a meeting left where it is.

use mailcraft_model::ical::{self, Attendee, CalEvent};
use mailcraft_model::pim::Pim;
use mailcraft_model::zone::Zone;

use super::*;
use crate::dav::{Dav, Where, sync};

const AT: Where<'static> = Where { email: USER, username: "", others: &[], imap_host: "imap.example", caldav: None, carddav: None, mails: true };

/// The user's meeting with a guest elsewhere, as the server has it.
fn meeting() -> CalEvent {
    let person = |email: &str| Attendee { email: email.into(), partstat: "ACCEPTED".into(), ..Default::default() };
    CalEvent {
        uid: "plan".into(),
        summary: "Planning".into(),
        start: 1_800_000_000,
        end: 1_800_003_600,
        organizer: Some(person(USER)),
        attendees: vec![person(USER), person("guest@elsewhere.example")],
        ..Default::default()
    }
}

/// A server with two calendars and `event` in the first, read once: what's kept here.
fn read(no_moves: bool, event: CalEvent) -> (Server, Pim) {
    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        d.home = true;
        d.schedules = true;
        d.no_moves = no_moves;
        d.put(&format!("{CAL}plan.ics"), &ical::write(&[event], None, 0));
    }
    let mut pim = Pim {
        caldav_home: Some(format!("{}/dav/cal/", server.base)),
        carddav_home: Some(String::new()),
        dav_checked: Some(100),
        ..Default::default()
    };
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 100, Zone::UTC).unwrap();
    pim.calendars = out.calendars.unwrap_or_default();
    pim.events = out.events;
    assert_eq!(pim.calendars.len(), 2, "{:?}", pim.calendars);
    assert_eq!(pim.events.len(), 1);
    (server, pim)
}

/// Moved here: as the engine leaves it, in the other calendar, still where it's stored.
fn moved(pim: &mut Pim, server: &Server) -> String {
    let home = format!("{}{HOME}", server.base);
    for e in &mut pim.events {
        e.calendar = home.clone();
        e.dirty = true;
    }
    home
}

/// Moved, and changed in nothing else: moved alone, not stored over (Nextcloud sends the guests
/// the meeting again for a copy stored over, though nothing in it changed).
#[test]
fn a_meeting_moved_to_another_calendar_is_moved_there_and_nobody_told() {
    let (server, mut pim) = read(false, meeting());
    let home = moved(&mut pim, &server);
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 200, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let d = server.data.lock().unwrap();
    assert!(d.log.contains(&format!("MOVE {CAL}plan.ics")), "{:?}", d.log);
    assert_eq!(d.items.keys().filter(|h| h.contains("plan")).collect::<Vec<_>>(), [&format!("{HOME}plan.ics")]);
    assert!(!d.log.iter().any(|l| l.starts_with("DELETE") || l.starts_with("PUT")), "moved alone: {:?}", d.log);
    let pushed = out.pushed_events.first().expect("pushed");
    assert_eq!((pushed.calendar.as_str(), pushed.href.as_str()), (home.as_str(), format!("{home}plan.ics").as_str()));
    assert!(!pushed.etag.is_empty());
    // Read in its new calendar, not again in the old one.
    assert!(out.events.iter().all(|e| e.calendar == home), "{:?}", out.events.iter().map(|e| &e.calendar).collect::<Vec<_>>());
}

/// Moved and changed: moved, then stored over where it went (its etag read there), the
/// client's to schedule.
#[test]
fn a_meeting_moved_and_changed_is_stored_over_where_it_went() {
    let (server, mut pim) = read(false, meeting());
    moved(&mut pim, &server);
    pim.events[0].event.summary = "Planning moved".into();
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 200, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let d = server.data.lock().unwrap();
    assert!(d.log.contains(&format!("MOVE {CAL}plan.ics")) && d.log.contains(&format!("PUT {HOME}plan.ics")), "{:?}", d.log);
    let there = &d.items[&format!("{HOME}plan.ics")].1;
    assert!(there.contains("Planning moved") && there.contains("SCHEDULE-AGENT=CLIENT"), "{there}");
}

/// A server that moves nothing: an event stored in the other calendar, then deleted where it
/// was; a meeting stays where it is (a second item of it is a new invitation to a server that
/// schedules), said, the server's copy read again.
#[test]
fn a_server_that_wont_move_gets_a_copy_of_an_event_and_keeps_a_meeting() {
    let mut plain = meeting();
    plain.organizer = None;
    plain.attendees.clear();
    let (server, mut pim) = read(true, plain);
    let home = moved(&mut pim, &server);
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 200, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(server.data.lock().unwrap().items.keys().filter(|h| h.contains("plan")).collect::<Vec<_>>(), [&format!("{HOME}plan.ics")]);
    assert_eq!(out.pushed_events.first().map(|e| e.href.clone()), Some(format!("{home}plan.ics")));

    let (server, mut pim) = read(true, meeting());
    moved(&mut pim, &server);
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 200, Zone::UTC).unwrap();
    assert!(out.errors.iter().any(|e| e.contains("won't move a meeting")), "{:?}", out.errors);
    assert_eq!(out.refused, ["plan"]);
    assert!(out.pushed_events.is_empty());
    let d = server.data.lock().unwrap();
    assert_eq!(d.items.keys().filter(|h| h.contains("plan")).collect::<Vec<_>>(), [&format!("{CAL}plan.ics")]);
    assert!(!d.log.iter().any(|l| l.starts_with("PUT") || l.starts_with("DELETE")), "{:?}", d.log);
}

/// Changed on the server since it was read: not moved, and the server's copy read again, as a
/// change refused for that.
#[test]
fn a_meeting_changed_there_meanwhile_isnt_moved() {
    let (server, mut pim) = read(false, meeting());
    moved(&mut pim, &server);
    let mut changed = meeting();
    changed.summary = "Planning, moved on".into();
    server.data.lock().unwrap().put(&format!("{CAL}plan.ics"), &ical::write(&[changed], None, 0));
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 200, Zone::UTC).unwrap();
    let d = server.data.lock().unwrap();
    assert!(d.items.contains_key(&format!("{CAL}plan.ics")) && !d.items.contains_key(&format!("{HOME}plan.ics")), "{:?}", d.items.keys());
    assert!(out.pushed_events.is_empty(), "{:?}", out.pushed_events);
}
