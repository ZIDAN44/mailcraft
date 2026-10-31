//! UI tests: online meetings (issue #25): the event form adds a room's meeting and takes it
//! out, through the commands; one is added to a meeting with guests when asked, Reply with
//! meeting included; the options card and the form draw with several rooms.

use super::*;

fn rooms(a: &mut MailApp) {
    a.run("meetings.addRoom", json!({"name": "BigBlueButton", "url": "https://meet.example.edu/rooms/abc-def-ghi", "code": "123-456-789"}));
    a.run("meetings.addRoom", json!({"name": "Jitsi", "url": "https://jitsi.example.org/Team"}));
    assert_eq!(a.session.prefs.meeting_rooms.len(), 2);
}

fn form(d: &mut Option<Dialog>) -> &mut crate::pim_dialogs::EventForm {
    match d {
        Some(Dialog::Event(f)) => f,
        _ => panic!("not the event form"),
    }
}

#[test]
fn the_event_form_adds_and_removes_a_meeting() {
    let (mut a, dir) = app("meeting-form");
    let ctx = egui::Context::default();
    rooms(&mut a);
    let mut d = Some(Dialog::new_event(&a.session, Some(NOW + 3 * 86_400)));
    let f = form(&mut d);
    f.summary = "Review".into();
    f.description = "Agenda".into();
    f.guests = "Zoe Park <zoe@studio.example>".into();
    assert_eq!(f.meeting.current, None, "not added unless asked");
    let jitsi = f.meeting.rooms[1].clone();
    crate::pim_dialogs::online_meeting_add(f, &jitsi);
    assert_eq!(f.location, "https://jitsi.example.org/Team");
    assert!(f.description.starts_with("Agenda\n\n---- Online meeting ----\nJoin: https://jitsi.example.org/Team\n"), "{}", f.description);
    let params = f.params();
    assert_eq!(params["onlineMeeting"]["url"], "https://jitsi.example.org/Team");
    a.ui.dialog = d;
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("event.create", params));
    assert!(a.ui.dialog.is_none(), "saved and closed");
    let (_, stored) =
        a.session.pim.iter().flat_map(|(acc, p)| p.events.iter().map(move |e| (acc, e))).find(|(_, e)| e.event.summary == "Review").unwrap();
    let e = stored.event.clone();
    assert_eq!(e.conferences()[0].uri, "https://jitsi.example.org/Team");
    assert_eq!(e.description.matches("---- Online meeting").count(), 1, "added once: {}", e.description);

    // Reopened: it has the meeting, and Join; Remove takes it out of the form and the event.
    let mut d = Dialog::open_event(&a.session, &e.uid, e.start);
    let f = form(&mut d);
    assert_eq!(f.meeting.current.as_deref(), Some("https://jitsi.example.org/Team"));
    assert_eq!(f.join.as_deref(), Some("https://jitsi.example.org/Team"));
    // The block edited away by hand: Remove still takes the link out of the place.
    let kept = f.description.clone();
    f.description = "Agenda".into();
    crate::pim_dialogs::online_meeting_remove(f);
    assert_eq!((f.location.as_str(), f.description.as_str(), f.join.clone()), ("", "Agenda", None));
    assert!(kept.contains("---- Online meeting"));
    let params = f.params();
    assert_eq!(params["onlineMeeting"], false);
    a.ui.dialog = d;
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("event.update", params));
    let e = a.session.find_event(&e.uid).unwrap().1.event.clone();
    assert!(e.conferences().is_empty() && e.description == "Agenda" && e.location.is_empty(), "{e:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// "Add an online meeting to all meetings with guests": the form shows it as soon as there are
/// guests (Reply with meeting has them at once), unless the user takes it out.
#[test]
fn meetings_with_guests_get_one_in_the_form() {
    let (mut a, dir) = app("meeting-all-form");
    let ctx = egui::Context::default();
    rooms(&mut a);
    a.run("meetings.forAll", json!({"on": true}));
    a.ui.dialog = Some(Dialog::new_event(&a.session, None));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(form(&mut a.ui.dialog).meeting.current, None, "no guests yet");
    form(&mut a.ui.dialog).guests = "zoe@studio.example".into();
    frame(&ctx, &mut a, vec![]);
    let f = form(&mut a.ui.dialog);
    assert_eq!(f.meeting.current.as_deref(), Some("https://meet.example.edu/rooms/abc-def-ghi"), "the first room's");
    assert!(f.description.contains("Access code: 123-456-789"));
    crate::pim_dialogs::online_meeting_remove(f);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(form(&mut a.ui.dialog).meeting.current, None, "taken out stays out");

    // Reply with meeting: the message's people are the guests.
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let parsed = a.session.reading.as_ref().and_then(|r| r.parsed.clone()).unwrap();
    a.ui.dialog = Some(Dialog::meeting_from(&a.session, &parsed));
    frame(&ctx, &mut a, vec![]);
    let f = form(&mut a.ui.dialog);
    assert!(!f.guests.is_empty());
    assert_eq!(f.meeting.current.as_deref(), Some("https://meet.example.edu/rooms/abc-def-ghi"));
    // A meeting typed in already: none added.
    a.ui.dialog = Some(Dialog::new_event(&a.session, None));
    let f = form(&mut a.ui.dialog);
    f.location = "https://zoom.us/j/123".into();
    f.guests = "zoe@studio.example".into();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(form(&mut a.ui.dialog).meeting.current, None);
    let _ = std::fs::remove_dir_all(dir);
}

/// The options card and the form's room menu draw, in light and dark, with rooms and hosts.
#[test]
fn the_options_card_and_the_room_menu_draw() {
    let (mut a, dir) = app("meeting-draw");
    let ctx = egui::Context::default();
    rooms(&mut a);
    a.run("meetings.addHost", json!({"host": "video.partner.example"}));
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        a.act(crate::Action::Backstage(crate::backstage::Page::Options));
        frame(&ctx, &mut a, vec![]);
        a.ui.backstage = None;
        a.ui.dialog = Some(Dialog::new_event(&a.session, None));
        frame(&ctx, &mut a, vec![]);
    }
    let _ = std::fs::remove_dir_all(dir);
}
