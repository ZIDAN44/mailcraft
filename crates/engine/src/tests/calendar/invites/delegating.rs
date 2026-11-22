//! Engine tests: a meeting the user is invited to, delegated to someone else (issue #254, RFC
//! 5546 §3.2.2.3): they're sent the organizer's invitation as the user's delegate, the organizer
//! the user's answer `DELEGATED` with them; the user's copy stays, handed on and free. The
//! organizer's copy takes the delegate in, and their answer then.

use mailcraft_model::ical::{self, Attendee, CalEvent};

use super::*;
use super::{ANA, WEEK, ZOE, deliver, invitation, me, parts, weekly_series};

const SAM: (&str, &str) = ("Sam Okafor", "sam@studio.example");
const KIM: (&str, &str) = ("Kim Ito", "kim@studio.example");

/// The meetings sent from the sample's Sent with `subject`, newest last: each one's method, to
/// whom, and its events.
fn sent(s: &Session, subject: &str) -> Vec<(String, Vec<String>, Vec<CalEvent>)> {
    let st = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    let mut found: Vec<_> = st.messages.iter().filter(|m| m.subject == subject).collect();
    found.sort_by_key(|m| m.key.uid);
    found
        .into_iter()
        .map(|m| {
            let cal = s.message_calendar(&m.key).unwrap();
            (cal.method.unwrap_or_default(), m.to.iter().map(|a| a.email.clone()).collect(), cal.events)
        })
        .collect()
}

fn guest<'a>(e: &'a CalEvent, email: &str) -> &'a Attendee {
    e.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(email)).unwrap_or_else(|| panic!("{email} isn't a guest: {:?}", e.attendees))
}

#[test]
fn a_meeting_is_delegated_to_someone_else() {
    let mut s = sample_session("delegate-meeting");
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation("d1", "Grade review", 2)], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let r = s.run("invite.delegate", &json!({"uid": "d1", "to": "Sam Okafor <sam@studio.example>"})).unwrap();
    assert_eq!(r["delegated"], SAM.1);
    // Sam: the organizer's invitation, Sam a guest the user handed the place to, asked to answer.
    let [(method, to, events)] = sent(&s, "Invitation: Grade review").try_into().unwrap_or_else(|v: Vec<_>| panic!("{}", v.len()));
    assert_eq!((method.as_str(), to.as_slice()), ("REQUEST", [SAM.1.to_string()].as_slice()));
    let e = &events[0];
    assert_eq!((e.uid.as_str(), e.sequence, e.organizer.as_ref().map(|o| o.email.as_str())), ("d1", 2, Some(ZOE.1)));
    let sam = guest(e, SAM.1);
    assert_eq!(
        (sam.partstat.as_str(), sam.rsvp, sam.delegated_from.as_str(), sam.name.as_str()),
        ("NEEDS-ACTION", true, "riley@studio.example", SAM.0)
    );
    assert_eq!((guest(e, "riley@studio.example").partstat.as_str(), guest(e, "riley@studio.example").delegated_to.as_str()), ("DELEGATED", SAM.1));
    assert_eq!(ical::reminder_minutes(e), None, "the user's reminders are their own");
    // Zoe: the user's answer, and whom to; the delegate with it, and no one else.
    let replies = sent(&s, "Delegated: Grade review");
    let (method, to, events) = replies.last().expect("the organizer is answered");
    assert_eq!((method.as_str(), to.as_slice()), ("REPLY", [ZOE.1.to_string()].as_slice()));
    let emails: Vec<&str> = events[0].attendees.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(emails, ["riley@studio.example", SAM.1]);
    assert_eq!(guest(&events[0], "riley@studio.example").delegated_to, SAM.1);
    // The user's copy: still there, handed on, and free.
    let copy = s.find_event("d1").unwrap().1.event.clone();
    assert_eq!(
        (guest(&copy, "riley@studio.example").partstat.as_str(), guest(&copy, SAM.1).delegated_from.as_str()),
        ("DELEGATED", "riley@studio.example")
    );
    assert_eq!(ical::show_as(&copy), ical::ShowAs::Free);

    // From the invitation open and not answered: into the calendar as delegated.
    deliver(&mut s, 2, ZOE, "REQUEST", &[invitation("d2", "Trailer", 0)], NOW);
    s.run("invite.delegate", &json!({"to": "kim@studio.example"})).unwrap();
    let copy = s.find_event("d2").expect("in the calendar").1.event.clone();
    assert_eq!(guest(&copy, "riley@studio.example").delegated_to, KIM.1);
    assert!(s.invite().is_none_or(|i| i.answer == "DELEGATED"), "tidied away as an answered one is, or saying so");
    assert_eq!(sent(&s, "Invitation: Trailer").len(), 1);

    // Refused: no one, two people, oneself, the organizer, one's own meeting.
    for to in ["", "sam@studio.example, kim@studio.example", "riley@studio.example", ZOE.1] {
        assert!(s.run("invite.delegate", &json!({"uid": "d2", "to": to})).is_err(), "{to}");
    }
    let own = s.run("event.create", &json!({"summary": "Mine", "start": NOW + 86_400, "attendees": "sam@studio.example"})).unwrap();
    assert!(s.run("invite.delegate", &json!({"uid": own["uid"], "to": "ana@soundlab.example"})).is_err());
}

/// One occurrence of a series: that one alone is handed on, the series stays the user's.
#[test]
fn one_occurrence_is_delegated_on_its_own() {
    let mut s = sample_session("delegate-occurrence");
    let series = weekly_series(&mut s);
    let at = series.start + 2 * WEEK;
    s.run("invite.delegate", &json!({"uid": series.uid, "to": "Kim Ito <kim@studio.example>", "occurrence": at})).unwrap();
    let all = parts(&s, &series.uid);
    let master = all.iter().find(|p| p.event.recurrence_id.is_none()).unwrap();
    assert_eq!(guest(&master.event, "riley@studio.example").partstat, "ACCEPTED", "the series stays");
    assert!(master.event.attendees.iter().all(|a| a.email != KIM.1));
    let one = all.iter().find(|p| p.event.recurrence_id == Some(at)).expect("the occurrence on its own");
    assert_eq!((guest(&one.event, "riley@studio.example").partstat.as_str(), one.dirty), ("DELEGATED", true));
    let [(_, to, events)] = sent(&s, "Invitation: Mix review").try_into().unwrap_or_else(|v: Vec<_>| panic!("{}", v.len()));
    assert_eq!(to, [KIM.1]);
    assert_eq!(events.iter().map(|e| e.recurrence_id).collect::<Vec<_>>(), [Some(at)], "that occurrence alone");
    let (_, to, events) = sent(&s, "Delegated: Mix review").pop().expect("Ana answered");
    assert_eq!((to, events[0].recurrence_id), (vec![ANA.1.to_string()], Some(at)));
}

/// The organizer's side: a guest's answer handing their place on adds the delegate, waiting; the
/// delegate's own answer is then recorded as it arrives.
#[test]
fn the_organizer_takes_in_the_delegate() {
    let mut s = sample_session("delegate-organizer");
    let p = json!({"summary": "Mix", "start": NOW + 86_400, "end": NOW + 90_000, "attendees": "Zoe Park <zoe@studio.example>, sam@studio.example"});
    let uid = s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    let own = s.find_event(&uid).unwrap().1.event.clone();
    let answer = |who: Vec<Attendee>| CalEvent { organizer: Some(me("")), attendees: who, ..own.clone() };
    let sam = Attendee { email: SAM.1.into(), partstat: "DELEGATED".into(), delegated_to: KIM.1.into(), ..Default::default() };
    let kim = Attendee { email: KIM.1.into(), name: KIM.0.into(), rsvp: true, delegated_from: SAM.1.into(), ..Default::default() };
    deliver(&mut s, 1, SAM, "REPLY", &[answer(vec![sam, kim.clone()])], NOW);
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((guest(&e, SAM.1).partstat.as_str(), guest(&e, SAM.1).delegated_to.as_str()), ("DELEGATED", KIM.1));
    let added = guest(&e, KIM.1);
    assert_eq!((added.partstat.as_str(), added.delegated_from.as_str(), added.name.as_str()), ("NEEDS-ACTION", SAM.1, KIM.0));
    deliver(&mut s, 2, KIM, "REPLY", &[answer(vec![Attendee { partstat: "ACCEPTED".into(), rsvp: false, ..kim }])], NOW);
    assert_eq!(guest(&s.find_event(&uid).unwrap().1.event, KIM.1).partstat, "ACCEPTED", "recorded as it arrived");
    // Someone named by an answer as handed a place, by a guest who didn't delegate: not added.
    let zoe = Attendee { email: ZOE.1.into(), partstat: "ACCEPTED".into(), ..Default::default() };
    let bo = Attendee { email: "bo@elsewhere.example".into(), delegated_from: ZOE.1.into(), ..Default::default() };
    deliver(&mut s, 3, ZOE, "REPLY", &[answer(vec![zoe, bo])], NOW);
    assert!(s.find_event(&uid).unwrap().1.event.attendees.iter().all(|a| a.email != "bo@elsewhere.example"));
    // An answer naming several as handed the place: only the one it hands it to. Handed again,
    // the one before who hasn't answered goes (answers in a guest's name can't fill the list).
    let to = |email: &str| Attendee { email: SAM.1.into(), partstat: "DELEGATED".into(), delegated_to: email.into(), ..Default::default() };
    let handed = |email: &str| Attendee { email: email.into(), delegated_from: SAM.1.into(), ..Default::default() };
    deliver(
        &mut s,
        4,
        SAM,
        "REPLY",
        &[answer(vec![to("x1@elsewhere.example"), handed("x1@elsewhere.example"), handed("x2@elsewhere.example")])],
        NOW,
    );
    deliver(&mut s, 5, SAM, "REPLY", &[answer(vec![to("x3@elsewhere.example"), handed("x3@elsewhere.example")])], NOW);
    let emails: Vec<String> = s.find_event(&uid).unwrap().1.event.attendees.iter().map(|a| a.email.clone()).collect();
    assert_eq!(emails, ["zoe@studio.example", SAM.1, KIM.1, "x3@elsewhere.example"], "Kim, who answered, stays");
}

/// The delegate's side, when a server that schedules put the meeting in their calendar before the
/// delegating guest's invitation came (SOGo does): it's answered all the same, the calendar's
/// copy kept as it is but for the answer; a newer version still has to come from the organizer.
#[test]
fn a_delegates_invitation_is_answered_when_the_calendar_has_it_already() {
    let mut s = sample_session("delegate-arrives");
    let sam = Attendee {
        email: SAM.1.into(),
        name: SAM.0.into(),
        partstat: "DELEGATED".into(),
        delegated_to: "riley@studio.example".into(),
        ..Default::default()
    };
    let riley = Attendee { delegated_from: SAM.1.into(), ..me("NEEDS-ACTION") };
    let meeting = CalEvent { attendees: vec![sam, riley], dtstamp: NOW - 86_400, ..invitation("d9", "Grading", 3) };
    let p = s.pim_mut("sample");
    p.events.push(mailcraft_model::pim::StoredEvent { calendar: mailcraft_model::pim::LOCAL.into(), event: meeting.clone(), ..Default::default() });
    s.save_pim("sample");
    let passed = CalEvent { summary: "Grading, moved by Sam".into(), start: meeting.start + 3600, dtstamp: NOW, ..meeting.clone() };
    deliver(&mut s, 1, SAM, "REQUEST", &[passed], NOW);
    // Arrived (stamped later than the calendar's copy), it changed nothing: only the organizer's do.
    assert_eq!(s.find_event("d9").unwrap().1.event.summary, "Grading");
    let inv = s.invite().expect("the invitation");
    assert_eq!((inv.problem, inv.handed_by.as_deref()), (None, Some(SAM.0)), "{inv:?}");
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let copy = s.find_event("d9").unwrap().1.event.clone();
    assert_eq!((copy.summary.as_str(), copy.start), ("Grading", meeting.start), "the calendar's copy stays");
    assert_eq!(guest(&copy, "riley@studio.example").partstat, "ACCEPTED");
    // A newer version from the guest, or the same from someone not a guest: not taken.
    deliver(&mut s, 2, SAM, "REQUEST", &[CalEvent { sequence: 4, ..meeting.clone() }], NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::NotFromOrganizer));
    deliver(&mut s, 3, ("Mallory", "mallory@evil.example"), "REQUEST", std::slice::from_ref(&meeting), NOW);
    assert_eq!(s.invite().unwrap().problem, Some(InviteProblem::NotFromOrganizer));
}

/// An account with calendars only has no mail to send the delegate their invitation, and its
/// server may not (Nextcloud and Stalwart don't): it doesn't delegate, and its copy stays.
#[test]
fn an_account_with_calendars_only_does_not_delegate() {
    let mut s = Session::open(Some(temp("delegate-calendar-only")), mailcraft_model::dates::Clock::new(NOW, 0));
    let p = json!({"email": "pat@example.org", "username": "pat", "password": "app-pw", "dav": "https://cloud.example.org/remote.php/dav"});
    let id = s.run("account.add", &p).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    let pat = Attendee { email: "pat@example.org".into(), partstat: "ACCEPTED".into(), ..Default::default() };
    let meeting = CalEvent { attendees: vec![pat], ..invitation("d10", "Budget", 1) };
    s.pim_mut(&id).events.push(mailcraft_model::pim::StoredEvent {
        calendar: mailcraft_model::pim::LOCAL.into(),
        event: meeting.clone(),
        ..Default::default()
    });
    assert!(!s.delegates_from(&id));
    assert!(s.run("invite.delegate", &json!({"uid": "d10", "to": SAM.1})).is_err());
    assert_eq!(s.find_event("d10").unwrap().1.event, meeting, "unchanged");
}

/// The delegate's answer can come first (both in one sync, read in any order): it waits for the
/// guest's handing their place on, and is recorded as that comes; someone naming a guest who
/// didn't hand them a place stays out.
#[test]
fn a_delegates_answer_before_the_guests_waits_for_it() {
    let mut s = sample_session("delegate-first");
    let p = json!({"summary": "Mix", "start": NOW + 86_400, "end": NOW + 90_000, "attendees": "sam@studio.example"});
    let uid = s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    let own = s.find_event(&uid).unwrap().1.event.clone();
    let answer = |who: Vec<Attendee>| CalEvent { organizer: Some(me("")), attendees: who, ..own.clone() };
    let kim = Attendee { email: KIM.1.into(), partstat: "ACCEPTED".into(), delegated_from: SAM.1.into(), ..Default::default() };
    deliver(&mut s, 1, KIM, "REPLY", &[answer(vec![kim.clone()])], NOW);
    assert!(s.find_event(&uid).unwrap().1.event.attendees.iter().all(|a| a.email != KIM.1), "not on the list yet");
    let sam = Attendee { email: SAM.1.into(), partstat: "DELEGATED".into(), delegated_to: KIM.1.into(), ..Default::default() };
    deliver(&mut s, 2, SAM, "REPLY", &[answer(vec![sam, Attendee { partstat: "NEEDS-ACTION".into(), ..kim }])], NOW);
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((guest(&e, SAM.1).partstat.as_str(), guest(&e, KIM.1).partstat.as_str()), ("DELEGATED", "ACCEPTED"), "{:?}", e.attendees);
    assert!(s.pim["sample"].delegate_answers.is_empty());
}

/// Found in review: a meeting the calendar has one occurrence of alone (invited to that one) is
/// handed on as it is, not kept beside a copy; handed on again, to someone else, the one before
/// isn't the user's delegate any more; answered after, the place is the user's again; and
/// delegating again to the same one (a send that failed) goes through.
#[test]
fn delegating_again_or_answering_after_keeps_one_delegate() {
    let mut s = sample_session("delegate-again");
    let at = NOW + 3 * 86_400;
    let occurrence = CalEvent { recurrence_id: Some(at), start: at, end: at + 3600, ..invitation("d11", "Standup", 1) };
    deliver(&mut s, 1, ZOE, "REQUEST", &[occurrence], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(parts(&s, "d11").len(), 1);
    s.run("invite.delegate", &json!({"uid": "d11", "to": SAM.1})).unwrap();
    let all = parts(&s, "d11");
    let [one] = all.as_slice() else { panic!("{} parts", all.len()) };
    assert_eq!((guest(&one.event, "riley@studio.example").partstat.as_str(), one.event.recurrence_id), ("DELEGATED", Some(at)));
    // To Kim instead: Sam isn't the user's delegate any more, and Zoe hears of Kim alone.
    s.run("invite.delegate", &json!({"uid": "d11", "to": KIM.1})).unwrap();
    let e = parts(&s, "d11")[0].event.clone();
    assert!(e.attendees.iter().all(|a| a.email != SAM.1), "{:?}", e.attendees);
    assert_eq!(guest(&e, "riley@studio.example").delegated_to, KIM.1);
    let (_, _, events) = sent(&s, "Delegated: Standup").pop().unwrap();
    assert!(events[0].attendees.iter().all(|a| a.email != SAM.1), "{:?}", events[0].attendees);
    // The same one again goes through; then accepted: the user's own place, no one handed it.
    s.run("invite.delegate", &json!({"uid": "d11", "to": KIM.1})).unwrap();
    let occurrence = CalEvent { recurrence_id: Some(at), start: at, end: at + 3600, ..invitation("d11", "Standup", 1) };
    deliver(&mut s, 2, ZOE, "REQUEST", &[occurrence], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let e = parts(&s, "d11")[0].event.clone();
    assert_eq!((guest(&e, "riley@studio.example").partstat.as_str(), guest(&e, "riley@studio.example").delegated_to.as_str()), ("ACCEPTED", ""));
    let (_, _, events) = sent(&s, "Accepted: Standup").pop().unwrap();
    assert_eq!(events[0].attendees.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["riley@studio.example"]);
}
