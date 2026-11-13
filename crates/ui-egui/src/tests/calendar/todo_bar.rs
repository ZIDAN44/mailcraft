//! The To-Do Bar beside the mail (issue #79): where it shows, what's in it, and what a click in
//! it does.

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// The text drawn in a frame of `size`: each piece with where it is.
fn drawn(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> Vec<(String, egui::Rect)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some((t.galley.job.text.clone(), egui::Rect::from_min_size(t.pos, t.galley.size()))),
            _ => None,
        })
        .collect()
}

/// A few frames, so the bar has read every folder's flagged mail and the panes have settled.
fn settle(ctx: &egui::Context, a: &mut MailApp) {
    for _ in 0..40 {
        frame(ctx, a, vec![]);
    }
}

fn click_at(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![button(true)]);
    frame(ctx, a, vec![button(false)]);
    frame(ctx, a, vec![]);
}

/// Where the piece of text `what` is drawn in the bar (past `from` across the window).
fn in_bar(ctx: &egui::Context, a: &mut MailApp, what: &str, from: f32) -> egui::Rect {
    let all = drawn(ctx, a, SIZE);
    all.iter().find(|(s, r)| s == what && r.left() >= from).map(|(_, r)| *r).unwrap_or_else(|| {
        panic!("no {what:?} in the bar; drawn there: {:?}", all.iter().filter(|(_, r)| r.left() >= from).map(|(s, _)| s.as_str()).collect::<Vec<_>>())
    })
}

/// The bar is beside the mail when a part of it is on: at the window's end, as wide as it was
/// left, with the month, what's coming up, and the tasks and flagged mail (one from a folder
/// that isn't on screen). Not in the other modules, nor in a window too narrow for it.
#[test]
fn the_bar_is_beside_the_mail() {
    let (mut a, dir) = app("todo-bar-shown");
    let ctx = egui::Context::default();
    settle(&ctx, &mut a);
    let before: Vec<String> = drawn(&ctx, &mut a, SIZE).into_iter().map(|(s, _)| s).collect();
    assert!(!before.iter().any(|s| s == "Type a new task" || s == "Appointments"), "off until asked for");

    a.run("view.todoBar", json!({"width": 320}));
    a.run("view.todoBar", json!({}));
    settle(&ctx, &mut a);
    let edge = SIZE.x - 320.0;
    for what in ["Calendar", "Appointments", "Tasks", "Type a new task"] {
        let at = in_bar(&ctx, &mut a, what, edge);
        assert!(at.right() <= SIZE.x, "{what} runs out of the window: {at:?}");
    }
    assert_eq!(a.session.prefs.todo_bar.width, 320, "its width is as it was left");
    // What's coming up is from the calendars: the first of it is in the bar.
    let s = &a.session;
    let today = mailcraft_engine::model::dates::today(&s.clock);
    let coming = s.agenda(s.local_midnight(today), s.local_midnight(today + 7)).into_iter().find(|it| !it.task && it.end > s.clock.now).unwrap();
    in_bar(&ctx, &mut a, &coming.summary, edge);

    // A part closed is gone; the others stay, and the tasks have its room: the flagged mail
    // under them is in view, the message in a folder that isn't on screen too.
    a.run("view.todoBar", json!({"part": "appointments", "on": false}));
    settle(&ctx, &mut a);
    let shown: Vec<String> = drawn(&ctx, &mut a, SIZE).into_iter().map(|(s, _)| s).collect();
    assert!(!shown.iter().any(|s| s == "Appointments") && shown.iter().any(|s| s == "Type a new task"));
    for what in ["No date", "Shot list v2", "Storyboard review on Thursday"] {
        let at = in_bar(&ctx, &mut a, what, edge);
        assert!(at.bottom() < SIZE.y - 24.0, "{what} is under the window's edge: {at:?}");
    }

    // Only beside the mail.
    for module in ["calendar", "people", "tasks", "notes"] {
        a.run("view.module", json!({"name": module}));
        settle(&ctx, &mut a);
        let shown: Vec<String> = drawn(&ctx, &mut a, SIZE).into_iter().map(|(s, _)| s).collect();
        assert!(!shown.iter().any(|s| s == "Type a new task"), "{module}");
    }
    a.run("view.module", json!({"name": "mail"}));
    // A window with no room for it shows the mail alone; it's back when there is room again,
    // still on.
    let narrow = egui::vec2(820.0, 800.0);
    for _ in 0..6 {
        drawn(&ctx, &mut a, narrow);
    }
    let shown: Vec<String> = drawn(&ctx, &mut a, narrow).into_iter().map(|(s, _)| s).collect();
    assert!(!shown.iter().any(|s| s == "Type a new task"), "no room for it at 820");
    assert!(a.session.prefs.todo_bar.tasks);
    settle(&ctx, &mut a);
    in_bar(&ctx, &mut a, "Type a new task", edge - 1.0);
    let _ = std::fs::remove_dir_all(dir);
}

/// A day clicked in the bar's month opens the calendar on that day; a task typed in its box is
/// made, due today; a flagged message clicked opens in a window, and its flag marks it complete.
#[test]
fn clicks_in_the_bar_go_where_outlooks_do() {
    let (mut a, dir) = app("todo-bar-clicks");
    let ctx = egui::Context::default();
    a.run("view.todoBar", json!({"part": "calendar", "on": true}));
    a.run("view.todoBar", json!({"part": "tasks", "on": true}));
    settle(&ctx, &mut a);
    let edge = SIZE.x - f32::from(a.session.prefs.todo_bar.width);

    // A task typed and entered.
    let field = in_bar(&ctx, &mut a, "Type a new task", edge);
    click_at(&ctx, &mut a, field.center());
    frame(&ctx, &mut a, vec![Event::Text("Send the invoice".into())]);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    settle(&ctx, &mut a);
    let made = a.session.tasks("invoice");
    assert_eq!(made.len(), 1, "the task typed is made");
    let today = mailcraft_engine::model::dates::today(&a.session.clock);
    assert_eq!(made[0].task.due.map(|d| d.div_euclid(86_400)), Some(today), "due today, as Outlook's");
    assert!(a.ui.todo_new.is_empty(), "the box is ready for the next");
    in_bar(&ctx, &mut a, "Send the invoice", edge);

    // A flagged message opens in a window of its own (it's in a folder that isn't on screen).
    let row = in_bar(&ctx, &mut a, "Shot list v2", edge);
    click_at(&ctx, &mut a, row.center());
    assert_eq!(a.session.windows.len(), 1);
    assert_eq!(a.session.windows[0].parsed.as_ref().map(|p| p.subject.as_str()), Some("Shot list v2"));
    let key_open = a.session.windows[0].key.clone();
    a.run("message.closeWindow", json!({"key": key_open.to_id()}));
    settle(&ctx, &mut a);

    // Its flag, at the row's end, marks it complete: off the list.
    let row = in_bar(&ctx, &mut a, "Shot list v2", edge);
    click_at(&ctx, &mut a, egui::pos2(SIZE.x - 8.0 - 13.0, row.center().y));
    settle(&ctx, &mut a);
    let shown: Vec<String> = drawn(&ctx, &mut a, SIZE).into_iter().map(|(s, _)| s).collect();
    assert!(!shown.iter().any(|s| s == "Shot list v2"), "marked complete, it's off the list");
    assert!(shown.iter().any(|s| s == "Storyboard review on Thursday"));

    // The 15th of the month shown: the calendar, on that day.
    let day = in_bar(&ctx, &mut a, "15", edge);
    click_at(&ctx, &mut a, day.center());
    assert_eq!(a.session.module, mailcraft_engine::Module::Calendar);
    assert_eq!(a.session.cal_mode, mailcraft_engine::CalMode::Day);
    let (_, _, d) = mailcraft_engine::model::dates::civil_from_days(a.session.cal_day());
    assert_eq!(d, 15);
    let _ = std::fs::remove_dir_all(dir);
}

/// The days with something on them are told from the free ones (the bar's month draws them
/// bold, as the Calendar's own does now).
#[test]
fn busy_days_are_the_ones_with_events() {
    let (a, dir) = app("todo-bar-busy");
    let s = &a.session;
    let today = mailcraft_engine::model::dates::today(&s.clock);
    let busy = crate::calendar::busy_days(s, today - 14, today + 28);
    let day_of = |t: i64| s.clock.local(t).div_euclid(86_400);
    let events: Vec<_> = s.agenda(s.local_midnight(today - 14), s.local_midnight(today + 28)).into_iter().filter(|it| !it.task).collect();
    assert!(!events.is_empty());
    for it in &events {
        assert!(busy.contains(&day_of(it.start)), "{} on {}", it.summary, day_of(it.start));
    }
    assert!(busy.len() < 42, "some days are free: {busy:?}");
    assert!(busy.iter().all(|d| (today - 14..today + 28).contains(d)));
    // No days asked about: none; an event that goes on for centuries marks only the days asked
    // about (walked day by day, it never came back).
    assert!(crate::calendar::busy_days(s, today, today).is_empty());
    let mut a = a;
    let events = &mut a.session.pim.get_mut("sample").unwrap().events;
    let mut forever = events[0].clone();
    forever.event.uid = "forever".into();
    forever.event.rrule = None;
    (forever.event.start, forever.event.end, forever.event.all_day) = (-2_000_000_000, 200_000_000_000, false);
    events.push(forever);
    let busy = crate::calendar::busy_days(&a.session, today, today + 7);
    assert!(busy.len() <= 7, "{busy:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// In a right-to-left interface the bar is at the window's left, where its lines end.
#[test]
fn the_bar_is_at_the_left_in_hebrew() {
    let (mut a, dir) = app("todo-bar-rtl");
    a.ui.language = "he".into();
    let ctx = egui::Context::default();
    a.run("view.todoBar", json!({}));
    settle(&ctx, &mut a);
    let all = drawn(&ctx, &mut a, SIZE);
    let at = all.iter().find(|(s, _)| s == "הקלד משימה חדשה").map(|(_, r)| *r).expect("the new task box");
    assert!(at.right() <= f32::from(a.session.prefs.todo_bar.width), "{at:?}");
    let _ = std::fs::remove_dir_all(dir);
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// View › To-Do Bar: a menu of its parts, each ticked on or off with the menu staying open, and
/// Off for all of them.
#[test]
fn the_view_tab_turns_its_parts_on_and_off() {
    let (mut a, dir) = app("todo-bar-menu");
    let ctx = egui::Context::default();
    a.ui.tab = crate::RibbonTab::View;
    settle(&ctx, &mut a);
    let find = |ctx: &egui::Context, a: &mut MailApp, what: &str, under: f32| {
        let all = drawn(ctx, a, SIZE);
        all.iter().find(|(s, r)| s == what && r.top() > under).map(|(_, r)| *r).unwrap_or_else(|| panic!("no {what:?} under {under}"))
    };
    let button = find(&ctx, &mut a, "To-Do Bar", 0.0);
    click_at(&ctx, &mut a, button.center());
    // The menu, under the ribbon: its three parts and Off.
    let under = button.bottom();
    for part in ["Calendar", "Appointments", "Tasks", "Off"] {
        find(&ctx, &mut a, part, under);
    }
    let tasks = find(&ctx, &mut a, "Tasks", under);
    click_at(&ctx, &mut a, tasks.center());
    let bar = a.session.prefs.todo_bar;
    assert_eq!((bar.calendar, bar.appointments, bar.tasks), (false, false, true));
    // Still open: another part ticked, then Off.
    let calendar = find(&ctx, &mut a, "Calendar", under);
    click_at(&ctx, &mut a, calendar.center());
    let bar = a.session.prefs.todo_bar;
    assert_eq!((bar.calendar, bar.appointments, bar.tasks), (true, false, true));
    settle(&ctx, &mut a);
    in_bar(&ctx, &mut a, "Type a new task", SIZE.x - f32::from(bar.width));
    let off = find(&ctx, &mut a, "Off", under);
    click_at(&ctx, &mut a, off.center());
    assert!(!a.session.prefs.todo_bar.shown());
    let _ = std::fs::remove_dir_all(dir);
}
