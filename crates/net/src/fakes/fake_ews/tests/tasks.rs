//! Issue #161: the mailbox's Tasks folder synced both ways, as Outlook keeps it: a task made in
//! Outlook read here with its days, status, priority, reminder, categories and notes; one made
//! here made there; one changed here changed there in place (its item kept), what's taken off
//! taken off; one deleted here deleted there.

use mailcraft_model::pim::{Pim, StoredTask, Task, TaskPriority, TaskStatus, Tombstone};

use super::*;

/// Seconds of a day (UTC midnight), as tasks keep their days.
fn day(y: i64, m: u32, d: u32) -> i64 {
    mailcraft_model::dates::days_from_civil(y, m, d) * 86_400
}

fn server_with(tasks: &[(&str, &str)]) -> (Server, String) {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let folder = d.folder("Tasks", None, Some("tasks"), "IPF.Task");
    for (id, xml) in tasks {
        let el = parse(xml);
        d.pim.push(PimItem::new(*id, folder.clone(), el.kids[0].clone()));
    }
    (serve(d), folder)
}

fn connect(server: &Server) -> crate::ews::Client {
    crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap()
}

#[test]
fn tasks_sync_with_the_tasks_folder() {
    // Made in Outlook: due on the 20th at a time zone's local midnight (UTC+6), started on the
    // 18th, half done, high, with a reminder, a category and notes.
    let outlook = "<t:Task><t:ItemClass>IPM.Task</t:ItemClass><t:Subject>Send the grading notes</t:Subject>\
        <t:Body BodyType=\"Text\">Reels 2 and 3\r\nwarmer</t:Body><t:Categories><t:String>Clients</t:String></t:Categories>\
        <t:Importance>High</t:Importance><t:ReminderDueBy>2026-11-20T08:00:00Z</t:ReminderDueBy><t:ReminderIsSet>true</t:ReminderIsSet>\
        <t:DueDate>2026-11-19T18:00:00Z</t:DueDate><t:PercentComplete>50</t:PercentComplete><t:StartDate>2026-11-17T18:00:00Z</t:StartDate>\
        <t:Status>InProgress</t:Status></t:Task>";
    let request = "<t:Task><t:ItemClass>IPM.TaskRequest</t:ItemClass><t:Subject>Not a task</t:Subject></t:Task>";
    let (server, _) = server_with(&[("T1", outlook), ("T2", request)]);
    let c = connect(&server);
    let mut pim = Pim::default();
    let mine = Task {
        uid: "t-local".into(),
        title: "Book the sound stage".into(),
        due: Some(day(2026, 11, 25)),
        status: TaskStatus::Waiting,
        priority: TaskPriority::Low,
        modified: 5,
        ..Default::default()
    };
    pim.tasks.push(StoredTask { task: mine, dirty: true, ..Default::default() });
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let list = sync.tasks.clone().expect("the Tasks folder read");
    assert_eq!(list.len(), 2, "the task request isn't a task: {list:?}");
    let t = &list.iter().find(|t| t.href == "task:T1").unwrap().task;
    assert_eq!(t.title, "Send the grading notes");
    assert_eq!((t.start, t.due), (Some(day(2026, 11, 18)), Some(day(2026, 11, 20))), "the local midnights' days");
    assert_eq!((t.status, t.percent, t.priority), (TaskStatus::InProgress, 50, TaskPriority::High));
    assert_eq!((t.notes.as_str(), t.categories.clone()), ("Reels 2 and 3\nwarmer", vec!["Clients".to_string()]));
    assert_eq!(t.reminder, mailcraft_jmap::utc_date("2026-11-20T08:00:00Z"));
    // Made here: made there, read back as it was made.
    let [pushed] = sync.pushed_tasks.as_slice() else { panic!("{:?}", sync.pushed_tasks) };
    assert!(pushed.href.starts_with("task:") && pushed.task.uid == "t-local");
    let made = &list.iter().find(|t| t.href == pushed.href).expect("made in the Tasks folder").task;
    assert_eq!(
        (made.title.as_str(), made.due, made.status, made.priority),
        ("Book the sound stage", Some(day(2026, 11, 25)), TaskStatus::Waiting, TaskPriority::Low)
    );

    // Changed here: done, its start and reminder taken off, not repeating. Changed in place.
    let mut done = list.iter().find(|t| t.href == "task:T1").unwrap().clone();
    done.task.set_done(true, day(2026, 11, 19) + 3600);
    done.task.start = None;
    done.task.reminder = None;
    done.task.categories.clear();
    done.dirty = true;
    let pim = Pim { tasks: vec![done], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_100).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.pushed_tasks.iter().map(|t| t.href.as_str()).collect::<Vec<_>>(), ["task:T1"], "the same item");
    let after = &sync.tasks.unwrap().into_iter().find(|t| t.href == "task:T1").unwrap().task;
    assert!(after.is_done() && after.percent == 100, "{after:?}");
    assert_eq!((after.start, after.reminder, after.categories.len()), (None, None, 0), "taken off there");
    assert_eq!(after.due, Some(day(2026, 11, 20)), "what wasn't changed stays");

    // Deleted here: deleted there.
    let pim = Pim { deleted: vec![Tombstone { href: "task:T1".into(), ..Default::default() }], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_200).unwrap();
    assert_eq!(sync.deleted, ["task:T1"]);
    assert!(sync.tasks.unwrap().iter().all(|t| t.href != "task:T1"));
}

/// A mailbox without a Tasks folder keeps its tasks on this computer: nothing read, nothing lost.
#[test]
fn without_a_tasks_folder_tasks_stay_here() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let server = serve(d);
    let c = connect(&server);
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    assert!(sync.tasks.is_none());
}
