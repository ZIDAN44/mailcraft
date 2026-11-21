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
    assert_eq!(r, json!({"calendar": true, "appointments": true, "tasks": true, "width": 260, "split": 50}));
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
    // Where the line between the appointments and the tasks is (issue #196), within what it
    // can be; the parts stay as they are.
    assert_eq!(s.run("view.todoBar", &json!({"split": 70})).unwrap()["split"], 70);
    assert_eq!(s.run("view.todoBar", &json!({"split": 2})).unwrap()["split"], 15);
    let bar = s.prefs.todo_bar;
    assert_eq!((bar.calendar, bar.appointments, bar.tasks, bar.split), (true, false, false, 15));
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

/// The sample's Projects made a folder of `n` messages, none flagged, open, and every folder's
/// flagged mail read by the bar.
fn projects_of(n: u32) -> Session {
    let mut s = big_projects(&Store::memory(), n);
    read_all(&mut s);
    s
}

/// The flagged messages of a folder the bar lists.
fn flagged_in(s: &Session, folder: &str) -> Vec<String> {
    mail(&s.todo_items()).into_iter().filter(|(_, _, k)| k.ends_with(&format!(":{folder}"))).map(|(t, ..)| t).collect()
}

/// Everything in a folder flagged at once (select all, then Flag) puts the newest 500 on the bar
/// at once, as many as reading the folder keeps: it put all of them there, until the folder was
/// read again.
#[test]
fn a_folder_flagged_at_once_keeps_its_newest_on_the_bar() {
    let mut s = projects_of(600);
    assert!(flagged_in(&s, "Projects").is_empty());
    let keys: Vec<_> = s.list.iter().map(|m| m.key.clone()).collect();
    s.todo_op("sample", &Op::SetFlag { keys, flag: Flag::Flagged, on: true });
    let on_bar = flagged_in(&s, "Projects");
    assert_eq!(on_bar.len(), 500);
    assert!(on_bar.contains(&"Report 1".to_string()) && !on_bar.contains(&"Report 600".to_string()), "the newest");
}

/// The same with the list showing the oldest first: still the newest 500 (it kept the first 500
/// of the list as it was arranged, so the oldest).
#[test]
fn a_folder_flagged_at_once_keeps_its_newest_however_the_list_is_arranged() {
    let mut s = projects_of(600);
    s.list.reverse();
    assert_eq!(s.list[0].subject, "Report 600", "the oldest first");
    let keys: Vec<_> = s.list.iter().map(|m| m.key.clone()).collect();
    s.todo_op("sample", &Op::SetFlag { keys, flag: Flag::Flagged, on: true });
    let on_bar = flagged_in(&s, "Projects");
    assert_eq!(on_bar.len(), 500);
    assert!(on_bar.contains(&"Report 1".to_string()) && on_bar.contains(&"Report 500".to_string()), "the newest");
    assert!(!on_bar.contains(&"Report 501".to_string()) && !on_bar.contains(&"Report 600".to_string()));
}

/// What flagging every message of a folder of 20,000 does to the bar's list, on the window's
/// thread (`cargo test --release -p mailcraft-engine --lib flagging_a_big_folder -- --ignored
/// --nocapture`).
#[test]
#[ignore = "a measurement: run in release"]
fn flagging_a_big_folder_for_the_bar_takes() {
    let mut took: Vec<std::time::Duration> = (0..5)
        .map(|_| {
            let mut s = projects_of(20_000);
            let keys: Vec<_> = s.list.iter().map(|m| m.key.clone()).collect();
            let op = Op::SetFlag { keys, flag: Flag::Flagged, on: true };
            let t = std::time::Instant::now();
            s.todo_op("sample", &op);
            t.elapsed()
        })
        .collect();
    took.sort();
    println!("MEASURE the bar told of 20,000 flagged: median {:?}, best {:?}", took[2], took[0]);
}

/// Issue #163: flagged mail is listed without its reply or forward prefix, as Outlook's task list
/// shows it, its `[tag]` kept; a subject that's all prefix stays as it is.
#[test]
fn flagged_replies_are_listed_without_their_prefix() {
    let mut s = sample_session("todo-prefixes");
    for subject in ["RE: [Ticket#4] Printer jam", "FW: Fwd: Budget", "Re:"] {
        let key = next_key(&s, "sample", "INBOX");
        let flags = mailcraft_model::Flags { flagged: true, ..Default::default() };
        file_in(&mut s, mailcraft_model::Summary { key, subject: subject.into(), flags, ..Default::default() }, None);
    }
    read_all(&mut s);
    let titles: Vec<String> = mail(&s.todo_items()).into_iter().map(|(t, ..)| t).collect();
    for want in ["[Ticket#4] Printer jam", "Budget", "Re:"] {
        assert!(titles.iter().any(|t| t == want), "{want}: {titles:?}");
    }
    assert!(!titles.iter().any(|t| t.starts_with("RE:") || t.starts_with("FW:")), "{titles:?}");
}

/// Issue #160: the To-Do Bar's menu on flagged mail in a folder that isn't on screen (nothing
/// selected): Follow Up by another day, a day of its own, no date, Mark Complete and Clear Flag,
/// each undone by Undo; what isn't on the bar, refused.
#[test]
fn flagged_mail_is_followed_up_from_the_bar_wherever_it_is() {
    let mut s = sample_session("todo-flag");
    read_all(&mut s);
    let elsewhere = mail(&s.todo_items()).into_iter().find(|(_, _, k)| !k.ends_with(":INBOX")).map(|(.., k)| k).expect("flagged elsewhere");
    s.selection.clear();
    let group = |s: &Session| mail(&s.todo_items()).into_iter().find(|(.., k)| *k == elsewhere).map(|(_, g, _)| g);
    for (due, want) in [("tomorrow", TodoGroup::Tomorrow), ("nextWeek", TodoGroup::NextWeek), ("none", TodoGroup::NoDate)] {
        s.run("todo.flag", &json!({"key": elsewhere, "due": due})).unwrap();
        assert_eq!(group(&s), Some(want), "{due}");
    }
    // A day of its own (Custom…).
    let later = mailcraft_model::dates::today(&s.clock) + 40;
    s.run("todo.flag", &json!({"key": elsewhere, "due": mailcraft_model::dates::day_string(later)})).unwrap();
    let item = s.todo_items().into_iter().find(|i| matches!(&i.what, TodoWhat::Mail { key } if key.to_id() == elsewhere)).unwrap();
    assert_eq!((item.due, item.group), (Some(later), TodoGroup::Later));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(group(&s), Some(TodoGroup::NoDate), "Undo: no date again");
    // Mark Complete, and Clear Flag: off the bar; Undo puts it back.
    for p in [json!({"key": elsewhere, "complete": true}), json!({"key": elsewhere})] {
        s.run("todo.flag", &p).unwrap();
        assert_eq!(group(&s), None, "{p}");
        s.run("edit.undo", &json!({})).unwrap();
        read_all(&mut s);
        assert_eq!(group(&s), Some(TodoGroup::NoDate), "{p}: back");
    }
    assert!(s.selection.is_empty(), "the selection left as it was");
    for (key, due) in [("", "today"), ("sample:1:999999:INBOX", "today"), (elsewhere.as_str(), "someday")] {
        assert!(s.run("todo.flag", &json!({"key": key, "due": due})).is_err(), "{key} {due}");
    }
}

/// Issue #205: the To-Do Bar's appointments are the user's own, as Outlook's bar shows: not a
/// colleague's calendar opened here (a holiday of theirs was listed under Today), nor one listed
/// with the user's as someone else's. The Calendar shows them all as before.
#[test]
fn the_bars_appointments_are_the_users_own() {
    use mailcraft_model::ical::CalEvent;
    use mailcraft_model::pim::{CalendarInfo, LOCAL, StoredEvent, shared_calendar_id};
    let mut s = sample_session("todo-own");
    let account = s.accounts[0].config.id.clone();
    let ana = shared_calendar_id("ana@contoso.example");
    let p = s.pim_mut(&account);
    p.calendars.push(CalendarInfo { id: ana.clone(), name: "Ana".into(), read_only: true, ..Default::default() });
    p.calendars.push(CalendarInfo { id: "sam".into(), name: "Sam".into(), foreign: true, ..Default::default() });
    for (calendar, summary) in [(LOCAL, "Mine"), (ana.as_str(), "Ana's holiday"), ("sam", "Sam's review")] {
        let event = CalEvent { uid: summary.into(), summary: summary.into(), start: NOW + 3600, end: NOW + 7200, ..Default::default() };
        p.events.push(StoredEvent { calendar: calendar.into(), event, ..Default::default() });
    }
    let names = |items: Vec<crate::AgendaItem>| {
        let mut n: Vec<String> = items.into_iter().map(|i| i.summary).filter(|n| n.contains("Mine") || n.contains('\'')).collect();
        n.sort();
        n
    };
    assert_eq!(names(s.agenda(NOW, NOW + 86_400)), ["Ana's holiday", "Mine", "Sam's review"], "the Calendar's");
    assert_eq!(names(s.todo_appointments(NOW, NOW + 86_400)), ["Mine"], "the bar's");
}
