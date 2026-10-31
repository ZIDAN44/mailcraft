//! A calendar server that schedules meetings itself (RFC 6638): MailCraft tells it to leave the
//! user's to MailCraft, and marks a guest's copy it puts there, once answered (SOGo answers for
//! any answer changed in a guest's copy, whatever it says).

use mailcraft_model::ical::{self, Attendee, CalEvent};
use mailcraft_model::pim::{Pim, StoredEvent};
use mailcraft_model::zone::Zone;

use super::*;
use crate::dav::{Dav, Where, sync};

const AT: Where<'static> = Where { email: USER, username: "", imap_host: "imap.example", caldav: None, carddav: None };

fn person(email: &str, partstat: &str) -> Attendee {
    Attendee { email: email.into(), partstat: partstat.into(), ..Default::default() }
}

/// A meeting `organizer` holds with the user and a guest elsewhere.
fn meeting(uid: &str, organizer: &str, answer: &str) -> CalEvent {
    CalEvent {
        uid: uid.into(),
        summary: "Planning".into(),
        start: 1_800_000_000,
        end: 1_800_003_600,
        organizer: Some(person(organizer, "ACCEPTED")),
        attendees: vec![person(USER, answer), person("guest@elsewhere.example", "NEEDS-ACTION")],
        ..Default::default()
    }
}

fn pim_at(server: &Server) -> (Pim, String) {
    let mut pim = Pim { caldav_home: Some(format!("{}/dav/cal/", server.base)), carddav_home: Some(String::new()), ..Default::default() };
    pim.ensure_defaults();
    (pim, format!("{}{CAL}", server.base))
}

fn stored(d: &Data, uid: &str) -> (String, String) {
    d.items.iter().find(|(_, (_, v))| v.contains(&format!("UID:{uid}"))).map(|(h, (_, v))| (h.clone(), v.replace("\r\n ", ""))).expect("stored")
}

/// The user's own meeting goes to a server that schedules saying MailCraft sends its
/// invitations (on the organizer, which SOGo reads, and on every guest, which the others read);
/// on one that doesn't, nothing is said. An event without guests says nothing either.
#[test]
fn the_users_meeting_tells_a_scheduling_server_to_leave_it() {
    for schedules in [true, false] {
        let server = serve();
        server.data.lock().unwrap().schedules = schedules;
        let (mut pim, cal) = pim_at(&server);
        let new = |e: CalEvent| StoredEvent { calendar: cal.clone(), event: e, dirty: true, ..Default::default() };
        let mut mine = meeting("mine", USER, "ACCEPTED");
        // Said already (read from a server that scheduled): not kept on one that doesn't.
        mine.client_scheduled = true;
        pim.events.push(new(mine));
        pim.events.push(new(CalEvent {
            uid: "dentist".into(),
            summary: "Dentist".into(),
            start: 1_800_000_000,
            end: 1_800_003_600,
            ..Default::default()
        }));
        let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 100, Zone::UTC).unwrap();
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.auto_schedule, Some(schedules));
        let d = server.data.lock().unwrap();
        let (_, data) = stored(&d, "mine");
        let said = data.lines().filter(|l| l.contains("SCHEDULE-AGENT=CLIENT")).count();
        assert_eq!(said, if schedules { 3 } else { 0 }, "{data}");
        assert!(!data.contains(ical::GUEST_COPY), "{data}");
        assert!(!stored(&d, "dentist").1.contains("SCHEDULE-AGENT"));
        // Read back, the event says who schedules it.
        let back = out.events.iter().find(|e| e.event.uid == "mine").expect("read back");
        assert_eq!(back.event.client_scheduled, schedules);
    }
}

/// An invitation from elsewhere put in the calendar here: the server never had the meeting, so
/// the copy is marked as the guest's own, and the server leaves its answers to MailCraft. A
/// meeting the server put in the calendar itself (a colleague's) is answered over its copy, as
/// it is: the server takes the answer on.
#[test]
fn a_guests_copy_is_marked_unless_the_server_put_it_there() {
    let server = serve();
    server.data.lock().unwrap().schedules = true;
    server
        .data
        .lock()
        .unwrap()
        .put(&format!("{CAL}colleagues-name.ics"), &ical::write(&[meeting("theirs", "ann@studio.example", "NEEDS-ACTION")], None, 0));
    let (mut pim, cal) = pim_at(&server);
    let new = |e: CalEvent| StoredEvent { calendar: cal.clone(), event: e, dirty: true, ..Default::default() };
    pim.events.push(new(meeting("from-mail", "zoe@elsewhere.example", "ACCEPTED")));
    pim.events.push(new(meeting("theirs", "ann@studio.example", "ACCEPTED")));
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 100, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let d = server.data.lock().unwrap();
    let (_, data) = stored(&d, "from-mail");
    assert!(data.contains(&format!("{}:TRUE", ical::GUEST_COPY)), "{data}");
    assert!(data.contains("ORGANIZER;PARTSTAT=ACCEPTED;SCHEDULE-AGENT=CLIENT:mailto:zoe@elsewhere.example"), "{data}");
    let (href, data) = stored(&d, "theirs");
    assert!(href.ends_with("colleagues-name.ics"), "over the server's copy: {href}");
    assert!(!data.contains("SCHEDULE-AGENT") && !data.contains(ical::GUEST_COPY), "{data}");
    assert!(data.contains("PARTSTAT=ACCEPTED:mailto:me@studio.example"), "{data}");
    let back = out.events.iter().find(|e| e.event.uid == "from-mail").expect("read back");
    assert!(ical::is_guest_copy(&back.event) && back.event.client_scheduled);
}

/// A guest's own copy answered again is stored over, still saying MailCraft answers (known
/// from the list as well as from the mark: an update from the organizer's mail replaces the
/// copy). One not answered yet waits for the answer: SOGo answers itself for any answer changed
/// in a copy it has, none for a copy stored new with it.
#[test]
fn a_guests_copy_waits_for_the_answer_and_keeps_saying_so() {
    let server = serve();
    server.data.lock().unwrap().schedules = true;
    let (mut pim, cal) = pim_at(&server);
    for (uid, marked) in [("marked", true), ("listed", false)] {
        let mut e = meeting(uid, "zoe@elsewhere.example", "ACCEPTED");
        e.client_scheduled = true;
        ical::mark_guest_copy(&mut e);
        let href = format!("{CAL}{uid}.ics");
        let etag = server.data.lock().unwrap().put(&href, &ical::write_for_server(&[e.clone()], 0));
        let mut now = meeting(uid, "zoe@elsewhere.example", "DECLINED");
        if marked {
            ical::mark_guest_copy(&mut now);
        } else {
            pim.guest_copies.push(uid.into());
        }
        pim.events.push(StoredEvent { calendar: cal.clone(), href: format!("{}{href}", server.base), etag, event: now, dirty: true });
    }
    pim.events.push(StoredEvent {
        calendar: cal.clone(),
        event: meeting("unanswered", "zoe@elsewhere.example", "NEEDS-ACTION"),
        dirty: true,
        ..Default::default()
    });
    server.data.lock().unwrap().log.clear();
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 100, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let d = server.data.lock().unwrap();
    for uid in ["marked", "listed"] {
        let (_, data) = stored(&d, uid);
        assert!(data.contains("PARTSTAT=DECLINED;SCHEDULE-AGENT=CLIENT:mailto:me@studio.example") && data.contains(ical::GUEST_COPY), "{data}");
        assert!(!d.log.iter().any(|l| l.starts_with("DELETE")), "{:?}", d.log);
    }
    assert!(!d.items.values().any(|(_, v)| v.contains("UID:unanswered")), "not stored before it's answered");
    assert!(!out.pushed_events.iter().any(|e| e.event.uid == "unanswered"));
}

/// A server that doesn't answer `OPTIONS` (or fails it) leaves what was known.
#[test]
fn whether_it_schedules_is_read_from_the_dav_header() {
    let h = |v: &str| vec![("Dav".to_string(), v.to_string())];
    assert!(crate::dav::schedules_in(&h("1, 2, access-control, calendar-access, calendar-schedule, calendar-auto-schedule, calendar-proxy")));
    assert!(crate::dav::schedules_in(&[("DAV".into(), "1, 2".into()), ("dav".into(), "Calendar-Auto-Schedule".into())]));
    assert!(!crate::dav::schedules_in(&h("1, 2, 3, calendar-access, addressbook, extended-mkcol")));
    assert!(!crate::dav::schedules_in(&h("calendar-auto-scheduled")));
    assert!(!crate::dav::schedules_in(&[("X-DAV".into(), "calendar-auto-schedule".into())]));
}

/// The organizer's MailCraft takes a guest's answer in from the mail while the server wrote
/// another guest's in its copy (SOGo writes a colleague's): the same version, so the server's
/// copy is stored again with the answer taken in, rather than kept without it (a real Mailcow:
/// the outside guest's answer was lost). A change made here too isn't lost that way: said.
#[test]
fn an_answer_taken_in_while_the_server_took_another_keeps_both() {
    let server = serve();
    let base = meeting("m", USER, "ACCEPTED");
    let href = format!("{CAL}m.ics");
    let etag = server.data.lock().unwrap().put(&href, &ical::write(std::slice::from_ref(&base), None, 0));
    let mut theirs = base.clone();
    theirs.attendees.push(person("colleague@studio.example", "ACCEPTED"));
    server.data.lock().unwrap().put(&href, &ical::write(std::slice::from_ref(&theirs), None, 0));
    let (mut pim, cal) = pim_at(&server);
    let mut mine = base.clone();
    mine.attendees.push(person("colleague@studio.example", "NEEDS-ACTION"));
    mine.attendees[1].partstat = "TENTATIVE".into();
    pim.events.push(StoredEvent {
        calendar: cal.clone(),
        href: format!("{}{href}", server.base),
        etag: etag.clone(),
        event: mine.clone(),
        dirty: true,
    });
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 100, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let back = out.events.iter().find(|e| e.event.uid == "m").expect("read back");
    let answers: Vec<(&str, &str)> = back.event.attendees.iter().map(|a| (a.email.as_str(), a.partstat.as_str())).collect();
    assert!(
        answers.contains(&("guest@elsewhere.example", "TENTATIVE")) && answers.contains(&("colleague@studio.example", "ACCEPTED")),
        "{answers:?}"
    );
    // Changed here too (its title, the same version): the server's kept, and that said.
    let mut pim2 = pim.clone();
    let current = server.data.lock().unwrap().items[&href].0.clone();
    server.data.lock().unwrap().put(&href, &ical::write(std::slice::from_ref(&theirs), None, 0));
    pim2.events[0].etag = current;
    pim2.events[0].event.summary = "Planning (renamed)".into();
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim2, 200, Zone::UTC).unwrap();
    assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
    assert!(!server.data.lock().unwrap().items[&href].1.contains("renamed"));
}

/// A colleague moved the meeting: SOGo wrote the move in the user's copy and asks for the answer
/// again (needs action), while the organizer's mail, taken in here, kept the old answer. The
/// server's copy stays, without a word: written over, SOGo would mail the organizer the old
/// answer as a new one.
#[test]
fn a_guests_copy_the_server_asks_again_is_left_to_it() {
    let server = serve();
    server.data.lock().unwrap().schedules = true;
    let href = format!("{CAL}moved.ics");
    let first = meeting("moved", "ann@studio.example", "TENTATIVE");
    let etag = server.data.lock().unwrap().put(&href, &ical::write(std::slice::from_ref(&first), None, 0));
    let moved = |answer: &str| CalEvent { sequence: 1, start: 1_800_007_200, end: 1_800_010_800, ..meeting("moved", "ann@studio.example", answer) };
    server.data.lock().unwrap().put(&href, &ical::write(&[moved("NEEDS-ACTION")], None, 0));
    let (mut pim, cal) = pim_at(&server);
    // The answer kept shows the time as tentative too.
    let mut mine = moved("TENTATIVE");
    ical::show_as_answered(&mut mine, "TENTATIVE");
    pim.events.push(StoredEvent { calendar: cal.clone(), href: format!("{}{href}", server.base), etag, event: mine, dirty: true });
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 100, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(out.refused, ["moved"]);
    assert!(server.data.lock().unwrap().items[&href].1.contains("PARTSTAT=NEEDS-ACTION:mailto:me@studio.example"));
}

/// The same crossing, with an answer the organizer's MailCraft had read before, changed on the
/// server since (the colleague declined in SOGo): the newer answer stays, and the one taken in
/// here is added. Storing the user's copy's answers over the server's put the colleague back to
/// "accepted", in the organizer's calendar and, through SOGo, in the colleague's own.
#[test]
fn an_answer_changed_on_the_server_isnt_put_back() {
    let server = serve();
    let mut base = meeting("m", USER, "ACCEPTED");
    base.attendees.push(person("colleague@studio.example", "ACCEPTED"));
    let href = format!("{CAL}m.ics");
    let etag = server.data.lock().unwrap().put(&href, &ical::write(std::slice::from_ref(&base), None, 0));
    let mut theirs = base.clone();
    theirs.attendees[2].partstat = "DECLINED".into();
    server.data.lock().unwrap().put(&href, &ical::write(std::slice::from_ref(&theirs), None, 0));
    let (mut pim, cal) = pim_at(&server);
    let mut mine = base.clone();
    mine.attendees[1].partstat = "TENTATIVE".into();
    pim.events.push(StoredEvent { calendar: cal.clone(), href: format!("{}{href}", server.base), etag, event: mine, dirty: true });
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 100, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let back = out.events.iter().find(|e| e.event.uid == "m").expect("read back");
    let answers: Vec<(&str, &str)> = back.event.attendees.iter().map(|a| (a.email.as_str(), a.partstat.as_str())).collect();
    assert!(
        answers.contains(&("guest@elsewhere.example", "TENTATIVE")) && answers.contains(&("colleague@studio.example", "DECLINED")),
        "{answers:?}"
    );
}

/// A guest's answer given here to a meeting the server put in the calendar, while the server
/// wrote another guest's answer in the user's copy (SOGo does in every guest's copy; the same
/// version): stored over the server's copy, with both. Kept as the server's, it was lost, and
/// the server, left to take it to the organizer, never had it (a real Mailcow, a decline).
#[test]
fn a_guests_answer_crossing_another_guests_is_stored() {
    let server = serve();
    server.data.lock().unwrap().schedules = true;
    let href = format!("{CAL}theirs.ics");
    let base = meeting("theirs", "ann@studio.example", "ACCEPTED");
    let etag = server.data.lock().unwrap().put(&href, &ical::write(std::slice::from_ref(&base), None, 0));
    let mut moved_on = base.clone();
    moved_on.attendees[1].partstat = "TENTATIVE".into();
    server.data.lock().unwrap().put(&href, &ical::write(std::slice::from_ref(&moved_on), None, 0));
    let (mut pim, cal) = pim_at(&server);
    // Shown as tentative before (an earlier answer), which the server's copy dropped.
    let mut mine = base.clone();
    ical::show_as_answered(&mut mine, "TENTATIVE");
    mine.attendees[0].partstat = "DECLINED".into();
    ical::show_as_answered(&mut mine, "DECLINED");
    pim.events.push(StoredEvent {
        calendar: cal.clone(),
        href: format!("{}{href}", server.base),
        etag: etag.clone(),
        event: mine.clone(),
        dirty: true,
    });
    pim.notices.push(mailcraft_model::pim::Notice {
        uid: "theirs".into(),
        method: "REPLY".into(),
        answer: "DECLINED".into(),
        event: mine,
        ..Default::default()
    });
    // Without the answer given here (only one kept from before, the server asking again), the
    // server's copy stays.
    let mut unasked = pim.clone();
    unasked.notices.clear();
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &unasked, 100, Zone::UTC).unwrap();
    assert!(server.data.lock().unwrap().items[&href].1.contains("PARTSTAT=ACCEPTED:mailto:me@studio.example"), "{:?}", out.errors);
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 100, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let data = server.data.lock().unwrap().items[&href].1.replace(
        "
 ", "",
    );
    assert!(data.contains("PARTSTAT=DECLINED:mailto:me@studio.example"), "{data}");
    assert!(data.contains("PARTSTAT=TENTATIVE:mailto:guest@elsewhere.example"), "{data}");
    assert!(!data.contains("SCHEDULE-AGENT") && !data.contains(ical::GUEST_COPY), "{data}");
}
