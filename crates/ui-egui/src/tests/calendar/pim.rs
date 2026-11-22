//! UI tests: calendar, people, notes, invitations and reminders.

use super::*;

/// People's list and A–Z index were a few pixels wider than the pane, so the pane (shared by
/// every module) grew each frame to its widest and Mail, Calendar and Notes stayed that wide.
#[test]
fn people_keep_the_pane_its_width() {
    let (mut a, _) = app("pane-width");
    let ctx = egui::Context::default();
    // A panel's id is its parent's (the window's root) salted with its name.
    let mut root = egui::Id::NULL;
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        root = ui.scope_id();
        a.ui(ui);
    });
    out.textures_delta.clear();
    let nav = root.with_salt(egui::IdSalt::new("nav"));
    let width = |ctx: &egui::Context| egui::containers::panel::PanelState::load(ctx, nav).map_or(0.0, |p| p.size().x);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let before = width(&ctx);
    a.run("view.people", json!({}));
    for _ in 0..60 {
        frame(&ctx, &mut a, vec![]);
    }
    a.run("view.mail", json!({}));
    frame(&ctx, &mut a, vec![]);
    let after = width(&ctx);
    assert!(before > 0.0 && after <= before + 1.0, "the pane went from {before} to {after}");
}

#[test]
fn calendar_and_people_forms_save_through_commands() {
    let (mut a, _) = app("pim-forms");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    pim_tour(&mut a, &ctx);
    // A new event from the form.
    let mut d = Dialog::new_event(&a.session, Some(NOW + 3 * 86_400));
    let params = match &mut d {
        Dialog::Event(f) => {
            f.summary = "Grade the trailer".into();
            f.location = "Suite B".into();
            f.repeat = crate::pim_dialogs::Repeat::Weekly;
            f.params()
        }
        _ => serde_json::Value::Null,
    };
    a.ui.dialog = Some(d);
    a.act(crate::Action::Submit("event.create", params));
    assert!(a.ui.dialog.is_none(), "saved and closed");
    let (from, to) = (NOW, NOW + 30 * 86_400);
    let made: Vec<_> = a.session.agenda(from, to).into_iter().filter(|i| i.summary == "Grade the trailer").collect();
    assert!(made.len() >= 4 && made.iter().all(|i| i.recurring && i.location == "Suite B"), "{made:?}");
    // Reopened, it shows what was saved; a bad time keeps the form open with the reason.
    let mut d = Dialog::open_event(&a.session, &made[0].uid, made[0].occurrence).unwrap();
    let params = match &mut d {
        Dialog::Event(f) => {
            assert_eq!((f.summary.as_str(), &f.repeat, f.recurring), ("Grade the trailer", &crate::pim_dialogs::Repeat::Weekly, true));
            f.start = "25:00".into();
            f.params()
        }
        _ => serde_json::Value::Null,
    };
    a.ui.dialog = Some(d);
    a.act(crate::Action::Submit("event.update", params));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Event(f)) if f.error.is_some()));
    // A person from the form is picked afterwards.
    a.ui.dialog = Some(Dialog::new_contact());
    let mut f = crate::pim_dialogs::ContactForm { name: "Mira Chen".into(), ..Default::default() };
    f.emails[0] = "mira@colour.example".into();
    f.phones[2] = "+32 490 12 34 56".into();
    f.addresses[1].city = "Gent".into();
    f.birthday = "1990-07-12".into();
    a.act(crate::Action::Submit("contact.create", f.params()));
    let picked = a.ui.person.clone().unwrap();
    let card = a.session.find_person(&picked).unwrap().card;
    assert_eq!(card.display(), "Mira Chen");
    // Issue #1: each number keeps its kind, the address and birthday their fields, and the form
    // shows them where they were.
    assert_eq!((card.phones[0].kind.as_str(), card.addresses[0].kind.as_str(), card.birthday.as_str()), ("cell", "home", "1990-07-12"));
    let back = crate::pim_dialogs::ContactForm::edit(card);
    assert_eq!((back.phones[2].as_str(), back.addresses[1].city.as_str(), back.address_shown), ("+32 490 12 34 56", "Gent", 1));
    f.emails[0] = "not an address".into();
    a.ui.dialog = Some(Dialog::new_contact());
    a.act(crate::Action::Submit("contact.create", f.params()));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Contact(f)) if f.error.is_some()));
}

/// Gap 9: on a 12-hour clock, an event's and a task's time boxes show their times as MailCraft
/// shows times elsewhere and take them as typed, on either clock; a time that isn't one keeps
/// the form open with the reason.
#[test]
fn time_boxes_on_a_12_hour_clock() {
    use mailcraft_engine::model::dates;
    let (mut a, _) = app("pim-12-hour");
    a.run("view.dateFormat", json!({"hours12": true}));
    let mut d = Dialog::new_event(&a.session, Some(NOW + 3 * 86_400 + 3600));
    let Dialog::Event(f) = &mut d else { panic!("an event form") };
    let clock = a.session.clock;
    let at = move |t: i64| {
        let c = dates::civil(t, &clock);
        (c.hour, c.minute)
    };
    let (hour, _) = at(NOW + 3 * 86_400 + 3600);
    assert_eq!(f.start, dates::time_text(hour, 0, clock.style), "shown on the 12-hour clock");
    assert!(f.start.ends_with(" AM") || f.start.ends_with(" PM"), "{}", f.start);
    f.summary = "Colour review".into();
    f.start = "1:30 pm".into();
    f.end = "2:45 P.M.".into();
    let params = f.params();
    assert_eq!((params["start"].as_str().map(|s| &s[10..]), params["end"].as_str().map(|s| &s[10..])), (Some("T13:30"), Some("T14:45")));
    a.ui.dialog = Some(d);
    a.act(crate::Action::Submit("event.create", params));
    assert!(a.ui.dialog.is_none(), "saved and closed");
    let made = a.session.agenda(NOW, NOW + 30 * 86_400).into_iter().find(|i| i.summary == "Colour review").unwrap();
    assert_eq!((at(made.start), at(made.end)), ((13, 30), (14, 45)));
    // Reopened, it shows them as it was saved them.
    let Some(Dialog::Event(mut f)) = Dialog::open_event(&a.session, &made.uid, made.occurrence) else { panic!("no form") };
    assert_eq!((f.start.as_str(), f.end.as_str()), ("1:30 PM", "2:45 PM"));
    for bad in ["13 PM", "25:00", "half past one"] {
        f.start = bad.into();
        let params = f.params();
        a.ui.dialog = Some(Dialog::Event(f.clone()));
        a.act(crate::Action::Submit("event.update", params));
        assert!(matches!(&a.ui.dialog, Some(Dialog::Event(f)) if f.error.is_some()), "{bad} refused");
    }
    // A task's reminder, typed on the 12-hour clock.
    let mut d = Dialog::new_task(&a.session, Some(NOW + 86_400));
    let Dialog::Task(f) = &mut d else { panic!("a task form") };
    (f.title, f.remind, f.reminder_date, f.reminder_time) = ("Call the lab".into(), true, "2026-10-20".into(), "9 pm".into());
    let params = f.params();
    a.ui.dialog = Some(d);
    a.act(crate::Action::Submit("task.create", params));
    let uid = a.ui.task.clone().unwrap();
    let reminder = a.session.find_task(&uid).unwrap().task.reminder.unwrap();
    assert_eq!(at(reminder), (21, 0));
    let Some(Dialog::Task(f)) = Dialog::open_task(&a.session, &uid) else { panic!("no form") };
    assert_eq!(f.reminder_time, "9:00 PM");
}

#[test]
fn people_lay_out_as_a_list_cards_or_a_phone_list() {
    let (mut a, _) = app("people-views");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "people"}));
    for v in crate::people::PeopleView::ALL {
        a.ui.people_view = v;
        frame(&ctx, &mut a, vec![]);
        frame(&ctx, &mut a, vec![]);
    }
    // The layout is remembered with the window's other settings.
    // The list's columns and sort are kept too.
    a.ui.people_list.columns = vec![crate::people::Column::Mobile, crate::people::Column::Book];
    a.ui.people_list.sort = Some(crate::people::Column::Book);
    frame(&ctx, &mut a, vec![]);
    let saved = a.saved_state();
    assert_eq!(saved["peopleView"], "phone");
    a.ui.people_view = crate::people::PeopleView::People;
    a.ui.people_list = Default::default();
    a.restore_state(&saved);
    assert_eq!(a.ui.people_view, crate::people::PeopleView::Phone);
    assert_eq!(a.ui.people_list.columns, [crate::people::Column::Mobile, crate::people::Column::Book]);
    assert_eq!(a.ui.people_list.sort, Some(crate::people::Column::Book));
}

/// Audit #471: the List view's Address book column names a server's address book as the side
/// pane does: one with no name (or the server's own "Contacts") is "Contacts" in the language
/// shown (the column left it blank).
#[test]
fn the_people_list_names_address_books_as_the_side_pane_does() {
    use mailcraft_engine::model::pim::{BookInfo, StoredCard};
    let (mut a, _) = app("people-book-names");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let pim = a.session.pim.get_mut("sample").unwrap();
    for (id, name, who) in [("server", "", "Zed Nameless"), ("server-2", "Contacts", "Yara Servers")] {
        pim.books.push(BookInfo { id: id.into(), name: name.into(), ..Default::default() });
        let card = mailcraft_engine::model::vcard::Card { uid: who.into(), name: who.into(), ..Default::default() };
        pim.cards.push(StoredCard { book: id.into(), card, ..Default::default() });
    }
    a.session.pim_revision += 1;
    a.run("view.module", json!({"name": "people"}));
    a.ui.people_view = crate::people::PeopleView::Phone;
    a.ui.people_list.columns = vec![crate::people::Column::Book];
    frame(&ctx, &mut a, vec![]);
    let texts = drawn_text(&ctx, &mut a, egui::vec2(1280.0, 800.0));
    for who in ["Zed Nameless", "Yara Servers"] {
        // The row's name in the list (right of the side pane's), and what's beside it.
        let row = texts.iter().filter(|d| d.text == who).max_by(|x, y| x.rect.min.x.total_cmp(&y.rect.min.x)).unwrap();
        let book: Vec<&str> = texts
            .iter()
            .filter(|d| d.rect.min.x > row.rect.max.x && (d.rect.center().y - row.rect.center().y).abs() < 2.0)
            .map(|d| d.text.as_str())
            .collect();
        assert_eq!(book, ["Contacts"], "{who}");
    }
}

/// Audit #472: in the List view, the row under the pointer lights up (its hover was read from a
/// widget nothing registered, and never did), and the others don't.
#[test]
fn the_people_list_view_lights_the_row_pointed_at() {
    let (mut a, _) = app("people-list-hover");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "people"}));
    a.ui.people_view = crate::people::PeopleView::Phone;
    let size = egui::vec2(1280.0, 800.0);
    frame(&ctx, &mut a, vec![]);
    let name = a.session.people("").first().map(|p| p.card.display()).unwrap();
    let texts = drawn_with(&ctx, &mut a, size, vec![]);
    // Its name in the list, right of the side pane's.
    let at = texts.iter().filter(|d| d.text == name).map(|d| d.rect.center()).max_by(|x, y| x.x.total_cmp(&y.x)).unwrap();
    let hover = a.ui.tokens.hover_row;
    let lit = |out: &egui::FullOutput, at: egui::Pos2| {
        out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == hover && r.rect.contains(at)))
    };
    let _ = sized_frame(&ctx, &mut a, size, vec![Event::PointerMoved(at)]);
    let out = sized_frame(&ctx, &mut a, size, vec![]);
    assert!(lit(&out, at), "the row pointed at lights up");
    let away = at + egui::vec2(0.0, 60.0);
    let _ = sized_frame(&ctx, &mut a, size, vec![Event::PointerMoved(away)]);
    let out = sized_frame(&ctx, &mut a, size, vec![]);
    assert!(!lit(&out, at) && lit(&out, away), "the light follows the pointer");
}

#[test]
fn notes_are_written_on_and_kept() {
    let (mut a, _) = app("notes-ui");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "notes"}));
    frame(&ctx, &mut a, vec![]);
    // New note: made, and written on at once.
    a.act(crate::Action::NewNote);
    let uid = a.ui.note_edit.as_ref().map(|(u, _)| u.clone()).unwrap();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("Water the plants".into())]);
    assert_eq!(a.ui.note_edit.as_ref().map(|(_, t)| t.as_str()), Some("Water the plants"));
    // Escape leaves it: the text is kept.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.note_edit.is_none());
    assert!(a.session.notes("").iter().any(|n| n.note.uid == uid && n.note.text == "Water the plants"));
    // Ctrl+1 goes back to Mail.
    frame(&ctx, &mut a, vec![key(Key::Num1, Modifiers::COMMAND)]);
    assert_eq!(a.session.module, mailcraft_engine::Module::Mail);
}

/// A note with files shows them at its foot, in both themes; opening one saves it for another
/// app (which isn't there in a test, and says so), and Save as… asks where first.
#[test]
fn a_note_s_files_open_from_the_note() {
    let (mut a, _) = app("note-files-ui");
    let ctx = egui::Context::default();
    a.run("view.module", json!({"name": "notes"}));
    let uid = a.session.notes("")[0].note.uid.clone();
    let line = "ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"swatch.pdf\":JVBERi0xLjc=".to_string();
    for p in a.session.pim.values_mut() {
        for n in p.notes.iter_mut().filter(|n| n.note.uid == uid) {
            n.note.files = vec![line.clone()];
        }
    }
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        frame(&ctx, &mut a, vec![]);
        frame(&ctx, &mut a, vec![]);
    }
    let notices = a.session.notices.len();
    a.act(crate::Action::NoteFile { uid: uid.clone(), index: 0, open: true });
    assert!(a.session.notices.iter().skip(notices).any(|n| n.text.contains("swatch.pdf")), "{:?}", a.session.notices);
    // In the test's own folder for files handed to other apps, not the user's.
    let open = own_scratch_dir("open").unwrap();
    assert_eq!(std::fs::read(open.join("swatch.pdf")).unwrap(), b"%PDF-1.7");
    // Save as… with nowhere chosen saves nothing and says nothing.
    let notices = a.session.notices.len();
    a.act(crate::Action::NoteFile { uid, index: 0, open: false });
    assert_eq!(a.session.notices.len(), notices);
}

/// Issue #1: an invitation opens with its notes to read, its links under their words, and the
/// dialog fits the window however long the notes are; Edit notes goes back to the box. Issue #4:
/// the list's footer and the close warning draw.
#[test]
fn invitations_read_and_the_new_footers_draw() {
    let (mut a, _) = app("invitation");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut notes = String::from("Join with Google Meet<https://meet.google.com/abc-defg-hij>\nMeeting link\n");
    for i in 0..60 {
        notes.push_str(&format!("Line {i} More options<https://calendar.google.com/calendar/event?action=VIEW&eid={}>\n", "x".repeat(120)));
    }
    let mut d = Dialog::new_event(&a.session, None);
    if let Dialog::Event(f) = &mut d {
        f.description = notes.clone();
        f.reading_notes = true;
    }
    a.ui.dialog = Some(d);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    // However long, the dialog stays within the window (its body scrolls).
    let shown = ctx.memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground).count());
    assert!(shown > 0, "the dialog is up");
    let screen = ctx.content_rect();
    let tallest = ctx.memory(|m| m.layer_ids().filter_map(|l| m.area_rect(l.id)).map(|r| r.height()).fold(0.0f32, f32::max));
    assert!(tallest <= screen.height() + 1.0, "{tallest} in {}", screen.height());
    if let Some(Dialog::Event(f)) = &mut a.ui.dialog {
        assert_eq!(f.description, notes, "reading changes nothing");
        f.reading_notes = false;
    }
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = None;

    // The list's footer: a search, before and after asking the server; the close warning.
    a.run("search.run", serde_json::json!({"query": "zzz-nothing", "scope": "all"}));
    frame(&ctx, &mut a, vec![]);
    a.session.server_search = Some(mailcraft_engine::ServerSearch { query: "zzz-nothing".into(), pending: 1, ..Default::default() });
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::LostOnClose { unsent: 2, changes: 1, sending: 1 });
    frame(&ctx, &mut a, vec![]);
}

#[test]
fn reminders_pop_up_snooze_and_dismiss() {
    let (mut a, _) = app("reminders");
    let now = std::sync::Arc::new(std::sync::Mutex::new(NOW));
    let shown = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Ticking(now.clone(), shown.clone()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::reminders::showing(&a.ui.reminders), "the sample has none due");
    // A new event's form starts with a 15-minute reminder, and saves it.
    let start = NOW + 3600;
    let mut d = Dialog::new_event(&a.session, Some(start));
    let params = match &mut d {
        Dialog::Event(f) => {
            assert_eq!(f.reminder, Some(crate::pim_dialogs::DEFAULT_REMINDER));
            f.summary = "Colour review".into();
            f.location = "Suite B".into();
            f.params()
        }
        _ => serde_json::Value::Null,
    };
    a.ui.dialog = Some(d);
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("event.create", params));
    let it = a.session.agenda(NOW, NOW + 86_400).into_iter().find(|i| i.summary == "Colour review").unwrap();
    let reopened = Dialog::open_event(&a.session, &it.uid, it.occurrence).unwrap();
    assert!(matches!(&reopened, Dialog::Event(f) if f.reminder == Some(15)));
    // Not yet; then due: the window and one desktop notification.
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::reminders::showing(&a.ui.reminders));
    *now.lock().unwrap() = start - 15 * 60;
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(crate::reminders::showing(&a.ui.reminders));
    assert_eq!(shown.lock().unwrap().iter().filter(|(t, _)| t == "Colour review").count(), 1, "told once");
    // Snoozed: gone until the snooze is over, then back (and told again).
    let key = a.ui.reminders.due[0].key.clone();
    a.act(crate::Action::Run("reminders.snooze", json!({"key": key, "minutes": 5})));
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::reminders::showing(&a.ui.reminders));
    *now.lock().unwrap() = start - 10 * 60;
    frame(&ctx, &mut a, vec![]);
    assert!(crate::reminders::showing(&a.ui.reminders));
    // Closed with the X: hidden, not dismissed; a new one brings it back with both.
    a.ui.reminders.closed.insert(key.clone());
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::reminders::showing(&a.ui.reminders));
    // Many at once, with long names, still draw.
    for n in 0..40 {
        let p = json!({"summary": format!("{} {n}", "Very long meeting name ".repeat(8)), "start": mailcraft_engine::model::dates::day_string((start).div_euclid(86_400)) + "T23:59", "reminder": 20160});
        a.run("event.create", p);
    }
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(crate::reminders::showing(&a.ui.reminders));
    assert!(a.ui.reminders.due.len() > 40);
    // Dismiss all: gone, and stays gone.
    a.act(crate::Action::Run("reminders.dismiss", json!({})));
    frame(&ctx, &mut a, vec![]);
    *now.lock().unwrap() += 3600;
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::reminders::showing(&a.ui.reminders));
}

/// Issue #32: the event form shows an event's Private and Show as and saves what's changed; the
/// calendar draws a private one with a lock and names one that came without a subject.
#[test]
fn the_event_form_sets_private_and_show_as() {
    use mailcraft_engine::model::ical::{Privacy, ShowAs, privacy, show_as};
    let (mut a, _) = app("event-private");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let lunch = a.session.agenda(NOW - 86_400, NOW + 86_400).into_iter().find(|i| i.summary == "Lunch with Jun").unwrap();
    assert_eq!(lunch.privacy, Privacy::Private, "the sample's lunch is private");
    let Some(Dialog::Event(mut f)) = Dialog::open_event(&a.session, &lunch.uid, lunch.occurrence) else { panic!("no form") };
    assert_eq!((f.privacy, f.show_as), (Privacy::Private, ShowAs::Busy));
    f.privacy = Privacy::Confidential;
    f.show_as = ShowAs::WorkingElsewhere;
    let params = f.params();
    a.ui.dialog = Some(Dialog::Event(f));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("event.update", params));
    let e = a.session.find_event(&lunch.uid).unwrap().1.event.clone();
    assert_eq!((privacy(&e), show_as(&e)), (Privacy::Confidential, ShowAs::WorkingElsewhere));
    // A new one is public and busy unless said otherwise.
    let Some(Dialog::Event(f)) = Some(Dialog::new_event(&a.session, None)) else { panic!() };
    assert_eq!((f.params()["privacy"].as_str(), f.params()["showAs"].as_str()), (Some("public"), Some("busy")));
    // Without a subject (a shared calendar's time only): named, never blank.
    use crate::panes::calendar::event_title;
    assert_eq!(event_title(" ", Privacy::Private, ShowAs::Busy), "Private appointment");
    assert_eq!(event_title("", Privacy::Public, ShowAs::OutOfOffice), "Out of office");
    assert_eq!(event_title("Lunch", Privacy::Private, ShowAs::Busy), "Lunch");
    for mode in ["day", "week", "month"] {
        a.run("view.module", json!({"name": "calendar"}));
        a.run("calendar.view", json!({"mode": mode}));
        frame(&ctx, &mut a, vec![]);
    }
}

/// Issue #33: in the Calendar the search box searches the calendar (not mail); what it finds is
/// listed, and one clicked goes to its day and opens.
#[test]
fn the_calendar_search_box_lists_and_opens_events() {
    let (mut a, _) = app("calendar-search");
    let ctx = egui::Context::default();
    a.run("view.module", json!({"name": "calendar"}));
    frame(&ctx, &mut a, vec![]);
    a.ui.focus_search = true;
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("storyboard".into())]);
    assert_eq!(a.ui.cal_search.query, "storyboard");
    assert!(a.session.search.is_none(), "mail isn't searched");
    frame(&ctx, &mut a, vec![]);
    let hits = a.ui.cal_search.results(&a.session).to_vec();
    assert_eq!(hits.iter().map(|h| h.summary.as_str()).collect::<Vec<_>>(), ["Storyboard review"]);
    let mut acts = Vec::new();
    crate::panes::calendar::open_hit(&a.session, &mut a.ui, &hits[0], &mut acts);
    for act in acts {
        a.act(act);
    }
    assert!(a.ui.cal_search.query.is_empty(), "back to the days");
    assert!(matches!(&a.ui.dialog, Some(Dialog::Event(f)) if f.summary == "Storyboard review"));
    let day = a.session.clock.local(hits[0].start).div_euclid(86_400);
    assert_eq!(a.session.cal_day(), day, "on its day");
    frame(&ctx, &mut a, vec![]);
}

/// The text drawn in a frame.
fn texts(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    drawn_strings(ctx, a, egui::vec2(1280.0, 1600.0))
}

/// Issue #30: events kept on this computer before the account's calendars on its server were
/// found are offered to move there, the default calendar marked; issue #31: the setting for
/// answered invitations, a folder per account when moving to one.
#[test]
fn local_events_are_offered_to_the_server() {
    let (mut a, _) = app("pim-local-offer");
    let ctx = egui::Context::default();
    let id =
        a.session.run("account.add", &json!({"email": "riley@work.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.run("event.create", &json!({"account": id, "summary": "Dentist", "start": NOW + 3600, "end": NOW + 7200})).unwrap();
    let cal = |id: &str, name: &str| mailcraft_engine::model::pim::CalendarInfo { id: id.into(), name: name.into(), ..Default::default() };
    let sync = mailcraft_engine::model::pim::DavSync {
        caldav_home: Some("https://dav.example/riley/".into()),
        calendars: Some(vec![cal("https://dav.example/riley/work/", "Work"), cal("https://dav.example/riley/personal/", "Personal")]),
        checked: NOW,
        ..Default::default()
    };
    a.session.apply(MailEvent::DavSynced { account: id.clone(), sync: Box::new(sync) });
    a.run("view.module", json!({"name": "calendar"}));
    frame(&ctx, &mut a, vec![]);
    let shown = texts(&ctx, &mut a);
    for want in ["Events kept on this computer only: 1", "Move them to Personal", "Keep them here", "default"] {
        assert!(shown.iter().any(|t| t == want), "{want:?} not in {shown:?}");
    }
    a.run("calendar.moveLocal", json!({"account": id}));
    let shown = texts(&ctx, &mut a);
    assert!(!shown.iter().any(|t| t.starts_with("Events kept on this computer")), "moved: not offered");
    // File › Options with answered invitations moved to a folder: a folder per account.
    a.run("options.answeredInvites", json!({"after": "move", "account": "sample", "folder": "Projects"}));
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let _ = texts(&ctx, &mut a);
    a.ui.backstage = None;
}

/// A contact with more postal addresses than the form has places for (two at home), and a number
/// and an address of kinds it has no place for (a pager, a postal address), saved from the form
/// as it opened keeps them all, each with its kind. The second home address was dropped, and the
/// pager number and the postal address, put in the form's Other places, lost their kinds.
#[test]
fn a_contact_saved_from_the_form_keeps_its_extra_addresses_and_kinds() {
    let (mut a, _) = app("contact-extras");
    let address = |kind: &str, city: &str| json!({"kind": kind, "street": "1 Quay", "city": city});
    let made = a.session.run(
        "contact.create",
        &json!({
            "name": "Ana Lima",
            "phones": [{"value": "+1 555 0100", "kind": "pager"}, {"value": "+1 555 0101", "kind": "home"}],
            "addresses": [address("home", "Gent"), address("home", "Brugge"), address("postal", "Antwerpen")],
        }),
    );
    let uid = made.unwrap()["uid"].as_str().unwrap().to_string();
    let before = a.session.find_person(&uid).unwrap().card.clone();
    let form = crate::pim_dialogs::ContactForm::edit(&before);
    a.act(crate::Action::Submit("contact.update", form.params()));
    let after = a.session.find_person(&uid).unwrap().card.clone();
    let kinds = |c: &mailcraft_engine::model::vcard::Card| {
        let mut phones: Vec<(String, String)> = c.phones.iter().map(|p| (p.kind.clone(), p.value.clone())).collect();
        let mut places: Vec<(String, String)> = c.addresses.iter().map(|x| (x.kind.clone(), x.city.clone())).collect();
        phones.sort();
        places.sort();
        (phones, places)
    };
    assert_eq!(kinds(&after), kinds(&before));
    assert_eq!(after.addresses.len(), 3);
}

/// The Email button on a card named the Outlook way, "Last, First", starts a message to that one
/// person. The name went in bare, and the comma parted it into two recipients, the first of
/// them ("Park") not an address.
#[test]
fn email_on_a_card_named_last_first_writes_to_that_person() {
    use mailcraft_engine::model::addr::parse_list;
    let (mut a, _) = app("email-last-first");
    assert!(parse_list("Park, Ann <ann@studio.example>").is_err(), "the name as it went in");
    for act in crate::panes::people::write_to("Park, Ann", "ann@studio.example") {
        a.act(act);
    }
    let to = a.session.compose.as_ref().map(|c| c.to.clone()).unwrap_or_default();
    let people = parse_list(&to).unwrap();
    assert_eq!(people.len(), 1, "{to}");
    assert_eq!((people[0].name.as_str(), people[0].email.as_str()), ("Park, Ann", "ann@studio.example"));
}

/// Every label of the event, task and contact forms fits their label column, in every language
/// (#198: Hungarian's "Ismétlődés gyakorisága", Repeats every, ran into its field, and labels in
/// German, Spanish, Polish, Russian and Slovak too); and the lists the column is measured from
/// hold every label the forms give a row.
#[test]
fn form_labels_fit_their_column() {
    use crate::pim_dialogs::{CONTACT_FORM_LABELS, ICON_FORM_LABELS, ICON_W, LABEL_W_MAX, label_column};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/dialogs");
    let mut files = vec![dir.join("pim_dialogs.rs")];
    files.extend(std::fs::read_dir(dir.join("pim_dialogs")).unwrap().map(|e| e.unwrap().path()));
    let mut seen = 0;
    for file in files {
        let src = std::fs::read_to_string(&file).unwrap();
        for call in src.split("form_row(ui, t, ").skip(1) {
            // The label, where it's written in the call (the contact's emails and phones are
            // named in a list of their own).
            let Some((args, _)) = call.split_once(", |ui|") else { continue };
            let Some(label) = args.split('"').nth(1).filter(|l| !l.is_empty()) else { continue };
            let list: &[&str] = if call.starts_with("true") { &ICON_FORM_LABELS } else { &CONTACT_FORM_LABELS };
            assert!(list.contains(&label), "{label:?} ({}) isn't in its form's labels", file.display());
            seen += 1;
        }
    }
    assert!(seen > 30, "{seen}");

    let (mut a, _) = app("form-labels");
    let ctx = egui::Context::default();
    for lang in crate::i18n::Lang::for_picker() {
        a.ui.language = lang.code().to_string();
        crate::i18n::set_current(lang);
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            a.ui(ui);
            for (icons, labels, indent) in [(true, &ICON_FORM_LABELS[..], ICON_W), (false, &CONTACT_FORM_LABELS[..], 0.0)] {
                let column = label_column(ui, icons);
                assert!(column < LABEL_W_MAX, "{}: the column is at its widest", lang.code());
                for l in labels {
                    let w =
                        ui.painter().layout_no_wrap(crate::i18n::t(l).to_string(), crate::theme::regular(13.0), egui::Color32::PLACEHOLDER).size().x;
                    assert!(indent + w < column, "{}: {l:?} is {w} wide, in a column {column} wide", lang.code());
                }
            }
        });
        out.textures_delta.clear();
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// People sorted by last name (issue #244): each still shown by their name as it reads, filed
/// under their last name's letter, and back by first name when chosen again.
#[test]
fn people_sort_by_last_name() {
    let (mut a, _) = app("people-last-name");
    for name in ["Tamsin Rook", "Ivo Marsh", "Quinn Ashby", "Lena van Ford"] {
        a.run("contact.create", json!({ "name": name }));
    }
    // Ours in the order shown, each with the letter it's under.
    let order = |a: &mut MailApp| -> Vec<(String, char)> {
        let list = crate::panes::people::shown(&a.session, &mut a.ui);
        let mut letter = '?';
        let mut out = Vec::new();
        for line in &list.lines {
            match line {
                crate::panes::people::Line::Letter(c) => letter = *c,
                crate::panes::people::Line::Person(i) => {
                    let at = &list.at[*i];
                    if let Some(p) = crate::shown::person(&a.session, at) {
                        let name = p.card.display();
                        if ["Tamsin Rook", "Ivo Marsh", "Quinn Ashby", "Lena van Ford"].contains(&name.as_str()) {
                            out.push((name, letter));
                        }
                    }
                }
            }
        }
        out
    };
    let pairs = |v: &[(&str, char)]| v.iter().map(|(n, c)| (n.to_string(), *c)).collect::<Vec<_>>();
    assert_eq!(order(&mut a), pairs(&[("Ivo Marsh", 'I'), ("Lena van Ford", 'L'), ("Quinn Ashby", 'Q'), ("Tamsin Rook", 'T')]));
    a.ui.people_list.by_last_name = true;
    assert_eq!(order(&mut a), pairs(&[("Quinn Ashby", 'A'), ("Lena van Ford", 'F'), ("Ivo Marsh", 'M'), ("Tamsin Rook", 'R')]));
    a.ui.people_list.by_last_name = false;
    assert_eq!(order(&mut a)[0], ("Ivo Marsh".to_string(), 'I'));
}
