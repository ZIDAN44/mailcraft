//! Engine tests: invitations already in the Inbox when an account comes (issue #237): those
//! waiting for an answer and not in the calendar are found when asked, and the ones picked go in
//! unanswered, with no answer sent.

use mailcraft_model::ical::CalEvent;

use super::*;
use super::{ZOE, deliver, invitation};

/// A message carrying `events` filed in `folder` without being taken in as arriving.
fn filed(s: &mut Session, n: u32, folder: &str, from: (&str, &str), method: &str, events: &[CalEvent]) {
    let ics = mailcraft_model::ical::write(events, Some(method), NOW);
    let raw = calendar_mail(from, &format!("{method} {n}"), &format!("text/calendar; method={method}"), ics.into_bytes(), &format!("<b{n}@x>"));
    let key = mailcraft_model::MessageKey { uid: 9300 + n, ..next_key(s, "sample", folder) };
    file_whole(s, key, &raw);
}

#[test]
fn waiting_invitations_are_found_and_placed_unanswered() {
    let mut s = sample_session("backlog");
    // Mail already there when the account came: nothing of it was taken in as it arrived.
    s.prefs.tentative_invites = false;
    s.prefs.record_answers = false;
    filed(&mut s, 1, "INBOX", ZOE, "REQUEST", &[invitation("b-wait", "Grade review", 0)]);
    // Two versions: the newest only.
    filed(&mut s, 2, "INBOX", ZOE, "REQUEST", &[invitation("b-two", "Trailer", 0)]);
    let moved = CalEvent { start: NOW + 5 * 86_400, end: NOW + 5 * 86_400 + 3600, ..invitation("b-two", "Trailer", 1) };
    filed(&mut s, 3, "INBOX", ZOE, "REQUEST", std::slice::from_ref(&moved));
    // Called off since.
    filed(&mut s, 4, "INBOX", ZOE, "REQUEST", &[invitation("b-off", "Board", 0)]);
    filed(&mut s, 5, "INBOX", ZOE, "CANCEL", &[CalEvent { status: "CANCELLED".into(), ..invitation("b-off", "Board", 1) }]);
    // Over already.
    let past = CalEvent { start: NOW - 3 * 86_400, end: NOW - 3 * 86_400 + 3600, ..invitation("b-past", "Old", 0) };
    filed(&mut s, 6, "INBOX", ZOE, "REQUEST", &[past]);
    // Answered already: the user's answer in Sent.
    filed(&mut s, 7, "INBOX", ZOE, "REQUEST", &[invitation("b-said", "Lunch", 0)]);
    let mut answer = invitation("b-said", "Lunch", 0);
    answer.attendees = vec![super::me("ACCEPTED")];
    filed(&mut s, 8, "Sent", ("Riley", "riley@studio.example"), "REPLY", &[answer]);
    // In the calendar already.
    deliver(&mut s, 9, ZOE, "REQUEST", &[invitation("b-there", "Mix", 0)], NOW);
    s.run("invite.respond", &json!({"answer": "tentative"})).unwrap();
    s.take_jobs();

    let r = s.run("invite.findWaiting", &json!({"account": "sample"})).unwrap();
    let uids: Vec<&str> = r["found"].as_array().unwrap().iter().filter_map(|f| f["uid"].as_str()).collect();
    assert_eq!(uids, ["b-wait", "b-two"], "{r}");
    let two = r["found"].as_array().unwrap().iter().find(|f| f["uid"] == "b-two").unwrap();
    assert_eq!(two["start"], moved.start, "its newest version");
    assert_eq!(r["reading"], 0);

    // Picked: into the calendar unanswered; nothing sent.
    let keys: Vec<Value> = r["found"].as_array().unwrap().iter().map(|f| f["key"].clone()).collect();
    assert_eq!(s.run("invite.placeWaiting", &json!({"keys": keys})).unwrap()["placed"], 2);
    for uid in ["b-wait", "b-two"] {
        let e = s.find_event(uid).unwrap_or_else(|| panic!("{uid} in the calendar")).1.event.clone();
        assert_eq!(e.attendees.iter().find(|a| a.email == "riley@studio.example").map(|a| a.partstat.as_str()), Some("NEEDS-ACTION"));
    }
    assert_eq!(s.find_event("b-two").unwrap().1.event.start, moved.start);
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })), "no answer sent");
    let sent = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    assert!(!sent.messages.iter().any(|m| m.subject.contains("Grade review") || m.subject.contains("Trailer")));
    // Asked again: none left.
    let r = s.run("invite.findWaiting", &json!({})).unwrap();
    assert_eq!(r["found"].as_array().map(Vec::len), Some(0), "{r}");
}
