//! Tasks: made, changed, done (a repeating one moving on), deleted, listed and searched, shown
//! in the calendar on their due day, and taken in from a calendar file.

use super::*;
use mailcraft_model::pim::{TaskPriority, TaskStatus};

fn day(y: i64, m: u32, d: u32) -> i64 {
    mailcraft_model::dates::days_from_civil(y, m, d) * 86_400
}

#[test]
fn tasks_are_made_changed_done_and_deleted() {
    let mut s = sample_session("tasks");
    assert_eq!(s.tasks("").len(), 6, "the sample's tasks");
    assert!(s.tasks("").last().is_some_and(|t| t.task.is_done()), "done ones last");
    // One with no day at all is a task like any other.
    let uid = s
        .run("task.create", &json!({"title": "Call the print shop", "priority": "high", "categories": "Work, Print", "notes": "About the paper"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let t = s.find_task(&uid).unwrap().task;
    assert_eq!((t.due, t.priority, t.status, t.categories.len()), (None, TaskPriority::High, TaskStatus::NotStarted, 2));
    assert!(s.run("task.create", &json!({"title": "  "})).is_err(), "a title is needed");
    assert!(s.run("task.create", &json!({"title": "x", "due": "2026-13-01"})).is_err());
    assert!(s.run("task.create", &json!({"title": "x", "start": "2026-11-05", "due": "2026-11-01"})).is_err(), "due before it starts");
    // Changed: a due day, how far along; what isn't given stays.
    s.set_clock(NOW + 60, 0);
    s.run("task.update", &json!({"uid": uid, "due": "2026-11-20", "percent": 30, "reminder": "2026-11-20T09:30"})).unwrap();
    let t = s.find_task(&uid).unwrap().task;
    assert_eq!((t.due, t.status, t.percent, t.notes.as_str()), (Some(day(2026, 11, 20)), TaskStatus::InProgress, 30, "About the paper"));
    assert_eq!((t.reminder, t.modified), (Some(day(2026, 11, 20) + 9 * 3600 + 1800), NOW + 60));
    assert!(s.run("task.update", &json!({"uid": uid, "status": "sideways"})).is_err());
    assert!(s.run("task.update", &json!({"uid": uid, "percent": 101})).is_err());
    // Searched by its words, without accents or case.
    assert_eq!(s.tasks("PRINT shop").len(), 1);
    assert_eq!(s.tasks("paper").len(), 1, "its notes");
    assert!(s.tasks("cafe").iter().any(|t| t.task.title.starts_with("Send the grading")), "accents ignored");
    // Done, and taken back up.
    s.run("task.complete", &json!({"uid": uid})).unwrap();
    let t = s.find_task(&uid).unwrap().task;
    assert_eq!((t.status, t.percent, t.completed), (TaskStatus::Completed, 100, Some(NOW + 60)));
    s.run("task.complete", &json!({"uid": uid, "done": false})).unwrap();
    assert_eq!(s.find_task(&uid).unwrap().task.status, TaskStatus::NotStarted);
    // Hidden when done, if asked.
    s.run("task.hideDone", &json!({"on": true})).unwrap();
    assert!(s.tasks("").iter().all(|t| !t.task.is_done()));
    s.run("task.hideDone", &json!({})).unwrap();
    // Kept across starts, then deleted.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    drop(s);
    let mut s = Session::open(Some(Store::open(&root).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert_eq!(s.tasks("").len(), 7);
    s.run("task.delete", &json!({"uid": uid})).unwrap();
    assert!(s.find_task(&uid).is_none());
    assert!(s.run("task.delete", &json!({"uid": uid})).is_err());
    done(s);
}

/// Marking a repeating task done keeps a done copy and moves the task to its next due day.
#[test]
fn a_repeating_task_moves_on_when_done() {
    let mut s = sample_session("tasks-repeat");
    let uid = s.run("task.create", &json!({"title": "Water the plants", "due": "2026-11-02", "repeat": "weekly"})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let r = s.run("task.complete", &json!({"uid": uid})).unwrap();
    let t = s.find_task(&uid).unwrap().task;
    assert_eq!((t.due, t.status, t.rrule.as_deref()), (Some(day(2026, 11, 9)), TaskStatus::NotStarted, Some("FREQ=WEEKLY")));
    let copy = s.find_task(r["done"].as_str().unwrap()).unwrap().task;
    assert_eq!((copy.due, copy.status, copy.rrule), (Some(day(2026, 11, 2)), TaskStatus::Completed, None));
    // A rule that has run out: just done.
    let last = s.run("task.create", &json!({"title": "Once more", "due": "2026-11-02", "repeat": "FREQ=DAILY;COUNT=1"})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    s.run("task.complete", &json!({"uid": last})).unwrap();
    assert!(s.find_task(&last).unwrap().task.is_done());
    done(s);
}

/// The list's orders; done ones always last.
#[test]
fn tasks_can_be_arranged() {
    let mut s = sample_session("tasks-order");
    let titles = |s: &Session| s.tasks("").into_iter().filter(|t| !t.task.is_done()).map(|t| t.task.title.clone()).collect::<Vec<_>>();
    s.run("task.arrange", &json!({"by": "due"})).unwrap();
    let by_due = titles(&s);
    assert_eq!(by_due.first().map(String::as_str), Some("Order the festival posters"), "the late one first");
    assert_eq!(by_due.last().map(String::as_str), Some("Renew the font licences"), "no due day: last");
    s.run("task.arrange", &json!({"by": "priority"})).unwrap();
    assert_eq!(titles(&s).first().map(String::as_str), Some("Send the grading notes to the client"));
    assert_eq!(titles(&s).last().map(String::as_str), Some("Renew the font licences"), "low");
    s.run("task.arrange", &json!({"by": "title"})).unwrap();
    assert_eq!(titles(&s).first().map(String::as_str), Some("Back up the edit drives"));
    assert!(s.run("task.arrange", &json!({"by": "size"})).is_err());
    done(s);
}

/// A task due shows in the calendar that day, drawn from the task (not an event); one with no
/// due day doesn't; the events stay as they were.
#[test]
fn tasks_show_in_the_calendar_on_their_due_day() {
    let mut s = sample_session("tasks-calendar");
    let events_before = s.pim.values().map(|p| p.events.len()).sum::<usize>();
    let today = s.local_midnight(mailcraft_model::dates::today(&s.clock));
    let items = s.task_items(today, today + 86_400);
    assert!(items.iter().any(|i| i.task && i.all_day && i.summary == "Send the grading notes to the client"), "{items:?}");
    assert!(items.iter().all(|i| i.task && i.calendar == crate::TASKS_CALENDAR));
    let week = s.task_items(today - 7 * 86_400, today + 7 * 86_400);
    assert!(!week.iter().any(|i| i.summary == "Renew the font licences"));
    assert!(week.iter().any(|i| i.summary == "Return the camera kit" && i.status == "COMPLETED"));
    assert!(s.agenda(today - 7 * 86_400, today + 7 * 86_400).iter().all(|i| !i.task), "the events are the events");
    assert_eq!(s.pim.values().map(|p| p.events.len()).sum::<usize>(), events_before);
    // At the ends of time, nothing overflows.
    s.run("task.create", &json!({"title": "Far off", "due": "9999-12-31"})).unwrap();
    assert!(s.task_items(i64::MIN, i64::MAX).iter().any(|i| i.summary == "Far off"));
    done(s);
}

/// A calendar file's to-dos come in as tasks beside its events.
#[test]
fn a_calendar_files_todos_come_in() {
    let mut s = sample_session("tasks-file");
    let dir = std::env::temp_dir().join(format!("mailcraft-tasks-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plan.ics");
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:todo-1\r\nSUMMARY:Pack the lenses\r\nEND:VTODO\r\n\
BEGIN:VEVENT\r\nUID:ev-1\r\nDTSTART:20261110T100000Z\r\nDTEND:20261110T110000Z\r\nSUMMARY:Shoot\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    std::fs::write(&path, ics).unwrap();
    let r = s.run("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!((r["added"].as_u64(), r["tasks"].as_u64()), (Some(1), Some(1)), "{r}");
    assert!(s.tasks("lenses").len() == 1);
    let _ = std::fs::remove_dir_all(&dir);
    done(s);
}

/// A task's reminder rings in the Reminders window once its time comes, until it's dismissed or
/// the task is done; snoozed, it comes again; one for a day long gone doesn't ring.
#[test]
fn a_tasks_reminder_rings() {
    let mut s = sample_session("tasks-reminder");
    let at = NOW + 3600;
    let uid = s.run("task.create", &json!({"title": "Call the lab", "reminder": at.to_string()})).unwrap()["uid"].as_str().unwrap().to_string();
    let due = |s: &Session| s.due_reminders(s.clock.now).into_iter().filter(|r| r.task).map(|r| r.summary).collect::<Vec<_>>();
    assert!(due(&s).is_empty(), "not yet");
    s.set_clock(at + 60, 0);
    assert_eq!(due(&s), ["Call the lab"]);
    let key = s.due_reminders(s.clock.now).into_iter().find(|r| r.task).unwrap().key;
    s.run("reminders.snooze", &json!({"key": key, "minutes": 10})).unwrap();
    assert!(due(&s).is_empty(), "snoozed");
    s.set_clock(at + 60 + 601, 0);
    assert_eq!(due(&s), ["Call the lab"]);
    s.run("task.complete", &json!({"uid": uid})).unwrap();
    assert!(due(&s).is_empty(), "done: no reminder");
    s.run("task.complete", &json!({"uid": uid, "done": false})).unwrap();
    s.run("reminders.dismiss", &json!({"key": key})).unwrap();
    assert!(due(&s).is_empty(), "dismissed");
    // Due long ago: its reminder doesn't ring now.
    s.run("task.create", &json!({"title": "Old", "due": "2020-01-01", "reminder": "2020-01-01T09:00"})).unwrap();
    assert!(due(&s).is_empty());
    done(s);
}

/// A rule that ends after so many times counts down as the task moves on, and the task is done
/// after its last time, with no copy for a time past it.
#[test]
fn a_repeating_task_with_a_count_ends() {
    let mut s = sample_session("tasks-count");
    let uid = s.run("task.create", &json!({"title": "c", "due": "2026-11-05", "repeat": "FREQ=DAILY;COUNT=2"})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let first = s.run("task.complete", &json!({"uid": uid})).unwrap();
    assert!(first["done"].is_string());
    let t = s.find_task(&uid).unwrap().task;
    assert_eq!((t.due, t.rrule.as_deref(), t.is_done()), (Some(day(2026, 11, 6)), Some("FREQ=DAILY;COUNT=1"), false));
    let count = |s: &Session| s.tasks("").len();
    let before = count(&s);
    let last = s.run("task.complete", &json!({"uid": uid})).unwrap();
    assert!(last.get("done").is_none(), "{last}");
    assert!(s.find_task(&uid).unwrap().task.is_done());
    assert_eq!(count(&s), before, "no copy for a time past the last");
    done(s);
}

/// A task brought in with no title (Outlook keeps tasks with no subject) can be done, taken
/// back up and changed; emptying a title that was there is still refused.
#[test]
fn an_untitled_task_can_be_done_and_changed() {
    let mut s = sample_session("tasks-untitled");
    let task = mailcraft_model::pim::Task { uid: "imp-1".into(), due: Some(day(2026, 11, 5)), ..Default::default() };
    s.pim.values_mut().next().unwrap().tasks.push(mailcraft_model::pim::StoredTask { task, ..Default::default() });
    s.run("task.update", &json!({"uid": "imp-1", "percent": 50})).unwrap();
    assert_eq!(s.find_task("imp-1").unwrap().task.percent, 50);
    s.run("task.complete", &json!({"uid": "imp-1"})).unwrap();
    assert!(s.find_task("imp-1").unwrap().task.is_done());
    s.run("task.complete", &json!({"uid": "imp-1", "done": false})).unwrap();
    // Repeating: its done copy comes in untitled too.
    s.run("task.update", &json!({"uid": "imp-1", "repeat": "weekly"})).unwrap();
    let r = s.run("task.complete", &json!({"uid": "imp-1"})).unwrap();
    assert_eq!(s.find_task(r["done"].as_str().unwrap()).unwrap().task.title, "");
    // A title given, then taken away: refused.
    s.run("task.update", &json!({"uid": "imp-1", "title": "Named"})).unwrap();
    assert!(s.run("task.update", &json!({"uid": "imp-1", "title": " "})).is_err());
    done(s);
}

/// A calendar file's to-dos come in whatever they lack (a title; a start no later than the due
/// day), keep when they were changed, and opening the file again replaces them, not adds them.
#[test]
fn a_calendar_file_opened_twice_keeps_one_of_each_todo() {
    let mut s = sample_session("tasks-file-again");
    let dir = std::env::temp_dir().join(format!("mailcraft-tasks-again-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("todos.ics");
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:untitled-1\r\nDUE;VALUE=DATE:20261105\r\nLAST-MODIFIED:20261001T120000Z\r\nEND:VTODO\r\n\
BEGIN:VTODO\r\nUID:backwards-1\r\nSUMMARY:Backwards\r\nDTSTART;VALUE=DATE:20261110\r\nDUE;VALUE=DATE:20261105\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
    std::fs::write(&path, ics).unwrap();
    let before = s.tasks("").len();
    for _ in 0..2 {
        let r = s.run("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!((r["tasks"].as_u64(), r["skipped"].as_u64()), (Some(2), Some(0)), "{r}");
    }
    assert_eq!(s.tasks("").len(), before + 2, "opened twice, each once");
    let untitled = s.find_task("untitled-1").unwrap().task;
    assert_eq!((untitled.title.as_str(), untitled.modified), ("", mailcraft_model::dates::days_from_civil(2026, 10, 1) * 86_400 + 12 * 3600));
    let backwards = s.find_task("backwards-1").unwrap().task;
    assert_eq!((backwards.start, backwards.due), (Some(day(2026, 11, 5)), Some(day(2026, 11, 5))));
    let _ = std::fs::remove_dir_all(&dir);
    done(s);
}

/// Dirty local tasks and notes whose server copy moved on keep the etag they were read with so
/// the next sync's `If-Match` still guards against overwriting the newer server copy.
#[test]
fn dirty_tasks_and_notes_keep_their_etag_when_server_moves_on() {
    use mailcraft_model::pim::{DavSync, Note, StoredNote, StoredTask, Task};

    let mut s = sample_session("tasks-etag-guard");
    let acc = s.accounts[0].config.id.clone();
    {
        let p = s.pim.get_mut(&acc).unwrap();
        p.tasks.push(StoredTask {
            href: "https://dav.example/tasks/t1.ics".into(),
            etag: "\"v1\"".into(),
            task: Task { uid: "t1".into(), title: "Mine".into(), ..Default::default() },
            dirty: true,
        });
        p.notes.push(StoredNote {
            href: "https://dav.example/notes/n1.ics".into(),
            etag: "\"v1\"".into(),
            note: Note { uid: "n1".into(), text: "Mine".into(), ..Default::default() },
            dirty: true,
        });
    }
    let sync = DavSync {
        tasks: Some(vec![StoredTask {
            href: "https://dav.example/tasks/t1.ics".into(),
            etag: "\"v2\"".into(),
            task: Task { uid: "t1".into(), title: "Theirs".into(), ..Default::default() },
            dirty: false,
        }]),
        notes: Some(vec![StoredNote {
            href: "https://dav.example/notes/n1.ics".into(),
            etag: "\"v2\"".into(),
            note: Note { uid: "n1".into(), text: "Theirs".into(), ..Default::default() },
            dirty: false,
        }]),
        ..Default::default()
    };
    s.apply_dav(&acc, &sync);
    let p = &s.pim[&acc];
    let t = p.tasks.iter().find(|t| t.task.uid == "t1").unwrap();
    let n = p.notes.iter().find(|n| n.note.uid == "n1").unwrap();
    assert_eq!((t.etag.as_str(), t.task.title.as_str(), t.dirty), ("\"v1\"", "Mine", true));
    assert_eq!((n.etag.as_str(), n.note.text.as_str(), n.dirty), ("\"v1\"", "Mine", true));

    // Once a push is refused (412/403), `refused_tasks` / `refused_notes` clear `dirty` so the
    // server's copy takes over instead of retrying forever.
    let refused_sync = DavSync { refused_tasks: vec!["t1".into()], refused_notes: vec!["n1".into()], ..sync };
    s.apply_dav(&acc, &refused_sync);
    let p2 = &s.pim[&acc];
    let t2 = p2.tasks.iter().find(|t| t.task.uid == "t1").unwrap();
    let n2 = p2.notes.iter().find(|n| n.note.uid == "n1").unwrap();
    assert_eq!((t2.etag.as_str(), t2.task.title.as_str(), t2.dirty), ("\"v2\"", "Theirs", false));
    assert_eq!((n2.etag.as_str(), n2.note.text.as_str(), n2.dirty), ("\"v2\"", "Theirs", false));
    done(s);
}
