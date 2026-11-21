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
    // Recorded by Riley's copy as it arrives (issue #236), and on the calendar server.
    let riley = s.accounts.first().map(|a| a.config.id.clone()).unwrap();
    pool.submit(Job::SyncAll { account: riley });
    let zoe_said = |s: &Session| s.find_event(&uid).and_then(|(_, e)| e.event.attendees.iter().find(|a| a.email == ZOE).map(|a| a.partstat.clone()));
    let mut pushed = false;
    drive(&mut s, &mut pool, Duration::from_secs(90), |e| {
        if let Event::DavSynced { sync, .. } = e {
            pushed |= sync.pushed_events.iter().any(|p| p.event.uid == uid);
        }
        pushed.then_some(Ok(()))
    })
    .unwrap();
    assert_eq!(zoe_said(&s).as_deref(), Some("ACCEPTED"), "recorded as it arrived");
    let back = from_server(&sv, &uid);
    assert!(back.attendees.iter().any(|a| a.email == ZOE && a.partstat == "ACCEPTED"), "{:?}", back.attendees);

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

/// Issue #238: Zoe proposes a later time for Riley's meeting; Riley takes it, and the calendar
/// server has the meeting at that time. Proposed again, Riley turns it down, and Zoe is told.
#[test]
#[ignore = "needs real mail and calendar servers in MAILCRAFT_REAL_MEETING"]
fn real_new_time_proposed_taken_and_declined() {
    let Some(sv) = servers() else {
        eprintln!("MAILCRAFT_REAL_MEETING isn't set: skipped");
        return;
    };
    let (mut s, mut pool, store) = open("riley-propose", RILEY, &sv);
    let calendar = s.pim.values().flat_map(|p| p.calendars.iter()).find(|c| c.id.starts_with("http")).map(|c| c.id.clone()).unwrap();
    let tag = std::process::id();
    let start = (chrono::Utc::now().timestamp() / 3600 + 30) * 3600;
    let summary = format!("Grade review {tag}");
    let p = json!({"summary": summary, "start": start, "end": start + 3600, "attendees": format!("Zoe Park <{ZOE}>"), "calendar": calendar});
    let uid = s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    send(&mut s, &mut pool);

    // Zoe proposes two hours later, saying maybe: the answer and the proposal go.
    let (mut z, mut zpool, zstore) = open("zoe-propose", ZOE, &sv);
    newest(&mut z, &mut zpool, &format!("Invitation: {summary}"), 1);
    z.run("invite.propose", &json!({"start": start + 7200, "answer": "tentative"})).unwrap();
    sent(&mut z, &mut zpool, 2);
    assert_imip(&source_of(&sv, RILEY, &format!("New time proposed: {summary}")), "COUNTER");

    // Riley takes it: the meeting moves, on the calendar server too, and Zoe is sent it.
    let riley = s.accounts.first().map(|a| a.config.id.clone()).unwrap();
    pool.submit(Job::SyncAll { account: riley.clone() });
    newest(&mut s, &mut pool, &format!("New time proposed: {summary}"), 1);
    let inv = s.invite().expect("the proposal");
    assert_eq!((inv.method.as_str(), inv.problem, inv.event.start), ("COUNTER", None, start + 7200), "{inv:?}");
    s.run("invite.acceptProposal", &json!({})).unwrap();
    send(&mut s, &mut pool);
    let back = from_server(&sv, &uid);
    assert_eq!((back.start, back.end), (start + 7200, start + 10_800), "moved on the calendar server");

    // Proposed again from the update, turned down: Zoe is told, the meeting stays.
    let (mut z2, mut zpool2, zstore2) = open("zoe-propose-2", ZOE, &sv);
    newest(&mut z2, &mut zpool2, &format!("Invitation: {summary}"), 2);
    assert_eq!(z2.invite().map(|i| i.event.start), Some(start + 7200), "the update, at the new time");
    z2.run("invite.propose", &json!({"start": start + 86_400})).unwrap();
    sent(&mut z2, &mut zpool2, 1);
    pool.submit(Job::SyncAll { account: riley });
    newest(&mut s, &mut pool, &format!("New time proposed: {summary}"), 2);
    s.run("invite.declineProposal", &json!({})).unwrap();
    sent(&mut s, &mut pool, 1);
    assert_imip(&source_of(&sv, ZOE, &format!("New time declined: {summary}")), "DECLINECOUNTER");
    assert_eq!(from_server(&sv, &uid).start, start + 7200, "the meeting stays");
    drop((z, zpool, z2, zpool2));
    for st in [store, zstore, zstore2] {
        let _ = std::fs::remove_dir_all(st.root());
    }
}

/// Each message sent, `n` of them (an answer goes with a proposal).
fn sent(c: &mut Session, pool: &mut Pool, n: usize) {
    let mut went = 0;
    drive(c, pool, Duration::from_secs(60), |e| match e {
        Event::Sent { .. } => {
            went += 1;
            (went >= n).then_some(Ok(()))
        }
        Event::SendFailed { error, .. } => Some(Err(error.clone())),
        _ => None,
    })
    .unwrap();
}

/// The newest message of `subject` in the Inbox opened, once there are `count` of them (a sync
/// may still be on), with its calendar read.
fn newest(c: &mut Session, pool: &mut Pool, subject: &str, count: usize) {
    let account = c.accounts.first().map(|a| a.config.id.clone()).unwrap_or_default();
    let inbox = c.accounts.first().and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Inbox)).map(|f| f.path.clone()).unwrap_or_default();
    let mut key = None;
    for _ in 0..30 {
        c.run("folder.open", &json!({"account": account, "folder": inbox})).unwrap();
        let found: Vec<_> = c.list.iter().filter(|m| m.subject == subject).map(|m| m.key.clone()).collect();
        if found.len() >= count {
            key = found.into_iter().max_by_key(|k| k.uid);
            break;
        }
        let _ = drive(c, pool, Duration::from_secs(2), |_| None::<Result<(), String>>);
    }
    let key = key.unwrap_or_else(|| panic!("{subject} wasn't delivered ({count})"));
    if c.message_calendar(&key).is_none() {
        pool.submit(Job::FetchBody { key: key.clone() });
        drive(c, pool, Duration::from_secs(30), |e| matches!(e, Event::Body { .. }).then_some(Ok(()))).unwrap();
    }
    c.run("list.open", &json!({"key": key.to_id()})).unwrap();
}

/// Issue #234: Zoe forwards Riley's meeting to Sam; Sam's program gets it as Riley's invitation,
/// and Sam's acceptance reaches Riley, whose copy adds Sam when asked, on the calendar server too.
#[test]
#[ignore = "needs real mail and calendar servers in MAILCRAFT_REAL_MEETING"]
fn real_meeting_forwarded_to_someone_else() {
    let Some(sv) = servers() else {
        eprintln!("MAILCRAFT_REAL_MEETING isn't set: skipped");
        return;
    };
    const SAM: &str = "sam@studio.test";
    let (mut s, mut pool, store) = open("riley-forward", RILEY, &sv);
    let calendar = s.pim.values().flat_map(|p| p.calendars.iter()).find(|c| c.id.starts_with("http")).map(|c| c.id.clone()).unwrap();
    let tag = std::process::id();
    let start = (chrono::Utc::now().timestamp() / 3600 + 50) * 3600;
    let summary = format!("Screening {tag}");
    let p = json!({"summary": summary, "start": start, "end": start + 3600, "attendees": format!("Zoe Park <{ZOE}>"), "calendar": calendar});
    let uid = s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    send(&mut s, &mut pool);

    // Zoe forwards it to Sam.
    let (mut z, mut zpool, zstore) = open("zoe-forward", ZOE, &sv);
    newest(&mut z, &mut zpool, &format!("Invitation: {summary}"), 1);
    z.run("invite.forward", &json!({"to": format!("Sam Okafor <{SAM}>")})).unwrap();
    sent(&mut z, &mut zpool, 1);
    assert_imip(&source_of(&sv, SAM, &format!("Fwd: {summary}")), "REQUEST");

    // Sam's program: Riley's meeting, Sam among its guests; Sam accepts.
    let (mut m, mut mpool, mstore) = open("sam-forward", SAM, &sv);
    newest(&mut m, &mut mpool, &format!("Fwd: {summary}"), 1);
    let inv = m.invite().expect("an invitation");
    assert_eq!((inv.method.as_str(), inv.event.uid.as_str(), inv.problem), ("REQUEST", uid.as_str(), None));
    assert_eq!(inv.event.organizer.as_ref().map(|o| o.email.as_str()), Some(RILEY), "Riley's meeting");
    m.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    sent(&mut m, &mut mpool, 1);

    // Riley's copy: Sam isn't on the list, so it waits; recorded when asked, on the server too.
    let riley = s.accounts.first().map(|a| a.config.id.clone()).unwrap();
    pool.submit(Job::SyncAll { account: riley });
    newest(&mut s, &mut pool, &format!("Accepted: {summary}"), 1);
    let answer = s.invite().expect("Sam's answer");
    assert!(answer.replier.as_ref().is_some_and(|r| r.email == SAM) && !answer.recorded, "{answer:?}");
    s.run("invite.apply", &json!({})).unwrap();
    // Recorded: nothing mailed, the calendar server told.
    drive(&mut s, &mut pool, Duration::from_secs(60), |e| {
        matches!(e, Event::DavSynced { sync, .. } if !sync.pushed_events.is_empty()).then_some(Ok(()))
    })
    .unwrap();
    let back = from_server(&sv, &uid);
    assert!(back.attendees.iter().any(|a| a.email == SAM && a.partstat == "ACCEPTED"), "{:?}", back.attendees);
    drop((z, zpool, m, mpool));
    for st in [store, zstore, mstore] {
        let _ = std::fs::remove_dir_all(st.root());
    }
}

/// Issue #237: invitations already in Riley's Inbox when Riley's copy of MailCraft comes (one of
/// them called off since): the one still on is found, put in the calendar unanswered, and the
/// calendar server has it so; nothing is sent.
#[test]
#[ignore = "needs real mail and calendar servers in MAILCRAFT_REAL_MEETING"]
fn real_invitations_waiting_in_the_inbox() {
    let Some(sv) = servers() else {
        eprintln!("MAILCRAFT_REAL_MEETING isn't set: skipped");
        return;
    };
    // Zoe invites Riley twice, and calls the second off: before Riley's copy is here.
    let (mut z, mut zpool, zstore) = open("zoe-backlog", ZOE, &sv);
    let tag = std::process::id();
    let start = (chrono::Utc::now().timestamp() / 3600 + 70) * 3600;
    let mut uids = Vec::new();
    for (i, name) in ["Colour grade", "Board"].iter().enumerate() {
        let at = start + i64::try_from(i).unwrap_or(0) * 86_400;
        let p = json!({"summary": format!("{name} {tag}"), "start": at, "end": at + 3600, "attendees": format!("Riley Moreno <{RILEY}>")});
        uids.push(z.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string());
        sent(&mut z, &mut zpool, 1);
    }
    z.run("event.delete", &json!({"uid": uids[1]})).unwrap();
    sent(&mut z, &mut zpool, 1);

    let (mut s, mut pool, store) = open("riley-backlog", RILEY, &sv);
    assert!(uids.iter().all(|u| s.find_event(u).is_none()), "nothing taken in at the first look");
    let mut asked = std::collections::HashSet::new();
    let account = s.accounts.first().map(|a| a.config.id.clone()).unwrap();
    let mut found = Vec::new();
    for _ in 0..30 {
        let (f, reading) = s.waiting_invitations(&account, &mut asked);
        found = f;
        if reading == 0 {
            break;
        }
        let _ = drive(&mut s, &mut pool, Duration::from_secs(2), |_| None::<Result<(), String>>);
    }
    let mine: Vec<&str> = found.iter().filter(|w| uids.contains(&w.uid)).map(|w| w.uid.as_str()).collect();
    assert_eq!(mine, [uids[0].as_str()], "the one still on: {found:?}");
    let keys: Vec<_> = found.iter().filter(|w| w.uid == uids[0]).map(|w| w.key.clone()).collect();
    assert_eq!(s.place_waiting_invitations(&keys), 1);
    // No answer goes: what the calendar needs is all that's asked of the servers.
    let jobs = s.take_jobs();
    assert!(!jobs.iter().any(|j| matches!(j, Job::Send { .. })), "nothing sent");
    for j in jobs {
        pool.submit(j);
    }
    drive(&mut s, &mut pool, Duration::from_secs(60), |e| {
        matches!(e, Event::DavSynced { sync, .. } if !sync.pushed_events.is_empty()).then_some(Ok(()))
    })
    .unwrap();
    let back = from_server(&sv, &uids[0]);
    assert!(back.attendees.iter().any(|a| a.email == RILEY && a.partstat == "NEEDS-ACTION"), "unanswered on the server: {:?}", back.attendees);
    drop((z, zpool));
    for st in [store, zstore] {
        let _ = std::fs::remove_dir_all(st.root());
    }
}
