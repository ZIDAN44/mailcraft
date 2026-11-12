//! The To-Do Bar (issue #79): its parts shown and hidden and its width kept, and its task list:
//! the tasks to do and the mail flagged in any folder, by when they're due.

use super::*;
use crate::{Prefs, TodoGroup, TodoItem, TodoWhat};

/// Every folder's flagged mail read, as the bar reads it a folder a frame.
fn read_all(s: &mut Session) {
    for _ in 0..500 {
        if !s.todo_step() {
            return;
        }
    }
    panic!("the folders were never all read");
}

fn mail(items: &[TodoItem]) -> Vec<(String, TodoGroup, String)> {
    items
        .iter()
        .filter_map(|i| match &i.what {
            TodoWhat::Mail { key } => Some((i.title.clone(), i.group, key.to_id())),
            TodoWhat::Task { .. } => None,
        })
        .collect()
}

#[test]
fn the_bars_parts_and_width_are_kept() {
    let mut s = sample_session("todo-bar");
    assert!(!s.prefs.todo_bar.shown(), "off until asked for");
    let r = s.run("view.todoBar", &json!({})).unwrap();
    assert_eq!(r, json!({"calendar": true, "appointments": true, "tasks": true, "width": 260}));
    // A part closed on its own (its ✕), another turned the other way.
    s.run("view.todoBar", &json!({"part": "appointments", "on": false})).unwrap();
    s.run("view.todoBar", &json!({"part": "tasks"})).unwrap();
    let bar = s.prefs.todo_bar;
    assert_eq!((bar.calendar, bar.appointments, bar.tasks), (true, false, false));
    // Its width as dragged, within what it can be; the parts stay as they are.
    assert_eq!(s.run("view.todoBar", &json!({"width": 9000})).unwrap()["width"], 480);
    assert_eq!(s.run("view.todoBar", &json!({"width": 3})).unwrap()["width"], 200);
    assert_eq!(s.run("view.todoBar", &json!({"width": 300})).unwrap()["width"], 300);
    let bar = s.prefs.todo_bar;
    assert_eq!((bar.calendar, bar.appointments, bar.tasks), (true, false, false));
    assert!(s.run("view.todoBar", &json!({"part": "sideways"})).is_err());
    // Kept from one start to the next; settings from before it have it off.
    let kept: Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(kept.todo_bar, s.prefs.todo_bar);
    let old: Prefs = serde_json::from_value(json!({"compact": true})).unwrap();
    assert!(!old.todo_bar.shown());
    // Off: every part.
    s.run("view.todoBar", &json!({"part": "tasks", "on": true})).unwrap();
    s.run("view.todoBar", &json!({"off": true})).unwrap();
    assert!(!s.prefs.todo_bar.shown());
    assert_eq!(s.prefs.todo_bar.width, 300, "its width is kept while it's off");
}

#[test]
fn tasks_and_flagged_mail_are_listed_by_when_they_are_due() {
    let mut s = sample_session("todo-list");
    assert!(mail(&s.todo_items()).is_empty(), "no folder read yet");
    read_all(&mut s);
    let items = s.todo_items();
    // The sample's tasks still to do, and its flagged mail: one in the Inbox, one in a folder
    // that isn't on screen.
    let tasks = items.iter().filter(|i| matches!(i.what, TodoWhat::Task { .. })).count();
    assert_eq!(tasks, s.tasks("").iter().filter(|t| !t.task.is_done()).count());
    assert!(tasks >= 4);
    let flagged = mail(&items);
    assert_eq!(flagged.iter().map(|(t, ..)| t.as_str()).collect::<Vec<_>>(), ["Shot list v2", "Storyboard review on Thursday"]);
    assert!(flagged.iter().all(|(_, g, _)| *g == TodoGroup::NoDate), "flagged with no day");
    assert!(items.windows(2).all(|w| w[0].group <= w[1].group), "under their groups, the soonest first");
    let (elsewhere, inbox) = (flagged[0].2.clone(), flagged[1].2.clone());
    assert!(!elsewhere.ends_with(":INBOX") && inbox.ends_with(":INBOX"), "{elsewhere} {inbox}");

    // Flagged for tomorrow: under Tomorrow.
    s.run("list.select", &json!({"key": inbox})).unwrap();
    s.run("mail.flag", &json!({"due": "tomorrow"})).unwrap();
    read_all(&mut s);
    let flagged = mail(&s.todo_items());
    assert_eq!(flagged[0], ("Storyboard review on Thursday".to_string(), TodoGroup::Tomorrow, inbox.clone()));

    // Opened from the bar: in a window of its own, wherever it is.
    s.run("todo.open", &json!({"key": elsewhere})).unwrap();
    assert_eq!(s.windows.len(), 1);
    assert_eq!(s.windows[0].key.to_id(), elsewhere);

    // Marked complete from the bar: gone at once, and no longer flagged. Undo brings it back.
    s.run("todo.complete", &json!({"key": elsewhere})).unwrap();
    assert_eq!(mail(&s.todo_items()).len(), 1, "gone before its folder is read again");
    read_all(&mut s);
    assert_eq!(mail(&s.todo_items()).len(), 1);
    assert!(s.run("todo.complete", &json!({"key": elsewhere})).is_err(), "no longer on the list");
    s.run("edit.undo", &json!({})).unwrap();
    read_all(&mut s);
    assert_eq!(mail(&s.todo_items()).len(), 2, "flagged again");

    // A flag cleared in the list leaves the bar at once.
    s.run("mail.flag", &json!({"on": false})).unwrap();
    assert_eq!(mail(&s.todo_items()).iter().map(|(t, ..)| t.as_str()).collect::<Vec<_>>(), ["Shot list v2"]);

    // A task done leaves it; one made is on it, under its day.
    let (y, m, d) = mailcraft_model::dates::civil_from_days(mailcraft_model::dates::today(&s.clock));
    let today = format!("{y:04}-{m:02}-{d:02}");
    let uid = s.run("task.create", &json!({"title": "Send the invoice", "due": today})).unwrap()["uid"].as_str().unwrap().to_string();
    let made = s.todo_items().into_iter().find(|i| i.what == TodoWhat::Task { uid: uid.clone() }).unwrap();
    assert_eq!((made.title.as_str(), made.group), ("Send the invoice", TodoGroup::Today));
    s.run("task.complete", &json!({"uid": uid})).unwrap();
    assert!(!s.todo_items().iter().any(|i| i.what == TodoWhat::Task { uid: uid.clone() }));

    // What isn't a flagged message: refused, nothing opened.
    for key in ["", "nonsense", "sample:1:999999:INBOX", "nobody:1:1:INBOX"] {
        assert!(s.run("todo.open", &json!({"key": key})).is_err(), "{key}");
        assert!(s.run("todo.complete", &json!({"key": key})).is_err(), "{key}");
    }
    assert_eq!(s.windows.len(), 1);
}

#[test]
fn things_go_under_the_day_they_are_due() {
    // Thursday 12 November 2026; weeks begin on Monday.
    let thu = mailcraft_model::dates::days_from_civil(2026, 11, 12);
    let of = |d: i64| TodoGroup::of(Some(thu + d), thu);
    assert_eq!(TodoGroup::of(None, thu), TodoGroup::NoDate);
    assert_eq!((of(-400), of(-1), of(0), of(1)), (TodoGroup::Overdue, TodoGroup::Overdue, TodoGroup::Today, TodoGroup::Tomorrow));
    assert_eq!((of(2), of(3)), (TodoGroup::ThisWeek, TodoGroup::ThisWeek), "Saturday and Sunday");
    assert_eq!((of(4), of(10)), (TodoGroup::NextWeek, TodoGroup::NextWeek), "Monday to Sunday after");
    assert_eq!((of(11), of(4000)), (TodoGroup::Later, TodoGroup::Later));
    // On a Sunday, Monday is tomorrow, and the week after it is next week.
    let sun = thu + 3;
    assert_eq!(TodoGroup::of(Some(sun + 1), sun), TodoGroup::Tomorrow);
    assert_eq!(TodoGroup::of(Some(sun + 2), sun), TodoGroup::NextWeek);
    // Days no calendar has don't overflow.
    assert_eq!(TodoGroup::of(Some(i64::MAX), i64::MIN), TodoGroup::Later);
    assert_eq!(TodoGroup::of(Some(i64::MIN), i64::MAX), TodoGroup::Overdue);
}
