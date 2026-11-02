//! Engine tests: meeting invitations that change one occurrence, and invitations that mustn't
//! be applied (from the wrong person, or older than what the calendar holds).

use mailcraft_model::ical::{Attendee, CalEvent};
use mailcraft_model::pim::{CalendarInfo, DavSync, StoredEvent};

use super::*;
use crate::InviteProblem;

const WEEK: i64 = 7 * 86_400;
const ANA: (&str, &str) = ("Ana Lima", "ana@soundlab.example");

/// Put a message carrying a calendar in the sample Inbox and open it.
pub(super) fn deliver(s: &mut Session, n: u32, from: (&str, &str), method: &str, events: &[CalEvent], stamp: i64) {
    let ics = mailcraft_model::ical::write(events, Some(method), stamp);
    deliver_ics(s, n, from, method, &format!("text/calendar; method={method}"), ics.into_bytes());
}

/// Put a message carrying this calendar file (its type and bytes), with the method in its subject,
/// in the sample Inbox and open it.
fn deliver_ics(s: &mut Session, n: u32, from: (&str, &str), method: &str, mime: &str, data: Vec<u8>) {
    let d = mailcraft_mime::Draft {
        to: vec![mailcraft_model::EmailAddress::new("Riley", "riley@studio.example")],
        subject: format!("{method} {n}"),
        body: "See the calendar.".into(),
        attachments: vec![mailcraft_mime::OutAttachment { name: "invite.ics".into(), mime: mime.into(), data, ..Default::default() }],
        ..Default::default()
    };
    let raw = mailcraft_mime::build(&d, &mailcraft_model::EmailAddress::new(from.0, from.1), NOW, &format!("<i{n}@x>")).unwrap();
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = mailcraft_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9000 + n };
    inbox.messages.insert(0, mailcraft_mime::summary_from_full(&raw, key.clone(), mailcraft_model::Flags::default(), 0, NOW).into());
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
}

pub(super) fn me(partstat: &str) -> Attendee {
    Attendee { email: "riley@studio.example".into(), name: "Riley".into(), partstat: partstat.into(), rsvp: true }
}

pub(super) fn parts<'a>(s: &'a Session, uid: &str) -> Vec<&'a StoredEvent> {
    s.pim.get("sample").map(|p| p.events.iter().filter(|e| e.event.uid == uid).collect()).unwrap_or_default()
}

/// A weekly meeting Ana organizes, in a server calendar: the series and one moved occurrence in
/// one item (as CalDAV keeps them), and one more moved occurrence stored as its own item (as
/// Exchange and Microsoft Graph keep them).
fn weekly_series(s: &mut Session) -> CalEvent {
    let cal = "https://dav.example.org/cal/work/".to_string();
    let master = CalEvent {
        uid: "weekly@soundlab.example".into(),
        summary: "Mix review".into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        rrule: Some("FREQ=WEEKLY;COUNT=8".into()),
        organizer: Some(Attendee { email: ANA.1.into(), name: ANA.0.into(), ..Default::default() }),
        attendees: vec![me("ACCEPTED")],
        sequence: 1,
        ..Default::default()
    };
    let moved = |week: i64| CalEvent {
        recurrence_id: Some(master.start + week * WEEK),
        start: master.start + week * WEEK + 7200,
        end: master.start + week * WEEK + 10_800,
        rrule: None,
        ..master.clone()
    };
    let stored = |e: CalEvent, href: &str| StoredEvent { calendar: cal.clone(), href: href.into(), etag: "\"1\"".into(), event: e, dirty: false };
    let sync = DavSync {
        caldav_home: Some("https://dav.example.org/cal/".into()),
        calendars: Some(vec![CalendarInfo {
            id: cal.clone(),
            name: "Work".into(),
            color: 1,
            read_only: false,
            ctag: "a".into(),
            ..Default::default()
        }]),
        events: vec![
            stored(master.clone(), &format!("{cal}weekly.ics")),
            stored(moved(1), &format!("{cal}weekly.ics")),
            stored(moved(5), "graph-occurrence-5"),
        ],
        synced_calendars: vec![cal.clone()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
    assert_eq!(parts(s, &master.uid).len(), 3);
    master
}

#[test]
fn answering_one_occurrence_keeps_the_series() {
    let mut s = sample_session("invite-one");
    let master = weekly_series(&mut s);
    let uid = master.uid.clone();
    let week = |n: i64| master.start + n * WEEK;
    let one = |n: i64, seq: u32| CalEvent {
        recurrence_id: Some(week(n)),
        start: week(n) + 3600,
        end: week(n) + 7200,
        rrule: None,
        sequence: seq,
        summary: format!("Mix review (week {n})"),
        attendees: vec![me("NEEDS-ACTION")],
        ..master.clone()
    };

    // Ana moves week 3 only: accepting it adds that occurrence and keeps the rest.
    deliver(&mut s, 1, ANA, "REQUEST", &[one(3, 2)], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let now = parts(&s, &uid);
    assert_eq!(now.len(), 4, "the series, weeks 1, 3 and 5: {now:?}");
    assert!(now.iter().any(|e| e.event.recurrence_id.is_none() && e.event.rrule.is_some()), "the series is kept");
    assert!(now.iter().any(|e| e.event.recurrence_id == Some(week(1))), "week 1 is kept");
    assert!(now.iter().any(|e| e.event.recurrence_id == Some(week(3)) && e.dirty), "week 3 goes to the server");
    assert!(s.pim["sample"].deleted.is_empty(), "nothing is deleted on the server");

    // Declining week 4 only: an exception on the series, nothing deleted.
    deliver(&mut s, 2, ANA, "REQUEST", &[one(4, 2)], NOW);
    s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
    let m = s.find_event(&uid).expect("the series stays").1.clone();
    assert!(m.event.exdates.contains(&week(4)) && m.dirty, "{m:?}");
    assert_eq!(parts(&s, &uid).len(), 4);
    assert!(s.pim["sample"].deleted.is_empty(), "nothing is deleted on the server");

    // Ana cancels week 1 (stored in the series' item): the item is written again without it.
    deliver(&mut s, 3, ANA, "CANCEL", &[CalEvent { status: "CANCELLED".into(), ..one(1, 3) }], NOW);
    s.run("invite.apply", &json!({})).unwrap();
    let m = s.find_event(&uid).expect("the series stays").1.clone();
    assert!(m.event.exdates.contains(&week(1)) && m.dirty);
    assert!(!parts(&s, &uid).iter().any(|e| e.event.recurrence_id == Some(week(1))), "the moved week 1 is gone");
    assert!(s.pim["sample"].deleted.is_empty(), "the series' item isn't deleted");

    // ... and week 5 (an item of its own on the server): only that item is deleted.
    deliver(&mut s, 4, ANA, "CANCEL", &[CalEvent { status: "CANCELLED".into(), ..one(5, 3) }], NOW);
    s.run("invite.apply", &json!({})).unwrap();
    let hrefs: Vec<&str> = s.pim["sample"].deleted.iter().map(|t| t.href.as_str()).collect();
    assert_eq!(hrefs, ["graph-occurrence-5"]);
    assert!(s.find_event(&uid).is_some_and(|(_, m)| m.event.exdates.contains(&week(5))));
    assert_eq!(parts(&s, &uid).len(), 2, "the series and week 3");

    // A message with the series itself still replaces it.
    let whole = CalEvent { summary: "Mix review (new room)".into(), sequence: 4, attendees: vec![me("NEEDS-ACTION")], ..master.clone() };
    deliver(&mut s, 5, ANA, "REQUEST", &[whole], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let all = parts(&s, &uid);
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].event.summary, "Mix review (new room)");
    done(s);
}

#[test]
fn invitations_from_others_or_out_of_date_are_not_applied() {
    let mut s = sample_session("invite-checks");
    let uid = "mix@sample.mailcraft";
    let stored = s.find_event(uid).unwrap().1.event.clone();
    assert_eq!(stored.organizer.as_ref().map(|o| o.email.as_str()), Some(ANA.1));

    // Someone else cancelling Ana's meeting: refused, the event stays.
    let cancel = CalEvent { status: "CANCELLED".into(), sequence: stored.sequence + 1, ..stored.clone() };
    deliver(&mut s, 1, ("Mallory", "mallory@evil.example"), "CANCEL", std::slice::from_ref(&cancel), NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::NotFromOrganizer), "the card says why");
    assert!(s.run("invite.apply", &json!({})).is_err());
    assert!(s.find_event(uid).is_some(), "not cancelled by a stranger");
    // ... or changing it.
    let fake = CalEvent { summary: "Moved to my place".into(), sequence: stored.sequence + 1, ..stored.clone() };
    deliver(&mut s, 2, ("Mallory", "mallory@evil.example"), "REQUEST", &[fake], NOW);
    assert!(s.run("invite.respond", &json!({"answer": "accepted"})).is_err());
    assert_eq!(s.find_event(uid).unwrap().1.event.summary, stored.summary);

    // Ana's update (sequence 5) is taken in.
    let newer = CalEvent { summary: "Sound mix playback (room 2)".into(), sequence: 5, ..stored.clone() };
    deliver(&mut s, 3, ANA, "REQUEST", &[newer], NOW);
    assert_eq!(s.invite().unwrap().problem, None);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(s.find_event(uid).unwrap().1.event.summary, "Sound mix playback (room 2)");
    // An older one opened afterwards (a lower sequence) doesn't undo it.
    let older = CalEvent { summary: "Sound mix playback (room 1)".into(), sequence: 4, ..stored.clone() };
    deliver(&mut s, 4, ANA, "REQUEST", &[older], NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::OutOfDate));
    assert!(s.run("invite.respond", &json!({"answer": "accepted"})).is_err());
    assert_eq!(s.find_event(uid).unwrap().1.event.summary, "Sound mix playback (room 2)");
    // Nor does one with the same sequence stamped earlier; it can be answered, though (a
    // server that keeps the calendar stamps its copy when it writes it, as SOGo does): the
    // calendar keeps its version, with the answer.
    let earlier = CalEvent { summary: "Sound mix playback (draft)".into(), sequence: 5, ..stored.clone() };
    deliver(&mut s, 5, ANA, "REQUEST", &[earlier], NOW - 3600);
    assert_eq!(s.invite().unwrap().problem, None);
    s.run("invite.respond", &json!({"answer": "tentative"})).unwrap();
    let kept = s.find_event(uid).unwrap().1.event.clone();
    assert_eq!(kept.summary, "Sound mix playback (room 2)");
    assert!(kept.attendees.iter().any(|a| a.partstat == "TENTATIVE"), "{:?}", kept.attendees);
    // Nor an older cancellation.
    let stale = CalEvent { status: "CANCELLED".into(), sequence: 3, ..stored.clone() };
    deliver(&mut s, 6, ANA, "CANCEL", &[stale], NOW);
    assert!(s.run("invite.apply", &json!({})).is_err());
    assert!(s.find_event(uid).is_some());

    // A guest's answer counts only from that guest.
    let own = s
        .run("event.create", &json!({"summary": "Trailer review", "start": "2026-10-21T15:00", "attendees": "Zoe Park <zoe@studio.example>"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let mut reply = s.find_event(&own).unwrap().1.event.clone();
    reply.attendees = vec![Attendee { email: "zoe@studio.example".into(), name: "Zoe Park".into(), partstat: "DECLINED".into(), rsvp: false }];
    deliver(&mut s, 7, ("Mallory", "mallory@evil.example"), "REPLY", std::slice::from_ref(&reply), NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::NotFromGuest));
    assert!(s.run("invite.apply", &json!({})).is_err());
    let zoe = |s: &Session| s.find_event(&own).unwrap().1.event.attendees.iter().find(|a| a.email == "zoe@studio.example").unwrap().partstat.clone();
    assert_eq!(zoe(&s), "NEEDS-ACTION");
    deliver(&mut s, 8, ("Zoe Park", "ZOE@studio.example"), "REPLY", &[reply], NOW);
    assert_eq!(s.invite().unwrap().problem, None, "the address is compared without case");
    s.run("invite.apply", &json!({})).unwrap();
    assert_eq!(zoe(&s), "DECLINED");
    done(s);
}

/// An invitation in a charset other than UTF-8 is read in it (the calendar file's bytes come as
/// they were sent, since attachments are saved that way).
#[test]
fn an_invitation_in_another_charset_is_read_in_it() {
    let mut s = sample_session("invite-charset");
    let event = CalEvent {
        uid: "latin@soundlab.example".into(),
        summary: "Caf\u{e9} review".into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        organizer: Some(Attendee { email: ANA.1.into(), name: ANA.0.into(), ..Default::default() }),
        attendees: vec![me("NEEDS-ACTION")],
        ..Default::default()
    };
    let ics = mailcraft_model::ical::write(&[event], Some("REQUEST"), NOW);
    // In Windows-1252 the accent is the one byte 0xE9, which isn't UTF-8.
    let latin: Vec<u8> = ics.chars().map(|c| u8::try_from(u32::from(c)).unwrap()).collect();
    assert!(latin.contains(&0xe9));
    deliver_ics(&mut s, 1, ANA, "REQUEST", "text/calendar; method=REQUEST; charset=windows-1252", latin);
    assert_eq!(s.invite().unwrap().event.summary, "Caf\u{e9} review");
    done(s);
}

/// The card asks for the invitation on the first frame of every message opened: one with no
/// calendar is looked at as it was parsed to be shown, and isn't read again (it was read and
/// parsed a second time on the window's thread, 104 to 149 ms for a big one). One with an
/// invitation is read once more for its calendar's bytes.
#[test]
fn a_message_without_an_invitation_is_not_read_again_for_one() {
    let mut s = sample_session("invite-no-read");
    let store = s.store.clone().unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let plain = s.list.first().map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": plain.to_id()})).unwrap();
    assert!(s.reading.as_ref().is_some_and(|r| r.parsed.is_some()));
    let (reads, parsed) = (store.reads(), s.bodies_parsed_here());
    assert!(s.invite().is_none());
    assert_eq!((store.reads(), s.bodies_parsed_here()), (reads, parsed), "nothing read or parsed again");
    let event = CalEvent {
        uid: "card@soundlab.example".into(),
        summary: "Card review".into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        organizer: Some(Attendee { email: ANA.1.into(), name: ANA.0.into(), ..Default::default() }),
        attendees: vec![me("NEEDS-ACTION")],
        ..Default::default()
    };
    deliver(&mut s, 1, ANA, "REQUEST", &[event], NOW);
    let reads = store.reads();
    assert_eq!(s.invite().unwrap().event.summary, "Card review");
    assert_eq!(store.reads(), reads + 1, "read once for the calendar's bytes");
    done(s);
}
