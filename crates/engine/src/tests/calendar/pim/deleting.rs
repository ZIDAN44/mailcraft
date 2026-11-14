//! Events deleted from a calendar on a server: what the server is told to delete.

use mailcraft_model::ical::CalEvent;
use mailcraft_model::pim::{CalendarInfo, StoredEvent};

use super::*;

/// A weekly series in a server calendar (`S1`), with two changed occurrences: one kept in the
/// series' own item (as CalDAV keeps them), one an item of its own (`I2`, as Exchange keeps
/// them). Returns the session and the two occurrences' times.
fn series(name: &str) -> (Session, i64, i64) {
    let mut s = sample_session(name);
    let (first, second) = (NOW + 7 * 86_400, NOW + 14 * 86_400);
    let p = s.pim_mut("sample");
    p.calendars.push(CalendarInfo { id: "CAL1".into(), name: "Work".into(), ..Default::default() });
    let master = CalEvent {
        uid: "weekly".into(),
        summary: "Stand-up".into(),
        start: NOW,
        end: NOW + 900,
        rrule: Some("FREQ=WEEKLY".into()),
        ..Default::default()
    };
    let moved = |at: i64| CalEvent { recurrence_id: Some(at), start: at + 3600, end: at + 4500, rrule: None, ..master.clone() };
    let item = |event: CalEvent, href: &str| StoredEvent { calendar: "CAL1".into(), href: href.into(), etag: "e".into(), event, dirty: false };
    p.events.extend([item(master.clone(), "S1"), item(moved(first), "S1"), item(moved(second), "I2")]);
    (s, first, second)
}

fn hrefs(s: &Session) -> Vec<String> {
    s.pim["sample"].deleted.iter().map(|t| t.href.clone()).collect()
}

/// What's left of the series.
fn left(s: &Session) -> Vec<&StoredEvent> {
    s.pim["sample"].events.iter().filter(|e| e.event.uid == "weekly").collect()
}

/// One occurrence deleted that's an item of its own on the server: that item is deleted there
/// (it stayed, and came back). One kept in the series' item: the series is written again
/// without it, nothing deleted.
#[test]
fn an_occurrence_with_its_own_item_is_deleted_on_the_server() {
    let (mut s, first, second) = series("delete-own-item");
    s.run("event.delete", &json!({"uid": "weekly", "occurrence": second})).unwrap();
    assert_eq!(hrefs(&s), ["I2"]);
    s.run("event.delete", &json!({"uid": "weekly", "occurrence": first})).unwrap();
    assert_eq!(hrefs(&s), ["I2"], "the series' own item isn't deleted");
    let master = left(&s).into_iter().find(|e| e.event.recurrence_id.is_none()).unwrap();
    assert!(master.dirty && master.event.exdates.contains(&first) && master.event.exdates.contains(&second));
    assert_eq!(left(&s).len(), 1, "both changed copies gone");
}

/// The whole series deleted: each of its items once (the series and an occurrence share one).
#[test]
fn a_series_deleted_deletes_each_item_once() {
    let (mut s, _, _) = series("delete-each-once");
    s.run("event.delete", &json!({"uid": "weekly"})).unwrap();
    let mut gone = hrefs(&s);
    gone.sort();
    assert_eq!(gone, ["I2", "S1"]);
    assert!(left(&s).is_empty());
}

/// A series that keeps as many deleted occurrences as it can: one more stayed in it, yet was
/// answered as deleted, the series' version went up, and the guests of the user's own meeting
/// were told it was off. It's refused, and nothing changes; one deleted already still is.
#[test]
fn an_occurrence_a_full_series_cant_drop_is_refused() {
    let (mut s, first, second) = series("delete-full-series");
    let gone: Vec<i64> = (1..=1000).map(|n| NOW - n * 7 * 86_400).collect();
    let master = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == "weekly" && e.event.recurrence_id.is_none()).unwrap();
    master.event.exdates = gone.clone();
    let before = format!("{:?}", left(&s));
    s.take_jobs();
    for at in [first, second, NOW + 21 * 86_400] {
        let r = s.run("event.delete", &json!({"uid": "weekly", "occurrence": at}));
        assert!(matches!(r, Err(CmdError::Failed(_))), "{r:?}");
    }
    assert_eq!(format!("{:?}", left(&s)), before);
    assert!(hrefs(&s).is_empty() && s.take_jobs().is_empty());
    s.run("event.delete", &json!({"uid": "weekly", "occurrence": gone[0]})).unwrap();
    assert_eq!(left(&s).iter().find(|e| e.event.recurrence_id.is_none()).unwrap().event.exdates.len(), 1000);
}
