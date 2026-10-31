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

/// The form with every choice of v0.31 at once (issues #25, #30, #32): a new event goes to the
/// account's default calendar, not a colleague's listed first; Private (time only), Show as and
/// a room's meeting draw together and are all saved.
#[test]
fn the_form_with_privacy_show_as_and_a_meeting() {
    use mailcraft_engine::model::ical::{Privacy, ShowAs};
    use mailcraft_engine::model::pim::CalendarInfo;
    let (mut a, dir) = app("meeting-together");
    let ctx = egui::Context::default();
    rooms(&mut a);
    let theirs = "https://dav.example.org/SOGo/dav/riley/Calendar/sam_team/";
    let own = "https://dav.example.org/SOGo/dav/riley/Calendar/personal/";
    let p = a.session.pim.entry("sample".into()).or_default();
    p.calendars = vec![
        CalendarInfo { id: theirs.into(), name: "Sam's team".into(), foreign: true, ..Default::default() },
        CalendarInfo { id: own.into(), name: "Personal".into(), ..Default::default() },
    ];
    let f = crate::pim_dialogs::EventForm::new(&a.session, Some(NOW + 86_400));
    assert_eq!(f.calendars.get(f.calendar).map(|c| c.1.as_str()), Some(own), "the account's own");
    a.run("calendar.setDefault", json!({"account": "sample", "calendar": theirs}));
    let f = crate::pim_dialogs::EventForm::new(&a.session, Some(NOW + 86_400));
    assert_eq!(f.calendars.get(f.calendar).map(|c| c.1.as_str()), Some(theirs), "the one chosen");
    a.run("calendar.setDefault", json!({"account": "sample", "calendar": ""}));

    let mut d = Some(Dialog::new_event(&a.session, Some(NOW + 86_400)));
    let f = form(&mut d);
    f.summary = "Doctor".into();
    f.guests = "zoe@studio.example".into();
    f.privacy = Privacy::Confidential;
    f.show_as = ShowAs::OutOfOffice;
    let room = f.meeting.rooms[0].clone();
    crate::pim_dialogs::online_meeting_add(f, &room);
    let params = f.params();
    assert_eq!((params["privacy"].as_str(), params["showAs"].as_str()), (Some("confidential"), Some("oof")));
    assert_eq!(params["calendar"], own);
    a.ui.dialog = d;
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        frame(&ctx, &mut a, vec![]);
    }
    a.act(crate::Action::Submit("event.create", params));
    assert!(a.ui.dialog.is_none(), "saved and closed");
    let (_, stored) =
        a.session.pim.iter().flat_map(|(acc, p)| p.events.iter().map(move |e| (acc, e))).find(|(_, e)| e.event.summary == "Doctor").unwrap();
    let e = &stored.event;
    assert_eq!(stored.calendar, own);
    assert_eq!((mailcraft_engine::model::ical::privacy(e), mailcraft_engine::model::ical::show_as(e)), (Privacy::Confidential, ShowAs::OutOfOffice));
    assert_eq!(e.online_meeting().as_deref(), Some(room.url.as_str()));
    let _ = std::fs::remove_dir_all(dir);
}
