//! Engine tests: online meetings (issue #25): rooms and hosts kept in the options, a room's
//! meeting added to an event and taken out, sent with the invitation, and added to every
//! meeting with guests when asked.

use super::*;

#[test]
fn rooms_and_hosts_are_kept_and_checked() {
    let mut s = sample_session("meeting-rooms");
    let r = s.run("meetings.addRoom", &json!({"name": "BBB", "url": "meet.example.edu/rooms/abc-def-ghi", "code": "123-456-789"})).unwrap();
    assert_eq!(r["rooms"][0]["url"], "https://meet.example.edu/rooms/abc-def-ghi");
    s.run("meetings.addRoom", &json!({"name": "Jitsi", "url": "https://jitsi.example.org/Team"})).unwrap();
    // The same name again: changed, not doubled.
    s.run("meetings.addRoom", &json!({"name": "bbb", "url": "https://meet.example.edu/rooms/abc-def-ghi", "code": "999"})).unwrap();
    assert_eq!(s.prefs.meeting_rooms.len(), 2);
    assert_eq!(s.prefs.meeting_rooms[0].code, "999");
    s.run("meetings.defaultRoom", &json!({"room": "jitsi"})).unwrap();
    assert_eq!(s.prefs.meeting_rooms[0].name, "Jitsi");
    for bad in [json!({"url": "javascript:alert(1)"}), json!({"url": ""}), json!({"name": "x"}), json!({"url": 5})] {
        assert!(s.run("meetings.addRoom", &bad).is_err(), "{bad}");
    }
    assert!(s.run("meetings.removeRoom", &json!({"room": "nope"})).is_err());

    // Another organisation's self-hosted Jitsi: its links join once its host is listed.
    let e = mailcraft_model::ical::CalEvent { location: "https://video.partner.example/Kickoff".into(), ..Default::default() };
    assert_eq!(s.join_link(&e), None);
    s.run("meetings.addHost", &json!({"host": "https://Video.Partner.example/"})).unwrap();
    s.run("meetings.addHost", &json!({"host": "video.partner.example"})).unwrap();
    assert_eq!(s.prefs.meeting_hosts, ["video.partner.example"], "once, as a host");
    assert_eq!(s.join_link(&e).as_deref(), Some("https://video.partner.example/Kickoff"));
    // The rooms' servers count too.
    let other = mailcraft_model::ical::CalEvent { description: "Use https://jitsi.example.org/Other".into(), ..Default::default() };
    assert_eq!(s.join_link(&other).as_deref(), Some("https://jitsi.example.org/Other"));
    assert!(s.run("meetings.addHost", &json!({"host": "not a host"})).is_err());
    s.run("meetings.removeHost", &json!({"host": "VIDEO.partner.example"})).unwrap();
    assert_eq!(s.join_link(&e), None);
    s.run("meetings.removeRoom", &json!({"room": "https://jitsi.example.org/Team"})).unwrap();
    assert_eq!(s.prefs.meeting_rooms.len(), 1);
    // The options are saved with the rest.
    let kept: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(kept.meeting_rooms, s.prefs.meeting_rooms);
    done(s);
}

/// The issue's example, end to end: a meeting with a room's link as its place, a `CONFERENCE`
/// property and the block in its notes, sent to the guests in the invitation; then taken out.
#[test]
fn a_rooms_meeting_goes_into_the_event_and_the_invitation() {
    let mut s = sample_session("meeting-add");
    s.run("meetings.addRoom", &json!({"name": "BigBlueButton", "url": "https://meet.example.edu/rooms/abc-def-ghi", "code": "123-456-789"})).unwrap();
    let uid = s
        .run(
            "event.create",
            &json!({"summary": "Team meeting", "description": "Team meeting", "start": "2026-10-21T15:00", "attendees": "Zoe Park <zoe@studio.example>", "onlineMeeting": "BigBlueButton"}),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!(e.location, "https://meet.example.edu/rooms/abc-def-ghi");
    assert!(e.description.contains("---- Online meeting ----\nJoin: https://meet.example.edu/rooms/abc-def-ghi\nAccess code: 123-456-789\n"));
    assert_eq!(s.join_link(&e).as_deref(), Some("https://meet.example.edu/rooms/abc-def-ghi"));
    let (method, raw) = sent_calendar(&s, "Sent", "Invitation: Team meeting");
    assert_eq!(method, "REQUEST");
    let unfolded = raw.replace("\r\n ", "");
    assert!(
        unfolded.contains("CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=\"BigBlueButton\":https://meet.example.edu/rooms/abc-def-ghi"),
        "{raw}"
    );
    assert!(unfolded.contains("LOCATION:https://meet.example.edu/rooms/abc-def-ghi"), "{raw}");
    assert!(unfolded.contains("---- Online meeting ----\\nJoin: https://meet.example.edu/rooms/abc-def-ghi\\nAccess code: 123-456-789"), "{raw}");
    // The guest's copy has it too: their Join finds it.
    let cal = s
        .message_calendar(
            &s.store
                .as_ref()
                .unwrap()
                .load_folder("sample", "Sent")
                .unwrap()
                .messages
                .iter()
                .find(|m| m.subject == "Invitation: Team meeting")
                .unwrap()
                .key,
        )
        .unwrap();
    assert_eq!(cal.events[0].join_link().as_deref(), Some("https://meet.example.edu/rooms/abc-def-ghi"));

    // Saved again with the meeting asked for: nothing doubles; then taken out, and the guests hear.
    s.run("event.update", &json!({"uid": uid, "onlineMeeting": true})).unwrap();
    assert_eq!(s.find_event(&uid).unwrap().1.event.description.matches("---- Online meeting").count(), 1);
    s.run("event.update", &json!({"uid": uid, "onlineMeeting": false})).unwrap();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((e.description.as_str(), e.location.as_str()), ("Team meeting", ""));
    assert!(e.conferences().is_empty());
    assert_eq!(s.join_link(&e), None);
    assert!(e.sequence >= 2, "the guests are told");

    // A room given whole, in the user's language; and ones that aren't rooms.
    let p = json!({"summary": "Sprechstunde", "start": "2026-10-22T09:00", "onlineMeeting": {"name": "Jitsi", "url": "https://jitsi.example.org/Office", "labels": {"heading": "Onlinebesprechung", "join": "Teilnehmen:"}}});
    let uid = s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    assert!(
        s.find_event(&uid).unwrap().1.event.description.starts_with("---- Onlinebesprechung ----\nTeilnehmen: https://jitsi.example.org/Office\n")
    );
    for bad in [json!("nope"), json!({"url": "javascript:x"}), json!({"name": "x"}), json!(5), json!([1])] {
        assert!(s.run("event.create", &json!({"summary": "x", "start": "2026-10-22T09:00", "onlineMeeting": bad})).is_err(), "{bad}");
    }
    s.prefs.meeting_rooms.clear();
    assert!(s.run("event.create", &json!({"summary": "x", "start": "2026-10-22T09:00", "onlineMeeting": true})).is_err(), "no rooms");
    done(s);
}

/// Outlook's "Add online meeting to all meetings": only meetings with guests, only when no
/// meeting is asked for or there already, and not when it was taken out.
#[test]
fn every_meeting_with_guests_gets_one_when_asked() {
    let mut s = sample_session("meeting-all");
    let make = |s: &mut Session, p: Value| {
        let uid = s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
        s.find_event(&uid).unwrap().1.event.clone()
    };
    let guests = json!({"summary": "Sync", "start": "2026-10-21T15:00", "attendees": "zoe@studio.example"});
    s.run("meetings.forAll", &json!({"on": true})).unwrap();
    assert_eq!(make(&mut s, guests.clone()).online_meeting(), None, "no rooms yet");
    s.run("meetings.addRoom", &json!({"name": "Jitsi", "url": "https://jitsi.example.org/Team"})).unwrap();
    assert_eq!(make(&mut s, guests.clone()).online_meeting().as_deref(), Some("https://jitsi.example.org/Team"));
    assert_eq!(make(&mut s, json!({"summary": "Alone", "start": "2026-10-21T15:00"})).online_meeting(), None, "no guests");
    let mut off = guests.clone();
    off["onlineMeeting"] = json!(false);
    assert_eq!(make(&mut s, off).online_meeting(), None, "taken out in the form");
    let mut zoom = guests.clone();
    zoom["location"] = json!("https://zoom.us/j/123");
    let e = make(&mut s, zoom);
    assert_eq!((e.online_meeting(), e.location.as_str()), (None, "https://zoom.us/j/123"), "a meeting of its own already");
    s.run("meetings.forAll", &json!({"on": false})).unwrap();
    assert_eq!(make(&mut s, guests).online_meeting(), None);
    done(s);
}
