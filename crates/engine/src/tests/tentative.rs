//! Engine tests: meeting invitations in the calendar as tentative as soon as they arrive, before
//! they're answered (issue #20).

use mailcraft_model::ical::{Attendee, CalEvent};

use super::invites::{deliver, me, parts};
use super::*;

const ZOE: (&str, &str) = ("Zoe Park", "zoe@studio.example");

fn invitation(uid: &str, summary: &str, sequence: u32) -> CalEvent {
    CalEvent {
        uid: uid.into(),
        summary: summary.into(),
        start: NOW + 2 * 86_400,
        end: NOW + 2 * 86_400 + 3600,
        organizer: Some(Attendee { email: ZOE.1.into(), name: ZOE.0.into(), ..Default::default() }),
        attendees: vec![me("NEEDS-ACTION")],
        sequence,
        ..Default::default()
    }
}

/// What the sample account's Sent Items holds: answers go there (it has no server).
fn sent(s: &mut Session) -> usize {
    let path = s.account("sample").and_then(|a| a.folder(FolderRole::Sent)).map(|f| f.path.clone()).unwrap_or_default();
    s.folder_state("sample", &path).messages.len()
}

fn answer(s: &Session, uid: &str) -> Vec<String> {
    parts(s, uid).iter().flat_map(|e| e.event.attendees.iter()).filter(|a| a.email == "riley@studio.example").map(|a| a.partstat.clone()).collect()
}

/// A new invitation is in the calendar, tentative, the moment it arrives; nothing is sent until
/// it's answered, and answering it keeps one copy.
#[test]
fn an_invitation_is_in_the_calendar_as_it_arrives() {
    let mut s = sample_session("tentative-arrives");
    let before = sent(&mut s);
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation("crit@studio.example", "Storyboard crit", 0)], NOW);
    assert_eq!(answer(&s, "crit@studio.example"), ["NEEDS-ACTION"]);
    let item = s.agenda(NOW, NOW + 7 * 86_400).into_iter().find(|i| i.uid == "crit@studio.example").expect("on the calendar");
    assert!(item.needs_reply, "drawn as tentative");
    assert_eq!(sent(&mut s), before, "nothing went to the organizer");
    let inv = s.invite().unwrap();
    assert!(inv.in_calendar && inv.problem.is_none() && inv.answer == "NEEDS-ACTION", "{inv:?}");
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(answer(&s, "crit@studio.example"), ["ACCEPTED"]);
    assert_eq!(sent(&mut s), before + 1, "the answer went");
    // Deleting the invitation leaves the meeting, as Outlook does.
    s.run("mail.delete", &json!({})).unwrap();
    assert_eq!(parts(&s, "crit@studio.example").len(), 1);
    done(s);
}

/// Sent by a calendar service for the organizer (Google's sender isn't the organizer): it can
/// still be answered, and declining takes it out.
#[test]
fn an_invitation_sent_for_the_organizer_can_be_answered() {
    let mut s = sample_session("tentative-service");
    deliver(
        &mut s,
        1,
        ("Zoe Park (calendar)", "calendar-notification@calendar.example"),
        "REQUEST",
        &[invitation("svc@studio.example", "Colour pass", 2)],
        NOW,
    );
    assert_eq!(answer(&s, "svc@studio.example"), ["NEEDS-ACTION"]);
    assert_eq!(s.invite().unwrap().problem, None);
    s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
    assert!(parts(&s, "svc@studio.example").is_empty(), "declined: out of the calendar");
    done(s);
}

/// Updates from the organizer change it (keeping the user's answer once given); one from anyone
/// else, or an older one, changes nothing; a cancellation takes out one never answered and marks
/// an answered one cancelled.
#[test]
fn updates_and_cancellations_from_the_organizer_follow() {
    let mut s = sample_session("tentative-updates");
    let uid = "grade@studio.example";
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation(uid, "Grade review", 1)], NOW);
    deliver(
        &mut s,
        2,
        ZOE,
        "REQUEST",
        &[CalEvent { start: NOW + 3 * 86_400, end: NOW + 3 * 86_400 + 3600, ..invitation(uid, "Grade review (moved)", 2) }],
        NOW,
    );
    let e = &parts(&s, uid)[0].event;
    assert_eq!((e.summary.as_str(), e.start, e.sequence), ("Grade review (moved)", NOW + 3 * 86_400, 2));
    assert_eq!(answer(&s, uid), ["NEEDS-ACTION"]);
    // Someone else's version, and an older one: nothing.
    deliver(&mut s, 3, ("Mallory", "mallory@evil.example"), "REQUEST", &[invitation(uid, "Grade review at my place", 9)], NOW);
    deliver(&mut s, 4, ZOE, "REQUEST", &[invitation(uid, "Grade review (old)", 1)], NOW);
    assert_eq!(parts(&s, uid)[0].event.summary, "Grade review (moved)");
    // Nor can anyone else cancel it.
    deliver(&mut s, 5, ("Mallory", "mallory@evil.example"), "CANCEL", &[invitation(uid, "Grade review", 9)], NOW);
    assert_eq!(parts(&s, uid).len(), 1);
    // The organizer's cancellation, never answered: gone.
    deliver(&mut s, 6, ZOE, "CANCEL", &[invitation(uid, "Grade review", 3)], NOW);
    assert!(parts(&s, uid).is_empty());

    // Answered, then updated: the answer stays; then cancelled: kept, marked.
    let uid = "mix@studio.example";
    deliver(&mut s, 7, ZOE, "REQUEST", &[invitation(uid, "Mix", 0)], NOW);
    s.run("invite.respond", &json!({"answer": "tentative"})).unwrap();
    deliver(&mut s, 8, ZOE, "REQUEST", &[invitation(uid, "Mix (room 3)", 1)], NOW);
    assert_eq!(parts(&s, uid)[0].event.summary, "Mix (room 3)");
    assert_eq!(answer(&s, uid), ["TENTATIVE"]);
    deliver(&mut s, 9, ZOE, "CANCEL", &[invitation(uid, "Mix", 2)], NOW);
    assert_eq!(parts(&s, uid)[0].event.status, "CANCELLED");
    done(s);
}

/// One already in the calendar (the server put it there, RFC 6638) isn't added twice; with the
/// option off, nothing is added; the user's own meeting isn't theirs to answer.
#[test]
fn nothing_is_added_twice_or_when_turned_off() {
    let mut s = sample_session("tentative-twice");
    let stored = s.find_event("mix@sample.mailcraft").unwrap().1.event.clone();
    deliver(&mut s, 1, ("Ana Lima", "ana@soundlab.example"), "REQUEST", std::slice::from_ref(&stored), NOW);
    assert_eq!(parts(&s, "mix@sample.mailcraft").len(), 1);
    // The same invitation twice (a copy of the mail): one.
    deliver(&mut s, 2, ZOE, "REQUEST", &[invitation("twice@studio.example", "Twice", 0)], NOW);
    deliver(&mut s, 3, ZOE, "REQUEST", &[invitation("twice@studio.example", "Twice", 0)], NOW);
    assert_eq!(parts(&s, "twice@studio.example").len(), 1);
    // My own meeting, sent to me as well.
    let mine = CalEvent {
        organizer: Some(Attendee { email: "riley@studio.example".into(), ..Default::default() }),
        ..invitation("own@studio.example", "Own", 0)
    };
    deliver(&mut s, 4, ("Riley", "riley@studio.example"), "REQUEST", &[mine], NOW);
    assert!(parts(&s, "own@studio.example").is_empty());
    s.run("calendar.tentativeInvites", &json!({"on": false})).unwrap();
    deliver(&mut s, 5, ZOE, "REQUEST", &[invitation("off@studio.example", "Off", 0)], NOW);
    assert!(parts(&s, "off@studio.example").is_empty());
    done(s);
}

/// Mail that arrives with its header only is looked at when its body comes.
#[test]
fn an_invitation_whose_body_comes_later_is_taken_then() {
    let mut s = sample_session("tentative-later");
    let ics = mailcraft_model::ical::write(&[invitation("later@studio.example", "Later", 0)], Some("REQUEST"), NOW);
    let d = mailcraft_mime::Draft {
        to: vec![mailcraft_model::EmailAddress::new("Riley", "riley@studio.example")],
        subject: "Invitation: Later".into(),
        body: "See the calendar.".into(),
        attachments: vec![mailcraft_mime::OutAttachment {
            name: "invite.ics".into(),
            mime: "text/calendar; method=REQUEST".into(),
            data: ics.into_bytes(),
        }],
        ..Default::default()
    };
    let raw = mailcraft_mime::build(&d, &mailcraft_model::EmailAddress::new(ZOE.0, ZOE.1), NOW, "<later@x>").unwrap();
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = mailcraft_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9500 };
    inbox.messages.insert(0, mailcraft_mime::summary_from_header(&raw, key.clone(), mailcraft_model::Flags::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    assert!(parts(&s, "later@studio.example").is_empty(), "no body yet");
    store.write_body(&key, &raw).unwrap();
    s.apply(Event::Body { key, raw });
    assert_eq!(answer(&s, "later@studio.example"), ["NEEDS-ACTION"]);
    done(s);
}
