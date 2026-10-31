//! Issue #25: online meetings in events: self-hosted rooms join, `CONFERENCE` is read and
//! written, and a room's meeting is added and taken out again cleanly.

use super::*;

fn room() -> MeetingRoom {
    MeetingRoom::new("BigBlueButton", "https://meet.example.edu/rooms/abc-def-ghi", "123-456-789", "").unwrap()
}

#[test]
fn self_hosted_rooms_join_by_their_shape_or_the_users_hosts() {
    // BigBlueButton through Greenlight 3 and PILOS, Greenlight 2's /b/, OpenTalk.
    for url in [
        "https://meet.example.edu/rooms/abc-def-ghi",
        "https://bbb.uni.example/rooms/abc-def-ghi-jkl/join",
        "https://greenlight.example.org/b/joh-k2x-9ab",
        "https://controller.opentalk.example/room/0b9a3e5c-1d2f-4a6b-8c7d-9e0f1a2b3c4d",
        "https://teams.microsoft.com/l/meetup-join/19%3a",
    ] {
        assert!(is_meeting_link_in(url, &[]), "{url}");
    }
    // Look-alikes: other paths, other codes.
    for url in [
        "https://hotel.example/rooms/double",
        "https://hotel.example/rooms/abc-def",
        "https://shop.example/b/ABC-DEF-GHI",
        "https://jitsi.example.org/TeamStandup",
        "ftp://meet.example.edu/rooms/abc-def-ghi",
        "",
    ] {
        assert!(!is_meeting_link_in(url, &[]), "{url}");
    }
    // A self-hosted Jitsi, by its host: the host and its subdomains, not names that end alike.
    let hosts = vec![host_entry("https://jitsi.example.org/").unwrap(), host_entry("Example.EDU/talk").unwrap()];
    assert_eq!(hosts, ["jitsi.example.org", "example.edu/talk"]);
    assert!(is_meeting_link_in("https://jitsi.example.org/TeamStandup", &hosts));
    assert!(is_meeting_link_in("https://eu.jitsi.example.org:8443/x", &hosts));
    assert!(!is_meeting_link_in("https://notjitsi.example.org/x", &hosts));
    assert!(is_meeting_link_in("https://cloud.example.edu/talk/abc", &hosts));
    assert!(!is_meeting_link_in("https://cloud.example.edu/files/abc", &hosts), "only under its path");
    assert!(!is_meeting_link_in("https://cloud.example.edu/talkshow/abc", &hosts), "a segment at a time");
    assert!(is_meeting_link_in("https://cloud.example.edu/talk", &hosts));
    for bad in ["", "localhost", "no spaces.example/a b", "-.example", "a..", "ex ample.org"] {
        assert_eq!(host_entry(bad), None, "{bad}");
    }
    // A saved room's server: its other rooms too.
    assert_eq!(room().host_entry().as_deref(), Some("meet.example.edu/rooms/"));
    assert_eq!(MeetingRoom::new("", "jitsi.example.org/Team", "", "").unwrap().host_entry().as_deref(), Some("jitsi.example.org"));
}

#[test]
fn rooms_are_checked_as_theyre_kept() {
    let r = MeetingRoom::new("  Team\u{7} room ", " meet.example.edu/rooms/abc-def-ghi ", " 42\n", "Dial in: +1 555 0100\r\nPIN 7").unwrap();
    assert_eq!((r.name.as_str(), r.url.as_str(), r.code.as_str()), ("Team room", "https://meet.example.edu/rooms/abc-def-ghi", "42"));
    assert_eq!(r.details, "Dial in: +1 555 0100\nPIN 7");
    assert_eq!(MeetingRoom::new("", "https://jitsi.example.org/a", "", "").unwrap().name, "jitsi.example.org", "named by its server");
    for bad in ["", "javascript:alert(1)", "https://", "https://a b.example/x", "mailto:a@b.example", "https://x.example/\"><", "https://localhost/a"]
    {
        assert_eq!(MeetingRoom::new("x", bad, "", ""), None, "{bad}");
    }
    assert_eq!(MeetingRoom::new("x", &format!("https://a.example/{}", "x".repeat(5000)), "", ""), None, "too long");
}

#[test]
fn conference_properties_are_read_and_join() {
    let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:c1\r\nDTSTART:20261102T100000Z\r\nSUMMARY:Review\r\n\
               CONFERENCE;VALUE=URI;FEATURE=PHONE;LABEL=Dial in:tel:+1-555-0100\r\n\
               CONFERENCE;VALUE=URI;FEATURE=VIDEO,AUDIO;LABEL=\"Room; 4\":https://jitsi.self.example/Review\r\n\
               END:VEVENT\r\nEND:VCALENDAR\r\n";
    let e = parse(ics, 0).events.remove(0);
    let c = e.conferences();
    assert_eq!(c.len(), 2);
    assert_eq!((c[1].uri.as_str(), c[1].label.as_str()), ("https://jitsi.self.example/Review", "Room; 4"));
    assert_eq!(c[1].features, ["VIDEO", "AUDIO"]);
    // Its host is known to nobody: the CONFERENCE says it joins; the phone number doesn't.
    assert_eq!(e.join_link().as_deref(), Some("https://jitsi.self.example/Review"));
    assert_eq!(e.online_meeting().as_deref(), Some("https://jitsi.self.example/Review"));
    // Hostile ones don't join.
    for line in
        ["CONFERENCE;VALUE=URI:javascript:alert(1)", "CONFERENCE;VALUE=URI;FEATURE=CHAT:https://chat.example/x", "CONFERENCES:https://a.example/x"]
    {
        let e = CalEvent { extra: vec![line.into()], ..Default::default() };
        assert_eq!(e.join_link(), None, "{line}");
    }
    // A label with quotes and line breaks still makes one line with a quoted parameter.
    let l = conference_line("https://a.example/x\r\nATTENDEE:mailto:evil@x", "Room \"A\";\nB");
    assert_eq!(l, "CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=\"Room A;B\":https://a.example/xATTENDEE:mailto:evil@x");
    let back = CalEvent { extra: vec![l], ..Default::default() }.conferences();
    assert_eq!(back[0].label, "Room A;B");
}

/// The issue's example: the place, `CONFERENCE` and a block in the notes; written, read back,
/// and taken out again to what it was.
#[test]
fn a_rooms_meeting_is_added_and_removed_cleanly() {
    let mut e = CalEvent {
        uid: "m1".into(),
        summary: "Team meeting".into(),
        description: "Team meeting".into(),
        start: 1_800_000_000,
        end: 1_800_003_600,
        ..Default::default()
    };
    let before = e.clone();
    e.add_online_meeting(&room(), &MeetingLabels::default());
    assert_eq!(e.location, "https://meet.example.edu/rooms/abc-def-ghi");
    assert_eq!(
        e.description,
        "Team meeting\n\n---- Online meeting ----\nJoin: https://meet.example.edu/rooms/abc-def-ghi\nAccess code: 123-456-789\n------------------------\n"
    );
    assert_eq!(e.extra, ["CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=\"BigBlueButton\":https://meet.example.edu/rooms/abc-def-ghi"]);
    // Added again: nothing doubles.
    let once = e.clone();
    e.add_online_meeting(&room(), &MeetingLabels::default());
    assert_eq!(e, once);
    // Through a file and back (folded lines, escaped text).
    let back = parse(&write(std::slice::from_ref(&e), Some("REQUEST"), 1_800_000_000), 0).events.remove(0);
    assert_eq!((back.description.as_str(), back.location.as_str()), (e.description.as_str(), e.location.as_str()));
    assert_eq!(back.conferences()[0].label, "BigBlueButton");
    assert_eq!(back.join_link().as_deref(), Some("https://meet.example.edu/rooms/abc-def-ghi"));
    // Out again: as it was.
    let mut out = back.clone();
    assert!(out.remove_online_meeting());
    assert_eq!((out.description.as_str(), out.location.as_str(), out.extra.len()), (before.description.as_str(), "", 0));
    assert!(!out.remove_online_meeting(), "nothing left to take out");
}

#[test]
fn a_meeting_keeps_the_place_and_notes_around_it() {
    let labels = MeetingLabels { heading: "Onlinebesprechung".into(), join: "Teilnehmen:".into(), code: "Zugangscode:".into() };
    let mut e = CalEvent { location: "Room 4".into(), description: "Agenda\n1. Budget\n".into(), ..Default::default() };
    e.add_online_meeting(&room(), &labels);
    assert_eq!(e.location, "Room 4; https://meet.example.edu/rooms/abc-def-ghi");
    assert!(e.description.contains("---- Onlinebesprechung ----\nTeilnehmen: https://"), "{}", e.description);
    // The user writes after the block, and Exchange gives it back with CR LF.
    e.description = e.description.replace('\n', "\r\n") + "\r\nPS: bring the slides";
    // Another room replaces it.
    let other = MeetingRoom::new("Jitsi", "https://jitsi.example.org/Team", "", "Dial in: +49 30 1234").unwrap();
    e.add_online_meeting(&other, &labels);
    assert_eq!(e.location, "Room 4; https://jitsi.example.org/Team");
    assert_eq!(e.conferences().len(), 1);
    assert!(!e.description.contains("abc-def-ghi"), "{}", e.description);
    assert!(e.description.starts_with("Agenda\r\n1. Budget\n\nPS: bring the slides\n\n---- Onlinebesprechung"), "{:?}", e.description);
    assert!(e.description.contains("Dial in: +49 30 1234\n"));
    assert!(e.remove_online_meeting());
    assert_eq!((e.location.as_str(), e.description.as_str()), ("Room 4", "Agenda\r\n1. Budget\n\nPS: bring the slides"));
    // Text that only looks like a block (no link in it), and a block never closed, stay.
    for notes in ["---- Minutes ----\nnothing yet\n--------\n", "---- Online meeting ----\nJoin: https://a.example/x\n"] {
        let mut e = CalEvent { description: notes.into(), ..Default::default() };
        assert!(!e.remove_online_meeting(), "{notes}");
        assert_eq!(e.description, notes);
    }
}

/// Hostile notes and rooms: nothing panics, nothing grows without bound.
#[test]
fn meetings_survive_junk() {
    let junk =
        ["", "----", "---- \u{1F600} ----\n", "---- x ----\r\n\r\n-----", "é----é----\n----", "\n\n\n", "---- a ----\nhttps://x.example/é\n----"];
    for d in junk {
        let mut e = CalEvent {
            description: d.into(),
            location: d.into(),
            extra: vec![d.into(), "CONFERENCE".into(), "CONFERENCE;:".into()],
            ..Default::default()
        };
        let _ = e.online_meeting();
        let _ = e.join_link_in(&[d.into()]);
        e.add_online_meeting(&room(), &MeetingLabels { heading: d.into(), join: d.into(), code: d.into() });
        assert_eq!(e.online_meeting().as_deref(), Some("https://meet.example.edu/rooms/abc-def-ghi"), "{d:?}");
        e.remove_online_meeting();
        assert_eq!(e.online_meeting(), None, "{d:?}: {:?}", e.description);
    }
    let mut big = CalEvent { description: "---- a ----\nhttps://x.example/a\n".repeat(20_000), ..Default::default() };
    big.remove_online_meeting();
    let mut many = CalEvent { extra: vec!["X-A:1".into(); 200], ..Default::default() };
    many.add_online_meeting(&room(), &MeetingLabels::default());
    assert_eq!(many.extra.len(), 200, "the kept lines are capped");
}

/// Exchange gives notes back made text from HTML, each link listed again at the end: the
/// meeting's footnote goes with its block, other links' stay.
#[test]
fn a_meeting_from_exchange_goes_with_its_link_footnote() {
    let notes = "Agenda[1]\n\n---- Online meeting ----\nJoin: https://meet.example.edu/rooms/abc-def-ghi[2]\n------------------------\n\n\
                 [1]: https://docs.example.org/agenda\n[2]: https://meet.example.edu/rooms/abc-def-ghi";
    let mut e = CalEvent { description: notes.into(), ..Default::default() };
    assert_eq!(e.online_meeting().as_deref(), Some("https://meet.example.edu/rooms/abc-def-ghi"), "the link without its footnote mark");
    assert!(e.remove_online_meeting());
    assert_eq!(e.description, "Agenda[1]\n\n[1]: https://docs.example.org/agenda");
}

/// Quoted mail in the notes (Reply with meeting puts the message there) has lines of dashes
/// too: an `-----Original Message-----` with a link and a dashed line under it isn't a meeting,
/// and adding or taking out one leaves it as written.
#[test]
fn quoted_mail_with_dashed_lines_isnt_a_meeting() {
    let quoted = "Shall we meet?\n\n-----Original Message-----\nFrom: Zoe\nSee https://docs.example.org/plan\n--------------------\nThanks";
    let mut e = CalEvent { description: quoted.into(), ..Default::default() };
    assert_eq!(e.online_meeting(), None);
    assert!(!e.remove_online_meeting());
    assert_eq!(e.description, quoted);
    e.add_online_meeting(&room(), &MeetingLabels::default());
    assert!(e.description.starts_with(quoted), "{:?}", e.description);
    assert!(e.remove_online_meeting());
    assert_eq!(e.description, quoted);
}

/// A room's link and details as kept are ones its block is found by again: added twice it's
/// there once, and it comes out whole without its `CONFERENCE` (Exchange keeps none).
#[test]
fn every_kept_room_is_found_again() {
    for bad in ["https://meet.example.org/r.", "https://x.example.org/a)", "https://x.example.org/a?"] {
        assert_eq!(MeetingRoom::new("R", bad, "", ""), None, "{bad}");
    }
    let details = (0..60).map(|i| format!("Dial-in {i}: +1 555 01{i:02}")).collect::<Vec<_>>().join("\n");
    let r = MeetingRoom::new("R", "https://meet.example.edu/rooms/abc-def-ghi", "", &details).unwrap();
    assert_eq!(r.details.lines().count(), 20);
    // Even a longer one from a hand-edited settings file.
    let long = MeetingRoom { details: details.clone(), ..r.clone() };
    for room in [r, long] {
        let mut e = CalEvent { location: "Room 4".into(), ..Default::default() };
        e.add_online_meeting(&room, &MeetingLabels::default());
        let once = e.clone();
        e.add_online_meeting(&room, &MeetingLabels::default());
        assert_eq!(e, once);
        e.extra.clear();
        assert!(e.remove_online_meeting());
        assert_eq!((e.location.as_str(), e.description.as_str()), ("Room 4", ""));
    }
}
