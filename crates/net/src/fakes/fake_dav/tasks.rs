//! Issue #201: tasks in a CalDAV server's task lists, end to end against the in-memory server:
//! a list of tasks alone (Nextcloud's), and a calendar that takes to-dos with its events
//! (SOGo's, Radicale's), read once for both.

use mailcraft_model::ical::{self, CalEvent};
use mailcraft_model::pim::{Pim, StoredTask, Task, Tombstone};
use mailcraft_model::zone::Zone;

use super::*;
use crate::dav::{Dav, Where, sync};

const AT: Where<'static> = Where { email: USER, username: "", others: &[], imap_host: "imap.example", caldav: None, carddav: None, mails: true };

fn task(uid: &str, title: &str) -> Task {
    Task { uid: uid.into(), title: title.into(), due: Some(1_800_000_000 / 86_400 * 86_400), modified: 1_799_000_000, ..Default::default() }
}

/// A server with both kinds of list: a task alone in each, and an event in the calendar.
fn server_with_tasks() -> Server {
    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        d.cal_tasks = true;
        d.task_list = Some(false);
        d.put(&format!("{TASKS}t1.ics"), &ical::write_calendar(&[], &[task("t1", "Buy milk")], 0));
        d.put(&format!("{CAL}t2.ics"), &ical::write_calendar(&[], &[task("t2", "Call the bank")], 0));
        let event = CalEvent { uid: "e1".into(), summary: "Review".into(), start: 1_800_000_000, end: 1_800_003_600, ..Default::default() };
        d.put(&format!("{CAL}e1.ics"), &ical::write(&[event], None, 0));
    }
    server
}

/// What the engine keeps after a sync, as these tests need it.
fn kept_after(pim: &mut Pim, s: &mailcraft_model::pim::DavSync) {
    pim.calendars = s.calendars.clone().unwrap_or_default();
    pim.events = s.events.clone();
    pim.task_lists = s.task_lists.clone().unwrap_or_default();
    pim.tasks = s.list_tasks.clone();
}

fn pim_at(server: &Server) -> Pim {
    Pim { caldav_home: Some(format!("{}/dav/cal/", server.base)), carddav_home: Some(String::new()), dav_checked: Some(100), ..Default::default() }
}

#[test]
fn task_lists_and_their_tasks_are_read() {
    let server = server_with_tasks();
    let dav = Dav::new(USER, PASSWORD);
    let s = sync(&dav, &AT, &pim_at(&server), 100, Zone::UTC).unwrap();
    let lists = s.task_lists.clone().unwrap();
    let (cal, tasks) = (format!("{}{CAL}", server.base), format!("{}{TASKS}", server.base));
    let got: Vec<(&str, &str, bool)> = lists.iter().map(|l| (l.id.as_str(), l.name.as_str(), l.tasks_only)).collect();
    assert_eq!(got, [(cal.as_str(), "Work", false), (tasks.as_str(), "Tasks", true)]);
    // The list of tasks alone isn't a calendar; the calendar's events and tasks come apart.
    assert_eq!(s.calendars.clone().unwrap().iter().map(|c| c.id.clone()).collect::<Vec<_>>(), std::slice::from_ref(&cal));
    assert_eq!(s.events.iter().map(|e| e.event.uid.as_str()).collect::<Vec<_>>(), ["e1"]);
    let mut read: Vec<(&str, &str, &str)> = s.list_tasks.iter().map(|t| (t.task.uid.as_str(), t.task.title.as_str(), t.list.as_str())).collect();
    read.sort();
    assert_eq!(read, [("t1", "Buy milk", tasks.as_str()), ("t2", "Call the bank", cal.as_str())]);
    assert!(s.list_tasks.iter().all(|t| !t.href.is_empty() && !t.etag.is_empty() && !t.dirty));
    let mut synced = s.synced_task_lists.clone();
    synced.sort();
    assert_eq!(synced, [tasks.clone(), cal.clone()], "tasks/ sorts before work/");

    // Nothing changed: neither is read again.
    let mut pim = pim_at(&server);
    kept_after(&mut pim, &s);
    let again = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
    assert!(again.synced_task_lists.is_empty() && again.list_tasks.is_empty(), "{again:?}");
    assert_eq!(again.task_lists.unwrap().len(), 2);
}

/// A server that keeps no tasks (a calendar that says nothing of what it takes, as most say
/// VEVENT alone): no task lists, and tasks stay on this computer.
#[test]
fn a_server_with_no_task_lists_has_none() {
    let server = serve();
    let s = sync(&Dav::new(USER, PASSWORD), &AT, &pim_at(&server), 100, Zone::UTC).unwrap();
    assert_eq!(s.task_lists, Some(Vec::new()));
    assert_eq!(s.calendars.unwrap().len(), 1, "its calendar as before");
}

#[test]
fn tasks_changed_here_are_stored_moved_and_deleted() {
    let server = server_with_tasks();
    let dav = Dav::new(USER, PASSWORD);
    let mut pim = pim_at(&server);
    let first = sync(&dav, &AT, &pim, 100, Zone::UTC).unwrap();
    kept_after(&mut pim, &first);
    let tasks_list = format!("{}{TASKS}", server.base);
    // A new task, one changed, and one deleted.
    pim.tasks.push(StoredTask { task: task("new-1", "Book flights"), list: tasks_list.clone(), dirty: true, ..Default::default() });
    let t1 = pim.tasks.iter_mut().find(|t| t.task.uid == "t1").unwrap();
    t1.task.title = "Buy oat milk".into();
    t1.dirty = true;
    let t2 = pim.tasks.iter().position(|t| t.task.uid == "t2").unwrap();
    let gone = pim.tasks.remove(t2);
    pim.deleted.push(Tombstone { href: gone.href.clone(), etag: gone.etag.clone(), ..Default::default() });
    let s = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
    assert!(s.errors.is_empty(), "{:?}", s.errors);
    let pushed: Vec<(&str, &str)> = s.pushed_tasks.iter().map(|t| (t.task.uid.as_str(), t.href.as_str())).collect();
    let new_href = format!("{tasks_list}new-1.ics");
    assert!(pushed.contains(&("new-1", new_href.as_str())), "{pushed:?}");
    assert_eq!(s.deleted, std::slice::from_ref(&gone.href));
    let d = server.data.lock().unwrap();
    let stored = &d.items[&format!("{TASKS}new-1.ics")].1;
    assert!(stored.contains("BEGIN:VTODO") && stored.contains("SUMMARY:Book flights") && !stored.contains("VEVENT"), "{stored}");
    assert!(d.items[&format!("{TASKS}t1.ics")].1.contains("SUMMARY:Buy oat milk"));
    assert!(!d.items.contains_key(&format!("{CAL}t2.ics")));
    // Each change with the version it was made on (If-Match), a new one only where there's none.
    assert!(d.log.iter().any(|l| l == &format!("PUT {TASKS}new-1.ics")), "{:?}", d.log);
}

/// Changed on the server meanwhile (another device): the change here is refused (412) and the
/// server's copy read again, in its place; nothing is sent over it.
#[test]
fn a_task_changed_on_the_server_meanwhile_gives_way() {
    let server = server_with_tasks();
    let dav = Dav::new(USER, PASSWORD);
    let mut pim = pim_at(&server);
    let first = sync(&dav, &AT, &pim, 100, Zone::UTC).unwrap();
    kept_after(&mut pim, &first);
    server.data.lock().unwrap().put(&format!("{TASKS}t1.ics"), &ical::write_calendar(&[], &[task("t1", "Buy milk and eggs")], 0));
    let t1 = pim.tasks.iter_mut().find(|t| t.task.uid == "t1").unwrap();
    t1.task.title = "Buy bread".into();
    t1.dirty = true;
    let s = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
    assert!(s.pushed_tasks.is_empty());
    assert_eq!(s.refused, ["t1"]);
    let theirs: Vec<&str> = s.list_tasks.iter().filter(|t| t.task.uid == "t1").map(|t| t.task.title.as_str()).collect();
    assert_eq!(theirs, ["Buy milk and eggs"]);
    assert!(server.data.lock().unwrap().items[&format!("{TASKS}t1.ics")].1.contains("Buy milk and eggs"));
}

/// Found on a real Radicale: two syncs one after the other, the second begun before the engine
/// took in the first, sent a new task again as new (`If-None-Match: *`), and the server, which
/// has it, refused it (412): "changed on the server meanwhile", at every sync. Now its item is
/// read in the same sync, and no error is said.
#[test]
fn a_new_task_stored_already_is_read_not_refused() {
    let server = server_with_tasks();
    let dav = Dav::new(USER, PASSWORD);
    let mut pim = pim_at(&server);
    let first = sync(&dav, &AT, &pim, 100, Zone::UTC).unwrap();
    kept_after(&mut pim, &first);
    let tasks_list = format!("{}{TASKS}", server.base);
    pim.tasks.push(StoredTask { task: task("twice", "Sent twice"), list: tasks_list.clone(), dirty: true, ..Default::default() });
    let once = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
    assert_eq!(once.pushed_tasks.len(), 1);
    // The second sync, from what the engine had before it took in the first.
    let again = sync(&dav, &AT, &pim, 300, Zone::UTC).unwrap();
    assert!(again.errors.is_empty(), "{:?}", again.errors);
    assert!(again.list_tasks.iter().any(|t| t.task.uid == "twice" && t.href == format!("{tasks_list}twice.ics")), "read in this sync");
}

/// What other programs store that MailCraft's tasks don't hold: a to-do with no uid gets one,
/// the same at every sync; one with an occurrence changed on its own (`RECURRENCE-ID`) comes as
/// the task alone, marked so, and isn't written back (its occurrence would be lost) even when
/// marked changed.
#[test]
fn todos_beyond_what_a_task_holds_are_kept_safe() {
    let server = server_with_tasks();
    {
        let mut d = server.data.lock().unwrap();
        d.put(&format!("{TASKS}no-uid.ics"), "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nSUMMARY:No uid\r\nEND:VTODO\r\nEND:VCALENDAR\r\n");
        let series = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:weekly\r\nSUMMARY:Water the plants\r\nDTSTART;VALUE=DATE:20270104\r\nRRULE:FREQ=WEEKLY\r\nEND:VTODO\r\n\
BEGIN:VTODO\r\nUID:weekly\r\nRECURRENCE-ID;VALUE=DATE:20270111\r\nSUMMARY:Water the plants (and the garden)\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        d.put(&format!("{TASKS}weekly.ics"), series);
    }
    let dav = Dav::new(USER, PASSWORD);
    let s = sync(&dav, &AT, &pim_at(&server), 100, Zone::UTC).unwrap();
    let no_uid: Vec<&StoredTask> = s.list_tasks.iter().filter(|t| t.task.title == "No uid").collect();
    assert_eq!(no_uid.len(), 1);
    assert!(no_uid[0].task.uid.ends_with("@caldav") && !no_uid[0].partial);
    let again = sync(&dav, &AT, &pim_at(&server), 200, Zone::UTC).unwrap();
    assert!(again.list_tasks.iter().any(|t| t.task.uid == no_uid[0].task.uid), "the same uid again");
    let weekly: Vec<&StoredTask> = s.list_tasks.iter().filter(|t| t.task.uid == "weekly").collect();
    assert_eq!(weekly.len(), 1, "the task alone");
    assert!(weekly[0].partial && weekly[0].task.title == "Water the plants");
    // Marked changed all the same: not written over.
    let mut pim = pim_at(&server);
    kept_after(&mut pim, &s);
    let w = pim.tasks.iter_mut().find(|t| t.task.uid == "weekly").unwrap();
    w.task.title = "Only this".into();
    w.dirty = true;
    let pushed = sync(&dav, &AT, &pim, 300, Zone::UTC).unwrap();
    assert!(pushed.pushed_tasks.is_empty());
    assert!(server.data.lock().unwrap().items[&format!("{TASKS}weekly.ics")].1.contains("RECURRENCE-ID"));
}

/// Found on a real Mailcow: SOGo's collection tag is the time of its last change in seconds,
/// so a task changed in its web page in the second MailCraft read the tag left the tag as it
/// was, and the change was never read. A tag that reads as the second of the sync isn't kept:
/// the next sync looks at the items.
#[test]
fn a_change_in_the_second_the_tag_was_read_is_read() {
    let server = server_with_tasks();
    let dav = Dav::new(USER, PASSWORD);
    let now = 1_795_338_902;
    server.data.lock().unwrap().ctag = now;
    let mut pim = pim_at(&server);
    let first = sync(&dav, &AT, &pim, i64::from(now), Zone::UTC).unwrap();
    assert!(first.task_lists.as_ref().unwrap().iter().all(|l| l.ctag.is_empty()), "not kept");
    kept_after(&mut pim, &first);
    // Changed in the same second: the tag stays.
    server
        .data
        .lock()
        .unwrap()
        .items
        .insert(format!("{TASKS}t1.ics"), ("\"e-later\"".into(), ical::write_calendar(&[], &[task("t1", "Buy milk and eggs")], 0)));
    let s = sync(&dav, &AT, &pim, i64::from(now) + 1, Zone::UTC).unwrap();
    assert!(s.list_tasks.iter().any(|t| t.task.title == "Buy milk and eggs"), "{:?}", s.list_tasks);
    // Long after: kept, and nothing read when nothing changed.
    kept_after(&mut pim, &s);
    let later = sync(&dav, &AT, &pim, i64::from(now) + 600, Zone::UTC).unwrap();
    assert!(later.task_lists.unwrap().iter().all(|l| l.ctag == now.to_string()));
}

/// Found in review: a deletion of an item changed on the server since (another device's change,
/// or a task moved to another list meanwhile) was refused (412) and tried again at every sync,
/// with the same version. It's given up, said, and the server's copy read again.
#[test]
fn a_deletion_of_a_task_changed_meanwhile_is_given_up() {
    let server = server_with_tasks();
    let dav = Dav::new(USER, PASSWORD);
    let mut pim = pim_at(&server);
    let first = sync(&dav, &AT, &pim, 100, Zone::UTC).unwrap();
    kept_after(&mut pim, &first);
    let t1 = pim.tasks.iter().position(|t| t.task.uid == "t1").unwrap();
    let gone = pim.tasks.remove(t1);
    pim.deleted.push(Tombstone { href: gone.href.clone(), etag: gone.etag.clone(), ..Default::default() });
    server.data.lock().unwrap().put(&format!("{TASKS}t1.ics"), &ical::write_calendar(&[], &[task("t1", "Buy milk and eggs")], 0));
    let s = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
    assert_eq!(s.deleted, std::slice::from_ref(&gone.href), "not tried again");
    assert!(s.errors.iter().any(|e| e.contains("wasn't deleted")), "{:?}", s.errors);
    assert!(s.list_tasks.iter().any(|t| t.task.title == "Buy milk and eggs"), "the server's copy");
    assert!(server.data.lock().unwrap().items.contains_key(&format!("{TASKS}t1.ics")));
}

/// A list the user may only read is said so.
#[test]
fn a_read_only_list_is_marked() {
    let server = server_with_tasks();
    server.data.lock().unwrap().task_list = Some(true);
    let s = sync(&Dav::new(USER, PASSWORD), &AT, &pim_at(&server), 100, Zone::UTC).unwrap();
    let ro: Vec<bool> = s.task_lists.unwrap().iter().map(|l| l.read_only).collect();
    assert_eq!(ro, [false, true]);
}
