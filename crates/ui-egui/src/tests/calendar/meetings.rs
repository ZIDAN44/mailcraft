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
    let said = [format!("Invitation: {summary}"), format!("Updated: {summary}")];
    st.messages.iter().filter(|m| said.contains(&m.subject)).count()
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

/// Issue #194: an invitation's card says how it fits the calendar: what it clashes with, by name
/// and time, and draws the hours around it with the user's events and the invitation among
/// them; a click on one of those opens it. Right after another, it says that.
#[test]
fn an_invitation_says_what_it_clashes_with() {
    let (mut a, _) = app("invite-clash");
    let ctx = egui::Context::default();
    let s = &a.session;
    let tomorrow = mailcraft_engine::model::dates::day_string(mailcraft_engine::model::dates::today(&s.clock) + 1);
    // The sample's render farm maintenance takes that morning: out of the way, for one clash.
    a.session.run("event.delete", &json!({"uid": "render@sample.mailcraft"})).unwrap();
    a.session
        .run("event.create", &json!({"summary": "Grade check", "start": format!("{tomorrow}T11:30"), "end": format!("{tomorrow}T12:30")}))
        .unwrap();
    let open = |a: &mut MailApp| {
        let k = a.session.list.iter().find(|m| m.subject == "Sound mix, version 3").map(|m| m.key.clone()).unwrap();
        a.run("list.open", json!({"key": k.to_id()}));
    };
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    open(&mut a);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts: Vec<Drawn> = drawn_text(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    let find = |what: &str| texts.iter().find(|d| d.text == what && d.seen()).map(|d| d.rect);
    find("Clashes with Grade check (11:30 – 12:30) on your calendar").expect("the clash said");
    let strip = find("Grade check").expect("in the hours around it");
    let invite = find("Sound mix playback").expect("the invitation among them");
    assert!(strip.min.x > invite.min.x, "half an hour after it starts: {strip:?} {invite:?}");
    // In a lane of its own above the events, its name clear of theirs (its comment on #194: they
    // were written over each other).
    assert!(invite.max.y <= strip.min.y + 1.0, "above the clashing event: {invite:?} {strip:?}");
    // Its own copy, already in the calendar, isn't a clash with itself.
    assert!(!texts.iter().any(|d| d.text.contains("Clashes with Sound mix")));
    // A click on the event opens it.
    click(&ctx, &mut a, strip.center());
    assert!(matches!(&a.ui.dialog, Some(Dialog::Event(f)) if f.summary == "Grade check"), "opened");
    a.ui.dialog = None;

    // Moved to start as the meeting ends: right after it, not a clash.
    let uid = a.session.agenda(0, i64::MAX / 2).into_iter().find(|it| it.summary == "Grade check").unwrap().uid;
    a.session.run("event.move", &json!({"uid": uid, "by": -5400})).unwrap();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts: Vec<Drawn> = drawn_text(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    assert!(
        texts.iter().any(|d| d.text == "Right after Grade check (10:00 – 11:00) on your calendar"),
        "{:?}",
        texts.iter().filter(|d| d.text.contains("calendar")).map(|d| &d.text).collect::<Vec<_>>()
    );
}

/// Issue #194 (its comment): an invitation that repeats says so on its card, how often when it's
/// once a day, week, month or year (on whichever days, until whenever), as Outlook's says it.
#[test]
fn an_invitation_says_it_repeats() {
    let words = crate::reading::invite_repeats_for_tests;
    assert_eq!(words("FREQ=WEEKLY;BYDAY=WE;INTERVAL=1;UNTIL=20270630T000000Z"), "Repeats · Every week");
    assert_eq!(words("freq=daily;count=5"), "Repeats · Every day");
    assert_eq!(words("FREQ=MONTHLY;BYMONTHDAY=3"), "Repeats · Every month");
    assert_eq!(words("FREQ=YEARLY"), "Repeats · Every year");
    assert_eq!(words("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO"), "Repeats", "every other week: said only that it repeats");
    assert_eq!(words("FREQ=HOURLY"), "Repeats");
    assert_eq!(words(";;=;junk"), "Repeats");
}

/// Issue #236: a guest's answer to the user's meeting is recorded as it arrives, and its card says
/// so, where it offered Record the answer, which said nothing once clicked.
#[test]
fn a_guests_answer_says_it_was_recorded() {
    let (mut a, _) = app("answer-recorded");
    let ctx = egui::Context::default();
    let p = json!({"summary": "Mix", "start": NOW + 86_400, "end": NOW + 90_000, "attendees": "Zoe Park <zoe@studio.example>"});
    let uid = a.session.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    let mut own = a.session.find_event(&uid).unwrap().1.event.clone();
    own.attendees = vec![mailcraft_engine::model::ical::Attendee {
        email: "zoe@studio.example".into(),
        name: "Zoe Park".into(),
        partstat: "ACCEPTED".into(),
        rsvp: false,
        ..Default::default()
    }];
    let ics = mailcraft_engine::model::ical::write(&[own], Some("REPLY"), NOW);
    let raw = format!(
        "From: Zoe Park <zoe@studio.example>\r\nTo: riley@studio.example\r\nSubject: Accepted: Mix\r\nMessage-ID: <answer-236@x>\r\n\
         Date: Mon, 16 Nov 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/calendar; method=REPLY; charset=utf-8\r\n\r\n{ics}"
    );
    let key = arrive(&mut a, 9236, raw.as_bytes());
    let zoe = a.session.find_event(&uid).unwrap().1.event.attendees.iter().find(|x| x.email == "zoe@studio.example").map(|x| x.partstat.clone());
    assert_eq!(zoe.as_deref(), Some("ACCEPTED"), "recorded as it arrived");
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    assert!(has(&texts, "Zoe Park accepted.") && has(&texts, "Recorded in your calendar."), "{texts:?}");
    assert!(!has(&texts, "Record the answer"), "nothing left to do");
}

/// Issue #238: a guest's proposal of a new time shows who proposes what, and Accept proposal
/// moves the meeting; an invitation offers Propose new time, whose form sends the proposal.
#[test]
fn a_new_time_is_proposed_and_taken() {
    let (mut a, _) = app("propose-time");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let p = json!({"summary": "Mix", "start": NOW + 86_400, "end": NOW + 90_000, "attendees": "Zoe Park <zoe@studio.example>"});
    let uid = a.session.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    let mut proposal = a.session.find_event(&uid).unwrap().1.event.clone();
    proposal.start += 86_400;
    proposal.end += 86_400;
    proposal.attendees.retain(|x| x.email == "zoe@studio.example");
    let ics = mailcraft_engine::model::ical::write(&[proposal.clone()], Some("COUNTER"), NOW);
    let raw = format!(
        "From: Zoe Park <zoe@studio.example>\r\nTo: riley@studio.example\r\nSubject: New time proposed: Mix\r\nMessage-ID: <counter-238@x>\r\n\
         Date: Mon, 16 Nov 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/calendar; method=COUNTER; charset=utf-8\r\n\r\n{ics}"
    );
    let key = arrive(&mut a, 9238, raw.as_bytes());
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    assert!(texts.iter().any(|t| t.starts_with("Zoe Park proposes a new time: ")), "{texts:?}");
    // Answered invitations kept where they are: the proposal stays open to read.
    a.session.prefs.answered_invites = mailcraft_engine::AnsweredInvites::Keep;
    click_named(&ctx, &mut a, "Accept proposal");
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.find_event(&uid).unwrap().1.event.start, proposal.start, "moved");
    // Taken, it says so (issue #238: it read "This invitation is out of date").
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    assert!(texts.iter().any(|t| t.starts_with("Proposal accepted: the meeting is at ")), "{texts:?}");
    assert!(!texts.iter().any(|t| t.contains("out of date")) && !has(&texts, "Accept proposal"), "{texts:?}");

    // Invited: Propose new time opens its form, which sends the proposal.
    let k = a.session.list.iter().find(|m| m.subject == "Sound mix, version 3").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "Propose new time");
    let Some(Dialog::Propose(f)) = a.ui.dialog.as_mut() else { panic!("the form: {:?}", a.ui.dialog) };
    let day = crate::pim_dialogs::pattern_day(&f.date).unwrap();
    f.date = mailcraft_engine::model::dates::day_string(day + 1);
    f.end_date = f.date.clone();
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "Send proposal");
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none(), "sent: {:?}", a.ui.dialog);
    let sent = a.session.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    assert!(
        sent.messages.iter().any(|m| m.subject.starts_with("New time proposed: ")),
        "{:?}",
        sent.messages.iter().map(|m| &m.subject).collect::<Vec<_>>()
    );
}

/// Issue #234: Forward on an invitation asks to whom and sends the organizer's invitation there;
/// the event form of the meeting, answered, offers Forward meeting… too.
#[test]
fn a_meeting_is_forwarded_from_its_card_and_its_form() {
    let (mut a, _) = app("forward-meeting");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let k = a.session.list.iter().find(|m| m.subject == "Sound mix, version 3").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    // The invitation card's Forward, under the message's own (the ribbon's and the header's).
    let size = windows::layout::SIZE.with(std::cell::Cell::get);
    let forwards = named(&ctx, &mut a, size).into_iter().filter(|n| n.label.as_deref() == Some("Forward") && !n.disabled);
    let card = forwards.filter_map(|n| n.centre()).max_by(|x, y| x.y.total_cmp(&y.y)).expect("the card's Forward");
    click(&ctx, &mut a, card);
    let Some(Dialog::PassMeeting(f)) = a.ui.dialog.as_mut() else { panic!("asked to whom: {:?}", a.ui.dialog) };
    assert_eq!(f.passing, crate::pim_dialogs::Passing::Forward);
    f.to = "Sam Okafor <sam@studio.example>".into();
    let uid = f.uid.clone();
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    // The dialog's Forward, drawn last.
    let ok = drawn_text(&ctx, &mut a, size).into_iter().rev().find(|d| d.text == "Forward").map(|d| d.rect.center()).expect("the dialog's Forward");
    click(&ctx, &mut a, ok);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none(), "sent: {:?}", a.ui.dialog);
    let sent = a.session.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    assert!(sent.messages.iter().any(|m| m.subject == "Fwd: Sound mix playback" && m.to.iter().any(|t| t.email == "sam@studio.example")));
    // The form of the meeting: Forward meeting… there too.
    let e = a.session.find_event(&uid).unwrap().1.event.clone();
    let f = crate::pim_dialogs::EventForm::open(&a.session, &uid, e.start).expect("its form");
    assert!(f.can_forward);
    a.ui.dialog = Some(Dialog::Event(Box::new(f)));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "Forward meeting…");
    assert!(matches!(&a.ui.dialog, Some(Dialog::PassMeeting(f)) if f.uid == uid && f.passing == crate::pim_dialogs::Passing::Forward));
    // One's own meeting: not offered.
    let own = a.session.run("event.create", &json!({"summary": "Mine", "start": NOW + 86_400, "attendees": "sam@studio.example"})).unwrap();
    let own = own["uid"].as_str().unwrap().to_string();
    let f = crate::pim_dialogs::EventForm::open(&a.session, &own, NOW + 86_400).unwrap();
    assert!(!f.can_forward);
}

/// Issue #237: Invitations in the Inbox… lists a meeting waiting there that the calendar hasn't
/// got, and Put in calendar puts it there unanswered.
#[test]
fn invitations_waiting_in_the_inbox_are_put_in_the_calendar() {
    let (mut a, _) = app("waiting-invites");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    // Mail from before the account came: not taken in as it arrived.
    a.session.prefs.tentative_invites = false;
    let e = mailcraft_engine::model::ical::CalEvent {
        uid: "wait-237".into(),
        summary: "Colour grade".into(),
        start: NOW + 2 * 86_400,
        end: NOW + 2 * 86_400 + 3600,
        organizer: Some(mailcraft_engine::model::ical::Attendee {
            email: "zoe@studio.example".into(),
            name: "Zoe Park".into(),
            ..Default::default()
        }),
        attendees: vec![mailcraft_engine::model::ical::Attendee {
            email: "riley@studio.example".into(),
            partstat: "NEEDS-ACTION".into(),
            rsvp: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let ics = mailcraft_engine::model::ical::write(&[e], Some("REQUEST"), NOW);
    let raw = format!(
        "From: Zoe Park <zoe@studio.example>\r\nTo: riley@studio.example\r\nSubject: Invitation: Colour grade\r\nMessage-ID: <wait-237@x>\r\n\
         Date: Mon, 16 Nov 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/calendar; method=REQUEST; charset=utf-8\r\n\r\n{ics}"
    );
    arrive(&mut a, 9237, raw.as_bytes());
    assert!(a.session.find_event("wait-237").is_none(), "not in the calendar");
    a.run("view.calendar", json!({}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "Invitations in the Inbox…");
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    assert!(texts.iter().any(|t| t.starts_with("Colour grade · ")), "{texts:?}");
    click_named(&ctx, &mut a, "Put in calendar");
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none(), "{:?}", a.ui.dialog);
    let placed = a.session.find_event("wait-237").expect("in the calendar").1.event.clone();
    assert!(placed.attendees.iter().any(|x| x.email == "riley@studio.example" && x.partstat == "NEEDS-ACTION"), "unanswered");
}

/// Issue #254: Delegate on an invitation asks who goes instead, and they're sent the invitation;
/// the meeting's form then lists the user as having handed their place on, and offers Delegate…
/// itself.
#[test]
fn a_meeting_is_delegated_from_its_card_and_its_form() {
    let (mut a, _) = app("delegate-meeting");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let k = a.session.list.iter().find(|m| m.subject == "Sound mix, version 3").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "Delegate");
    let Some(Dialog::PassMeeting(f)) = a.ui.dialog.as_mut() else { panic!("asked who goes: {:?}", a.ui.dialog) };
    assert_eq!((f.passing, f.occurrence), (crate::pim_dialogs::Passing::Delegate, None));
    f.to = "Sam Okafor <sam@studio.example>".into();
    let uid = f.uid.clone();
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    // The form's own Delegate, drawn over the card (whose Delegate is on screen behind it).
    let size = windows::layout::SIZE.with(std::cell::Cell::get);
    let at = drawn_text(&ctx, &mut a, size).into_iter().rev().find(|d| d.text == "Delegate").map(|d| d.rect.center()).expect("the form's Delegate");
    click(&ctx, &mut a, at);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none(), "sent: {:?}", a.ui.dialog);
    let sent = a.session.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    assert!(sent.messages.iter().any(|m| m.subject == "Invitation: Sound mix playback" && m.to.iter().any(|t| t.email == "sam@studio.example")));
    // The form: the user's place handed on, said so; Delegate… there too.
    let e = a.session.find_event(&uid).unwrap().1.event.clone();
    let f = crate::pim_dialogs::EventForm::open(&a.session, &uid, e.start).expect("its form");
    assert!(f.can_delegate);
    assert!(f.attendees.iter().any(|(_, answer, to)| answer == "DELEGATED" && to == "Sam Okafor"), "{:?}", f.attendees);
    a.ui.dialog = Some(Dialog::Event(Box::new(f)));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    assert!(texts.iter().any(|t| t.contains("(delegated to Sam Okafor)")), "{texts:?}");
    click_named(&ctx, &mut a, "Delegate…");
    assert!(matches!(&a.ui.dialog, Some(Dialog::PassMeeting(f)) if f.uid == uid && f.passing == crate::pim_dialogs::Passing::Delegate));
    // One's own meeting: not offered.
    let own = a.session.run("event.create", &json!({"summary": "Mine", "start": NOW + 86_400, "attendees": "sam@studio.example"})).unwrap();
    let f = crate::pim_dialogs::EventForm::open(&a.session, own["uid"].as_str().unwrap(), NOW + 86_400).unwrap();
    assert!(!f.can_delegate);
}

/// Issue #253: a meeting's guests are typed as a message's To is: the people known suggested,
/// Ctrl+K checking the names (several offered in the form to pick from), and Invite… opening the
/// address book, the form coming back with who was picked there.
#[test]
fn a_meetings_guests_are_suggested_checked_and_picked() {
    let (mut a, _) = app("guests-box");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 3 * 86_400)));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "Invite");
    frame(&ctx, &mut a, vec![Event::Text("zo".into())]);
    frame(&ctx, &mut a, vec![]);
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    assert!(texts.iter().any(|t| t == "zoe@studio.example"), "suggested: {texts:?}");
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert!(form(&mut a.ui.dialog).guests.starts_with("Zoe Park <zoe@studio.example>"), "{}", form(&mut a.ui.dialog).guests);
    // Ctrl+K: a name that's several people, offered to pick from.
    form(&mut a.ui.dialog).guests = "studio".into();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    let asks = form(&mut a.ui.dialog).name_asks.clone();
    let [ask] = asks.as_slice() else { panic!("{asks:?}") };
    let pick = ask.people.get(1).expect("several").clone();
    click_named(&ctx, &mut a, &pick.name);
    frame(&ctx, &mut a, vec![]);
    let f = form(&mut a.ui.dialog);
    assert!(f.guests.contains(&pick.email) && !f.guests.contains("studio;") && f.name_asks.is_empty(), "{} {:?}", f.guests, f.name_asks);
    // Invite…: the address book, what's typed in it; OK brings the form back with them.
    click_named(&ctx, &mut a, "Invite…");
    assert!(!a.ui.dialog.as_ref().is_some_and(Dialog::for_compose), "the form's, not a message window's");
    let Some(Dialog::SelectNames(names)) = a.ui.dialog.as_mut() else { panic!("{:?}", a.ui.dialog) };
    assert!(names.to.contains(&pick.email) && names.purpose == crate::dialogs::NamesFor::Guests);
    names.to = format!("{}; ana@soundlab.example", names.to);
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "OK");
    frame(&ctx, &mut a, vec![]);
    let f = form(&mut a.ui.dialog);
    assert!(f.guests.contains("ana@soundlab.example") && f.guests.contains(&pick.email), "{}", f.guests);
    // Cancelled, the form comes back as it was.
    let before = f.guests.clone();
    click_named(&ctx, &mut a, "Invite…");
    if let Some(Dialog::SelectNames(names)) = a.ui.dialog.as_mut() {
        names.to = "kim@studio.example".into();
    }
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "Cancel");
    frame(&ctx, &mut a, vec![]);
    assert_eq!(form(&mut a.ui.dialog).guests, before);
}

/// Issue #234: a series opened on one occurrence ("This occurrence") and forwarded: the form asks
/// this occurrence or the whole series, as Delegate does, and the occurrence goes alone. The whole
/// series went, every day.
#[test]
fn one_occurrence_is_forwarded_from_its_form() {
    let (mut a, _) = app("forward-occurrence");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let k = a.session.list.iter().find(|m| m.subject == "Sound mix, version 3").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("invite.respond", json!({"answer": "accepted"}));
    let (uid, start) = a
        .session
        .pim
        .get("sample")
        .and_then(|p| {
            p.events
                .iter()
                .find(|e| e.event.summary == "Sound mix playback" && e.event.recurrence_id.is_none())
                .map(|e| (e.event.uid.clone(), e.event.start))
        })
        .expect("in the calendar");
    // A daily series, as the tester's was.
    for e in a.session.pim.get_mut("sample").unwrap().events.iter_mut().filter(|e| e.event.uid == uid) {
        e.event.rrule = Some("FREQ=DAILY;COUNT=5".into());
    }
    let at = start + 2 * 86_400;
    let f = crate::pim_dialogs::EventForm::open_occurrence(&a.session, &uid, at).expect("the occurrence's form");
    a.ui.dialog = Some(Dialog::Event(Box::new(f)));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "Forward meeting…");
    let Some(Dialog::PassMeeting(f)) = a.ui.dialog.as_mut() else { panic!("asked to whom: {:?}", a.ui.dialog) };
    assert_eq!((f.passing, f.occurrence, f.only_this), (crate::pim_dialogs::Passing::Forward, Some(at), true));
    f.to = "Sam Okafor <sam@studio.example>".into();
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    assert!(has(&texts, "This occurrence") && has(&texts, "The whole series"), "{texts:?}");
    let size = windows::layout::SIZE.with(std::cell::Cell::get);
    let ok = drawn_text(&ctx, &mut a, size).into_iter().rev().find(|d| d.text == "Forward").map(|d| d.rect.center()).expect("the form's Forward");
    click(&ctx, &mut a, ok);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none(), "sent: {:?}", a.ui.dialog);
    let sent = a.session.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    let m = sent.messages.iter().find(|m| m.subject == "Fwd: Sound mix playback").expect("forwarded");
    let cal = a.session.message_calendar(&m.key).expect("a meeting");
    assert_eq!(cal.events.iter().map(|e| e.recurrence_id).collect::<Vec<_>>(), [Some(at)], "that occurrence alone");
}

/// Issue #235: deleting the user's own meeting with guests asks first, as Outlook: the
/// cancellation goes with the user's words; asked not to, none goes. In German the mail is in
/// German, and the default categories, untouched, take German names.
#[test]
fn deleting_a_meeting_asks_how_its_guests_are_told() {
    let (mut a, _) = app("cancel-meeting");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let made = |a: &mut MailApp, summary: &str| {
        let p = json!({"summary": summary, "start": "2026-10-21T15:00", "attendees": "Zoe Park <zoe@studio.example>"});
        a.session.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string()
    };
    let delete = |a: &mut MailApp, uid: &str, button: &str| {
        let start = a.session.find_event(uid).unwrap().1.event.start;
        a.ui.dialog = Dialog::open_event(&a.session, uid, start);
        for _ in 0..3 {
            frame(&ctx, a, vec![]);
        }
        click_named(&ctx, a, "Delete");
        let Some(Dialog::CancelMeeting(f)) = a.ui.dialog.as_mut() else { panic!("asked first: {:?}", a.ui.dialog) };
        f.message = "The room is gone; next week instead.".into();
        for _ in 0..2 {
            frame(&ctx, a, vec![]);
        }
        click_named(&ctx, a, button);
        frame(&ctx, a, vec![]);
        assert!(a.ui.dialog.is_none(), "done: {:?}", a.ui.dialog);
        assert!(a.session.find_event(uid).is_none(), "deleted");
    };
    let sent = |a: &MailApp, subject: &str| {
        let st = a.session.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
        st.messages.iter().find(|m| m.subject == subject).map(|m| {
            let raw = a.session.store.as_ref().unwrap().read_body(&m.key).unwrap();
            mailcraft_engine::mime::parse(&raw).unwrap().text
        })
    };
    let uid = made(&mut a, "Grading");
    delete(&mut a, &uid, "Send cancellation");
    let text = sent(&a, "Cancelled: Grading").expect("the guests told");
    assert!(text.contains("Grading has been cancelled.") && text.contains("The room is gone; next week instead."), "{text}");
    let uid = made(&mut a, "Pitch");
    delete(&mut a, &uid, "Delete without sending");
    assert!(sent(&a, "Cancelled: Pitch").is_none(), "nobody told");

    // In German: the mail, and the categories as they came.
    a.session.prefs.categories = mailcraft_engine::model::organize::default_categories();
    a.ui.language = "de".into();
    frame(&ctx, &mut a, vec![]);
    let uid = made(&mut a, "Schnitt");
    assert!(sent(&a, "Einladung: Schnitt").is_some(), "the invitation in German");
    a.session.run("event.delete", &json!({"uid": uid})).unwrap();
    assert_eq!(sent(&a, "Abgesagt: Schnitt").as_deref().map(str::trim), Some("Schnitt wurde abgesagt."));
    assert_eq!(a.session.prefs.categories[0].name, "Wichtig");
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// The new event's form: with no meeting rooms set up, no online meeting row (a greyed button
/// over the title read as an empty box, the first thing in the form); with rooms, a switch
/// under the place, which adds the first room.
#[test]
fn the_online_meeting_is_a_switch_under_the_place() {
    let (mut a, _) = app("meeting-switch");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let size = egui::vec2(1280.0, 900.0);
    let open = |a: &mut MailApp| {
        a.ui.dialog = Some(Dialog::new_event(&a.session, None));
        for _ in 0..3 {
            let _ = sized_frame(&ctx, a, size, vec![]);
        }
    };
    open(&mut a);
    let none = named(&ctx, &mut a, size);
    assert!(!none.iter().any(|n| n.name().is_some_and(|l| l.contains("online meeting") || l == "Online meeting")), "no row without rooms");
    rooms(&mut a);
    open(&mut a);
    let nodes = named(&ctx, &mut a, size);
    let top = |name: &str| nodes.iter().find(|n| n.name() == Some(name)).and_then(|n| n.rect).map(|r| r.top());
    let (switch, title) = (top("Online meeting").expect("the switch"), top("Add a title"));
    assert!(title.is_none_or(|t| switch > t), "under the title: {switch} {title:?}");
    click_named(&ctx, &mut a, "Online meeting");
    let Some(Dialog::Event(f)) = &a.ui.dialog else { panic!("the form") };
    assert!(f.meeting.current.is_some(), "on: the first room added");
}
