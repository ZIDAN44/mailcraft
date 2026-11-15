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
    let (mut a, _) = app("meeting-form");
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
}

/// "Add an online meeting to all meetings with guests": the form shows it as soon as there are
/// guests (Reply with meeting has them at once), unless the user takes it out.
#[test]
fn meetings_with_guests_get_one_in_the_form() {
    let (mut a, _) = app("meeting-all-form");
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
}

/// The options card and the form's room menu draw, in light and dark, with rooms and hosts.
#[test]
fn the_options_card_and_the_room_menu_draw() {
    let (mut a, _) = app("meeting-draw");
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
}

/// The form with every choice of v0.31 at once (issues #25, #30, #32): a new event goes to the
/// account's default calendar, not a colleague's listed first; Private (time only), Show as and
/// a room's meeting draw together and are all saved.
#[test]
fn the_form_with_privacy_show_as_and_a_meeting() {
    use mailcraft_engine::model::ical::{Privacy, ShowAs};
    use mailcraft_engine::model::pim::CalendarInfo;
    let (mut a, _) = app("meeting-together");
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
}

/// A guest whose name has a comma (`"Park, Zoe"`) is one guest in the form, so the event opened
/// and saved again keeps them; the organizer shows by name and address.
#[test]
fn a_guest_named_with_a_comma_stays_one_guest() {
    let (mut a, _) = app("guest-comma");
    let made = a.session.run(
        "event.create",
        &json!({"summary": "Cut review", "start": "2026-10-20T10:00", "end": "2026-10-20T11:00", "attendees": "\"Park, Zoe\" <zoe@studio.example>, ann@x.example"}),
    );
    assert!(made.is_ok(), "{made:?}");
    let e = a.session.agenda(NOW, NOW + 60 * 86_400).into_iter().find(|i| i.summary == "Cut review").unwrap();
    let Some(Dialog::Event(f)) = Dialog::open_event(&a.session, &e.uid, e.occurrence) else { panic!("no form") };
    assert_eq!(f.guests, "\"Park, Zoe\" <zoe@studio.example>, ann@x.example");
    assert_eq!(f.organizer.as_deref(), Some("Riley Moreno <riley@studio.example>"));
    let params = f.params();
    a.ui.dialog = Some(Dialog::Event(f));
    a.act(crate::Action::Submit("event.update", params));
    assert!(
        a.ui.dialog.is_none(),
        "saved: {:?}",
        match &a.ui.dialog {
            Some(Dialog::Event(f)) => f.error.clone(),
            _ => None,
        }
    );
    let (_, kept) = a.session.find_event(&e.uid).unwrap();
    let guests: Vec<(&str, &str)> = kept.event.attendees.iter().map(|g| (g.name.as_str(), g.email.as_str())).collect();
    assert_eq!(guests, [("Park, Zoe", "zoe@studio.example"), ("", "ann@x.example")]);
}

/// The invitations and updates in Sent (the sample account files what it sends there).
fn sent_updates(a: &MailApp, summary: &str) -> usize {
    let st = a.session.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    st.messages.iter().filter(|m| m.subject == format!("Invitation: {summary}")).count()
}

/// Issue #102: the user's meeting opened in the form and saved without a change sends the
/// guests nothing and keeps its version; a change of the user's own reminder or how it shows
/// their time neither; a new place does.
#[test]
fn a_meeting_saved_unchanged_sends_no_update() {
    let (mut a, _) = app("meeting-unchanged");
    let ctx = egui::Context::default();
    let mut d = Some(Dialog::new_event(&a.session, Some(NOW + 3 * 86_400)));
    let f = form(&mut d);
    f.summary = "Planning".into();
    f.guests = "Zoe Park <zoe@studio.example>".into();
    f.reminder = Some(15);
    let params = f.params();
    a.ui.dialog = d;
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("event.create", params));
    assert_eq!(sent_updates(&a, "Planning"), 1, "the invitation");
    let e = a.session.pim.values().flat_map(|p| p.events.iter()).find(|e| e.event.summary == "Planning").unwrap().event.clone();

    let save = |a: &mut MailApp, change: &dyn Fn(&mut crate::pim_dialogs::EventForm)| {
        let mut d = Dialog::open_event(&a.session, &e.uid, e.start);
        change(form(&mut d));
        let params = form(&mut d).params();
        a.ui.dialog = d;
        frame(&ctx, a, vec![]);
        a.act(crate::Action::Submit("event.update", params));
        a.session.find_event(&e.uid).unwrap().1.event.sequence
    };
    assert_eq!(save(&mut a, &|_| {}), e.sequence, "saved unchanged: the same version");
    assert_eq!(sent_updates(&a, "Planning"), 1, "and no update");
    assert_eq!(save(&mut a, &|f| f.reminder = Some(30)), e.sequence, "the user's own reminder");
    assert_eq!(mailcraft_engine::model::ical::reminder_minutes(&a.session.find_event(&e.uid).unwrap().1.event), Some(30));
    assert_eq!(sent_updates(&a, "Planning"), 1);
    assert_eq!(save(&mut a, &|f| f.location = "Room 4".into()), e.sequence + 1, "a new place is a new version");
    assert_eq!(sent_updates(&a, "Planning"), 2, "and the guests hear of it");
}
