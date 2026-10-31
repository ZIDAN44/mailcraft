//! Engine tests: the calendar's features of v0.31 together: Private and Show as (issue #32),
//! online meetings (#25), held invitations and the default calendar (#30), answered invitations
//! out of the Inbox (#31) and the calendar's search (#33), on one event and one invitation.

use mailcraft_model::ical::{self, Attendee, CalEvent, Privacy, ShowAs};

use super::invite_placing::{OWN, arrive, found, imap_session, parts};
use super::*;

const ROOM: &str = "https://meet.example.edu/rooms/abc-def-ghi";

/// How many of the event's own lines start so.
fn lines(e: &CalEvent, start: &str) -> usize {
    e.extra.iter().filter(|l| l.to_ascii_uppercase().starts_with(start)).count()
}

/// One event made private, free and with a room's meeting at once, then saved again as the form
/// saves it: each of CLASS, TRANSP, the busy status, CONFERENCE and the notes' block once, in
/// the event, in the invitation sent, and back from it.
#[test]
fn private_show_as_and_a_meeting_on_one_event() {
    let mut s = sample_session("together-form");
    s.run("meetings.addRoom", &json!({"name": "BBB", "url": ROOM, "code": "123-456-789"})).unwrap();
    let params = json!({"summary": "Review", "description": "Agenda", "start": "2026-10-21T15:00", "attendees": "Zoe Park <zoe@studio.example>",
        "privacy": "private", "showAs": "oof", "reminder": 15, "onlineMeeting": "BBB"});
    let uid = s.run("event.create", &params).unwrap()["uid"].as_str().unwrap().to_string();
    let check = |e: &CalEvent| {
        assert_eq!((ical::privacy(e), ical::show_as(e)), (Privacy::Private, ShowAs::OutOfOffice), "{:?}", e.extra);
        for start in ["CLASS", "TRANSP", "X-MICROSOFT-CDO-BUSYSTATUS", "CONFERENCE", "BEGIN:VALARM"] {
            assert_eq!(lines(e, start), 1, "{start}: {:?}", e.extra);
        }
        assert_eq!(e.description.matches("---- Online meeting").count(), 1, "{}", e.description);
        assert_eq!(e.location, ROOM);
    };
    let e = s.find_event(&uid).unwrap().1.event.clone();
    check(&e);
    // Saved again from the form: everything it shows, the meeting asked for again.
    let mut again = params.clone();
    again["uid"] = json!(uid);
    again["description"] = json!(e.description);
    again["location"] = json!(e.location);
    again["onlineMeeting"] = json!({"name": "BBB", "url": ROOM, "code": "123-456-789"});
    s.run("event.update", &again).unwrap();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    check(&e);
    // The guests' copy, and the event written and read back.
    let back = ical::parse(&ical::write(std::slice::from_ref(&e), None, NOW), 0);
    check(&back.events[0]);
    let st = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    let sent = st.messages.iter().filter(|m| m.subject == "Invitation: Review").map(|m| s.message_calendar(&m.key).unwrap()).collect::<Vec<_>>();
    assert!(!sent.is_empty());
    for cal in sent {
        check(&cal.events[0]);
    }
    // Found by the room's link, its notes, and as private.
    for q in [ROOM, "\"rooms/abc-def-ghi\"", "notes:123-456-789", "location:meet.example.edu", "is:private review", "-is:recurring meet"] {
        let hits = s.run("calendar.search", &json!({"query": q})).unwrap();
        assert!(hits.as_array().unwrap().iter().any(|h| h["uid"] == uid.as_str()), "{q}: {hits}");
    }
    let hits = s.run("calendar.search", &json!({"query": "-is:private review"})).unwrap();
    assert!(!hits.as_array().unwrap().iter().any(|h| h["uid"] == uid.as_str()));
    done(s);
}

/// Outlook's invitation with a meeting (`CONFERENCE`), sent tentative but meant busy, arriving
/// before the account's calendars are found: held, accepted in the message (busy), its mail out
/// of the Inbox, its reply sent without the organizer's alarm, and put in the account's own
/// calendar on the server once found, with its meeting.
#[test]
fn an_outlook_meeting_held_then_accepted() {
    let (mut s, id) = imap_session("together-held");
    let e = CalEvent {
        uid: "plan@studio.example".into(),
        summary: "Planning".into(),
        start: NOW + 86_400,
        end: NOW + 90_000,
        organizer: Some(Attendee { email: "zoe@studio.example".into(), name: "Zoe Park".into(), ..Default::default() }),
        attendees: vec![Attendee { email: "riley@studio.example".into(), partstat: "NEEDS-ACTION".into(), rsvp: true, ..Default::default() }],
        extra: vec![
            format!("CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Join:{ROOM}"),
            "CLASS:PUBLIC".into(),
            "TRANSP:OPAQUE".into(),
            "X-MICROSOFT-CDO-BUSYSTATUS:TENTATIVE".into(),
            "X-MICROSOFT-CDO-INTENDEDSTATUS:BUSY".into(),
            "BEGIN:VALARM".into(),
            "ACTION:DISPLAY".into(),
            "TRIGGER:-PT15M".into(),
            "END:VALARM".into(),
        ],
        ..Default::default()
    };
    let key = arrive(&mut s, &id, 1, "REQUEST", std::slice::from_ref(&e));
    assert_eq!(s.pim[&id].held.len(), 1, "held");
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.take_jobs();
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 1, "{r}");
    assert!(s.pim[&id].held.is_empty(), "dealt with");
    let jobs = s.take_jobs();
    let reply = jobs
        .iter()
        .find_map(|j| match j {
            Job::Send { raw, .. } => Some(String::from_utf8_lossy(raw).to_string()),
            _ => None,
        })
        .expect("the answer is sent");
    assert!(jobs.iter().any(|j| matches!(j, Job::Apply { op: Op::Move { to, .. }, .. } if to == "Trash")), "{jobs:?}");
    // The reply's calendar: no piece of the organizer's alarm left.
    let ics = reply.split("BEGIN:VCALENDAR").nth(1).map(|x| format!("BEGIN:VCALENDAR{x}")).unwrap();
    for gone in ["TRIGGER", "END:VALARM", "ACTION:DISPLAY"] {
        assert!(!ics.contains(gone), "{gone} in {ics}");
    }
    found(&mut s, &id, Vec::new());
    let p = parts(&s, &id, "plan@studio.example");
    assert_eq!(p.len(), 1, "{p:?}");
    let ev = &p[0].event;
    assert_eq!(p[0].calendar, OWN);
    assert_eq!(ical::show_as(ev), ShowAs::Busy);
    assert_eq!(s.join_link(ev).as_deref(), Some(ROOM));
    assert!(ev.attendees.iter().any(|a| a.partstat == "ACCEPTED"));
    // Made private by the user: found as private, by its meeting's link.
    s.run("event.update", &json!({"uid": "plan@studio.example", "privacy": "confidential"})).unwrap();
    let hits = s.run("calendar.search", &json!({"query": format!("is:private {ROOM}")})).unwrap();
    assert_eq!(hits.as_array().map(Vec::len), Some(1), "{hits}");
    assert_eq!(hits[0]["showAs"], "busy");
    done(s);
}
