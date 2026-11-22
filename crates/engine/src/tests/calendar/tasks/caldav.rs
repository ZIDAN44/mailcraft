//! Issue #201: tasks of an account whose CalDAV server keeps task lists (Nextcloud, SOGo,
//! Stalwart, Radicale): new tasks go to its default list, changes are sent, the server's are
//! taken in, a list the user may only read stays as it is, and tasks kept here alone before go
//! to the server, as an Exchange account's do.

use mailcraft_model::pim::{DavSync, StoredTask, Task, TaskList};

use super::*;

const CAL: &str = "https://dav.example.org/calendars/zoe/personal/";
const TASKS: &str = "https://dav.example.org/calendars/zoe/tasks/";
const SHARED: &str = "https://dav.example.org/calendars/ana/team/";

fn list(id: &str, name: &str, tasks_only: bool, read_only: bool) -> TaskList {
    TaskList { id: id.into(), name: name.into(), tasks_only, read_only, ctag: "1".into(), ..Default::default() }
}

fn server_task(uid: &str, title: &str, list: &str) -> StoredTask {
    let task = Task { uid: uid.into(), title: title.into(), modified: 1, ..Default::default() };
    StoredTask { task, list: list.into(), href: format!("{list}{uid}.ics"), etag: format!("\"{uid}-1\""), dirty: false, partial: false }
}

/// A calendar-only account at a CalDAV server, with a task kept here before its lists were
/// known.
fn account(name: &str) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let added = s.run("account.add", &json!({"email": "zoe@example.org", "username": "zoe", "password": "pw", "dav": "https://dav.example.org/"}));
    let id = added.unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    (s, id)
}

/// What the server has: its lists, and the tasks of those read in full.
fn synced(s: &mut Session, id: &str, lists: Vec<TaskList>, tasks: Vec<StoredTask>, synced: &[&str], kept: &[&str]) -> DavSync {
    let sync = DavSync {
        caldav_home: Some("https://dav.example.org/calendars/zoe/".into()),
        calendars: Some(vec![mailcraft_model::pim::CalendarInfo { id: CAL.into(), name: "Personal".into(), ..Default::default() }]),
        task_lists: Some(lists),
        list_tasks: tasks,
        synced_task_lists: synced.iter().map(|x| x.to_string()).collect(),
        kept: kept.iter().map(|x| x.to_string()).collect(),
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.into(), sync: Box::new(sync.clone()) });
    sync
}

fn stored<'a>(s: &'a Session, id: &str, uid: &str) -> &'a StoredTask {
    s.pim[id].tasks.iter().find(|t| t.task.uid == uid).unwrap()
}

#[test]
fn tasks_sync_with_the_servers_task_lists() {
    let (mut s, id) = account("caldav-tasks");
    // Kept here before the server's lists were known: stays here, not to send.
    let before = s.run("task.create", &json!({"title": "Water the plants", "account": id})).unwrap()["uid"].as_str().unwrap().to_string();
    assert_eq!((stored(&s, &id, &before).list.as_str(), stored(&s, &id, &before).dirty), ("", false));
    assert!(s.run("task.lists", &json!({})).unwrap()["lists"].as_array().unwrap().is_empty());

    // The server's lists: its tasks come in, and the one kept here goes to the list made for
    // tasks (not the calendar that takes them too), to send now.
    let lists = vec![list(CAL, "Personal", false, false), list(TASKS, "Tasks", true, false), list(SHARED, "Team", true, true)];
    let theirs = vec![server_task("t1", "Buy milk", TASKS), server_task("t2", "Plan the trip", CAL), server_task("t3", "Their task", SHARED)];
    synced(&mut s, &id, lists.clone(), theirs, &[CAL, TASKS, SHARED], &[]);
    assert_eq!(s.pim[&id].task_lists.len(), 3);
    assert_eq!(s.pim[&id].tasks.len(), 4, "{:?}", s.pim[&id].tasks);
    let moved = stored(&s, &id, &before);
    assert_eq!((moved.list.as_str(), moved.dirty, moved.href.as_str()), (TASKS, true, ""));
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "sent at once");
    let listed = s.run("task.lists", &json!({})).unwrap();
    let defaults: Vec<&str> =
        listed["lists"].as_array().unwrap().iter().filter(|l| l["default"] == true).map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(defaults, ["Tasks"]);

    // Made here: to the default list, or the one named; not one the user may only read.
    let uid = s.run("task.create", &json!({"title": "Book flights", "account": id})).unwrap()["uid"].as_str().unwrap().to_string();
    assert_eq!((stored(&s, &id, &uid).list.as_str(), stored(&s, &id, &uid).dirty), (TASKS, true));
    let in_cal = s.run("task.create", &json!({"title": "Pack", "account": id, "list": CAL})).unwrap()["uid"].as_str().unwrap().to_string();
    assert_eq!(stored(&s, &id, &in_cal).list, CAL);
    assert!(s.run("task.create", &json!({"title": "x", "account": id, "list": SHARED})).is_err(), "read-only");
    assert!(s.run("task.create", &json!({"title": "x", "account": id, "list": "https://elsewhere.example/x/"})).is_err());
    let answered = s.run("task.list", &json!({"query": "Pack"})).unwrap();
    assert_eq!(answered["tasks"][0]["list"], CAL);

    // A task in a list the user may only read can't be changed, done or deleted here.
    assert!(s.run("task.update", &json!({"uid": "t3", "title": "Mine now"})).is_err());
    assert!(s.run("task.complete", &json!({"uid": "t3"})).is_err());
    assert!(s.run("task.delete", &json!({"uid": "t3"})).is_err());
    assert_eq!(stored(&s, &id, "t3").task.title, "Their task");

    // Changed here: to send. Moved to another list: stored there, deleted where it was.
    s.run("task.update", &json!({"uid": "t1", "title": "Buy oat milk"})).unwrap();
    assert!(stored(&s, &id, "t1").dirty);
    let old_href = stored(&s, &id, "t2").href.clone();
    s.run("task.update", &json!({"uid": "t2", "list": TASKS})).unwrap();
    let t2 = stored(&s, &id, "t2");
    assert_eq!((t2.list.as_str(), t2.href.as_str(), t2.dirty), (TASKS, "", true));
    assert!(s.pim[&id].deleted.iter().any(|t| t.href == old_href));
    // Deleted: from the server too.
    s.run("task.delete", &json!({"uid": in_cal})).unwrap();
    assert!(s.pim[&id].tasks.iter().all(|t| t.task.uid != in_cal));

    // The server took the changes: their items and versions, no longer to send.
    let mut pushed = stored(&s, &id, "t1").clone();
    pushed.etag = "\"t1-2\"".into();
    pushed.dirty = false;
    let sync = DavSync { pushed_tasks: vec![pushed], checked: NOW, ..Default::default() };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    assert_eq!((stored(&s, &id, "t1").etag.as_str(), stored(&s, &id, "t1").dirty), ("\"t1-2\"", false));
}

/// The server's changes come in; one changed here stays (to send); one gone from the server
/// goes; one the server refused gives way to its copy; a list that's gone takes its tasks with
/// it, but one changed here goes to the default list.
#[test]
fn the_servers_changes_come_in() {
    let (mut s, id) = account("caldav-tasks-merge");
    let lists = vec![list(TASKS, "Tasks", true, false), list(CAL, "Personal", false, false)];
    let theirs = vec![server_task("a", "A", TASKS), server_task("b", "B", TASKS), server_task("c", "C", TASKS), server_task("d", "D", CAL)];
    synced(&mut s, &id, lists.clone(), theirs, &[TASKS, CAL], &[]);
    s.run("task.update", &json!({"uid": "b", "title": "B changed here"})).unwrap();
    s.run("task.update", &json!({"uid": "d", "title": "D changed here"})).unwrap();
    s.take_jobs();
    // TASKS changed on the server: `a` renamed, `b` renamed too, `c` deleted.
    let changed = vec![server_task("a", "A changed there", TASKS), server_task("b", "B changed there", TASKS)];
    synced(&mut s, &id, lists.clone(), changed, &[TASKS], &[]);
    assert_eq!(stored(&s, &id, "a").task.title, "A changed there");
    assert_eq!(stored(&s, &id, "b").task.title, "B changed here", "changed here: to send");
    assert!(s.pim[&id].tasks.iter().all(|t| t.task.uid != "c"), "gone from the server");
    assert_eq!(stored(&s, &id, "d").task.title, "D changed here", "its list not read: as it is");

    // The server refused `b` for good: its copy comes in place of the change.
    let sync = DavSync {
        caldav_home: Some("https://dav.example.org/calendars/zoe/".into()),
        task_lists: Some(lists.clone()),
        list_tasks: vec![server_task("a", "A changed there", TASKS), server_task("b", "B as the server has it", TASKS)],
        synced_task_lists: vec![TASKS.into()],
        refused: vec!["b".into()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    assert_eq!((stored(&s, &id, "b").task.title.as_str(), stored(&s, &id, "b").dirty), ("B as the server has it", false));

    // CAL is no longer a task list: its tasks go, but `d`, changed here, goes to the default
    // list, as a new item there.
    synced(&mut s, &id, vec![list(TASKS, "Tasks", true, false)], vec![server_task("a", "A changed there", TASKS)], &[TASKS], &[]);
    let d = stored(&s, &id, "d");
    assert_eq!((d.list.as_str(), d.href.as_str(), d.dirty), (TASKS, "", true));
    assert_eq!(s.pim[&id].task_lists.len(), 1);
}

/// A task changed here and not yet stored, that a sync finds on the server already (stored by
/// a sync taken in after this one began, found on a real Radicale): it takes that item's
/// address and version, so the change goes over it, not again as new (refused, every sync).
#[test]
fn a_task_sent_twice_takes_its_item_on_the_server() {
    let (mut s, id) = account("caldav-tasks-twice");
    let lists = vec![list(TASKS, "Tasks", true, false)];
    synced(&mut s, &id, lists.clone(), Vec::new(), &[TASKS], &[]);
    let uid = s.run("task.create", &json!({"title": "Sent twice", "account": id})).unwrap()["uid"].as_str().unwrap().to_string();
    s.run("task.update", &json!({"uid": uid, "notes": "changed since"})).unwrap();
    let on_server = server_task(&uid, "Sent twice", TASKS);
    synced(&mut s, &id, lists, vec![on_server.clone()], &[TASKS], &[]);
    let t = stored(&s, &id, &uid);
    assert_eq!(
        (t.href.as_str(), t.etag.as_str(), t.dirty, t.task.notes.as_str()),
        (on_server.href.as_str(), on_server.etag.as_str(), true, "changed since")
    );
    assert_eq!(s.pim[&id].tasks.iter().filter(|t| t.task.uid == uid).count(), 1);
}

/// A task whose item on the server holds occurrences changed on their own isn't changed,
/// done, moved or deleted here: they'd be lost.
#[test]
fn a_task_with_occurrences_of_its_own_is_left_as_it_is() {
    let (mut s, id) = account("caldav-tasks-partial");
    let lists = vec![list(TASKS, "Tasks", true, false), list(CAL, "Personal", false, false)];
    let weekly = StoredTask { partial: true, ..server_task("weekly", "Water the plants", TASKS) };
    synced(&mut s, &id, lists, vec![weekly], &[TASKS], &[]);
    assert!(s.task_partial("weekly"));
    for (cmd, params) in [
        ("task.update", json!({"uid": "weekly", "title": "x"})),
        ("task.complete", json!({"uid": "weekly"})),
        ("task.delete", json!({"uid": "weekly"})),
        ("task.update", json!({"uid": "weekly", "list": CAL})),
    ] {
        assert!(s.run(cmd, &params).is_err(), "{cmd} {params}");
    }
    let t = stored(&s, &id, "weekly");
    assert_eq!((t.task.title.as_str(), t.dirty, t.list.as_str()), ("Water the plants", false, TASKS));
}

/// Found in review: a task moved to another list, or deleted, while a sync was storing it left
/// the item stored behind (it came back as a task of its own), or its deletion with the version
/// before (refused at every sync). The item stored is deleted, with the version just stored.
#[test]
fn moved_or_deleted_while_stored_leaves_nothing_behind() {
    let (mut s, id) = account("caldav-tasks-race");
    let lists = vec![list(TASKS, "Tasks", true, false), list(CAL, "Personal", false, false)];
    synced(&mut s, &id, lists.clone(), vec![server_task("old", "Old", TASKS)], &[TASKS], &[]);
    let pushed_as = |s: &Session, uid: &str, list: &str, etag: &str| StoredTask {
        list: list.into(),
        href: format!("{list}{uid}.ics"),
        etag: etag.into(),
        dirty: false,
        ..stored(s, &id, uid).clone()
    };
    let push = |s: &mut Session, x: StoredTask| {
        let sync = DavSync { pushed_tasks: vec![x], checked: NOW, ..Default::default() };
        s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    };
    // New, stored in TASKS, moved to CAL meanwhile: the item in TASKS goes.
    let uid = s.run("task.create", &json!({"title": "New", "account": id})).unwrap()["uid"].as_str().unwrap().to_string();
    let x = pushed_as(&s, &uid, TASKS, "\"n-1\"");
    s.run("task.update", &json!({"uid": uid, "list": CAL})).unwrap();
    push(&mut s, x.clone());
    assert!(s.pim[&id].deleted.iter().any(|g| g.href == x.href && g.etag == x.etag), "{:?}", s.pim[&id].deleted);
    assert_eq!((stored(&s, &id, &uid).list.as_str(), stored(&s, &id, &uid).dirty), (CAL, true));
    // On the server, changed and moved meanwhile: its deletion takes the version just stored.
    s.run("task.update", &json!({"uid": "old", "title": "Old changed"})).unwrap();
    let y = pushed_as(&s, "old", TASKS, "\"old-2\"");
    s.run("task.update", &json!({"uid": "old", "list": CAL})).unwrap();
    push(&mut s, y.clone());
    let marks: Vec<&str> = s.pim[&id].deleted.iter().filter(|g| g.href == y.href).map(|g| g.etag.as_str()).collect();
    assert_eq!(marks, ["\"old-2\""]);
    // Deleted meanwhile: the item stored goes too.
    let gone = s.run("task.create", &json!({"title": "Gone", "account": id})).unwrap()["uid"].as_str().unwrap().to_string();
    let z = pushed_as(&s, &gone, TASKS, "\"g-1\"");
    s.run("task.delete", &json!({"uid": gone})).unwrap();
    push(&mut s, z.clone());
    assert!(s.pim[&id].deleted.iter().any(|g| g.href == z.href));
}

/// Found in review: the same task in two lists (one of them a colleague's, as SOGo lists those
/// subscribed to) went wrong: deleting one deleted both on the server, the item stored for one
/// was taken for the other's, one changed here kept the other's server changes out. Each is now
/// an item of its own.
#[test]
fn the_same_task_in_two_lists_stays_apart() {
    let (mut s, id) = account("caldav-tasks-twins");
    let lists = vec![list(TASKS, "Tasks", true, false), list(CAL, "Personal", false, false)];
    synced(&mut s, &id, lists.clone(), vec![server_task("twin", "In Tasks", TASKS), server_task("twin", "In Personal", CAL)], &[TASKS, CAL], &[]);
    assert_eq!(s.pim[&id].tasks.iter().filter(|t| t.task.uid == "twin").count(), 2);
    // The one in TASKS changed here; CAL's changed on the server: CAL's comes in all the same.
    s.run("task.update", &json!({"uid": "twin", "notes": "mine"})).unwrap();
    synced(&mut s, &id, lists.clone(), vec![server_task("twin", "In Personal, changed there", CAL)], &[CAL], &[]);
    let titles: Vec<(&str, &str, bool)> =
        s.pim[&id].tasks.iter().filter(|t| t.task.uid == "twin").map(|t| (t.list.as_str(), t.task.title.as_str(), t.dirty)).collect();
    assert!(titles.contains(&(TASKS, "In Tasks", true)) && titles.contains(&(CAL, "In Personal, changed there", false)), "{titles:?}");
    // Stored, the item stored is TASKS's alone.
    let pushed = StoredTask {
        etag: "\"t-2\"".into(),
        dirty: false,
        ..s.pim[&id].tasks.iter().find(|t| t.task.uid == "twin" && t.list == TASKS).unwrap().clone()
    };
    let sync = DavSync { pushed_tasks: vec![pushed], checked: NOW, ..Default::default() };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    let cal_copy = s.pim[&id].tasks.iter().find(|t| t.task.uid == "twin" && t.list == CAL).unwrap();
    assert_eq!(cal_copy.href, format!("{CAL}twin.ics"));
    // Deleted: that one alone, here and on the server.
    s.run("task.delete", &json!({"uid": "twin"})).unwrap();
    assert_eq!(s.pim[&id].tasks.iter().filter(|t| t.task.uid == "twin").count(), 1);
    assert_eq!(s.pim[&id].deleted.iter().filter(|g| g.href.contains("twin")).count(), 1);
}

/// Found in review: `task.update` with a list it can't go to saved the rest of the change and
/// then said it failed; `"list": ""` moved the task to the default list.
#[test]
fn a_list_refused_saves_nothing() {
    let (mut s, id) = account("caldav-tasks-update");
    let lists = vec![list(TASKS, "Tasks", true, false), list(CAL, "Personal", false, false), list(SHARED, "Team", true, true)];
    synced(&mut s, &id, lists, vec![server_task("a", "A", CAL)], &[CAL], &[]);
    assert!(s.run("task.update", &json!({"uid": "a", "title": "Changed", "list": SHARED})).is_err());
    assert!(s.run("task.update", &json!({"uid": "a", "title": "Changed", "list": "https://elsewhere.example/"})).is_err());
    assert_eq!((stored(&s, &id, "a").task.title.as_str(), stored(&s, &id, "a").dirty), ("A", false), "nothing saved");
    s.run("task.update", &json!({"uid": "a", "title": "Changed", "list": ""})).unwrap();
    assert_eq!((stored(&s, &id, "a").task.title.as_str(), stored(&s, &id, "a").list.as_str()), ("Changed", CAL), "stays in its list");
}

/// Unchanged items (`kept`, by their etag) stay as they are; an Exchange account's tasks are
/// none of this (they're in no list).
#[test]
fn unchanged_tasks_stay() {
    let (mut s, id) = account("caldav-tasks-kept");
    let lists = vec![list(TASKS, "Tasks", true, false)];
    synced(&mut s, &id, lists.clone(), vec![server_task("a", "A", TASKS), server_task("b", "B", TASKS)], &[TASKS], &[]);
    let a_href = stored(&s, &id, "a").href.clone();
    // Read again: `a` unchanged (kept), `b` changed.
    synced(&mut s, &id, lists, vec![server_task("b", "B again", TASKS)], &[TASKS], &[&a_href]);
    assert_eq!(stored(&s, &id, "a").task.title, "A");
    assert_eq!(stored(&s, &id, "b").task.title, "B again");
    assert_eq!(s.pim[&id].tasks.len(), 2);
}
