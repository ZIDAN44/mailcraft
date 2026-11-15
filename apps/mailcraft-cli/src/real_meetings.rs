//! Issue #25 through real servers on this computer (never a real account), run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_MEETING="127.0.0.1:3143 127.0.0.1:3025 http://127.0.0.1:5232/riley@studio.test/work/" \
//!   cargo test -p mailcraft-cli real_meeting -- --ignored --nocapture
//! ```
//!
//! An IMAP and SMTP server that takes any password and makes mailboxes as they're used
//! (GreenMail), and a calendar server holding Riley's calendar at the address given, password
//! `pw`: Radicale, or Stalwart with its user name after the address
//! (`… http://127.0.0.1:8080/dav/cal/riley/default/ riley`). The whole app's way: the engine's commands, the worker
//! against the servers, a second copy of MailCraft reading back what the servers kept.

use super::*;
use mailcraft_engine::model::ical::{self, CalEvent};

const RILEY: &str = "riley@studio.test";
const ZOE: &str = "zoe@studio.test";
const ROOM: &str = "https://meet.example.edu/rooms/abc-def-ghi";

struct Servers {
    imap: (String, u16),
    smtp: (String, u16),
    caldav: String,
    /// The calendar server's own user name (else the mail's).
    dav_user: Option<String>,
}

fn servers() -> Option<Servers> {
    let v = std::env::var("MAILCRAFT_REAL_MEETING").ok()?;
    let p: Vec<&str> = v.split_whitespace().collect();
    let addr = |a: &str| a.rsplit_once(':').and_then(|(h, p)| Some((h.to_string(), p.parse().ok()?)));
    Some(Servers { imap: addr(p.first()?)?, smtp: addr(p.get(1)?)?, caldav: p.get(2)?.to_string(), dav_user: p.get(3).map(|u| u.to_string()) })
}

/// A copy of MailCraft with one account on those servers, signed in, its calendars read.
fn open(name: &str, email: &str, sv: &Servers) -> (Session, Pool, Store) {
    let dir = std::env::temp_dir().join(format!("mailcraft-real-meeting-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut s = Session::open(Some(store.clone()), clock());
    let p = json!({
        "email": email, "deferSignIn": true, "username": email,
        "imapHost": sv.imap.0, "imapPort": sv.imap.1, "imapSecurity": "none",
        "smtpHost": sv.smtp.0, "smtpPort": sv.smtp.1, "smtpSecurity": "none",
    });
    let id = s.run("account.add", &p).unwrap()["account"].as_str().unwrap().to_string();
    if email == RILEY {
        let mut dav = json!({"account": id, "caldav": sv.caldav});
        if let Some(u) = &sv.dav_user {
            dav["username"] = json!(u);
            dav["password"] = json!("pw");
        }
        s.run("account.dav", &dav).unwrap();
    }
    let config = s.account(&id).unwrap().config.clone();
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    // The calendar server's password to the keychain (here, memory); the sign-in with the mail's.
    for j in s.take_jobs().into_iter().filter(|j| !matches!(j, Job::Start { .. })) {
        pool.submit(j);
    }
    pool.submit(Job::Start { account: config, password: Some(Secret("pw".into())) });
    let dav = email == RILEY;
    let (mut online, mut synced) = (false, !dav);
    drive(&mut s, &mut pool, Duration::from_secs(90), |e| {
        online |= matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. });
        synced |= matches!(e, Event::DavSynced { .. });
        (online && synced).then_some(Ok(()))
    })
    .unwrap();
    (s, pool, store)
}

/// The event as a fresh copy of MailCraft reads it from the calendar server.
fn from_server(sv: &Servers, uid: &str) -> CalEvent {
    let (s, _pool, store) = open("reader", RILEY, sv);
    let e = s.find_event(uid).map(|(_, e)| e.event.clone()).unwrap_or_else(|| panic!("{uid} isn't on the server"));
    let _ = std::fs::remove_dir_all(store.root());
    e
}

/// The source of the message with `subject` in `email`'s Inbox, as its server keeps it, read by a
/// fresh copy of MailCraft.
fn source_of(sv: &Servers, email: &str, subject: &str) -> String {
    let (mut s, mut pool, store) = open("source", email, sv);
    let key = s.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap_or_else(|| panic!("{subject} wasn't delivered"));
    let params = json!({"key": key.to_id()});
    if s.run("message.source", &params).unwrap()["pending"].as_bool() == Some(true) {
        pool.submit(Job::FetchBody { key: key.clone() });
        drive(&mut s, &mut pool, Duration::from_secs(30), |e| matches!(e, Event::Body { .. }).then_some(Ok(()))).unwrap();
    }
    let text = s.run("message.source", &params).unwrap()["text"].as_str().unwrap_or_default().to_string();
    let _ = std::fs::remove_dir_all(store.root());
    text
}

/// A meeting's mail as iMIP sends it (RFC 6047 §2.4, issue #98): the calendar inline, the last
/// alternative beside the text, with its method and character set; not a file.
fn assert_imip(source: &str, method: &str) {
    let unfolded = source.replace("\r\n ", " ").replace("\r\n\t", " ");
    let types: Vec<&str> = unfolded.lines().filter_map(|l| l.strip_prefix("Content-Type: ")).collect();
    assert!(types.first().is_some_and(|t| t.starts_with("multipart/alternative")), "{source}");
    assert!(types.iter().any(|t| *t == format!("text/calendar; charset=\"utf-8\"; method=\"{method}\"")), "{source}");
    assert!(!source.contains("Content-Disposition") && !source.contains("invite.ics"), "{source}");
    assert!(source.contains(&format!("METHOD:{method}")), "{source}");
}

/// Run the jobs a change queued until the server has the event and the guests have mail.
fn send(s: &mut Session, pool: &mut Pool) {
    let (mut pushed, mut sent) = (false, false);
    drive(s, pool, Duration::from_secs(60), |e| {
        match e {
            Event::DavSynced { sync, .. } => {
                assert!(sync.errors.is_empty(), "{:?}", sync.errors);
                pushed |= !sync.pushed_events.is_empty();
            }
            Event::Sent { .. } => sent = true,
            Event::SendFailed { error, .. } => return Some(Err(error.clone())),
            _ => {}
        }
        (pushed && sent).then_some(Ok(()))
    })
    .unwrap();
}

#[test]
#[ignore = "needs real mail and calendar servers in MAILCRAFT_REAL_MEETING"]
fn real_meeting_through_a_calendar_server_and_an_invitation() {
    let Some(sv) = servers() else {
        eprintln!("MAILCRAFT_REAL_MEETING isn't set: skipped");
        return;
    };
    let (mut s, mut pool, store) = open("riley", RILEY, &sv);
    s.run("meetings.addRoom", &json!({"name": "BigBlueButton", "url": ROOM, "code": "123-456-789"})).unwrap();
    let calendar = s.pim.values().flat_map(|p| p.calendars.iter()).find(|c| c.id.starts_with("http")).map(|c| c.id.clone()).unwrap();
    let tag = std::process::id();
    let start = chrono::Utc::now().timestamp() + 86_400;
    let r = s
        .run(
            "event.create",
            &json!({"summary": format!("Team meeting {tag}"), "description": "Team meeting", "start": start, "end": start + 3600,
                    "attendees": format!("Zoe Park <{ZOE}>"), "onlineMeeting": "BigBlueButton", "calendar": calendar,
                    "privacy": "private", "showAs": "oof"}),
        )
        .unwrap();
    let uid = r["uid"].as_str().unwrap().to_string();
    send(&mut s, &mut pool);

    // What the calendar server kept, read by another copy of MailCraft.
    let back = from_server(&sv, &uid);
    eprintln!("from the calendar server: location {:?}\n{}\n{:?}", back.location, back.description, back.conferences());
    assert_eq!(back.location, ROOM);
    assert!(back.description.contains(&format!("---- Online meeting ----\nJoin: {ROOM}\nAccess code: 123-456-789\n")), "{}", back.description);
    let c = back.conferences();
    assert_eq!((c.len(), c[0].uri.as_str(), c[0].label.as_str()), (1, ROOM, "BigBlueButton"));
    assert_eq!(c[0].features, ["AUDIO", "VIDEO"]);
    assert_eq!(back.join_link().as_deref(), Some(ROOM));
    // Private and out of office with it (issue #32), kept by the server too.
    assert_eq!((ical::privacy(&back), ical::show_as(&back)), (ical::Privacy::Private, ical::ShowAs::OutOfOffice), "{:?}", back.extra);

    // The invitation, as Zoe's server delivered it.
    let (mut z, mut zpool, zstore) = open("zoe", ZOE, &sv);
    let subject = format!("Invitation: Team meeting {tag}");
    let key = z.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap_or_else(|| panic!("{subject} wasn't delivered"));
    if z.message_calendar(&key).is_none() {
        zpool.submit(Job::FetchBody { key: key.clone() });
        drive(&mut z, &mut zpool, Duration::from_secs(30), |e| matches!(e, Event::Body { .. }).then_some(Ok(()))).unwrap();
    }
    let cal = z.message_calendar(&key).unwrap();
    assert_eq!(cal.method.as_deref(), Some("REQUEST"));
    let invited = &cal.events[0];
    eprintln!("invitation: {:?} {:?}", invited.location, invited.conferences());
    assert_eq!(z.join_link(invited).as_deref(), Some(ROOM));
    assert_eq!(invited.conferences()[0].label, "BigBlueButton");
    assert!(invited.description.contains("Access code: 123-456-789"));
    assert_eq!((ical::privacy(invited), ical::show_as(invited)), (ical::Privacy::Private, ical::ShowAs::OutOfOffice));
    assert_imip(&source_of(&sv, ZOE, &subject), "REQUEST");

    // Zoe's answer, as Riley's server delivered it.
    z.run("list.open", &json!({"key": key.to_id()})).unwrap();
    z.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    drive(&mut z, &mut zpool, Duration::from_secs(60), |e| match e {
        Event::Sent { .. } => Some(Ok(())),
        Event::SendFailed { error, .. } => Some(Err(error.clone())),
        _ => None,
    })
    .unwrap();
    assert_imip(&source_of(&sv, RILEY, &format!("Accepted: Team meeting {tag}")), "REPLY");

    // Taken out: the calendar server's copy and the guests' update lose it, the notes stay.
    s.run("event.update", &json!({"uid": uid, "onlineMeeting": false})).unwrap();
    send(&mut s, &mut pool);
    let back = from_server(&sv, &uid);
    assert_eq!((back.description.as_str(), back.location.as_str()), ("Team meeting", ""));
    assert!(back.conferences().is_empty() && back.join_link().is_none(), "{back:?}");

    // Called off: the guest's cancellation.
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    drive(&mut s, &mut pool, Duration::from_secs(60), |e| match e {
        Event::Sent { .. } => Some(Ok(())),
        Event::SendFailed { error, .. } => Some(Err(error.clone())),
        _ => None,
    })
    .unwrap();
    assert_imip(&source_of(&sv, ZOE, &format!("Cancelled: Team meeting {tag}")), "CANCEL");
    for st in [store, zstore] {
        let _ = std::fs::remove_dir_all(st.root());
    }
}

/// Issue #100: a daily meeting with one day taken out, then moved ten minutes later. The day
/// taken out stays out on the calendar server and for the guest; it came back, at the old
/// time in the `EXDATE`.
#[test]
#[ignore = "needs real mail and calendar servers in MAILCRAFT_REAL_MEETING"]
fn real_series_moved_keeps_its_day_taken_out() {
    let Some(sv) = servers() else {
        eprintln!("MAILCRAFT_REAL_MEETING isn't set: skipped");
        return;
    };
    let (mut s, mut pool, store) = open("riley-series", RILEY, &sv);
    let calendar = s.pim.values().flat_map(|p| p.calendars.iter()).find(|c| c.id.starts_with("http")).map(|c| c.id.clone()).unwrap();
    let tag = std::process::id();
    let day = 86_400;
    let first = (chrono::Utc::now().timestamp() / day + 2) * day + 17 * 3600 + 600;
    let r = s
        .run(
            "event.create",
            &json!({"summary": format!("Stand-up {tag}"), "start": first, "end": first + 1800, "repeat": "FREQ=DAILY;COUNT=5",
                    "attendees": format!("Zoe Park <{ZOE}>"), "calendar": calendar}),
        )
        .unwrap();
    let uid = r["uid"].as_str().unwrap().to_string();
    send(&mut s, &mut pool);
    s.run("event.delete", &json!({"uid": uid, "occurrence": first + 2 * day})).unwrap();
    send(&mut s, &mut pool);
    // Saved unchanged (issue #102): no update, so the guest gets two invitations, not three.
    s.run("event.update", &json!({"uid": uid, "summary": format!("Stand-up {tag}")})).unwrap();
    assert!(s.take_jobs().iter().all(|j| !matches!(j, Job::Send { .. })), "nothing sent for a save without a change");
    s.run("event.update", &json!({"uid": uid, "start": first + 600})).unwrap();
    send(&mut s, &mut pool);

    // On the calendar server, read by another copy of MailCraft.
    let back = from_server(&sv, &uid);
    assert_eq!((back.start, back.exdates.as_slice()), (first + 600, [first + 2 * day + 600].as_slice()), "{back:?}");
    let on_third: Vec<i64> = ical::occurrences(&[back], first + 2 * day - 3600, first + 3 * day - 3600).iter().map(|o| o.start).collect();
    assert!(on_third.is_empty(), "the day taken out stays out: {on_third:?}");

    // For the guest: the newest of the invitations, by the meeting's version (two may come
    // within a second).
    let (mut z, mut zpool, zstore) = open("zoe-series", ZOE, &sv);
    let subject = format!("Invitation: Stand-up {tag}");
    let keys: Vec<_> = z.list.iter().filter(|m| m.subject == subject).map(|m| m.key.clone()).collect();
    assert_eq!(keys.len(), 2, "the invitation and the update");
    for k in &keys {
        if z.message_calendar(k).is_none() {
            zpool.submit(Job::FetchBody { key: k.clone() });
            drive(&mut z, &mut zpool, Duration::from_secs(30), |e| matches!(e, Event::Body { key, .. } if key == k).then_some(Ok(()))).unwrap();
        }
    }
    let series =
        keys.iter().filter_map(|k| z.message_calendar(k)?.events.into_iter().find(|e| e.recurrence_id.is_none())).max_by_key(|e| e.sequence).unwrap();
    assert_eq!((series.start, series.exdates.as_slice()), (first + 600, [first + 2 * day + 600].as_slice()));
    for st in [store, zstore] {
        let _ = std::fs::remove_dir_all(st.root());
    }
}
