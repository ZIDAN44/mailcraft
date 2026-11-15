//! Events deleted from a real calendar server on this computer (never a real account), run by
//! hand:
//!
//! ```text
//! MAILCRAFT_REAL_CALENDAR="127.0.0.1:21510 http://127.0.0.1:21532/riley@studio.example/personal/" \
//!   cargo test -p mailcraft-cli --test real_calendar -- --ignored --nocapture
//! ```
//!
//! An IMAP server (Dovecot: any user, password `pw`) for the account, and a CalDAV server
//! (Radicale, any password, with a calendar `personal` made for the user). The engine and the
//! network layer run together, as in the app.

#[path = "support/servers.rs"]
mod servers;

use mailcraft_engine::model::Event;
use serde_json::json;
use servers::{App, basic, http};

const USER: &str = "riley@studio.example";

/// A weekly series with one occurrence moved, in one item as CalDAV keeps them.
fn series(uid: &str, summary: &str) -> String {
    [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//Test//EN",
        "BEGIN:VEVENT",
        &format!("UID:{uid}"),
        "DTSTAMP:20261101T090000Z",
        "DTSTART:20261102T090000Z",
        "DTEND:20261102T091500Z",
        "RRULE:FREQ=WEEKLY",
        &format!("SUMMARY:{summary}"),
        "END:VEVENT",
        "BEGIN:VEVENT",
        &format!("UID:{uid}"),
        "DTSTAMP:20261101T090000Z",
        "RECURRENCE-ID:20261109T090000Z",
        "DTSTART:20261109T100000Z",
        "DTEND:20261109T101500Z",
        &format!("SUMMARY:{summary} (moved)"),
        "END:VEVENT",
        "END:VCALENDAR",
        "",
    ]
    .join("\r\n")
}

/// An item of the calendar read straight from the server: its status and text.
fn item(calendar: &str, uid: &str) -> (u16, String) {
    let got = http("GET", &format!("{calendar}{uid}.ics"), &[("Authorization", &basic(&format!("{USER}:pw")))], b"");
    (got.status, got.text())
}

/// Every calendar sync said to have gone through.
fn watch(e: &Event) {
    if let Event::DavSynced { sync, .. } = e {
        eprintln!("calendars: {:?}, {} deleted", sync.calendars.as_ref().map(Vec::len), sync.deleted.len());
        assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    }
}

/// A series with a moved occurrence, deleted whole: its item is deleted on the server, once, and
/// nothing is left to delete. Another's moved occurrence deleted on its own: the series is
/// written again without it, an exception in its place.
#[test]
#[ignore = "needs real IMAP and CalDAV servers in MAILCRAFT_REAL_CALENDAR"]
fn real_series_and_occurrence_deleted() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_CALENDAR") else {
        eprintln!("MAILCRAFT_REAL_CALENDAR isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, calendar] = p.as_slice() else { panic!("MAILCRAFT_REAL_CALENDAR: imap calendar") };
    let home = calendar.trim_end_matches('/').rsplit_once('/').unwrap().0.to_string() + "/";
    let tag = std::process::id();
    let (whole, one) = (format!("whole-{tag}@test.example"), format!("one-{tag}@test.example"));
    let auth = basic(&format!("{USER}:pw"));
    for (uid, summary) in [(&whole, "Stand-up"), (&one, "Review")] {
        let put = http(
            "PUT",
            &format!("{calendar}{uid}.ics"),
            &[("Authorization", &auth), ("Content-Type", "text/calendar")],
            series(uid, summary).as_bytes(),
        );
        assert!(put.status < 300, "PUT {}: {}", put.status, put.text());
    }
    let mut app = App::open("calendar-delete", "pw");
    app.watch = watch;
    let id = app.add_plain(imap, "127.0.0.1:1", json!({"email": USER, "password": "pw"}));
    app.s.run("account.dav", &json!({"account": id, "caldav": home})).unwrap();
    app.sync_dav(&id);
    let parts = |app: &App, uid: &str| app.s.pim[&id].events.iter().filter(|e| e.event.uid == uid).count();
    assert_eq!((parts(&app, &whole), parts(&app, &one)), (2, 2), "each series and its moved occurrence");

    app.s.run("event.delete", &json!({"uid": whole})).unwrap();
    assert_eq!(app.s.pim[&id].deleted.len(), 1, "its one item, once");
    app.sync_dav(&id);
    assert!(app.s.pim[&id].deleted.is_empty(), "nothing left to delete");
    assert_eq!(item(calendar, &whole).0, 404, "gone from the server");

    let moved =
        app.s.pim[&id].events.iter().find(|e| e.event.uid == one && e.event.recurrence_id.is_some()).and_then(|e| e.event.recurrence_id).unwrap();
    app.s.run("event.delete", &json!({"uid": one, "occurrence": moved})).unwrap();
    assert!(app.s.pim[&id].deleted.is_empty(), "the series' item isn't deleted");
    app.sync_dav(&id);
    let (status, ics) = item(calendar, &one);
    assert!(status == 200 && ics.contains("EXDATE:20261109T090000Z") && !ics.contains("RECURRENCE-ID"), "{status}: {ics}");
    assert_eq!(parts(&app, &one), 1);
    http("DELETE", &format!("{calendar}{one}.ics"), &[("Authorization", &auth)], b"");
    eprintln!("a series deleted once, an occurrence taken out of another");
}

/// An event holding a component of 100,000 lines, then its alarm: synced, it keeps a few hundred
/// of them, and its reminder.
#[test]
#[ignore = "needs real IMAP and CalDAV servers in MAILCRAFT_REAL_CALENDAR"]
fn real_event_with_a_huge_component() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_CALENDAR") else {
        eprintln!("MAILCRAFT_REAL_CALENDAR isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, calendar] = p.as_slice() else { panic!("MAILCRAFT_REAL_CALENDAR: imap calendar") };
    let home = calendar.trim_end_matches('/').rsplit_once('/').unwrap().0.to_string() + "/";
    let uid = format!("big-{}@test.example", std::process::id());
    let auth = basic(&format!("{USER}:pw"));
    let ics = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261101T090000Z\r\nDTSTART:20261102T090000Z\r\nDTEND:20261102T091500Z\r\nSUMMARY:Board\r\nBEGIN:X-BIG\r\n{}END:X-BIG\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Board\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        "X-LINE:junk\r\n".repeat(100_000)
    );
    let put = http("PUT", &format!("{calendar}{uid}.ics"), &[("Authorization", &auth), ("Content-Type", "text/calendar")], ics.as_bytes());
    assert!(put.status < 300, "PUT {}: {}", put.status, put.text());
    let mut app = App::open("calendar-huge", "pw");
    app.watch = watch;
    let id = app.add_plain(imap, "127.0.0.1:1", json!({"email": USER, "password": "pw"}));
    app.s.run("account.dav", &json!({"account": id, "caldav": home})).unwrap();
    app.sync_dav(&id);
    let e = &app.s.pim[&id].events.iter().find(|e| e.event.uid == uid).expect("synced").event;
    eprintln!("kept {} lines; reminder {:?}", e.extra.len(), mailcraft_engine::model::ical::reminder_minutes(e));
    assert!(e.extra.len() <= 400, "{}", e.extra.len());
    assert_eq!(mailcraft_engine::model::ical::reminder_minutes(e), Some(15));
    http("DELETE", &format!("{calendar}{uid}.ics"), &[("Authorization", &auth)], b"");
}
