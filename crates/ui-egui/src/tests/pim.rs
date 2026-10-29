//! UI tests: calendar, people, notes, invitations and reminders.

use super::*;

/// People's list and A–Z index were a few pixels wider than the pane, so the pane (shared by
/// every module) grew each frame to its widest and Mail, Calendar and Notes stayed that wide.
#[test]
fn people_keep_the_pane_its_width() {
    let (mut a, dir) = app("pane-width");
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
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn calendar_and_people_forms_save_through_commands() {
    let (mut a, dir) = app("pim-forms");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    pim_tour(&mut a, &ctx);
    // A new event from the form.
    let mut d = Dialog::new_event(&a.session, Some(NOW + 3 * 86_400));
    let params = match &mut d {
        Dialog::Event(f) => {
            f.summary = "Grade the trailer".into();
            f.location = "Suite B".into();
            f.repeat = "weekly".into();
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
            assert_eq!((f.summary.as_str(), f.repeat.as_str(), f.recurring), ("Grade the trailer", "weekly", true));
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
    let back = crate::pim_dialogs::ContactForm::edit(&card);
    assert_eq!((back.phones[2].as_str(), back.addresses[1].city.as_str(), back.address_shown), ("+32 490 12 34 56", "Gent", 1));
    f.emails[0] = "not an address".into();
    a.ui.dialog = Some(Dialog::new_contact());
    a.act(crate::Action::Submit("contact.create", f.params()));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Contact(f)) if f.error.is_some()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn people_lay_out_as_a_list_cards_or_a_phone_list() {
    let (mut a, dir) = app("people-views");
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
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn notes_are_written_on_and_kept() {
    let (mut a, dir) = app("notes-ui");
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
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #1: an invitation opens with its notes to read, its links under their words, and the
/// dialog fits the window however long the notes are; Edit notes goes back to the box. Issue #4:
/// the list's footer and the close warning draw.
#[test]
fn invitations_read_and_the_new_footers_draw() {
    let (mut a, dir) = app("invitation");
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
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reminders_pop_up_snooze_and_dismiss() {
    let (mut a, dir) = app("reminders");
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
    let _ = std::fs::remove_dir_all(dir);
}
