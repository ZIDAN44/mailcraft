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

use mailcraft_engine::model::{Event, Job};
use serde_json::json;
use servers::{App, basic, between, http, http_as, now};

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

/// Issue #192: an event dragged to another day and time, and an all-day one to another day,
/// moved on the server, all of each, as long as it was.
#[test]
#[ignore = "needs real IMAP and CalDAV servers in MAILCRAFT_REAL_CALENDAR"]
fn real_events_moved() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_CALENDAR") else {
        eprintln!("MAILCRAFT_REAL_CALENDAR isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, calendar] = p.as_slice() else { panic!("MAILCRAFT_REAL_CALENDAR: imap calendar") };
    let home = calendar.trim_end_matches('/').rsplit_once('/').unwrap().0.to_string() + "/";
    let tag = std::process::id();
    let (timed, all_day) = (format!("grade-{tag}@test.example"), format!("festival-{tag}@test.example"));
    let auth = basic(&format!("{USER}:pw"));
    let one = |uid: &str, when: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261101T090000Z\r\n{when}SUMMARY:Moved {tag}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    };
    for (uid, when) in [
        (&timed, "DTSTART:20261103T100000Z\r\nDTEND:20261103T113000Z\r\n"),
        (&all_day, "DTSTART;VALUE=DATE:20261105\r\nDTEND;VALUE=DATE:20261107\r\n"),
    ] {
        let put =
            http("PUT", &format!("{calendar}{uid}.ics"), &[("Authorization", &auth), ("Content-Type", "text/calendar")], one(uid, when).as_bytes());
        assert!(put.status < 300, "PUT {}: {}", put.status, put.text());
    }
    let mut app = App::open("calendar-move", "pw");
    app.watch = watch;
    let id = app.add_plain(imap, "127.0.0.1:1", json!({"email": USER, "password": "pw"}));
    app.s.run("account.dav", &json!({"account": id, "caldav": home})).unwrap();
    app.sync_dav(&id);
    // A day and a quarter of an hour later; the festival a day earlier.
    app.s.run("event.move", &json!({"uid": timed, "by": 86_400 + 900})).unwrap();
    app.s.run("event.move", &json!({"uid": all_day, "by": -86_400})).unwrap();
    app.sync_dav(&id);
    let (status, ics) = item(calendar, &timed);
    eprintln!("{ics}");
    assert!(status == 200 && ics.contains("DTSTART:20261104T101500Z") && ics.contains("DTEND:20261104T114500Z"), "{status}: {ics}");
    let (status, ics) = item(calendar, &all_day);
    eprintln!("{ics}");
    assert!(status == 200 && ics.contains("DTSTART;VALUE=DATE:20261104") && ics.contains("DTEND;VALUE=DATE:20261106"), "{status}: {ics}");
    // Read back as moved, and nothing left to send.
    app.sync_dav(&id);
    let start = |uid: &str| app.s.pim[&id].events.iter().find(|e| e.event.uid == uid).map(|e| (e.event.start, e.dirty));
    assert_eq!(start(&timed), Some((mailcraft_engine::model::dates::days_from_civil(2026, 11, 4) * 86_400 + 10 * 3600 + 900, false)));
    for uid in [&timed, &all_day] {
        http("DELETE", &format!("{calendar}{uid}.ics"), &[("Authorization", &auth)], b"");
    }
}

/// Issue #192 over Exchange Web Services (grommunio in Docker, never a real account), run by
/// hand with `MAILCRAFT_REAL_EWS="<EWS address> <user> <password>"`: an event made here and
/// dragged an hour and a quarter later and a day on is moved on the server, as a fresh MailCraft
/// reads it.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_event_moved() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_EWS") else {
        eprintln!("MAILCRAFT_REAL_EWS isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [url, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_EWS: url user password") };
    let open = |tag: &str| {
        let mut app = App::open(tag, password);
        app.watch = watch;
        let id = app.add(json!({"email": user, "username": user, "password": password, "ews": url}));
        app.sync_dav(&id);
        (app, id)
    };
    let (mut app, id) = open("ews-move");
    let summary = format!("Moved {}", std::process::id());
    app.s.run("event.create", &json!({"summary": summary, "start": "2026-11-24T10:00", "end": "2026-11-24T11:00", "account": id})).unwrap();
    app.sync_dav(&id);
    // Read back from the server, by the server's own UID.
    let (uid, start, end) =
        app.s.pim[&id].events.iter().find(|e| e.event.summary == summary).map(|e| (e.event.uid.clone(), e.event.start, e.event.end)).unwrap();
    app.s.run("event.move", &json!({"uid": uid, "by": 86_400 + 4500})).unwrap();
    app.sync_dav(&id);
    let (fresh, fid) = open("ews-move-read");
    let found = fresh.s.pim[&fid].events.iter().find(|e| e.event.summary == summary).map(|e| (e.event.start, e.event.end));
    eprintln!("made {start}..{end}, read back {found:?}");
    let delete = app.s.run("event.delete", &json!({"uid": uid}));
    app.sync_dav(&id);
    delete.unwrap();
    assert_eq!(found, Some((start + 86_400 + 4500, end + 86_400 + 4500)), "moved on the server");
}

/// Issue #208 over Exchange Web Services (grommunio in Docker), run by hand with
/// `MAILCRAFT_REAL_EWS` and `MAILCRAFT_REAL_EWS_PEOPLE` (two other users of the server): a
/// calendar group made, its people's calendars are listed in seconds, read on their own, with no
/// whole calendar pass asked for.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_group_people_come_at_once() {
    let (Ok(v), Ok(people)) = (std::env::var("MAILCRAFT_REAL_EWS"), std::env::var("MAILCRAFT_REAL_EWS_PEOPLE")) else {
        eprintln!("MAILCRAFT_REAL_EWS or MAILCRAFT_REAL_EWS_PEOPLE isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [url, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_EWS: url user password") };
    let people: Vec<String> = people.split_whitespace().map(str::to_ascii_lowercase).collect();
    let mut app = App::open("ews-group", password);
    let id = app.add(json!({"email": user, "username": user, "password": password, "ews": url}));
    app.sync_dav(&id);
    let began = std::time::Instant::now();
    app.s.run("calendar.groupCreate", &json!({"name": "Team 208", "members": people.join("; "), "account": id})).unwrap();
    let jobs = app.s.take_jobs();
    assert!(!jobs.iter().any(|j| matches!(j, Job::SyncDav { .. })), "no whole pass: {jobs:?}");
    for j in jobs {
        app.pool.submit(j);
    }
    let ids: Vec<String> = people.iter().map(|e| mailcraft_engine::model::pim::shared_calendar_id(e)).collect();
    app.pump(60, "their calendars", |s, e| {
        matches!(e, Event::DavSynced { sync, .. } if sync.people_only) && ids.iter().all(|c| s.pim[&id].calendars.iter().any(|x| &x.id == c))
    });
    for c in app.s.pim[&id].calendars.iter().filter(|c| ids.contains(&c.id)) {
        eprintln!("{} free/busy alone: {} read-only: {}", c.name, c.free_busy, c.read_only);
    }
    eprintln!("listed after {:?}", began.elapsed());
    let _ = app.s.run("calendar.groupDelete", &json!({"name": "Team 208", "account": id}));
}

/// Issue #245 over Exchange Web Services (`MAILCRAFT_REAL_EWS`, its user given a job title and
/// department in the directory: on grommunio, `grommunio-admin user create --property
/// title=… --property departmentname=…`): an organisation's signature that names them is filled
/// from the directory once the account is online, with no details typed.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_organisation_signature_from_the_directory() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_EWS") else { return eprintln!("MAILCRAFT_REAL_EWS isn't set: skipped") };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [url, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_EWS: url user password") };
    let mut app = App::open("ews-org", password);
    let profile = json!({"signature": {"name": "Org", "text": "{name}\n{title}, {department}", "locked": true}});
    app.s.set_organisation(mailcraft_engine::policy::Organisation::from_sources(vec![("test".into(), profile, true)]));
    let id = app.add(json!({"email": user, "username": user, "password": password, "ews": url}));
    let details = |s: &mailcraft_engine::Session| s.account(&id).map(|a| a.config.details.clone()).unwrap_or_default();
    app.pump(60, "the directory's answer", |s, e| matches!(e, Event::Directory { .. }) && details(s).contains_key("title"));
    let filled = app.s.account(&id).and_then(|a| a.config.signature_named("Org")).map(|g| g.text.clone()).unwrap_or_default();
    eprintln!("details {:?}; signature {filled:?}", details(&app.s));
    let (title, department) = (&details(&app.s)["title"], &details(&app.s)["department"]);
    assert!(!title.is_empty() && !department.is_empty() && filled.ends_with(&format!("\n{title}, {department}")), "{filled:?}");
}

/// Issue #241: one occurrence of a series changed and another moved on their own, as a fresh
/// MailCraft reads them back from the server: the series as it was, those two its changed
/// occurrences. `occurrences(app, id)`: each part's (`RECURRENCE-ID`, summary, start).
fn occurrences(app: &App, id: &str, uid: &str) -> Vec<(Option<i64>, String, i64)> {
    let mut v: Vec<_> =
        app.s.pim[id].events.iter().filter(|e| e.event.uid == uid).map(|e| (e.event.recurrence_id, e.event.summary.clone(), e.event.start)).collect();
    v.sort();
    v
}

/// Over CalDAV (`MAILCRAFT_REAL_CALENDAR`, as above): kept in the series' own item.
#[test]
#[ignore = "needs real IMAP and CalDAV servers in MAILCRAFT_REAL_CALENDAR"]
fn real_occurrence_changed_on_its_own() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_CALENDAR") else {
        eprintln!("MAILCRAFT_REAL_CALENDAR isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, calendar] = p.as_slice() else { panic!("MAILCRAFT_REAL_CALENDAR: imap calendar") };
    let home = calendar.trim_end_matches('/').rsplit_once('/').unwrap().0.to_string() + "/";
    let open = |tag: &str| {
        let mut app = App::open(tag, "pw");
        app.watch = watch;
        let id = app.add_plain(imap, "127.0.0.1:1", json!({"email": USER, "password": "pw"}));
        app.s.run("account.dav", &json!({"account": id, "caldav": home})).unwrap();
        app.sync_dav(&id);
        (app, id)
    };
    let (mut app, id) = open("occurrence-dav");
    let summary = format!("Stand-up {}", std::process::id());
    let uid = app
        .s
        .run("event.create", &json!({"summary": summary, "start": "2026-11-23T09:00", "end": "2026-11-23T09:15", "repeat": "weekly", "account": id}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    app.sync_dav(&id);
    let first = app.s.find_event(&uid).unwrap().1.event.start;
    let week = 7 * 86_400;
    app.s.run("event.update", &json!({"uid": uid, "occurrence": first + week, "summary": format!("{summary} (Room 2)")})).unwrap();
    app.s.run("event.move", &json!({"uid": uid, "occurrence": first + 2 * week, "by": 86_400 + 3600})).unwrap();
    app.sync_dav(&id);
    // The series' item as the server has it (MailCraft names its items its own way).
    let href = app.s.find_event(&uid).unwrap().1.href.clone();
    let got_item = http("GET", &href, &[("Authorization", &basic(&format!("{USER}:pw")))], b"");
    let (status, ics) = (got_item.status, got_item.text());
    eprintln!("{status}: {ics}");
    let (fresh, fid) = open("occurrence-dav-read");
    let got = occurrences(&fresh, &fid, &uid);
    app.s.run("event.delete", &json!({"uid": uid})).unwrap();
    app.sync_dav(&id);
    assert_eq!(
        got,
        [
            (None, summary.clone(), first),
            (Some(first + week), format!("{summary} (Room 2)"), first + week),
            (Some(first + 2 * week), summary.clone(), first + 2 * week + 86_400 + 3600)
        ],
        "read back"
    );
    assert_eq!(ics.matches("RECURRENCE-ID").count(), 2, "{ics}");
}

/// Over Exchange Web Services (`MAILCRAFT_REAL_EWS`, as above): each changed occurrence made from
/// the series' own on the server.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_occurrence_changed_on_its_own() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_EWS") else {
        eprintln!("MAILCRAFT_REAL_EWS isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [url, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_EWS: url user password") };
    let open = |tag: &str| {
        let mut app = App::open(tag, password);
        app.watch = watch;
        let id = app.add(json!({"email": user, "username": user, "password": password, "ews": url}));
        app.sync_dav(&id);
        (app, id)
    };
    let (mut app, id) = open("ews-occurrence");
    let summary = format!("Stand-up {}", std::process::id());
    app.s
        .run("event.create", &json!({"summary": summary, "start": "2026-11-23T09:00", "end": "2026-11-23T09:15", "repeat": "weekly", "account": id}))
        .unwrap();
    app.sync_dav(&id);
    // By the server's own UID, as it reads it back.
    let (uid, first) = app.s.pim[&id].events.iter().find(|e| e.event.summary == summary).map(|e| (e.event.uid.clone(), e.event.start)).unwrap();
    let week = 7 * 86_400;
    app.s.run("event.update", &json!({"uid": uid, "occurrence": first + week, "summary": format!("{summary} (Room 2)")})).unwrap();
    app.s.run("event.move", &json!({"uid": uid, "occurrence": first + 2 * week, "by": 86_400 + 3600})).unwrap();
    app.sync_dav(&id);
    let (fresh, fid) = open("ews-occurrence-read");
    let got = occurrences(&fresh, &fid, &uid);
    let shown: Vec<(i64, String)> =
        fresh.s.agenda(first, first + 4 * week).into_iter().filter(|a| a.uid == uid).map(|a| (a.start, a.summary)).collect();
    eprintln!("read back {got:?}, shown {shown:?}");
    app.s.run("event.delete", &json!({"uid": uid})).unwrap();
    app.sync_dav(&id);
    assert_eq!(
        shown,
        [
            (first, summary.clone()),
            (first + week, format!("{summary} (Room 2)")),
            (first + 2 * week + 86_400 + 3600, summary.clone()),
            (first + 3 * week, summary.clone())
        ]
    );
}

/// Issue #242: a colleague's free/busy time from a CalDAV server that schedules (RFC 6638), run
/// by hand with `MAILCRAFT_REAL_FREE_BUSY="<server> <user> <colleague> <password>"`: two users of
/// one server, one password for both, each `address` or `address=sign-in name`:
/// - Stalwart: `http://127.0.0.1:23080 ann@st.test bob@st.test <password>`;
/// - Nextcloud: `http://127.0.0.1:26080/remote.php/dav ann@nc.test=ann bob@nc.test=bob <password>`;
/// - SOGo (Mailcow): `http://127.0.0.1:22080/SOGo/dav ann@mc31.test bob@mc31.test <password>`.
///
/// The colleague's meeting is made in a MailCraft of their own, and their calendar opened to the
/// user with WebDAV ACL (`read-free-busy`) where the server takes it (Stalwart; Nextcloud and
/// SOGo give any user's free/busy time to the others). The user opens it with Open shared
/// calendar, and their times are asked of the server while it's shown.
#[test]
#[ignore = "needs a real CalDAV server that schedules in MAILCRAFT_REAL_FREE_BUSY"]
fn real_caldav_colleagues_times() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_FREE_BUSY") else {
        eprintln!("MAILCRAFT_REAL_FREE_BUSY isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [server, user, colleague, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_FREE_BUSY: server user colleague password") };
    let person = |s: &str| s.split_once('=').map_or((s.to_string(), s.to_string()), |(e, l)| (e.to_string(), l.to_string()));
    let ((user, user_login), (colleague, colleague_login)) = (person(user), person(colleague));
    let open = |tag: &str, email: &str, login: &str| {
        let mut app = App::open(tag, password);
        app.watch = watch;
        let id = app.add(json!({"email": email, "username": login, "password": password, "dav": server}));
        app.sync_dav(&id);
        (app, id)
    };
    // The colleague's meeting, Tuesday of next week at 09:00 for an hour (UTC).
    let (mut theirs, tid) = open("free-busy-colleague", &colleague, &colleague_login);
    let day = 86_400;
    let today = now() / day * day;
    // 1970-01-01 was a Thursday: the days since a Monday are (days + 3) % 7.
    let monday = today + (7 - (today / day + 3) % 7) * day;
    let start = monday + day + 9 * 3600;
    let summary = format!("Review {}", std::process::id());
    let at = |t: i64| {
        let (y, m, d) = mailcraft_engine::model::dates::civil_from_days(t / day);
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}", t % day / 3600, t % 3600 / 60)
    };
    let uid = theirs.s.run("event.create", &json!({"summary": summary, "start": at(start), "end": at(start + 3600), "account": tid})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    theirs.sync_dav(&tid);
    let calendar = theirs.s.find_event(&uid).expect("the meeting on their server").1.calendar.clone();

    // The user's MailCraft: its server schedules, so other people's calendars open from it.
    let (mut app, id) = open("free-busy-user", &user, &user_login);
    assert!(app.s.opens_others(&id), "the server schedules");
    let home = app.s.pim[&id].caldav_home.clone().expect("the calendars found");
    let pal = http_as(
        "PROPFIND",
        &home,
        &format!("{user_login}:{password}"),
        "application/xml",
        br#"<d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#,
    );
    // The principal's href, whatever prefix the server gives `DAV:`.
    let text = pal.text();
    let principal = text.split_once("current-user-principal").and_then(|(_, rest)| between(rest, "href>", "<")).expect("the user's principal");
    let acl = format!(
        r#"<?xml version="1.0"?><d:acl xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:ace><d:principal><d:href>{principal}</d:href></d:principal><d:grant><d:privilege><c:read-free-busy/></d:privilege></d:grant></d:ace></d:acl>"#
    );
    let granted = http_as("ACL", &calendar, &format!("{colleague_login}:{password}"), "application/xml", acl.as_bytes());
    eprintln!("ACL on {calendar}: {}", granted.status);

    app.s.run("calendar.openShared", &json!({"email": colleague, "account": id})).unwrap();
    app.s.run("view.calendar", &json!({})).unwrap();
    app.s.run("calendar.goto", &json!({"date": &at(start)[..10]})).unwrap();
    let cal = mailcraft_engine::model::pim::shared_calendar_id(&colleague.to_ascii_lowercase());
    app.pump(60, "their times", |_, e| matches!(e, Event::FreeBusy { .. }));
    let state = format!("{:?}", app.s.free_busy_state(&id, &cal));
    let shown: Vec<(i64, i64, String)> = app
        .s
        .agenda(monday, monday + 7 * day)
        .into_iter()
        .filter(|a| a.busy_only && a.calendar == cal)
        .map(|a| (a.start, a.end, format!("{:?}", a.show_as)))
        .collect();
    eprintln!("{state}: {shown:?}");
    // Issue #277: a meeting's form asks for the same times as the colleague is invited, their
    // calendar not shown, and Next free time goes past their hour.
    app.s.run("calendar.show", &json!({"account": id, "calendar": cal, "shown": false})).unwrap();
    app.s.run("view.mail", &json!({})).unwrap();
    let ask = json!({"account": id, "attendees": colleague, "date": &at(start)[..10]});
    app.s.run("event.scheduling", &ask).unwrap();
    // Known already from the calendar shown, or asked for now.
    if matches!(app.s.guest_times(&id, &colleague), mailcraft_engine::GuestTimes::Asking) {
        app.pump(60, "the guest's times", |s, _| !matches!(s.guest_times(&id, &colleague), mailcraft_engine::GuestTimes::Asking));
    }
    let times = app.s.run("event.scheduling", &ask).unwrap();
    eprintln!("the form's: {times}");
    let busy = times["guests"][0]["busy"].as_array().cloned().unwrap_or_default();
    let next = app.s.run("event.nextFree", &json!({"account": id, "attendees": colleague, "start": at(start), "minutes": 60})).unwrap();
    theirs.s.run("event.delete", &json!({"uid": uid})).unwrap();
    theirs.sync_dav(&tid);
    assert!(shown.contains(&(start, start + 3600, "Busy".to_string())), "{state}: {shown:?}");
    assert!(busy.iter().any(|b| b["start"] == start && b["end"] == start + 3600), "{times}");
    assert!(next["found"] == true && next["start"].as_i64().is_some_and(|t| t >= start + 3600), "{next}");
}

/// Issue #201: tasks in a CalDAV server's task lists, run by hand with
/// `MAILCRAFT_REAL_TASKS="<server> <user> <password>"` (the user `address` or
/// `address=sign-in name`, as `MAILCRAFT_REAL_FREE_BUSY`; the servers its setup scripts make:
/// Nextcloud, SOGo, Stalwart, Radicale). Two MailCraft of the same user, and changes made on the
/// server itself as another program would: a task made in one is in the other, changed and done
/// there, comes back; a change on the server comes in; one crossing it gives way to it; a task
/// made before the server's lists were known goes there; deleted, it's gone from the server.
/// A server with no task list (a new Nextcloud user's, until its Tasks app makes one) gets one
/// made as Nextcloud's Tasks makes them (`MKCALENDAR`, to-dos alone).
#[test]
#[ignore = "needs a real CalDAV server with task lists in MAILCRAFT_REAL_TASKS"]
fn real_caldav_tasks() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_TASKS") else {
        eprintln!("MAILCRAFT_REAL_TASKS isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [server, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_TASKS: server user password") };
    let (email, login) = user.split_once('=').unwrap_or((user, user));
    let auth = format!("{login}:{password}");
    let open = |tag: &str| {
        let mut app = App::open(tag, password);
        app.watch = watch;
        let id = app.add(json!({"email": email, "username": login, "password": password, "dav": server}));
        (app, id)
    };
    // Synced until `done` says what's waited for is there (a sync can end in the second it was
    // asked in, and be one begun before: `App::sync_dav` counts in seconds).
    let settle = |app: &mut App, id: &str, done: &dyn Fn(&mailcraft_engine::Session) -> bool| {
        for _ in 0..5 {
            app.sync_dav(id);
            if done(&app.s) {
                return;
            }
        }
        panic!("not after five syncs");
    };
    let (mut a, aid) = open("tasks-a");
    // Made before the server's lists are known: kept here, then sent to the default list.
    let early = format!("Early task {}", std::process::id());
    let early_uid = a.s.run("task.create", &json!({"title": early, "account": aid})).unwrap()["uid"].as_str().unwrap().to_string();
    a.sync_dav(&aid);
    if a.s.pim[&aid].task_lists.is_empty() {
        let home = a.s.pim[&aid].caldav_home.clone().expect("the calendars found");
        let mk = r#"<?xml version="1.0"?><c:mkcalendar xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:set><d:prop><d:displayname>Tasks</d:displayname><c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set></d:prop></d:set></c:mkcalendar>"#;
        let made = http_as("MKCALENDAR", &format!("{}/mailcraft-tasks/", home.trim_end_matches('/')), &auth, "application/xml", mk.as_bytes());
        eprintln!("no task list: one made ({})", made.status);
        a.sync_dav(&aid);
    }
    let lists: Vec<String> = a.s.pim[&aid].task_lists.iter().map(|l| format!("{} {} tasks-only={}", l.name, l.id, l.tasks_only)).collect();
    eprintln!("task lists: {lists:?}");
    assert!(!lists.is_empty(), "the server's task lists");
    settle(&mut a, &aid, &|s| s.pim.values().flat_map(|p| &p.tasks).any(|t| t.task.uid == early_uid && !t.dirty));
    let sent = a.s.pim[&aid].tasks.iter().find(|t| t.task.uid == early_uid).cloned().unwrap();
    assert!(!sent.list.is_empty() && !sent.href.is_empty() && !sent.dirty, "the early task on the server: {sent:?}");

    // A task made in A, with a due day and a priority: on the server, as a to-do.
    let title = format!("Book flights {}", std::process::id());
    let uid = a
        .s
        .run("task.create", &json!({"title": title, "due": "2027-03-15", "priority": "high", "notes": "Window seat", "account": aid}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    settle(&mut a, &aid, &|s| s.pim.values().flat_map(|p| &p.tasks).any(|t| t.task.uid == uid && !t.dirty));
    let stored = a.s.pim[&aid].tasks.iter().find(|t| t.task.uid == uid).cloned().unwrap();
    assert!(!stored.href.is_empty() && !stored.dirty, "{stored:?}");
    let item = http("GET", &stored.href, &[("Authorization", &basic(&auth))], b"");
    let ics = item.text();
    assert!(item.status == 200 && ics.contains("BEGIN:VTODO") && ics.contains(&title) && ics.contains("20270315"), "{}: {ics}", item.status);

    // B, the same user elsewhere: the task as it was made.
    let (mut b, bid) = open("tasks-b");
    settle(&mut b, &bid, &|s| s.find_task(&uid).is_some());
    let t = b.s.find_task(&uid).expect("in B").task;
    assert_eq!((t.title.as_str(), t.notes.as_str(), t.priority.code()), (title.as_str(), "Window seat", "high"));
    assert_eq!(t.due.map(|d| mailcraft_engine::model::dates::civil_from_days(d / 86_400)), Some((2027, 3, 15)));
    // Changed and done in B: in A after its next sync.
    b.s.run("task.update", &json!({"uid": uid, "title": format!("{title} (aisle)")})).unwrap();
    b.s.run("task.complete", &json!({"uid": uid})).unwrap();
    settle(&mut b, &bid, &|s| s.pim.values().flat_map(|p| &p.tasks).any(|t| t.task.uid == uid && !t.dirty));
    settle(&mut a, &aid, &|s| s.find_task(&uid).is_some_and(|t| t.task.is_done()));
    let t = a.s.find_task(&uid).unwrap().task;
    assert_eq!((t.title.clone(), t.is_done()), (format!("{title} (aisle)"), true));

    // Changed on the server by another program: comes in.
    let now = a.s.pim[&aid].tasks.iter().find(|t| t.task.uid == uid).cloned().unwrap();
    let theirs = mailcraft_engine::model::pim::Task { title: format!("{title} (changed elsewhere)"), ..now.task.clone() };
    let data = mailcraft_engine::model::ical::write_calendar(&[], &[theirs], 1_800_000_000);
    let put = http(
        "PUT",
        &now.href,
        &[("Authorization", &basic(&auth)), ("Content-Type", "text/calendar; charset=utf-8"), ("If-Match", &now.etag)],
        data.as_bytes(),
    );
    assert!((200..300).contains(&put.status), "PUT {}: {}", put.status, put.text());
    let elsewhere = format!("{title} (changed elsewhere)");
    settle(&mut a, &aid, &|s| s.find_task(&uid).is_some_and(|t| t.task.title == elsewhere));
    assert_eq!(a.s.find_task(&uid).unwrap().task.title, format!("{title} (changed elsewhere)"));

    // A change crossing one on the server (B changes it, A changes it before it syncs): A's,
    // made on the version before, is refused (412), and the server's comes in its place.
    settle(&mut b, &bid, &|s| s.find_task(&uid).is_some_and(|t| t.task.title == elsewhere));
    b.s.run("task.update", &json!({"uid": uid, "notes": "From B"})).unwrap();
    settle(&mut b, &bid, &|s| s.pim.values().flat_map(|p| &p.tasks).any(|t| t.task.uid == uid && !t.dirty));
    a.s.run("task.update", &json!({"uid": uid, "notes": "From A"})).unwrap();
    // The user is told their change didn't go (and only that): from here on in A, as a sync
    // already asked for may still try it again from what was saved before (refused again,
    // nothing written over).
    a.watch = |e| {
        if let Event::DavSynced { sync, .. } = e {
            assert!(sync.errors.iter().all(|x| x.ends_with("changed on the server meanwhile")), "{:?}", sync.errors);
        }
    };
    settle(&mut a, &aid, &|s| s.pim.values().flat_map(|p| &p.tasks).any(|t| t.task.uid == uid && !t.dirty));
    let an = a.s.find_task(&uid).unwrap().task.notes;
    settle(&mut b, &bid, &|_| true);
    let bn = b.s.find_task(&uid).unwrap().task.notes;
    eprintln!("after crossing changes: A has {an:?}, B has {bn:?}");
    assert_eq!(an, "From B", "the server's copy, not A's sent over it");
    assert_eq!(bn, "From B");

    // A to-do another program stored with more than a task keeps (a due time in a zone, two
    // alarms, when it was made): its title changed here, the rest stays on the server.
    let list = a.s.pim[&aid].default_task_list().map(|l| l.id.clone()).expect("a list");
    let rich = format!("{}/rich-{}.ics", list.trim_end_matches('/'), std::process::id());
    let rich_uid = format!("rich-{}", std::process::id());
    let todo = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//Another program//EN",
        "BEGIN:VTIMEZONE",
        "TZID:Europe/Berlin",
        "BEGIN:STANDARD",
        "DTSTART:19701025T030000",
        "TZOFFSETFROM:+0200",
        "TZOFFSETTO:+0100",
        "RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU",
        "END:STANDARD",
        "BEGIN:DAYLIGHT",
        "DTSTART:19700329T020000",
        "TZOFFSETFROM:+0100",
        "TZOFFSETTO:+0200",
        "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU",
        "END:DAYLIGHT",
        "END:VTIMEZONE",
        "BEGIN:VTODO",
        &format!("UID:{rich_uid}"),
        "DTSTAMP:20261122T090000Z",
        "CREATED:20261120T080000Z",
        "SUMMARY:Submit the report",
        "DUE;TZID=Europe/Berlin:20271125T170000",
        "BEGIN:VALARM",
        "ACTION:DISPLAY",
        "DESCRIPTION:Soon",
        "TRIGGER;RELATED=END:-PT1H",
        "END:VALARM",
        "BEGIN:VALARM",
        "ACTION:DISPLAY",
        "DESCRIPTION:Tomorrow",
        "TRIGGER;RELATED=END:-P1D",
        "END:VALARM",
        "END:VTODO",
        "END:VCALENDAR",
        "",
    ]
    .join("\r\n");
    let put = http("PUT", &rich, &[("Authorization", &basic(&auth)), ("Content-Type", "text/calendar; charset=utf-8")], todo.as_bytes());
    assert!((200..300).contains(&put.status), "PUT {}: {}", put.status, put.text());
    settle(&mut a, &aid, &|s| s.find_task(&rich_uid).is_some());
    a.s.run("task.update", &json!({"uid": rich_uid, "title": "Submit the final report"})).unwrap();
    settle(&mut a, &aid, &|s| s.pim.values().flat_map(|p| &p.tasks).any(|t| t.task.uid == rich_uid && !t.dirty));
    let stored = http("GET", &rich, &[("Authorization", &basic(&auth))], b"").text();
    let flat = stored.replace("\r\n ", "").replace("\n ", "");
    eprintln!("as stored after the change: {flat}");
    assert!(flat.contains("Submit the final report"), "{flat}");
    assert!(flat.contains("20271125T160000Z") || flat.contains("20271125T170000"), "its due time: {flat}");
    assert_eq!(flat.matches("BEGIN:VALARM").count(), 2, "both alarms: {flat}");
    assert!(flat.contains("CREATED:20261120T080000Z"), "{flat}");
    a.s.run("task.delete", &json!({"uid": rich_uid})).unwrap();

    // Deleted in A: gone from the server, and from B.
    let href = a.s.pim[&aid].tasks.iter().find(|t| t.task.uid == uid).map(|t| t.href.clone()).unwrap();
    a.s.run("task.delete", &json!({"uid": uid})).unwrap();
    a.s.run("task.delete", &json!({"uid": early_uid})).unwrap();
    settle(&mut a, &aid, &|s| s.pim.values().all(|p| p.deleted.is_empty()));
    assert_eq!(http("GET", &href, &[("Authorization", &basic(&auth))], b"").status, 404);
    settle(&mut b, &bid, &|s| s.find_task(&uid).is_none());
    assert!(b.s.find_task(&uid).is_none() && b.s.find_task(&early_uid).is_none());
}

/// Issue #254: a meeting delegated by a guest whose calendar is on a server that schedules, run
/// by hand with `MAILCRAFT_REAL_DELEGATE="<server> <organizer> <guest> <delegate> <password>
/// <imap> <smtp>"`: three users of one calendar server, each `address` or `address=sign-in name`
/// (as `MAILCRAFT_REAL_FREE_BUSY`), their mail on GreenMail (any password, its IMAP and SMTP):
/// - Stalwart: `http://127.0.0.1:23080 ann@st.test bob@st.test carol@st.test <password> 127.0.0.1:3143 127.0.0.1:3025`;
/// - Nextcloud: `http://127.0.0.1:26080/remote.php/dav ann@nc.test=ann bob@nc.test=bob carol@nc.test=carol <password> 127.0.0.1:3143 127.0.0.1:3025`;
/// - SOGo (Mailcow): `http://127.0.0.1:22080/SOGo/dav ann@mc31.test bob@mc31.test carol@mc31.test <password> 127.0.0.1:3143 127.0.0.1:3025`.
///
/// The organizer's invitation reaches the guest by mail, and the guest hands their place to the
/// delegate from it: the delegate gets one invitation, as the guest's delegate, and
/// accepts it; the organizer's copy then has the guest's place handed on and the delegate's
/// answer, both recorded as they arrived, in whichever order.
#[test]
#[ignore = "needs a real CalDAV server that schedules, and GreenMail, in MAILCRAFT_REAL_DELEGATE"]
fn real_caldav_meeting_delegated() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_DELEGATE") else {
        eprintln!("MAILCRAFT_REAL_DELEGATE isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [server, organizer, guest, delegate, password, imap, smtp] = p.as_slice() else {
        panic!("MAILCRAFT_REAL_DELEGATE: server organizer guest delegate password imap smtp")
    };
    let person = |s: &str| s.split_once('=').map_or((s.to_string(), s.to_string()), |(e, l)| (e.to_string(), l.to_string()));
    let ((organizer, organizer_login), (guest, guest_login), (delegate, delegate_login)) = (person(organizer), person(guest), person(delegate));
    let open = |tag: &str, email: &str, login: &str| {
        let (mut app, id) = App::at(imap, smtp, email, email, password, tag);
        app.watch = watch;
        app.s.run("account.dav", &json!({"account": id, "caldav": server, "username": login, "password": password})).unwrap();
        app.pump(120, "online", |_, e| matches!(e, Event::Status { status: mailcraft_engine::model::SyncStatus::Online { .. }, .. }));
        app.sync_dav(&id);
        (app, id)
    };
    let settled = |app: &mut App, id: &str, uid: &str| {
        let (id, uid) = (id.to_string(), uid.to_string());
        app.pump(120, "stored and sent", |s, _| s.find_event(&uid).is_none_or(|(_, e)| !e.dirty) && s.pim[&id].notices.is_empty());
        app.settle(3);
    };
    let said = |e: &mailcraft_engine::model::ical::CalEvent, email: &str| {
        e.attendees
            .iter()
            .find(|a| a.email.eq_ignore_ascii_case(email))
            .map(|a| (a.partstat.clone(), a.delegated_to.clone(), a.delegated_from.clone()))
    };
    let day = 86_400;
    let start = (now() / day + 9) * day + 11 * 3600;
    let at = |t: i64| {
        let (y, m, d) = mailcraft_engine::model::dates::civil_from_days(t / day);
        format!("{y:04}-{m:02}-{d:02}T{:02}:00", t % day / 3600)
    };
    let summary = format!("Handover {}", std::process::id());
    let (mut org, oid) = open("delegate-organizer", &organizer, &organizer_login);
    let uid = org.s.run("event.create", &json!({"summary": summary, "start": at(start), "account": oid, "attendees": guest})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    settled(&mut org, &oid, &uid);

    // The guest: the invitation (MailCraft mails it: the server is told to send none), handed to
    // the delegate from the message.
    let (mut g, gid) = open("delegate-guest", &guest, &guest_login);
    let key = g.wait_for_mail(&gid, &format!("Invitation: {summary}"));
    g.sync_dav(&gid);
    g.s.run("folder.open", &json!({"account": gid, "folder": "INBOX"})).unwrap();
    g.s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert!(g.s.invite().is_some_and(|i| i.delegable && i.problem.is_none()), "{:?}", g.s.invite());
    g.s.run("invite.delegate", &json!({"to": delegate})).unwrap();
    settled(&mut g, &gid, &uid);
    g.sync_dav(&gid);
    let copy = g.s.find_event(&uid).map(|(_, e)| e.event.clone()).unwrap();
    eprintln!("the guest's copy: guest {:?}, delegate {:?}", said(&copy, &guest), said(&copy, &delegate));
    assert_eq!(said(&copy, &guest).map(|x| x.0), Some("DELEGATED".to_string()));

    // The delegate: one invitation, as the guest's delegate; accepted.
    let (mut d, did) = open("delegate-delegate", &delegate, &delegate_login);
    let key = d.wait_for_mail(&did, &format!("Invitation: {summary}"));
    std::thread::sleep(std::time::Duration::from_secs(3));
    d.pool.submit(mailcraft_engine::model::Job::SyncFolder { account: did.clone(), folder: "INBOX".into() });
    d.settle(3);
    let invitations = d.s.folder_state(&did, "INBOX").messages.iter().filter(|m| m.subject.contains(&summary)).count();
    assert_eq!(invitations, 1, "one invitation");
    d.sync_dav(&did);
    d.s.run("folder.open", &json!({"account": did, "folder": "INBOX"})).unwrap();
    d.s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    let inv = d.s.invite().expect("the invitation");
    eprintln!("the delegate's card: handed by {:?}, problem {:?}", inv.handed_by, inv.problem);
    assert!(inv.handed_by.is_some() && inv.problem.is_none(), "{inv:?}");
    d.s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    settled(&mut d, &did, &uid);

    // The organizer: the guest's place handed on, and the delegate's answer.
    let mut last = None;
    for _ in 0..20 {
        org.pool.submit(mailcraft_engine::model::Job::SyncFolder { account: oid.clone(), folder: "INBOX".into() });
        org.settle(2);
        org.sync_dav(&oid);
        let e = org.s.find_event(&uid).map(|(_, e)| e.event.clone()).unwrap();
        if said(&e, &delegate).is_some_and(|x| x.0 == "ACCEPTED") {
            last = Some(e);
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    let e = last.expect("the delegate's answer never reached the organizer");
    eprintln!("the organizer's copy: guest {:?}, delegate {:?}", said(&e, &guest), said(&e, &delegate));
    assert_eq!(said(&e, &guest).map(|x| x.0), Some("DELEGATED".to_string()));
    let _ = org.s.run("event.delete", &json!({"uid": uid}));
    org.sync_dav(&oid);
}
