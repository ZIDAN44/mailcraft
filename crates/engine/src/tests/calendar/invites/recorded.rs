//! Engine tests: guests' answers to the user's meetings recorded as they arrive (issue #236), as
//! Outlook, Exchange and SOGo do; those that need a decision (someone not on the list, an answer
//! to an earlier version, one not from the guest it names) wait for Record the answer.

use mailcraft_model::ical::{Attendee, CalEvent};

use super::*;
use super::{ZOE, deliver};

/// The user's meeting a day out with Zoe and Sam invited; its event.
fn meeting(s: &mut Session) -> CalEvent {
    let p = json!({"summary": "Mix", "start": NOW + 86_400, "end": NOW + 90_000, "attendees": "Zoe Park <zoe@studio.example>, sam@studio.example"});
    let r = s.run("event.create", &p).unwrap();
    s.find_event(r["uid"].as_str().unwrap()).unwrap().1.event.clone()
}

/// `from`'s answer `stat` to `own`, as their program sends it.
fn reply(own: &CalEvent, from: (&str, &str), stat: &str) -> CalEvent {
    CalEvent {
        organizer: Some(Attendee { email: "riley@studio.example".into(), ..Default::default() }),
        attendees: vec![Attendee { email: from.1.into(), name: from.0.into(), partstat: stat.into(), rsvp: false, ..Default::default() }],
        ..own.clone()
    }
}

/// What the meeting in the calendar says of `email`'s answer.
fn answer_of(s: &Session, uid: &str, email: &str) -> Option<String> {
    s.find_event(uid).and_then(|(_, e)| e.event.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(email)).map(|a| a.partstat.clone()))
}

/// A message carrying `events` with `method` arriving in the sample Inbox, not opened (one tidied as
/// it arrives isn't there to open).
fn arrive(s: &mut Session, n: u32, from: (&str, &str), method: &str, events: &[CalEvent]) {
    let ics = mailcraft_model::ical::write(events, Some(method), NOW);
    let raw = calendar_mail(from, &format!("{method} {n}"), &format!("text/calendar; method={method}"), ics.into_bytes(), &format!("<r{n}@x>"));
    let key = mailcraft_model::MessageKey { uid: 9100 + n, ..next_key(s, "sample", "INBOX") };
    file_whole(s, key, &raw);
}

fn in_inbox(s: &mut Session, subject: &str) -> bool {
    let path = s.account("sample").and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()).unwrap_or_default();
    s.folder_state("sample", &path).messages.iter().any(|m| m.subject == subject)
}

#[test]
fn a_guests_answer_is_recorded_as_it_arrives() {
    let mut s = sample_session("recorded-arrives");
    let own = meeting(&mut s);
    deliver(&mut s, 1, ZOE, "REPLY", &[reply(&own, ZOE, "ACCEPTED")], NOW);
    assert_eq!(answer_of(&s, &own.uid, ZOE.1).as_deref(), Some("ACCEPTED"), "recorded without asking");
    assert!(s.find_event(&own.uid).unwrap().1.dirty || s.find_event(&own.uid).unwrap().1.calendar == mailcraft_model::pim::LOCAL);
    let inv = s.invite().expect("the answer");
    assert!(inv.recorded && inv.problem.is_none(), "the card says so: {inv:?}");
    assert!(in_inbox(&mut s, "REPLY 1"), "tidied only as the option for answers says (off)");
    // Recorded again by the button: nothing more changes, and it says so the same.
    s.run("invite.apply", &json!({})).unwrap();
    assert_eq!(answer_of(&s, &own.uid, ZOE.1).as_deref(), Some("ACCEPTED"));
    // With answers tidied: recorded, and moved out of the Inbox.
    s.run("options.answeredInvites", &json!({"replies": true})).unwrap();
    arrive(&mut s, 2, ("Sam", "sam@studio.example"), "REPLY", &[reply(&own, ("Sam", "sam@studio.example"), "DECLINED")]);
    assert_eq!(answer_of(&s, &own.uid, "sam@studio.example").as_deref(), Some("DECLINED"));
    assert!(!in_inbox(&mut s, "REPLY 2"), "tidied");
}

#[test]
fn answers_that_need_a_decision_wait() {
    let mut s = sample_session("recorded-waits");
    let own = meeting(&mut s);
    // Someone not on the list (the meeting forwarded to them): theirs waits.
    let bo = ("Bo", "bo@elsewhere.example");
    deliver(&mut s, 1, bo, "REPLY", &[reply(&own, bo, "ACCEPTED")], NOW);
    assert_eq!(answer_of(&s, &own.uid, bo.1), None, "not added without asking");
    assert!(!s.invite().unwrap().recorded);
    s.run("invite.apply", &json!({})).unwrap();
    assert_eq!(answer_of(&s, &own.uid, bo.1).as_deref(), Some("ACCEPTED"), "added when asked");
    // Not from the guest it names (sent by someone else): not taken.
    deliver(&mut s, 2, ("Mallory", "mallory@evil.example"), "REPLY", &[reply(&own, ZOE, "DECLINED")], NOW);
    assert_eq!(answer_of(&s, &own.uid, ZOE.1).as_deref(), Some("NEEDS-ACTION"));
    // An answer to an earlier version of the meeting: waits.
    let p = s.pim_mut("sample");
    if let Some(e) = p.events.iter_mut().find(|e| e.event.uid == own.uid) {
        e.event.sequence = own.sequence + 1;
    }
    deliver(&mut s, 3, ZOE, "REPLY", &[reply(&own, ZOE, "ACCEPTED")], NOW);
    assert_eq!(answer_of(&s, &own.uid, ZOE.1).as_deref(), Some("NEEDS-ACTION"), "to an earlier version");
    // The option off: nothing recorded by itself.
    s.run("calendar.recordAnswers", &json!({"on": false})).unwrap();
    let newest = CalEvent { sequence: own.sequence + 1, ..own.clone() };
    deliver(&mut s, 4, ZOE, "REPLY", &[reply(&newest, ZOE, "TENTATIVE")], NOW);
    assert_eq!(answer_of(&s, &own.uid, ZOE.1).as_deref(), Some("NEEDS-ACTION"), "off");
    s.run("calendar.recordAnswers", &json!({"on": true})).unwrap();
    deliver(&mut s, 5, ZOE, "REPLY", &[reply(&newest, ZOE, "TENTATIVE")], NOW);
    assert_eq!(answer_of(&s, &own.uid, ZOE.1).as_deref(), Some("TENTATIVE"), "on again");
}
