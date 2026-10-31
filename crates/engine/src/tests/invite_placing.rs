//! Engine tests: invitations that arrive before an account's calendars on its server are found
//! (issue #30) are held, then put there; the calendar on this computer gives its events up to
//! the server's; the account's own calendar is the default rather than someone else's.

use mailcraft_model::ical::{Attendee, CalEvent};
use mailcraft_model::pim::{CalendarInfo, DavSync, LOCAL, StoredEvent};
use mailcraft_model::{Folder, MessageKey, Summary};

use super::*;

const ZOE: (&str, &str) = ("Zoe Park", "zoe@studio.example");
const OWN: &str = "https://dav.example.org/SOGo/dav/riley/Calendar/personal/";
const THEIRS: &str = "https://dav.example.org/SOGo/dav/riley/Calendar/sam_team/";

/// An IMAP account with an Inbox whose first look is done (so what comes next arrives).
fn imap_session(name: &str) -> (Session, String) {
    let store = temp(name);
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "riley@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    folders(&mut s, &id);
    save_inbox(&s, &id, Vec::new());
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    s.take_jobs();
    (s, id)
}

/// The account's folders, as its server lists them on connecting.
fn folders(s: &mut Session, id: &str) {
    let f = |p: &str, role| Folder { account: id.into(), path: p.into(), role, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.into(), folders: vec![f("INBOX", FolderRole::Inbox), f("Trash", FolderRole::Trash)] });
}

fn save_inbox(s: &Session, id: &str, messages: Vec<Summary>) {
    let st = mailcraft_store::FolderState { validity: 7, uid_next: 1000, highest_modseq: 0, messages, window: 0 };
    s.store.as_ref().unwrap().save_folder(id, "INBOX", &st).unwrap();
}

fn invitation(uid: &str, sequence: u32) -> CalEvent {
    CalEvent {
        uid: uid.into(),
        summary: format!("Meeting {uid}"),
        start: NOW + 86_400,
        end: NOW + 90_000,
        organizer: Some(Attendee { email: ZOE.1.into(), name: ZOE.0.into(), ..Default::default() }),
        attendees: vec![Attendee { email: "riley@studio.example".into(), partstat: "NEEDS-ACTION".into(), rsvp: true, ..Default::default() }],
        sequence,
        ..Default::default()
    }
}

/// A message carrying a calendar arrives in the account's Inbox (its body downloaded).
fn arrive(s: &mut Session, id: &str, n: u32, method: &str, events: &[CalEvent]) -> MessageKey {
    let ics = mailcraft_model::ical::write(events, Some(method), NOW);
    let d = mailcraft_mime::Draft {
        to: vec![mailcraft_model::EmailAddress::new("Riley", "riley@studio.example")],
        subject: format!("{method} {n}"),
        body: "See the calendar.".into(),
        attachments: vec![mailcraft_mime::OutAttachment {
            name: "invite.ics".into(),
            mime: format!("text/calendar; method={method}"),
            data: ics.into_bytes(),
        }],
        ..Default::default()
    };
    let raw = mailcraft_mime::build(&d, &mailcraft_model::EmailAddress::new(ZOE.0, ZOE.1), NOW, &format!("<p{n}@x>")).unwrap();
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder(id, "INBOX").unwrap();
    let key = MessageKey { account: id.into(), folder: "INBOX".into(), validity: 7, uid: n };
    st.messages.insert(0, mailcraft_mime::summary_from_full(&raw, key.clone(), mailcraft_model::Flags::default(), 0, NOW));
    store.save_folder(id, "INBOX", &st).unwrap();
    store.write_body(&key, &raw).unwrap();
    s.apply(Event::FolderChanged { account: id.into(), folder: "INBOX".into() });
    key
}

fn cal(id: &str, name: &str, foreign: bool) -> CalendarInfo {
    CalendarInfo { id: id.into(), name: name.into(), ctag: "1".into(), foreign, ..Default::default() }
}

/// The server's calendars found: someone else's (writable, listed first, as SOGo can) and the
/// account's own, with these events in them.
fn found(s: &mut Session, id: &str, events: Vec<StoredEvent>) {
    let sync = DavSync {
        caldav_home: Some("https://dav.example.org/SOGo/dav/riley/Calendar/".into()),
        calendars: Some(vec![cal(THEIRS, "Sam's team", true), cal(OWN, "Personal Calendar", false)]),
        events,
        synced_calendars: vec![THEIRS.into(), OWN.into()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.into(), sync: Box::new(sync) });
}

fn parts<'a>(s: &'a Session, id: &str, uid: &str) -> Vec<&'a StoredEvent> {
    s.pim[id].events.iter().filter(|e| e.event.uid == uid).collect()
}

/// A calendar sync is asked for, once (a second would start before the first one's results are
/// merged, and send its new items again).
fn asks_sync(s: &mut Session) -> bool {
    s.take_jobs().iter().filter(|j| matches!(j, Job::SyncDav { .. })).count() == 1
}

/// Held until the calendars are found (across a restart), then put in the account's own
/// calendar on the server, not someone else's listed first, to be stored there.
#[test]
fn held_until_the_calendars_are_found() {
    let (mut s, id) = imap_session("placing-held");
    assert!(s.calendars_pending(&id));
    arrive(&mut s, &id, 1, "REQUEST", &[invitation("a@studio.example", 0)]);
    assert!(parts(&s, &id, "a@studio.example").is_empty(), "not on this computer's calendar");
    assert_eq!(s.pim[&id].held.len(), 1);
    // A restart keeps it.
    let store = s.store.clone().unwrap();
    drop(s);
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW + 60, 0));
    assert_eq!(s.pim[&id].held.len(), 1, "kept with the calendar");
    s.take_jobs();
    found(&mut s, &id, Vec::new());
    let e = parts(&s, &id, "a@studio.example");
    assert_eq!(e.len(), 1);
    assert!(e[0].calendar == OWN && e[0].dirty && e[0].href.is_empty(), "{:?}", e[0]);
    assert!(s.pim[&id].held.is_empty() && s.pim[&id].placed.is_empty());
    assert!(asks_sync(&mut s), "stored on the server next");
    assert!(!s.calendars_pending(&id));
    // Found: what arrives now goes in at once (after the first look since the restart).
    folders(&mut s, &id);
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    arrive(&mut s, &id, 2, "REQUEST", &[invitation("b@studio.example", 0)]);
    assert_eq!(parts(&s, &id, "b@studio.example")[0].calendar, OWN);
    done(s);
}

/// Waited too long (the server slow or unreachable): on this computer, then moved to the
/// server's calendar once found; one the server has already keeps the server's copy with the
/// answer given here. A rule moving the message away meanwhile loses nothing.
#[test]
fn too_long_to_wait_goes_here_then_moves() {
    let (mut s, id) = imap_session("placing-timeout");
    let key = arrive(&mut s, &id, 1, "REQUEST", &[invitation("slow@studio.example", 0)]);
    arrive(&mut s, &id, 2, "REQUEST", &[invitation("dup@studio.example", 0)]);
    // As if a rule had moved it on: its body isn't under that key any more.
    s.store.as_ref().unwrap().remove_body(&key);
    s.clock.now = NOW + crate::session::invite::HOLD_SECS - 1;
    s.tick();
    assert!(parts(&s, &id, "slow@studio.example").is_empty(), "still waiting");
    s.clock.now = NOW + crate::session::invite::HOLD_SECS;
    s.tick();
    assert_eq!(parts(&s, &id, "slow@studio.example")[0].calendar, LOCAL);
    assert_eq!(s.pim[&id].placed, ["slow@studio.example", "dup@studio.example"]);
    // Answered here meanwhile.
    s.run("event.respond", &json!({"uid": "dup@studio.example", "answer": "accepted"})).unwrap();
    s.take_jobs();
    // The server put one in its calendar itself (CalDAV scheduling).
    let server_copy = StoredEvent {
        calendar: OWN.into(),
        href: format!("{OWN}dup.ics"),
        etag: "\"1\"".into(),
        event: invitation("dup@studio.example", 0),
        dirty: false,
    };
    found(&mut s, &id, vec![server_copy]);
    let slow = parts(&s, &id, "slow@studio.example");
    assert!(slow.len() == 1 && slow[0].calendar == OWN && slow[0].dirty && slow[0].href.is_empty(), "{slow:?}");
    let dup = parts(&s, &id, "dup@studio.example");
    assert_eq!(dup.len(), 1, "not doubled: {dup:?}");
    assert!(dup[0].calendar == OWN && dup[0].href.ends_with("dup.ics") && dup[0].dirty, "{dup:?}");
    assert!(dup[0].event.attendees.iter().any(|a| a.email == "riley@studio.example" && a.partstat == "ACCEPTED"), "the answer kept");
    let p = &s.pim[&id];
    assert!(p.placed.is_empty() && !p.calendars.iter().any(|c| c.id == LOCAL), "the calendar here is empty, so gone");
    assert!(asks_sync(&mut s));
    done(s);
}

/// An account whose server has no calendars: invitations go here, as they should; given a
/// calendar server by hand later (issue #7), they move there.
#[test]
fn no_calendars_then_a_server_given_by_hand() {
    let (mut s, id) = imap_session("placing-given");
    arrive(&mut s, &id, 1, "REQUEST", &[invitation("x@studio.example", 0)]);
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(DavSync { checked: NOW, ..Default::default() }) });
    assert_eq!(parts(&s, &id, "x@studio.example")[0].calendar, LOCAL);
    assert!(s.local_events(&id).is_none(), "nowhere to move them yet");
    // One the user made here: not MailCraft's to move by itself.
    s.run("event.create", &json!({"account": id, "summary": "Dentist", "start": NOW + 3600, "end": NOW + 7200})).unwrap();
    s.run("account.dav", &json!({"account": id, "caldav": "https://dav.example.org/SOGo/dav/riley/"})).unwrap();
    assert!(s.calendars_pending(&id), "looked for again");
    arrive(&mut s, &id, 2, "REQUEST", &[invitation("y@studio.example", 0)]);
    assert!(parts(&s, &id, "y@studio.example").is_empty(), "held");
    s.take_jobs();
    found(&mut s, &id, Vec::new());
    assert!(asks_sync(&mut s), "one sync for the one moved and the one held");
    assert_eq!(parts(&s, &id, "x@studio.example")[0].calendar, OWN);
    assert_eq!(parts(&s, &id, "y@studio.example")[0].calendar, OWN);
    let dentist = s.pim[&id].events.iter().find(|e| e.event.summary == "Dentist").unwrap();
    assert_eq!(dentist.calendar, LOCAL);
    // Offered: Move 1 event to Personal Calendar.
    let offer = s.local_events(&id).unwrap();
    assert_eq!((offer.count, offer.calendar.id.as_str()), (1, OWN));
    let r = s.run("calendar.moveLocal", &json!({"account": id})).unwrap();
    assert_eq!(r["moved"], 1);
    assert!(s.pim[&id].events.iter().all(|e| e.calendar == OWN));
    assert!(s.local_events(&id).is_none() && !s.pim[&id].calendars.iter().any(|c| c.id == LOCAL));
    done(s);
}

/// The order invitations came in is kept: one cancelled while held never shows.
#[test]
fn held_in_order() {
    let (mut s, id) = imap_session("placing-order");
    arrive(&mut s, &id, 1, "REQUEST", &[invitation("c@studio.example", 0)]);
    arrive(&mut s, &id, 2, "CANCEL", &[invitation("c@studio.example", 1)]);
    // The same message twice is held once.
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    assert_eq!(s.pim[&id].held.len(), 2);
    found(&mut s, &id, Vec::new());
    assert!(parts(&s, &id, "c@studio.example").is_empty());
    done(s);
}

/// The Default calendar: the user's choice, kept to calendars that can be added to; offered
/// moves can be declined for good.
#[test]
fn the_default_calendar_and_keeping_events_here() {
    let (mut s, id) = imap_session("placing-default");
    s.run("event.create", &json!({"account": id, "summary": "Gym", "start": NOW + 3600, "end": NOW + 7200})).unwrap();
    found(&mut s, &id, Vec::new());
    assert_eq!(s.pim[&id].default_calendar().unwrap().id, OWN, "the account's own, not the one listed first");
    s.run("calendar.setDefault", &json!({"account": id, "calendar": THEIRS})).unwrap();
    assert_eq!(s.pim[&id].default_calendar().unwrap().id, THEIRS, "the user's choice");
    assert_eq!(s.local_events(&id).unwrap().calendar.id, THEIRS);
    for bad in [json!({"account": id, "calendar": "nope"}), json!({"account": "nobody", "calendar": OWN})] {
        assert!(s.run("calendar.setDefault", &bad).is_err(), "{bad}");
    }
    s.pim.get_mut(&id).unwrap().calendars.push(CalendarInfo { id: "https://dav.example.org/ro/".into(), read_only: true, ..Default::default() });
    assert!(s.run("calendar.setDefault", &json!({"account": id, "calendar": "https://dav.example.org/ro/"})).is_err());
    assert!(s.run("calendar.moveLocal", &json!({"account": id, "calendar": "https://dav.example.org/ro/"})).is_err());
    assert!(s.run("calendar.moveLocal", &json!({"account": id, "calendar": LOCAL})).is_err());
    s.run("calendar.setDefault", &json!({"account": id, "calendar": ""})).unwrap();
    assert_eq!(s.pim[&id].default_calendar().unwrap().id, OWN);
    s.run("calendar.keepLocal", &json!({"account": id})).unwrap();
    assert!(s.local_events(&id).is_none(), "not offered again");
    assert!(s.pim[&id].events.iter().any(|e| e.calendar == LOCAL));
    // The sample mailbox's calendar is on this computer only.
    let mut sample = sample_session("placing-sample");
    assert!(!sample.calendars_pending("sample") && sample.local_events("sample").is_none());
    assert!(sample.run("calendar.moveLocal", &json!({"account": "sample"})).is_err());
    done(sample);
    done(s);
}

/// An OAuth account without a calendar sign-in of its own is never asked for calendars: its
/// invitations don't wait.
#[test]
fn accounts_never_asked_dont_wait() {
    let (mut s, id) = imap_session("placing-oauth");
    s.accounts.iter_mut().filter(|a| a.config.id == id).for_each(|a| a.config.oauth = Some("google".into()));
    assert!(!s.calendars_pending(&id));
    arrive(&mut s, &id, 1, "REQUEST", &[invitation("o@studio.example", 0)]);
    assert_eq!(parts(&s, &id, "o@studio.example")[0].calendar, LOCAL);
    done(s);
}

/// Trying to break it: an invitation too big to hold, more than can be held, a held calendar
/// that's junk (a damaged `pim.json`), and the account removed while some wait. Nothing is lost
/// that can be kept, nothing panics.
#[test]
fn holding_has_its_limits() {
    let (mut s, id) = imap_session("placing-limits");
    // Too big to keep as text: put on this computer at once.
    let mut big = invitation("big@studio.example", 0);
    big.description = "x".repeat(300 << 10);
    arrive(&mut s, &id, 1, "REQUEST", &[big]);
    assert_eq!(parts(&s, &id, "big@studio.example")[0].calendar, LOCAL);
    assert!(s.pim[&id].held.is_empty());
    // Junk in what's held.
    let junk = mailcraft_model::pim::HeldInvite {
        key: MessageKey::default(),
        since: NOW,
        ics: "BEGIN:VCALENDAR\r\n\u{0}\u{ffff}".into(),
        from: String::new(),
    };
    s.pim.get_mut(&id).unwrap().held.push(junk);
    // More than can be held: the rest go in at once.
    for n in 0..205u32 {
        arrive(&mut s, &id, 10 + n, "REQUEST", &[invitation(&format!("m{n}@studio.example"), 0)]);
    }
    let held = s.pim[&id].held.len();
    assert_eq!(held, 200, "{held}");
    assert_eq!(s.pim[&id].events.iter().filter(|e| e.event.uid.starts_with('m')).count(), 6, "past the limit: at once");
    found(&mut s, &id, Vec::new());
    assert_eq!(s.pim[&id].events.iter().filter(|e| e.event.uid.starts_with('m') && e.calendar == OWN).count(), 205);
    // Removed while invitations wait: they go with it.
    let (mut s2, id2) = imap_session("placing-removed");
    arrive(&mut s2, &id2, 1, "REQUEST", &[invitation("r@studio.example", 0)]);
    s2.pim.get_mut(&id2).unwrap().held[0].since = NOW - 10_000;
    s2.accounts.retain(|a| a.config.id != id2);
    s2.tick();
    assert!(s2.pim[&id2].held.is_empty() && s2.pim[&id2].events.is_empty());
    done(s2);
    done(s);
}

/// Trying to break it: a calendar sent for reading isn't held (it has nothing for the
/// calendar), and big invitations are held only up to a size: the rest go to the calendar on
/// this computer at once, none lost, so the calendar's file stays small.
#[test]
fn only_invitations_are_held_and_only_so_much() {
    let (mut s, id) = imap_session("placing-held-bytes");
    arrive(&mut s, &id, 1, "PUBLISH", &[invitation("p@studio.example", 0)]);
    assert!(s.pim.get(&id).is_none_or(|p| p.held.is_empty()), "a published calendar isn't held");
    let big = |n: u32| CalEvent { description: "x".repeat(240 << 10), ..invitation(&format!("big{n}@studio.example"), 0) };
    for n in 0..12 {
        arrive(&mut s, &id, 10 + n, "REQUEST", &[big(n)]);
    }
    let held = s.pim[&id].held.len();
    let bytes: usize = s.pim[&id].held.iter().map(|h| h.ics.len()).sum();
    assert!(held > 0 && held < 12 && bytes <= 2 << 20, "{held} held, {bytes} bytes");
    let here = (0..12).filter(|n| parts(&s, &id, &format!("big{n}@studio.example")).iter().any(|e| e.calendar == LOCAL)).count();
    assert_eq!(held + here, 12, "none lost");
    found(&mut s, &id, Vec::new());
    assert!((0..12).all(|n| parts(&s, &id, &format!("big{n}@studio.example")).iter().all(|e| e.calendar == OWN)));
    done(s);
}

/// Trying to break it: an invitation answered (declined) from its message while held, or a
/// cancellation taken in while its invitation is held, doesn't come back into the calendar as
/// tentative once the calendars are found.
#[test]
fn dealt_with_while_held_stays_dealt_with() {
    let (mut s, id) = imap_session("placing-declined-held");
    let key = arrive(&mut s, &id, 1, "REQUEST", &[invitation("no@studio.example", 0)]);
    arrive(&mut s, &id, 2, "REQUEST", &[invitation("off@studio.example", 0)]);
    let cancel = arrive(&mut s, &id, 3, "CANCEL", &[invitation("off@studio.example", 1)]);
    // A newer version of the declined one, also held: still to be seen.
    arrive(&mut s, &id, 4, "REQUEST", &[invitation("later@studio.example", 0)]);
    assert_eq!(s.pim[&id].held.len(), 4);
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
    s.run("list.open", &json!({"key": cancel.to_id()})).unwrap();
    s.run("invite.apply", &json!({})).unwrap();
    found(&mut s, &id, Vec::new());
    assert!(parts(&s, &id, "no@studio.example").is_empty(), "declined: not back as tentative");
    assert!(parts(&s, &id, "off@studio.example").is_empty(), "cancelled: not back");
    assert_eq!(parts(&s, &id, "later@studio.example").len(), 1, "the others held still go in");
    done(s);
}
