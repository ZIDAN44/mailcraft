//! Engine tests: a meeting the user is invited to, forwarded to someone else (issue #234): the
//! organizer's invitation goes to them, with them among its guests; the user's own copy and
//! answer stay; an organizer who doesn't allow it isn't overruled.

use mailcraft_model::ical::CalEvent;

use super::*;
use super::{WEEK, ZOE, deliver, invitation, parts, weekly_series};

/// The meeting in the message sent with `subject` from the sample's Sent.
fn sent_meeting(s: &Session, subject: &str) -> (String, Vec<String>, CalEvent) {
    let st = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    let m = st.messages.iter().find(|m| m.subject == subject).unwrap_or_else(|| panic!("no {subject}"));
    let cal = s.message_calendar(&m.key).unwrap();
    (cal.method.unwrap_or_default(), m.to.iter().map(|a| a.email.clone()).collect(), cal.events.into_iter().next().unwrap())
}

#[test]
fn a_meeting_is_forwarded_as_the_organizers_invitation() {
    let mut s = sample_session("forward-meeting");
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation("f1", "Grade review", 2)], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let before = s.find_event("f1").unwrap().1.event.clone();
    // Answered, the invitation is tidied away: forwarded from the calendar.
    let r = s.run("invite.forward", &json!({"uid": "f1", "to": "Sam Okafor <sam@studio.example>, riley@studio.example"})).unwrap();
    assert_eq!(r["forwarded"], 1, "not to oneself");
    let (method, to, e) = sent_meeting(&s, "Fwd: Grade review");
    assert_eq!((method.as_str(), to), ("REQUEST", vec!["sam@studio.example".to_string()]));
    assert_eq!((e.uid.as_str(), e.sequence), ("f1", 2), "the same meeting, the same version");
    assert_eq!(e.organizer.as_ref().map(|o| o.email.as_str()), Some(ZOE.1), "the organizer's still");
    let sam = e.attendees.iter().find(|a| a.email == "sam@studio.example").expect("a guest now");
    assert_eq!((sam.partstat.as_str(), sam.rsvp), ("NEEDS-ACTION", true));
    assert!(e.attendees.iter().any(|a| a.email == "riley@studio.example" && a.partstat == "ACCEPTED"), "the user's place as it is");
    assert_eq!(s.find_event("f1").unwrap().1.event, before, "the user's copy unchanged");
    // From an invitation not answered yet, open.
    deliver(&mut s, 3, ZOE, "REQUEST", &[invitation("f3", "Trailer", 0)], NOW);
    s.run("invite.forward", &json!({"to": "ana@soundlab.example"})).unwrap();
    assert_eq!(sent_meeting(&s, "Fwd: Trailer").2.uid, "f3");

    // Not allowed by the organizer, no one to forward to, or one's own meeting: refused.
    let closed = CalEvent { extra: vec!["X-MICROSOFT-DONOTFORWARDMEETING:TRUE".into()], ..invitation("f2", "Board", 0) };
    assert!(!crate::forwarding_allowed(&closed));
    deliver(&mut s, 2, ZOE, "REQUEST", &[closed], NOW);
    assert!(s.run("invite.forward", &json!({"to": "sam@studio.example"})).is_err());
    assert!(s.run("invite.forward", &json!({"uid": "f1", "to": ""})).is_err());
    assert!(s.run("invite.forward", &json!({"uid": "f1", "to": "not an address"})).is_err());
    let own = s.run("event.create", &json!({"summary": "Mine", "start": NOW + 86_400, "attendees": "sam@studio.example"})).unwrap();
    assert!(s.run("invite.forward", &json!({"uid": own["uid"], "to": "ana@soundlab.example"})).is_err());
}

/// Issue #234: one occurrence of a series forwarded ("This occurrence") goes alone, at its own
/// time, as one delegated does; the whole series went to the guest every week. The user's copy
/// stays as it was, no part made for it.
#[test]
fn one_occurrence_is_forwarded_on_its_own() {
    let mut s = sample_session("forward-occurrence");
    let series = weekly_series(&mut s);
    let before: Vec<CalEvent> = parts(&s, &series.uid).iter().map(|p| p.event.clone()).collect();
    let at = series.start + 3 * WEEK;
    let r = s.run("invite.forward", &json!({"uid": series.uid, "to": "Sam Okafor <sam@studio.example>", "occurrence": at})).unwrap();
    assert_eq!(r["occurrence"], at);
    let (method, to, e) = sent_meeting(&s, "Fwd: Mix review");
    assert_eq!((method.as_str(), to), ("REQUEST", vec!["sam@studio.example".to_string()]));
    assert_eq!((e.recurrence_id, e.start, e.rrule.as_deref()), (Some(at), at, None), "that occurrence alone");
    assert!(e.attendees.iter().any(|a| a.email == "sam@studio.example" && a.partstat == "NEEDS-ACTION"));
    let st = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    let m = st.messages.iter().find(|m| m.subject == "Fwd: Mix review").unwrap();
    let body = String::from_utf8_lossy(&s.store.as_ref().unwrap().read_body(&m.key).unwrap()).to_string();
    let day = mailcraft_model::dates::full_time(at, &s.clock);
    assert!(body.contains(&day), "the mail says that occurrence's day: {day}");
    let after: Vec<CalEvent> = parts(&s, &series.uid).iter().map(|p| p.event.clone()).collect();
    assert_eq!(after, before, "the user's copy unchanged");
    // The whole series, as before; an occurrence the series hasn't, refused.
    s.run("invite.forward", &json!({"uid": series.uid, "to": "kim@studio.example"})).unwrap();
    assert!(s.run("invite.forward", &json!({"uid": series.uid, "to": "kim@studio.example", "occurrence": at + 60})).is_err());
}
