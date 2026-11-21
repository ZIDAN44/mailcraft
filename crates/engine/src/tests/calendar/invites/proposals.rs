//! Engine tests: a new time proposed for a meeting (issue #238, iTIP's COUNTER and
//! DECLINECOUNTER): sent by a guest, taken or turned down by the organizer, refused where the
//! organizer takes none.

use mailcraft_model::ical::{Attendee, CalEvent};

use super::*;
use super::{ZOE, deliver, invitation};

const SAM: (&str, &str) = ("Sam Okafor", "sam@studio.example");

#[test]
fn a_guest_proposes_a_new_time() {
    let mut s = sample_session("propose-guest");
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation("p1", "Grade review", 0)], NOW);
    let later = NOW + 3 * 86_400;
    let r = s.run("invite.propose", &json!({"start": later, "answer": "tentative"})).unwrap();
    assert_eq!(r["proposed"], later);
    // Answered as asked; the meeting itself not moved here.
    let stored = s.find_event("p1").unwrap().1.event.clone();
    assert_eq!(stored.start, NOW + 2 * 86_400, "the organizer moves it, if they take it");
    assert_eq!(stored.attendees.iter().find(|a| a.email == "riley@studio.example").map(|a| a.partstat.as_str()), Some("TENTATIVE"));
    let (method, raw) = sent_calendar(&s, "Sent", "New time proposed: Grade review");
    assert_eq!(method, "COUNTER");
    assert!(raw.contains("COUNTER"));
    let key = s
        .store
        .as_ref()
        .unwrap()
        .load_folder("sample", "Sent")
        .unwrap()
        .messages
        .iter()
        .find(|m| m.subject.starts_with("New time"))
        .map(|m| m.key.clone())
        .unwrap();
    let sent = s.message_calendar(&key).unwrap().events.into_iter().find(|e| e.uid == "p1").expect("the meeting");
    assert_eq!((sent.start, sent.end), (later, later + 3600), "as long as the meeting");
    assert!(sent.attendees.len() == 1 && sent.attendees[0].email == "riley@studio.example", "the user's place only");
    // From the calendar too, with an end of its own.
    s.run("invite.propose", &json!({"uid": "p1", "start": later + 3600, "end": later + 9000})).unwrap();
    // The same time as the meeting's: nothing to propose.
    assert!(s.run("invite.propose", &json!({"uid": "p1", "start": stored.start})).is_err());
    // An organizer who takes no proposals: none sent.
    let closed = CalEvent { extra: vec!["X-MICROSOFT-DISALLOW-COUNTER:TRUE".into()], ..invitation("p2", "Board", 0) };
    assert!(!crate::proposals_allowed(&closed));
    deliver(&mut s, 2, ZOE, "REQUEST", &[closed], NOW);
    assert!(s.run("invite.propose", &json!({"start": later})).is_err());
    // The user's own meeting: not theirs to propose for.
    let own = s.run("event.create", &json!({"summary": "Mine", "start": NOW + 86_400, "attendees": "sam@studio.example"})).unwrap();
    assert!(s.run("invite.propose", &json!({"uid": own["uid"], "start": later})).is_err());
}

/// The user's meeting a day out with Zoe and Sam invited.
fn meeting(s: &mut Session) -> CalEvent {
    let p = json!({"summary": "Mix", "start": NOW + 86_400, "end": NOW + 90_000, "attendees": "Zoe Park <zoe@studio.example>, sam@studio.example"});
    let r = s.run("event.create", &p).unwrap();
    s.find_event(r["uid"].as_str().unwrap()).unwrap().1.event.clone()
}

/// `from`'s proposal of `start`..`end` for `own`.
fn counter(own: &CalEvent, from: (&str, &str), start: i64, end: i64) -> CalEvent {
    CalEvent {
        start,
        end,
        organizer: Some(Attendee { email: "riley@studio.example".into(), ..Default::default() }),
        attendees: vec![Attendee { email: from.1.into(), name: from.0.into(), partstat: "TENTATIVE".into(), rsvp: false }],
        ..own.clone()
    }
}

#[test]
fn the_organizer_takes_or_turns_down_a_proposal() {
    let mut s = sample_session("propose-organizer");
    let own = meeting(&mut s);
    let (start, end) = (NOW + 2 * 86_400, NOW + 2 * 86_400 + 3600);
    deliver(&mut s, 1, ZOE, "COUNTER", &[counter(&own, ZOE, start, end)], NOW);
    let inv = s.invite().expect("the proposal");
    assert_eq!((inv.method.as_str(), inv.problem, inv.replier.map(|r| r.email)), ("COUNTER", None, Some(ZOE.1.to_string())));
    assert_eq!(inv.event.start, start, "the time proposed");
    s.run("invite.acceptProposal", &json!({})).unwrap();
    let moved = s.find_event(&own.uid).unwrap().1.event.clone();
    assert_eq!((moved.start, moved.end), (start, end));
    assert!(moved.sequence > own.sequence, "a new version for the guests");
    let (method, raw) = sent_calendar(&s, "Sent", "Invitation: Mix");
    assert_eq!(method, "REQUEST");
    assert!(raw.contains("zoe@studio.example") && raw.contains("sam@studio.example"), "every guest told");

    // Sam proposes too, for the version before: out of date, nothing to take.
    deliver(&mut s, 2, SAM, "COUNTER", &[counter(&own, SAM, start + 3600, end + 3600)], NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::OutOfDate));
    assert!(s.run("invite.acceptProposal", &json!({})).is_err());
    // For the meeting as it is: turned down, Sam told, the meeting as it was.
    deliver(&mut s, 3, SAM, "COUNTER", &[counter(&moved, SAM, start + 3600, end + 3600)], NOW);
    s.run("invite.declineProposal", &json!({})).unwrap();
    assert_eq!(s.find_event(&own.uid).unwrap().1.event.start, start);
    let (method, raw) = sent_calendar(&s, "Sent", "New time declined: Mix");
    assert_eq!(method, "DECLINECOUNTER");
    assert!(raw.contains(SAM.1));

    // Someone not on the list, or proposing for a guest they aren't: not taken.
    let bo = ("Bo", "bo@elsewhere.example");
    deliver(&mut s, 4, bo, "COUNTER", &[counter(&moved, bo, start + 7200, end + 7200)], NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::NotFromGuest));
    deliver(&mut s, 5, ("Mallory", "mallory@evil.example"), "COUNTER", &[counter(&moved, ZOE, start + 7200, end + 7200)], NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::NotFromGuest));
    assert!(s.run("invite.acceptProposal", &json!({})).is_err());
    assert_eq!(s.find_event(&own.uid).unwrap().1.event.start, start, "unmoved");
}

/// The organizer's no to the user's proposal: said, from them only.
#[test]
fn a_declined_proposal_is_said() {
    let mut s = sample_session("propose-declined");
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation("p3", "Grade review", 0)], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let mut no = invitation("p3", "Grade review", 0);
    no.attendees = vec![super::me("ACCEPTED")];
    deliver(&mut s, 2, ZOE, "DECLINECOUNTER", std::slice::from_ref(&no), NOW);
    let inv = s.invite().unwrap();
    assert_eq!((inv.method.as_str(), inv.problem), ("DECLINECOUNTER", None));
    deliver(&mut s, 3, ("Mallory", "mallory@evil.example"), "DECLINECOUNTER", &[no], NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::NotFromOrganizer));
}

/// Found with GreenMail and Radicale: a proposal made from an update not answered yet is for that
/// version, not the calendar's older one (which the organizer took for out of date).
#[test]
fn a_proposal_is_for_the_newest_version() {
    let mut s = sample_session("propose-newest");
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation("p4", "Grade review", 0)], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let moved = CalEvent { start: NOW + 3 * 86_400, end: NOW + 3 * 86_400 + 3600, ..invitation("p4", "Grade review", 1) };
    deliver(&mut s, 2, ZOE, "REQUEST", &[moved], NOW + 60);
    s.run("invite.propose", &json!({"start": NOW + 4 * 86_400})).unwrap();
    let st = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    let key = st.messages.iter().find(|m| m.subject.starts_with("New time proposed")).map(|m| m.key.clone()).unwrap();
    let sent = s.message_calendar(&key).unwrap().events.into_iter().next().unwrap();
    assert_eq!((sent.sequence, sent.end - sent.start), (1, 3600), "the update's version, its length");
}
