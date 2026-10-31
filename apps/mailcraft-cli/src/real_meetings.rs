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
use mailcraft_engine::model::ical::CalEvent;

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
#[ignore]
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
                    "attendees": format!("Zoe Park <{ZOE}>"), "onlineMeeting": "BigBlueButton", "calendar": calendar}),
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

    // Taken out: the calendar server's copy and the guests' update lose it, the notes stay.
    s.run("event.update", &json!({"uid": uid, "onlineMeeting": false})).unwrap();
    send(&mut s, &mut pool);
    let back = from_server(&sv, &uid);
    assert_eq!((back.description.as_str(), back.location.as_str()), ("Team meeting", ""));
    assert!(back.conferences().is_empty() && back.join_link().is_none(), "{back:?}");
    for st in [store, zstore] {
        let _ = std::fs::remove_dir_all(st.root());
    }
}
