//! Tasks: the module (Ctrl+4), a task made from its form and picked, marked done from the
//! ribbon, a due one drawn in the calendar on its day (and opened from there as a task), and the
//! list mirrored right to left.

use super::super::rtl::drawn_text;
use super::*;

#[test]
fn tasks_are_made_listed_and_shown_in_the_calendar() {
    let (mut a, dir) = app("tasks-ui");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    // Ctrl+4, as Outlook's key for Tasks.
    frame(&ctx, &mut a, vec![key(Key::Num4, Modifiers::COMMAND)]);
    assert_eq!(a.session.module, mailcraft_engine::Module::Tasks);
    let size = egui::vec2(1280.0, 800.0);
    let texts = drawn_text(&ctx, &mut a, size);
    for title in ["Send the grading notes to the client", "Renew the font licences", "Return the camera kit"] {
        assert!(texts.iter().any(|(s, ..)| s == title), "{title} listed: {texts:?}");
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
    assert!(texts.iter().any(|(s, ..)| s == "Send the grading notes to the client"), "the task's chip: {texts:?}");
    let today = a.session.local_midnight(mailcraft_engine::model::dates::today(&a.session.clock));
    let item = a.session.task_items(today, today + 86_400).into_iter().next().unwrap();
    let d = Dialog::open_task(&a.session, &item.uid);
    assert!(matches!(&d, Some(Dialog::Task(f)) if f.title == "Send the grading notes to the client" && f.uid.is_some()));
    for mode in ["week", "month"] {
        a.run("calendar.view", json!({"mode": mode}));
        frame(&ctx, &mut a, vec![]);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// In Arabic the task list is mirrored: a task's due day is left of its title, and the Due date
/// heading left of the subject's; in English the other way round.
#[test]
fn the_task_list_is_mirrored() {
    for (lang, subject, due) in [("ar", "الموضوع", "تاريخ الاستحقاق"), ("en", "Task subject", "Due date")] {
        let (mut a, dir) = app(&format!("tasks-rtl-{lang}"));
        a.ui.language = lang.into();
        let ctx = egui::Context::default();
        a.run("view.module", json!({"name": "tasks"}));
        for _ in 0..3 {
            frame(&ctx, &mut a, vec![]);
        }
        let texts = drawn_text(&ctx, &mut a, egui::vec2(1280.0, 800.0));
        let at = |name: &str| {
            texts.iter().find(|(s, ..)| s.starts_with(name)).map(|(_, r, _)| *r).unwrap_or_else(|| panic!("{lang}: no {name:?} in {texts:?}"))
        };
        let rtl = lang == "ar";
        assert_eq!(at(due).center().x < at(subject).center().x, rtl, "{lang}: the headings");
        // On its row: the title on its first line, the due day centred on the row.
        let title = at("Book the sound stage");
        let date = texts
            .iter()
            .filter(|(s, r, _)| !s.is_empty() && s != "Book the sound stage" && (r.center().y - title.center().y).abs() < 12.0)
            .map(|(_, r, _)| *r)
            .next()
            .unwrap_or_else(|| panic!("{lang}: nothing beside the task in {texts:?}"));
        assert_eq!(date.center().x < title.center().x, rtl, "{lang}: the due day's side");
        let _ = std::fs::remove_dir_all(dir);
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
}
