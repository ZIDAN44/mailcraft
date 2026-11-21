//! The To-Do Bar beside the mail (issue #79): where it shows, what's in it, and what a click in
//! it does.

use mailcraft_engine::TodoGroup;

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// The text drawn in a frame of `size`: each piece with where it is.
fn drawn(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> Vec<(String, egui::Rect)> {
    drawn_text(ctx, a, size).into_iter().map(|d| (d.text, d.rect)).collect()
}

/// A few frames, so the bar has read every folder's flagged mail and the panes have settled.
fn settle(ctx: &egui::Context, a: &mut MailApp) {
    for _ in 0..40 {
        frame(ctx, a, vec![]);
    }
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
    let (mut a, _) = app("todo-bar-shown");
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
}

/// A day clicked in the bar's month lists its events under it, double-clicked opens the calendar
/// on it (issue #154); a task typed in its box is made, due today; a flagged message clicked
/// opens in a window, and its flag marks it complete.
#[test]
fn clicks_in_the_bar_go_where_outlooks_do() {
    let (mut a, _) = app("todo-bar-clicks");
    let ctx = egui::Context::default();
    a.run("view.todoBar", json!({"part": "calendar", "on": true}));
    a.run("view.todoBar", json!({"part": "tasks", "on": true}));
    settle(&ctx, &mut a);
    let edge = SIZE.x - f32::from(a.session.prefs.todo_bar.width);

    // A task typed and entered.
    let field = in_bar(&ctx, &mut a, "Type a new task", edge);
    click(&ctx, &mut a, field.center());
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
    click(&ctx, &mut a, row.center());
    assert_eq!(a.session.windows.len(), 1);
    assert_eq!(a.session.windows[0].parsed.as_ref().map(|p| p.subject.as_str()), Some("Shot list v2"));
    let key_open = a.session.windows[0].key.clone();
    a.run("message.closeWindow", json!({"key": key_open.to_id()}));
    settle(&ctx, &mut a);

    // Its flag, at the row's end, marks it complete: off the list.
    let row = in_bar(&ctx, &mut a, "Shot list v2", edge);
    click(&ctx, &mut a, egui::pos2(SIZE.x - 8.0 - 13.0, row.center().y));
    settle(&ctx, &mut a);
    let shown: Vec<String> = drawn(&ctx, &mut a, SIZE).into_iter().map(|(s, _)| s).collect();
    assert!(!shown.iter().any(|s| s == "Shot list v2"), "marked complete, it's off the list");
    assert!(shown.iter().any(|s| s == "Storyboard review on Thursday"));

    // The 15th of the month shown: its events under it, the mail kept on screen (issue #154).
    let day = in_bar(&ctx, &mut a, "15", edge);
    click(&ctx, &mut a, day.center());
    settle(&ctx, &mut a);
    assert_eq!(a.session.module, mailcraft_engine::Module::Mail);
    let picked = a.ui.todo_day.expect("the day picked");
    assert_eq!(mailcraft_engine::model::dates::civil_from_days(picked).2, 15);
    // Clicked again: what's coming up, as before.
    click(&ctx, &mut a, day.center());
    settle(&ctx, &mut a);
    assert_eq!(a.ui.todo_day, None);
    // Double-clicked: the calendar, on that day (#79).
    click(&ctx, &mut a, day.center());
    click(&ctx, &mut a, day.center());
    assert_eq!(a.session.module, mailcraft_engine::Module::Calendar);
    assert_eq!(a.session.cal_mode, mailcraft_engine::CalMode::Day);
    let (_, _, d) = mailcraft_engine::model::dates::civil_from_days(a.session.cal_day());
    assert_eq!(d, 15);
}

/// The days with something on them are told from the free ones (the bar's month draws them
/// bold, as the Calendar's own does now).
#[test]
fn busy_days_are_the_ones_with_events() {
    let (a, _) = app("todo-bar-busy");
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
}

/// In a right-to-left interface the bar is at the window's left, where its lines end.
#[test]
fn the_bar_is_at_the_left_in_hebrew() {
    let (mut a, _) = app("todo-bar-rtl");
    a.ui.language = "he".into();
    let ctx = egui::Context::default();
    a.run("view.todoBar", json!({}));
    settle(&ctx, &mut a);
    let all = drawn(&ctx, &mut a, SIZE);
    let at = all.iter().find(|(s, _)| s == "הקלד משימה חדשה").map(|(_, r)| *r).expect("the new task box");
    assert!(at.right() <= f32::from(a.session.prefs.todo_bar.width), "{at:?}");
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// View › To-Do Bar: a menu of its parts, each ticked on or off with the menu staying open, and
/// Off for all of them.
#[test]
fn the_view_tab_turns_its_parts_on_and_off() {
    let (mut a, _) = app("todo-bar-menu");
    let ctx = egui::Context::default();
    a.ui.tab = crate::RibbonTab::View;
    settle(&ctx, &mut a);
    let find = |ctx: &egui::Context, a: &mut MailApp, what: &str, under: f32| {
        let all = drawn(ctx, a, SIZE);
        all.iter().find(|(s, r)| s == what && r.top() > under).map(|(_, r)| *r).unwrap_or_else(|| panic!("no {what:?} under {under}"))
    };
    let button = find(&ctx, &mut a, "To-Do Bar", 0.0);
    click(&ctx, &mut a, button.center());
    // The menu, under the ribbon: its three parts and Off.
    let under = button.bottom();
    for part in ["Calendar", "Appointments", "Tasks", "Off"] {
        find(&ctx, &mut a, part, under);
    }
    let tasks = find(&ctx, &mut a, "Tasks", under);
    click(&ctx, &mut a, tasks.center());
    let bar = a.session.prefs.todo_bar;
    assert_eq!((bar.calendar, bar.appointments, bar.tasks), (false, false, true));
    // Still open: another part ticked, then Off.
    let calendar = find(&ctx, &mut a, "Calendar", under);
    click(&ctx, &mut a, calendar.center());
    let bar = a.session.prefs.todo_bar;
    assert_eq!((bar.calendar, bar.appointments, bar.tasks), (true, false, true));
    settle(&ctx, &mut a);
    in_bar(&ctx, &mut a, "Type a new task", SIZE.x - f32::from(bar.width));
    let off = find(&ctx, &mut a, "Off", under);
    click(&ctx, &mut a, off.center());
    assert!(!a.session.prefs.todo_bar.shown());
}

/// A day picked in the bar's month: its events in Appointments, those already over too, under
/// the day's name; a day with none says so; today brings back what's coming up (issue #154).
#[test]
fn a_day_picked_shows_its_events() {
    let (mut a, _) = app("todo-bar-day");
    let ctx = egui::Context::default();
    a.run("view.todoBar", json!({"part": "calendar", "on": true}));
    a.run("view.todoBar", json!({"part": "appointments", "on": true}));
    settle(&ctx, &mut a);
    let today = mailcraft_engine::model::dates::today(&a.session.clock);
    // A day this month with an event of the sample's, past or to come, and one with none.
    let events = |a: &MailApp, d: i64| -> Vec<String> {
        let all = a.session.agenda(a.session.local_midnight(d), a.session.local_midnight(d + 1));
        all.into_iter().filter(|i| !i.task && i.status != "CANCELLED").map(|i| i.summary).collect()
    };
    let (_, _, dd) = mailcraft_engine::model::dates::civil_from_days(today);
    let first = today - i64::from(dd) + 1;
    let busy = (first..first + 28).find(|d| *d != today && !events(&a, *d).is_empty()).expect("a day with an event");
    let free = (first..first + 28).find(|d| *d != today && events(&a, *d).is_empty()).expect("a day with none");
    a.ui.todo_day = Some(busy);
    settle(&ctx, &mut a);
    let shown: Vec<String> = drawn(&ctx, &mut a, SIZE).into_iter().map(|(s, _)| s).collect();
    for e in events(&a, busy) {
        // Its title, or as much of it as fits ("…" after it).
        let listed = shown.iter().any(|s| *s == e || s.strip_suffix('…').is_some_and(|cut| !cut.is_empty() && e.starts_with(cut)));
        assert!(listed, "{e} listed: {shown:?}");
    }
    a.ui.todo_day = Some(free);
    settle(&ctx, &mut a);
    let shown: Vec<String> = drawn(&ctx, &mut a, SIZE).into_iter().map(|(s, _)| s).collect();
    assert!(shown.iter().any(|s| s == "Nothing on this day"), "{shown:?}");
    // Today clicked: what's coming up again.
    let edge = SIZE.x - f32::from(a.session.prefs.todo_bar.width);
    let (_, _, td) = mailcraft_engine::model::dates::civil_from_days(today);
    let day = in_bar(&ctx, &mut a, &td.to_string(), edge);
    click(&ctx, &mut a, day.center());
    settle(&ctx, &mut a);
    assert_eq!(a.ui.todo_day, None);
}

/// Issue #163: every appointment of the coming week is listed, not the first 8 of them (a busy
/// calendar showed only today and tomorrow).
#[test]
fn the_whole_week_is_listed() {
    let (mut a, _) = app("todo-bar-week");
    let ctx = egui::Context::default();
    a.run("view.todoBar", json!({"part": "appointments", "on": true}));
    let today = mailcraft_engine::model::dates::today(&a.session.clock);
    // Twelve, two a day for six days from tomorrow.
    for n in 0..12 {
        let at = a.session.local_midnight(today + 1 + n / 2) + 9 * 3600 + (n % 2) * 3 * 3600;
        a.act(crate::Action::Submit("event.create", json!({"summary": format!("Planning {n:02}"), "start": at, "end": at + 1800})));
    }
    let tall = egui::vec2(1280.0, 3000.0);
    for _ in 0..3 {
        drawn(&ctx, &mut a, tall);
    }
    let shown: Vec<String> = drawn(&ctx, &mut a, tall).into_iter().map(|(s, _)| s).collect();
    let listed = (0..12).filter(|n| shown.iter().any(|s| *s == format!("Planning {n:02}"))).count();
    assert_eq!(listed, 12, "{shown:?}");
}

/// Issue #160: a right-click on a row of the task list opens its menu, as Outlook's: flagged mail
/// in a folder that isn't on screen followed up by another day from it (nothing selected), its
/// flag cleared; a task given a day, and deleted. Custom… asks for the day.
#[test]
fn a_rows_menu_follows_it_up() {
    let (mut a, _) = app("todo-bar-row-menu");
    let ctx = egui::Context::default();
    a.run("view.todoBar", json!({}));
    a.run("view.todoBar", json!({"part": "appointments", "on": false}));
    settle(&ctx, &mut a);
    let edge = SIZE.x - f32::from(a.session.prefs.todo_bar.width);
    let shot = |a: &MailApp| a.session.todo_items().into_iter().find(|i| i.title == "Shot list v2").map(|i| i.group);
    assert_eq!(shot(&a), Some(TodoGroup::NoDate));
    let open_menu = |ctx: &egui::Context, a: &mut MailApp, row: &str| {
        let at = in_bar(ctx, a, row, edge).center();
        click_at(ctx, a, at, egui::PointerButton::Secondary, Modifiers::NONE);
        frame(ctx, a, vec![]);
    };
    let pick = |ctx: &egui::Context, a: &mut MailApp, what: &str| {
        // The menu's, drawn over the rest last (the ribbon has a Delete too).
        let at = drawn(ctx, a, SIZE).into_iter().rev().find(|(s, _)| s == what).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {what:?}"));
        click(ctx, a, at);
        frame(ctx, a, vec![]);
    };

    open_menu(&ctx, &mut a, "Shot list v2");
    let shown: Vec<String> = drawn(&ctx, &mut a, SIZE).into_iter().map(|(s, _)| s).collect();
    for what in ["Open", "Follow up", "Mark complete", "Clear flag"] {
        assert!(shown.iter().any(|s| s == what), "{what}: {shown:?}");
    }
    pick(&ctx, &mut a, "Follow up");
    pick(&ctx, &mut a, "Tomorrow");
    settle(&ctx, &mut a);
    assert_eq!(shot(&a), Some(TodoGroup::Tomorrow));
    assert!(a.session.selection.iter().all(|k| a.session.todo_message(k).is_none()), "the selection isn't what changed");

    // Custom…: the day asked for, in its own form.
    open_menu(&ctx, &mut a, "Shot list v2");
    pick(&ctx, &mut a, "Follow up");
    pick(&ctx, &mut a, "Custom…");
    assert!(matches!(&a.ui.dialog, Some(Dialog::FollowUp(f)) if f.subject == "Shot list v2" && !f.remind), "the Custom form");
    a.ui.dialog = None;
    // Add reminder…: that form, its reminder on (gap 9).
    open_menu(&ctx, &mut a, "Shot list v2");
    pick(&ctx, &mut a, "Follow up");
    pick(&ctx, &mut a, "Add reminder…");
    assert!(
        matches!(&a.ui.dialog, Some(Dialog::FollowUp(f)) if f.remind && matches!(&f.target, crate::pim_dialogs::follow_up::FollowUpTarget::Todo(_))),
        "the form, to remind"
    );
    a.ui.dialog = None;

    // Clear Flag: off the list.
    open_menu(&ctx, &mut a, "Shot list v2");
    pick(&ctx, &mut a, "Clear flag");
    settle(&ctx, &mut a);
    assert_eq!(shot(&a), None);

    // A task: given a day, then deleted.
    let title = "Renew the font licences";
    let task = |a: &MailApp| a.session.tasks("font licences").into_iter().next().map(|r| r.task.clone());
    open_menu(&ctx, &mut a, title);
    pick(&ctx, &mut a, "Follow up");
    pick(&ctx, &mut a, "Next week");
    settle(&ctx, &mut a);
    let next_week = mailcraft_engine::model::dates::FollowUp::NextWeek.due(&a.session.clock);
    assert_eq!(task(&a).and_then(|t| t.due).map(|d| d.div_euclid(86_400)), Some(next_week));
    open_menu(&ctx, &mut a, title);
    pick(&ctx, &mut a, "Delete");
    settle(&ctx, &mut a);
    assert!(task(&a).is_none(), "deleted");
}

/// Issue #196: the line between the appointments and the tasks is dragged down to give the
/// appointments more room (and the tasks less), and stays where it was let go, a restart too;
/// from the keyboard, Up moves it up.
#[test]
fn the_line_between_appointments_and_tasks_is_dragged() {
    let (mut a, _) = app("todo-bar-split");
    let ctx = egui::Context::default();
    a.run("view.todoBar", json!({"width": 320}));
    a.run("view.todoBar", json!({}));
    settle(&ctx, &mut a);
    let edge = SIZE.x - 320.0;
    let tasks = in_bar(&ctx, &mut a, "Tasks", edge);
    assert_eq!(a.session.prefs.todo_bar.split, 50, "half each to start");
    // The line is just above the tasks' heading.
    let from = egui::pos2(tasks.center().x + 60.0, tasks.min.y - 8.0);
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let to = from + egui::vec2(0.0, 120.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(from)]);
    frame(&ctx, &mut a, vec![press(from, true)]);
    for k in 1..=4 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))]);
    }
    frame(&ctx, &mut a, vec![press(to, false)]);
    settle(&ctx, &mut a);
    let split = a.session.prefs.todo_bar.split;
    assert!(split > 60, "dragged down, the appointments have more: {split}");
    let moved = in_bar(&ctx, &mut a, "Tasks", edge);
    assert!((moved.min.y - tasks.min.y - 120.0).abs() < 12.0, "the tasks start where it was let go: {tasks:?} {moved:?}");
    // Kept with the settings.
    let kept: mailcraft_engine::Prefs = serde_json::from_value(serde_json::to_value(&a.session.prefs).unwrap()).unwrap();
    assert_eq!(kept.todo_bar.split, split);
    // From the keyboard: a click puts it on the line, Up moves it up a step.
    let line = egui::pos2(moved.center().x + 60.0, moved.min.y - 8.0);
    click(&ctx, &mut a, line);
    frame(&ctx, &mut a, vec![Event::Key { key: egui::Key::ArrowUp, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    settle(&ctx, &mut a);
    assert_eq!(a.session.prefs.todo_bar.split, split - 5);
    // Never past what leaves each a few rows.
    a.run("view.todoBar", json!({"split": 99}));
    assert_eq!(a.session.prefs.todo_bar.split, 85);
    assert!(a.session.prefs.todo_bar.appointments && a.session.prefs.todo_bar.tasks, "a split alone leaves the parts as they are");
}

/// Issue #195: each appointment says how long it is under its time, and one starting within the
/// hour how soon (`In 20 min`), as Outlook for Mac's day list; one further off has no countdown.
#[test]
fn appointments_say_how_long_and_how_soon() {
    let (mut a, _) = app("todo-bar-soon");
    let ctx = egui::Context::default();
    a.run("view.todoBar", json!({"width": 360}));
    a.run("view.todoBar", json!({"part": "appointments", "on": true}));
    let at = |a: &MailApp, ahead: i64| {
        let local = a.session.clock.local(a.session.clock.now + ahead);
        let (day, secs) = (local.div_euclid(86_400), local.rem_euclid(86_400));
        format!("{}T{:02}:{:02}", mailcraft_engine::model::dates::day_string(day), secs / 3600, secs % 3600 / 60)
    };
    let (soon, later) = (at(&a, 20 * 60), at(&a, 2 * 3600));
    let end = at(&a, 20 * 60 + 90 * 60);
    a.session.run("event.create", &json!({"summary": "Colour review", "start": soon, "end": end, "location": "Grading suite"})).unwrap();
    let later_end = at(&a, 2 * 3600 + 30 * 60);
    a.session.run("event.create", &json!({"summary": "Sound check", "start": later, "end": later_end})).unwrap();
    settle(&ctx, &mut a);
    let edge = SIZE.x - 360.0;
    let shown: Vec<(String, egui::Rect)> = drawn(&ctx, &mut a, SIZE).into_iter().filter(|(_, r)| r.left() >= edge).collect();
    let find = |what: &str| shown.iter().find(|(s, _)| s == what).map(|(_, r)| *r);
    let review = find("Colour review").expect("listed");
    let (long, badge) = (find("1 h 30 min").expect("its length"), find("In 20 min").expect("how soon"));
    assert!((long.center().y - review.center().y).abs() < 30.0 && long.min.y > review.min.y, "under its time: {long:?} {review:?}");
    // Above its time, at the row's start, the title's room its own (its comment on #195: at the
    // row's end it hid the end of the title).
    assert!(badge.max.y <= review.min.y + 1.0 && review.min.y - badge.max.y < 12.0, "above it: {badge:?} {review:?}");
    assert!(badge.min.x < review.min.x, "at the row's start: {badge:?} {review:?}");
    // The length, wider than its time, doesn't run into the place beside it.
    let place = find("Grading suite").expect("its place");
    assert!(long.max.x <= place.min.x, "{long:?} runs into {place:?}");
    let check = find("Sound check").expect("listed");
    assert!(find("30 min").is_some_and(|r| (r.center().y - check.center().y).abs() < 30.0), "its length");
    assert!(!shown.iter().any(|(s, r)| s.starts_with("In ") && (r.center().y - check.center().y).abs() < 12.0), "two hours off: no countdown");
}

/// Issue #205: a colleague's calendar opened here stays in the Calendar: their holiday, all day
/// today, isn't among the bar's appointments; the user's own meeting is.
#[test]
fn a_colleagues_calendar_stays_out_of_the_bar() {
    use mailcraft_engine::model::ical::CalEvent;
    use mailcraft_engine::model::pim::{CalendarInfo, StoredEvent, shared_calendar_id};
    let (mut a, _) = app("todo-bar-own");
    let ctx = egui::Context::default();
    a.run("view.todoBar", json!({"width": 360}));
    a.run("view.todoBar", json!({"part": "appointments", "on": true}));
    let s = &mut a.session;
    let now = s.clock.now;
    let today = s.local_midnight(mailcraft_engine::model::dates::today(&s.clock));
    let account = s.accounts[0].config.id.clone();
    let ana = shared_calendar_id("ana@contoso.example");
    let p = s.pim_mut(&account);
    p.calendars.push(CalendarInfo { id: ana.clone(), name: "Ana".into(), read_only: true, ..Default::default() });
    let holiday =
        CalEvent { uid: "h1".into(), summary: "Ana on holiday".into(), start: today, end: today + 86_400, all_day: true, ..Default::default() };
    p.events.push(StoredEvent { calendar: ana, event: holiday, ..Default::default() });
    let mine = CalEvent { uid: "m1".into(), summary: "Edit review".into(), start: now + 3600, end: now + 7200, ..Default::default() };
    p.events.push(StoredEvent { calendar: mailcraft_engine::model::pim::LOCAL.into(), event: mine, ..Default::default() });
    settle(&ctx, &mut a);
    let edge = SIZE.x - 360.0;
    let shown: Vec<String> = drawn(&ctx, &mut a, SIZE).into_iter().filter(|(_, r)| r.left() >= edge).map(|(s, _)| s).collect();
    assert!(shown.iter().any(|s| s == "Edit review"), "{shown:?}");
    assert!(!shown.iter().any(|s| s == "Ana on holiday"), "{shown:?}");
    // Nor is the day of a holiday of hers with nothing of the user's on it bold in the bar's
    // month, as it is in the Calendar's.
    let s = &mut a.session;
    let day = mailcraft_engine::model::dates::today(&s.clock) + 3;
    let free = !crate::calendar::busy_days(s, day, day + 1).contains(&day);
    let start = s.local_midnight(day);
    let ana = shared_calendar_id("ana@contoso.example");
    let away = CalEvent { uid: "h2".into(), summary: "Ana away".into(), start, end: start + 86_400, all_day: true, ..Default::default() };
    s.pim_mut(&account).events.push(StoredEvent { calendar: ana, event: away, ..Default::default() });
    assert!(free && crate::calendar::busy_days(s, day, day + 1).contains(&day), "the Calendar's month");
    assert!(!crate::calendar::bar_busy_days(s, day, day + 1).contains(&day), "the bar's");
}
