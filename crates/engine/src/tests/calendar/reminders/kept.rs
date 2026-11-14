//! Answers to reminders kept while their reminder can still be listed, and forgotten after.

use super::*;

/// An event in the sample's own calendar with a 15-minute reminder, from `start` to `end`.
fn event(s: &mut Session, uid: &str, start: i64, end: i64) {
    let mut e = CalEvent { uid: uid.into(), summary: format!("Event {uid}"), start, end, dtstamp: NOW, ..Default::default() };
    ical::set_reminder(&mut e, Some(15));
    s.pim_mut("sample").events.push(StoredEvent { calendar: mailcraft_model::pim::LOCAL.into(), event: e, ..Default::default() });
}

fn key(uid: &str, at: i64) -> String {
    format!("sample|{uid}|{at}")
}

/// A reminder dismissed, then another snoozed four weeks: the first stays dismissed. The snooze
/// took its own end for "now", and forgot the answers of occurrences that began more than two
/// days ago.
#[test]
fn a_long_snooze_keeps_other_answers() {
    let mut s = sample_session("reminders-long-snooze");
    let day = 86_400;
    event(&mut s, "week", NOW - 5 * day, NOW + 5 * day);
    event(&mut s, "soon", NOW + 600, NOW + 4200);
    s.run("reminders.dismiss", &json!({"key": key("week", NOW - 5 * day)})).unwrap();
    s.run("reminders.snooze", &json!({"key": key("soon", NOW + 600), "minutes": 28 * 24 * 60})).unwrap();
    assert!(!due(&s).contains(&"week".to_string()), "{:?}", due(&s));
}

/// An event that runs for weeks, dismissed long after it began, and a task whose reminder is
/// weeks old but which is due in three days: both stay dismissed. They were forgotten as they
/// were dismissed (by when they began or reminded), and due again at once.
#[test]
fn answers_stay_while_their_reminder_can_be_listed() {
    let mut s = sample_session("reminders-kept");
    let day = 86_400;
    event(&mut s, "season", NOW - 40 * day, NOW + 2 * day);
    s.run("reminders.dismiss", &json!({"key": key("season", NOW - 40 * day)})).unwrap();
    assert!(!due(&s).contains(&"season".to_string()), "{:?}", due(&s));
    s.run("task.create", &json!({"title": "Report", "due": "2026-10-17", "reminder": "2026-09-01T09:00"})).unwrap();
    let k = s.due_reminders(NOW).into_iter().find(|r| r.task && r.summary == "Report").map(|r| r.key).unwrap();
    s.run("reminders.dismiss", &json!({"key": k})).unwrap();
    assert!(!s.due_reminders(NOW).iter().any(|r| r.key == k), "the task's reminder came back");
}

/// Answers whose reminder can't be listed again go as the next is given: an occurrence that
/// ended days ago, an event or task that's gone, a task done.
#[test]
fn answers_go_once_their_reminder_cant_be_listed() {
    let mut s = sample_session("reminders-forgotten");
    let day = 86_400;
    event(&mut s, "over", NOW - 4 * day, NOW - 4 * day + 3600);
    event(&mut s, "soon", NOW + 600, NOW + 4200);
    s.run("task.create", &json!({"title": "Done", "due": "2026-10-20", "reminder": "2026-10-01T09:00"})).unwrap();
    let done_task = s.pim["sample"].tasks.iter().find(|t| t.task.title == "Done").map(|t| (t.task.uid.clone(), t.task.reminder)).unwrap();
    s.run("task.complete", &json!({"uid": done_task.0})).unwrap();
    for k in [key("over", NOW - 4 * day), key("gone", NOW - day), format!("sample|{}|{}", done_task.0, done_task.1.unwrap())] {
        s.prefs.reminders.insert(k, 0);
    }
    s.run("reminders.dismiss", &json!({"key": key("soon", NOW + 600)})).unwrap();
    let kept: Vec<&String> = s.prefs.reminders.keys().collect();
    assert_eq!(kept, [&key("soon", NOW + 600)]);
}

/// An event whose uid has a `|` in it (a server's uid can): its reminder dismissed stays
/// dismissed. Its answer was read back as another account's, of no event, and forgotten as it
/// was given: due again at once.
#[test]
fn a_uid_with_a_bar_is_dismissed_for_good() {
    let mut s = sample_session("reminders-bar");
    event(&mut s, "040000|abc|", NOW + 600, NOW + 4200);
    let k = s.due_reminders(NOW).into_iter().find(|r| r.uid == "040000|abc|").map(|r| r.key).unwrap();
    s.run("reminders.dismiss", &json!({"key": k})).unwrap();
    assert!(!due(&s).contains(&"040000|abc|".to_string()), "{:?}", due(&s));
    assert!(s.prefs.reminders.contains_key(&k));
}

/// Over 5,000 answers kept, the oldest reminders' go first: those of one event went before
/// those of another by its account's and uid's names.
#[test]
fn the_oldest_answers_go_first() {
    let mut s = sample_session("reminders-many");
    let (day, hour) = (86_400, 3600);
    event(&mut s, "zed", NOW + 600, NOW + 4200);
    event(&mut s, "ann", NOW + 600, NOW + 4200);
    for i in 0..2600 {
        s.prefs.reminders.insert(key("zed", NOW + i * hour), 0);
        s.prefs.reminders.insert(key("ann", NOW + 400 * day + i * hour), 0);
    }
    s.snooze_reminder(&key("zed", NOW + 600), NOW + 1200);
    assert_eq!(s.prefs.reminders.len(), 5000);
    assert_eq!(s.prefs.reminders.keys().filter(|k| k.starts_with("sample|ann|")).count(), 2600, "the latest kept");
    assert!(!s.prefs.reminders.contains_key(&key("zed", NOW)) && s.prefs.reminders.contains_key(&key("zed", NOW + 2599 * hour)));
}
