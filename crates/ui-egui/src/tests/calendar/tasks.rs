//! Tasks: the module (Ctrl+4), a task made from its form and picked, marked done from the
//! ribbon, a due one drawn in the calendar on its day (and opened from there as a task), and the
//! list mirrored right to left.

use super::*;

#[test]
fn tasks_are_made_listed_and_shown_in_the_calendar() {
    let (mut a, _) = app("tasks-ui");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    // Ctrl+4, as Outlook's key for Tasks.
    frame(&ctx, &mut a, vec![key(Key::Num4, Modifiers::COMMAND)]);
    assert_eq!(a.session.module, mailcraft_engine::Module::Tasks);
    let size = egui::vec2(1280.0, 800.0);
    let texts = drawn_text(&ctx, &mut a, size);
    for title in ["Send the grading notes to the client", "Renew the font licences", "Return the camera kit"] {
        assert!(texts.iter().any(|d| d.text == title), "{title} listed: {texts:?}");
    }
    // A new task from its form, with no due day: made, and picked.
    let mut d = Dialog::new_task(&a.session, None);
    if let Dialog::Task(f) = &mut d {
        f.title = "Pack the lenses".into();
        f.priority = mailcraft_engine::model::pim::TaskPriority::High;
    }
    a.ui.dialog = Some(d.clone());
    frame(&ctx, &mut a, vec![]);
    if let Dialog::Task(f) = &d {
        a.act(crate::Action::Submit("task.create", f.params()));
    }
    assert!(a.ui.dialog.is_none(), "saved");
    let uid = a.ui.task.clone().expect("the new task is picked");
    let t = a.session.find_task(&uid).unwrap().task;
    assert_eq!((t.title.as_str(), t.due, t.priority), ("Pack the lenses", None, mailcraft_engine::model::pim::TaskPriority::High));
    // A form without a title says why, and stays.
    a.ui.dialog = Some(Dialog::new_task(&a.session, None));
    a.act(crate::Action::Submit("task.create", json!({"title": " "})));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Task(f)) if f.error.is_some()));
    a.ui.dialog = None;
    // Marked done, as the ribbon's Mark complete does; then it's last in the list.
    a.act(crate::Action::Run("task.complete", json!({"uid": uid, "done": true})));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.find_task(&uid).unwrap().task.is_done());
    assert!(a.session.tasks("").iter().rev().take(2).any(|t| t.task.uid == uid));
    // The calendar's day: the task due today drawn on it, opened as a task.
    a.run("view.module", json!({"name": "calendar"}));
    a.run("calendar.view", json!({"mode": "day"}));
    a.run("calendar.goto", json!({"date": "today"}));
    frame(&ctx, &mut a, vec![]);
    let texts = drawn_text(&ctx, &mut a, size);
    assert!(texts.iter().any(|d| d.text == "Send the grading notes to the client"), "the task's chip: {texts:?}");
    let today = a.session.local_midnight(mailcraft_engine::model::dates::today(&a.session.clock));
    let item = a.session.task_items(today, today + 86_400).into_iter().next().unwrap();
    let d = Dialog::open_task(&a.session, &item.uid);
    assert!(matches!(&d, Some(Dialog::Task(f)) if f.title == "Send the grading notes to the client" && f.uid.is_some()));
    for mode in ["week", "month"] {
        a.run("calendar.view", json!({"mode": mode}));
        frame(&ctx, &mut a, vec![]);
    }
}

/// In Arabic the task list is mirrored: a task's due day is left of its title, and the Due date
/// heading left of the subject's; in English the other way round.
#[test]
fn the_task_list_is_mirrored() {
    for (lang, subject, due) in [("ar", "الموضوع", "تاريخ الاستحقاق"), ("en", "Task subject", "Due date")] {
        let (mut a, _) = app(&format!("tasks-rtl-{lang}"));
        a.ui.language = lang.into();
        let ctx = egui::Context::default();
        a.run("view.module", json!({"name": "tasks"}));
        for _ in 0..3 {
            frame(&ctx, &mut a, vec![]);
        }
        let texts = drawn_text(&ctx, &mut a, egui::vec2(1280.0, 800.0));
        let at =
            |name: &str| texts.iter().find(|d| d.text.starts_with(name)).map(|d| d.ink).unwrap_or_else(|| panic!("{lang}: no {name:?} in {texts:?}"));
        let rtl = lang == "ar";
        assert_eq!(at(due).center().x < at(subject).center().x, rtl, "{lang}: the headings");
        // On its row: the title on its first line, the due day centred on the row.
        let title = at("Book the sound stage");
        let date = texts
            .iter()
            .filter(|d| !d.text.is_empty() && d.text != "Book the sound stage" && (d.ink.center().y - title.center().y).abs() < 12.0)
            .map(|d| d.ink)
            .next()
            .unwrap_or_else(|| panic!("{lang}: nothing beside the task in {texts:?}"));
        assert_eq!(date.center().x < title.center().x, rtl, "{lang}: the due day's side");
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// A day as the task list, the To-Do Bar and the Reminders window name it: Today, Tomorrow,
/// Yesterday from the viewer's own today, whatever their time zone; else the day this year
/// (`Mon Oct 26`, `Mon 26 Oct` with the day first) or the whole date in another year, as the
/// Dates option writes it.
#[test]
fn days_are_named_from_the_viewers_today_as_the_dates_option_writes_them() {
    use mailcraft_engine::model::dates::{Clock, DateOrder, DateStyle, days_from_civil, zone::Zone};
    let name = |clock: Clock, (y, m, d): (i64, u32, u32)| {
        let s = Session::open(None, clock);
        crate::panes::tasks::relative_day(days_from_civil(y, m, d), &s)
    };
    let styled = |order| Clock::new(NOW, 0).styled(DateStyle { order, ..DateStyle::default() });
    // NOW is Wednesday 2026-10-14 12:00 UTC.
    let utc = Clock::new(NOW, 0);
    assert_eq!(name(utc, (2026, 10, 14)), "Today");
    assert_eq!(name(utc, (2026, 10, 15)), "Tomorrow");
    // Overdue: yesterday, earlier this year, a year before.
    assert_eq!(name(utc, (2026, 10, 13)), "Yesterday");
    assert_eq!(name(utc, (2026, 1, 5)), "Mon Jan 5");
    assert_eq!(name(utc, (2025, 10, 27)), "Mon 2025-10-27");
    assert_eq!(name(utc, (2026, 10, 26)), "Mon Oct 26");
    // As the Dates option writes them.
    let dmy = styled(DateOrder::DayMonthYear);
    assert_eq!(name(dmy, (2026, 10, 26)), "Mon 26 Oct");
    assert_eq!(name(dmy, (2025, 10, 27)), "Mon 27/10/2025");
    assert_eq!(name(styled(DateOrder::MonthDayYear), (2027, 3, 1)), "Mon 03/01/2027");
    assert_eq!(name(styled(DateOrder::DayMonthYearDots), (2026, 10, 16)), "Fri 16 Oct");
    // Far east of UTC it's already Thursday the 15th; far west, still Tuesday the 13th.
    let east = Clock::new(NOW, 14 * 3600);
    assert_eq!((name(east, (2026, 10, 14)), name(east, (2026, 10, 15))), ("Yesterday".into(), "Today".into()));
    let west = Clock::new(NOW, -13 * 3600);
    assert_eq!((name(west, (2026, 10, 13)), name(west, (2026, 10, 14))), ("Today".into(), "Tomorrow".into()));
    // In a named zone, at 23:30 on New Year's Eve in New York it's still 2026 there.
    let ny = Zone::named("America/New_York");
    let eve = days_from_civil(2027, 1, 1) * 86_400 + 4 * 3600 + 1800;
    let ny = Clock::new(eve, -5 * 3600).in_zone(ny);
    assert_eq!(name(ny, (2026, 12, 31)), "Today");
    assert_eq!(name(ny, (2027, 1, 5)), "Tue 2027-01-05");
    assert_eq!(name(ny, (2026, 12, 20)), "Sun Dec 20");
    // An hour later in Lisbon (UTC), it's 2027.
    let lisbon = Clock::new(eve + 3600, 0);
    assert_eq!((name(lisbon, (2027, 1, 5)), name(lisbon, (2026, 12, 20))), ("Tue Jan 5".into(), "Sun 2026-12-20".into()));
    // An all-day event, as the To-Do Bar's appointments are: its start is the local midnight of
    // its day, and the bar names the day that midnight falls on.
    let s = Session::open(None, ny);
    let day = days_from_civil(2027, 1, 1);
    assert_eq!(crate::panes::tasks::relative_day(s.clock.local_day(s.local_midnight(day)), &s), "Tomorrow");
    // A task's due day is kept as UTC midnight: the same day, whatever the zone.
    let task = mailcraft_engine::model::pim::Task { due: Some(day * 86_400), ..Default::default() };
    assert_eq!(task.due_day().map(|d| crate::panes::tasks::relative_day(d, &s)), Some("Tomorrow".into()));
    // Days from a damaged file, far off either way: a name all the same.
    for far in [i64::MIN, i64::MAX, i64::MIN / 86_400, i64::MAX / 86_400] {
        assert!(!crate::panes::tasks::relative_day(far, &s).is_empty());
    }
}

/// Issue #162: the task form's days are shown and typed in the order File › Options › Dates
/// writes dates in, each with the month to pick a day from beside it, as the event form's;
/// `YYYY-MM-DD` is still read, and the task is given the same days either way.
#[test]
fn a_tasks_days_are_written_as_options_says_and_picked() {
    let (mut a, _) = app("task-days");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = windows::layout::SIZE.with(std::cell::Cell::get);
    a.run("view.dateFormat", json!({"order": "dmy."}));
    let due = mailcraft_engine::model::dates::days_from_civil(2026, 11, 18);
    a.ui.dialog = Some(Dialog::new_task(&a.session, Some(due * 86_400)));
    drawn_text(&ctx, &mut a, size);
    drawn_text(&ctx, &mut a, size);
    let drawn = drawn_text(&ctx, &mut a, size);
    let strings: Vec<&str> = drawn.iter().map(|d| d.text.as_str()).collect();
    assert!(strings.contains(&"18.11.2026") && !strings.contains(&"2026-11-18"), "{strings:?}");
    let pickers = named(&ctx, &mut a, size).iter().filter(|n| n.name() == Some("Pick a date")).count();
    assert_eq!(pickers, 2, "Start date and Due date");

    // Typed as Options writes it, into the empty Start date.
    let start = drawn.iter().find(|d| d.text == "None" && d.seen()).map(|d| d.rect.center()).expect("the empty Start date");
    click(&ctx, &mut a, start);
    frame(&ctx, &mut a, vec![Event::Text("20.11.2026".into())]);
    let Some(Dialog::Task(f)) = &a.ui.dialog else { panic!("the form") };
    assert_eq!(f.start, "2026-11-20");
    // And as MailCraft's own, into Due date.
    let due_box = drawn.iter().find(|d| d.text == "18.11.2026" && d.seen()).map(|d| d.rect.center()).unwrap();
    click(&ctx, &mut a, due_box);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![Event::Text("2026-12-01".into())]);
    let Some(Dialog::Task(f)) = &a.ui.dialog else { panic!("the form") };
    assert_eq!((f.start.as_str(), f.due.as_str()), ("2026-11-20", "2026-12-01"));
    let p = f.params();
    assert_eq!((p["start"].as_str(), p["due"].as_str()), (Some("2026-11-20"), Some("2026-12-01")));
    // Something that isn't a day is kept as typed, and refused on saving as before.
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![Event::Text("31.02.2026".into())]);
    let Some(Dialog::Task(f)) = &a.ui.dialog else { panic!("the form") };
    assert_eq!(f.due, "31.02.2026");
}
