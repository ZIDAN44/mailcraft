//! Engine tests: meeting messages and a calendar server that schedules them itself (RFC 6638),
//! as SOGo, Stalwart, Nextcloud do. MailCraft stays the one that mails them, once the server has
//! the change; a guest's answer to a meeting the server put in the calendar is the server's to
//! send. Found against a real Mailcow, Stalwart and Radicale
//! (`apps/mailcraft-cli/tests/real_scheduling.rs`).

use mailcraft_model::ical::{self, Attendee, CalEvent};
use mailcraft_model::pim::{CalendarInfo, DavSync, StoredEvent};

use super::invite_placing::{OWN, arrive, imap_session, parts};
use super::*;

const ANN: &str = "ann@studio.example";

/// What the server said in a sync: whether it schedules, what it stored and deleted, and its
/// copies read again.
fn synced(s: &mut Session, id: &str, schedules: bool, pushed: Vec<StoredEvent>, events: Vec<StoredEvent>, deleted: Vec<String>) {
    let sync = DavSync {
        caldav_home: Some("https://dav.example.org/SOGo/dav/riley/Calendar/".into()),
        calendars: Some(vec![CalendarInfo { id: OWN.into(), name: "Personal Calendar".into(), ctag: "1".into(), ..Default::default() }]),
        events,
        synced_calendars: vec![OWN.into()],
        pushed_events: pushed,
        deleted,
        auto_schedule: Some(schedules),
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.into(), sync: Box::new(sync) });
}

/// The meeting messages sent: their method and calendar.
fn sent(s: &mut Session) -> Vec<(String, String)> {
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::Send { raw, .. } => {
                let text = String::from_utf8_lossy(&raw).to_string();
                let method = text.split("method=").nth(1).map(|m| m.chars().take_while(char::is_ascii_alphabetic).collect()).unwrap_or_default();
                let ics = mailcraft_mime::parse(&raw)
                    .and_then(|p| p.attachments.first().and_then(|a| mailcraft_mime::attachment_data(&raw, a.index)))
                    .map(|d| String::from_utf8_lossy(&d).to_string())
                    .unwrap_or_default();
                Some((method, ics))
            }
            _ => None,
        })
        .collect()
}

/// The stored copy of an event as the server has it now: its item, a new etag, and what the
/// server changed (`change`).
fn as_stored(s: &Session, id: &str, uid: &str, change: impl Fn(&mut CalEvent)) -> StoredEvent {
    let mine = parts(s, id, uid)[0].clone();
    let mut e = mine.event.clone();
    change(&mut e);
    StoredEvent { href: format!("{OWN}{uid}.ics"), etag: format!("\"{}\"", e.sequence + 7), event: e, dirty: false, ..mine }
}

/// The server stored what was sent (`pushed`), and its copy is read again.
fn stored(s: &mut Session, id: &str, uid: &str, change: impl Fn(&mut CalEvent)) {
    let server = as_stored(s, id, uid, change);
    let mine = parts(s, id, uid)[0].clone();
    let pushed = StoredEvent { href: server.href.clone(), etag: server.etag.clone(), dirty: false, ..mine };
    synced(s, id, true, vec![pushed], vec![server], Vec::new());
}

/// The user's meeting on a server that schedules: nothing is mailed until the server has it,
/// then the guests get the version it stored (SOGo raised its SEQUENCE: the copy it put in a
/// colleague's calendar is that one, so MailCraft's mail isn't "out of date" there). A change,
/// the same; the meeting called off, once the server deleted it.
#[test]
fn the_users_meeting_is_mailed_once_the_server_has_it() {
    let (mut s, id) = imap_session("sched-organizer");
    synced(&mut s, &id, true, Vec::new(), Vec::new(), Vec::new());
    s.take_jobs();
    let uid = s
        .run("event.create", &json!({"account": id, "summary": "Planning", "start": "2026-11-10T10:00", "attendees": format!("Ann <{ANN}>")}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(sent(&mut s).is_empty(), "not before the server has it");
    assert_eq!(s.pim[&id].notices.len(), 1);
    // The sync failed: still waiting.
    synced(&mut s, &id, true, Vec::new(), Vec::new(), Vec::new());
    assert!(sent(&mut s).is_empty() && s.pim[&id].notices.len() == 1);
    stored(&mut s, &id, &uid, |e| e.sequence = 1);
    let mail = sent(&mut s);
    assert_eq!(mail.len(), 1, "{mail:?}");
    assert_eq!(mail[0].0, "REQUEST");
    assert!(mail[0].1.contains("SEQUENCE:1") && !mail[0].1.contains("SCHEDULE-AGENT"), "{}", mail[0].1);
    assert!(s.pim[&id].notices.is_empty());

    s.run("event.update", &json!({"uid": uid, "summary": "Planning (moved)"})).unwrap();
    assert!(sent(&mut s).is_empty());
    stored(&mut s, &id, &uid, |e| e.sequence = 3);
    let mail = sent(&mut s);
    assert!(mail.len() == 1 && mail[0].1.contains("SEQUENCE:3") && mail[0].1.contains("Planning (moved)"), "{mail:?}");

    s.run("event.delete", &json!({"uid": uid})).unwrap();
    assert!(sent(&mut s).is_empty(), "not before the server deleted it");
    let href = format!("{OWN}{uid}.ics");
    assert!(s.pim[&id].deleted.iter().any(|t| t.href == href));
    synced(&mut s, &id, true, Vec::new(), Vec::new(), vec![href]);
    let mail = sent(&mut s);
    assert!(mail.len() == 1 && mail[0].0 == "CANCEL" && mail[0].1.contains("SEQUENCE:4"), "{mail:?}");
    done(s);
}

/// Called off before the server had it: the guests, never invited, hear nothing. And a server
/// that delivered the invitations itself after all (it says so: `SCHEDULE-STATUS`), though
/// asked not to: MailCraft doesn't send them again.
#[test]
fn nothing_is_mailed_twice_or_for_nothing() {
    let (mut s, id) = imap_session("sched-nothing");
    synced(&mut s, &id, true, Vec::new(), Vec::new(), Vec::new());
    let create = |s: &mut Session, title: &str| {
        s.run("event.create", &json!({"account": id, "summary": title, "start": "2026-11-10T10:00", "attendees": ANN})).unwrap()["uid"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let gone = create(&mut s, "Gone");
    s.run("event.delete", &json!({"uid": gone})).unwrap();
    assert!(sent(&mut s).is_empty() && s.pim[&id].notices.is_empty(), "{:?}", s.pim[&id].notices);

    let theirs = create(&mut s, "Delivered");
    stored(&mut s, &id, &theirs, |e| e.server_delivered = true);
    assert!(sent(&mut s).is_empty(), "the server sent them");
    done(s);
}

/// On a server that doesn't schedule, everything is mailed at once, as before.
#[test]
fn a_server_that_doesnt_schedule_changes_nothing() {
    let (mut s, id) = imap_session("sched-plain");
    synced(&mut s, &id, false, Vec::new(), Vec::new(), Vec::new());
    s.take_jobs();
    s.run("event.create", &json!({"account": id, "summary": "Planning", "start": "2026-11-10T10:00", "attendees": ANN})).unwrap();
    assert_eq!(sent(&mut s).len(), 1);
    assert!(s.pim[&id].notices.is_empty());
    done(s);
}

/// A meeting a colleague on the same server organised, as the server put it in the user's
/// calendar.
fn colleagues(uid: &str, partstat: &str) -> StoredEvent {
    let e = CalEvent {
        uid: uid.into(),
        summary: "Review".into(),
        start: NOW + 86_400,
        end: NOW + 90_000,
        organizer: Some(Attendee { email: ANN.into(), ..Default::default() }),
        attendees: vec![Attendee { email: "riley@studio.example".into(), partstat: partstat.into(), rsvp: true, ..Default::default() }],
        // Copied as the organizer's MailCraft stored it.
        client_scheduled: true,
        ..Default::default()
    };
    StoredEvent { calendar: OWN.into(), href: format!("{OWN}{uid}.ics"), etag: "\"1\"".into(), event: e, dirty: false }
}

/// A meeting the server put in the user's calendar: answered there, and the server takes the
/// answer to the organizer (SOGo mails it, or writes it in the organizer's copy). MailCraft
/// mailing it too gave the organizer two.
#[test]
fn an_answer_to_a_meeting_the_server_put_there_is_the_servers_to_send() {
    let (mut s, id) = imap_session("sched-delivered");
    synced(&mut s, &id, true, Vec::new(), vec![colleagues("c1", "NEEDS-ACTION")], Vec::new());
    s.take_jobs();
    s.run("event.respond", &json!({"uid": "c1", "answer": "accepted"})).unwrap();
    assert!(sent(&mut s).is_empty());
    stored(&mut s, &id, "c1", |_| {});
    assert!(sent(&mut s).is_empty(), "the server sends it");
    assert!(s.pim[&id].notices.is_empty());
    // Declined: deleted on the server, which tells the organizer.
    s.run("event.respond", &json!({"uid": "c1", "answer": "declined"})).unwrap();
    stored(&mut s, &id, "c1", |_| {});
    assert!(sent(&mut s).is_empty());
    done(s);
}

/// An invitation from elsewhere, put in the calendar by MailCraft (the copy is marked as the
/// guest's own): answered from the calendar, and again, MailCraft mails each answer once the
/// server has it.
#[test]
fn an_answer_to_a_guests_own_copy_is_mailed() {
    let (mut s, id) = imap_session("sched-guest-copy");
    let mut copy = colleagues("g1", "ACCEPTED");
    copy.event.organizer = Some(Attendee { email: "zoe@elsewhere.example".into(), ..Default::default() });
    ical::mark_guest_copy(&mut copy.event);
    synced(&mut s, &id, true, Vec::new(), vec![copy], Vec::new());
    assert_eq!(s.pim[&id].guest_copies, ["g1"], "learnt from the mark");
    s.take_jobs();
    s.run("event.respond", &json!({"uid": "g1", "answer": "tentative"})).unwrap();
    assert!(sent(&mut s).is_empty(), "not before the server has it");
    stored(&mut s, &id, "g1", |_| {});
    let mail = sent(&mut s);
    assert!(mail.len() == 1 && mail[0].0 == "REPLY", "{mail:?}");
    assert!(mail[0].1.contains("PARTSTAT=TENTATIVE") && !mail[0].1.contains(ical::GUEST_COPY), "{}", mail[0].1);
    done(s);
}

/// An invitation answered in its message before the calendar was read again: MailCraft can't
/// tell yet whether the server has the meeting (a colleague's) or not (from elsewhere). The
/// answer waits for the sync: the server had it, so it's the server's to send; it didn't, so
/// the copy MailCraft stored is the guest's own, and MailCraft mails it.
#[test]
fn an_answer_from_the_message_waits_to_know_who_sends_it() {
    for server_had_it in [true, false] {
        let (mut s, id) = imap_session(if server_had_it { "sched-msg-theirs" } else { "sched-msg-mine" });
        synced(&mut s, &id, true, Vec::new(), Vec::new(), Vec::new());
        let inv = CalEvent { client_scheduled: false, ..colleagues("m1", "NEEDS-ACTION").event };
        let key = arrive(&mut s, &id, 1, "REQUEST", &[inv]);
        s.take_jobs();
        s.run("list.open", &json!({"key": key.to_id()})).unwrap();
        s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
        assert!(sent(&mut s).is_empty());
        stored(&mut s, &id, "m1", |e| {
            if !server_had_it {
                ical::mark_guest_copy(e);
            }
        });
        let mail = sent(&mut s);
        assert_eq!(mail.len(), usize::from(!server_had_it), "{mail:?}");
        done(s);
    }
}

/// An invitation from elsewhere declined in its message, its copy stored on the server before:
/// the answer is mailed once the copy is deleted there (MailCraft's own copy, which the server
/// answers nothing for).
#[test]
fn a_guests_own_copy_declined_is_mailed_once_deleted() {
    let (mut s, id) = imap_session("sched-declined");
    let mut copy = colleagues("d1", "NEEDS-ACTION");
    copy.event.organizer = Some(Attendee { email: "zoe@studio.example".into(), ..Default::default() });
    ical::mark_guest_copy(&mut copy.event);
    synced(&mut s, &id, true, Vec::new(), vec![copy.clone()], Vec::new());
    let mut inv = copy.event.clone();
    inv.extra.clear();
    inv.client_scheduled = false;
    let key = arrive(&mut s, &id, 1, "REQUEST", &[inv]);
    s.take_jobs();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
    assert!(parts(&s, &id, "d1").is_empty());
    assert!(sent(&mut s).is_empty(), "not before it's deleted");
    synced(&mut s, &id, true, Vec::new(), Vec::new(), vec![copy.href.clone()]);
    let mail = sent(&mut s);
    assert!(mail.len() == 1 && mail[0].0 == "REPLY" && mail[0].1.contains("PARTSTAT=DECLINED"), "{mail:?}");
    done(s);
}

/// Microsoft Graph mails the invitations, changes and cancellations of a meeting stored there
/// itself ("When you create an event that includes attendees, the server sends invitations"):
/// MailCraft's own made two of each. A guest's answer still goes by mail (Graph's copy can't
/// be answered by changing it).
#[test]
fn graph_sends_the_users_meeting_itself() {
    let mut s = Session::open(Some(temp("sched-graph")), mailcraft_model::dates::Clock::new(NOW, 0));
    s.oauth_available = vec!["microsoft".into()];
    let id = s.run("account.add", &json!({"email": "ana@contoso.example", "graph": true})).unwrap()["account"].as_str().unwrap().to_string();
    let cal = "graph:calendar".to_string();
    let sync = DavSync {
        calendars: Some(vec![CalendarInfo { id: cal.clone(), name: "Calendar".into(), ..Default::default() }]),
        synced_calendars: vec![cal.clone()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    s.take_jobs();
    let uid = s.run("event.create", &json!({"account": id, "summary": "Planning", "start": "2026-11-10T10:00", "attendees": ANN})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(parts(&s, &id, &uid)[0].calendar, cal);
    s.run("event.update", &json!({"uid": uid, "summary": "Planning (moved)"})).unwrap();
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    assert!(sent(&mut s).is_empty());
    assert!(s.pim[&id].notices.is_empty());
    // One occurrence of a series off: Graph's copy keeps it (it's stored without exceptions), so
    // Graph cancels nothing, and MailCraft mails that occurrence's cancellation as it did.
    let uid = s
        .run("event.create", &json!({"account": id, "summary": "Weekly", "start": "2026-11-10T10:00", "repeat": "weekly", "attendees": ANN}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(sent(&mut s).is_empty());
    let week = parts(&s, &id, &uid)[0].event.start + 7 * 86_400;
    s.run("event.delete", &json!({"uid": uid, "occurrence": week})).unwrap();
    let mail = sent(&mut s);
    assert!(mail.len() == 1 && mail[0].0 == "CANCEL" && mail[0].1.contains("RECURRENCE-ID"), "{mail:?}");
    done(s);
}
