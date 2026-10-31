//! Meeting invitations between two users of a real Mailcow on this computer, both in MailCraft:
//! ann invites bob (with an online meeting, issue #25) through Postfix; SOGo, which schedules
//! for its own users itself, puts the meeting in bob's calendar and mails him too. bob's calendar
//! can't be reached when the invitations come (issue #30); once it can, they go there. Answered
//! in MailCraft from the message and from the calendar, they leave bob's Inbox (issue #31), and
//! ann's MailCraft takes his answers in. Never a real account; run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_MAILCOW="127.0.0.1:22143 127.0.0.1:22588 http://127.0.0.1:22080 \
//!   ann@mc31.test:<password> bob@mc31.test:<password> carol@mc31.test:<password>" \
//!   cargo test -p mailcraft-cli --test real_mailcow_invitations -- --ignored --nocapture --test-threads 1
//! ```
//!
//! See `real_mailcow_mail.rs` for the Mailcow it needs.

#[path = "support/mailcow.rs"]
mod mailcow;

use std::time::{Duration, Instant};

use mailcow::{App, Mailcow, proxy};
use mailcraft_engine::model::{Event, Job, SyncStatus};
use serde_json::json;

const MARK: &str = "Mailcow meeting";

/// Take out what an earlier run left in ann's and bob's calendars and Inboxes.
fn clean(m: &Mailcow) {
    for n in [0, 1] {
        let cal = format!("{}personal/", m.dav_home(n));
        for (href, data) in m.calendar(n, &cal) {
            if data.contains(&format!("SUMMARY:{MARK}")) {
                m.dav(n, "DELETE", &format!("{}{href}", m.web), None, "");
            }
        }
        let mut imap = m.imap_as(n);
        for folder in ["INBOX", "Trash"] {
            imap.cmd(&format!("SELECT {folder}"));
            imap.cmd("UID STORE 1:* +FLAGS.SILENT (\\Deleted)");
            imap.cmd("EXPUNGE");
        }
    }
}

/// The subjects in a user's folder, waiting until `ok` holds (or failing after a minute).
fn wait_subjects(m: &Mailcow, n: usize, folder: &str, what: &str, ok: impl Fn(&[String]) -> bool) -> Vec<String> {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let s = m.imap_as(n).subjects(folder);
        if ok(&s) {
            return s;
        }
        assert!(Instant::now() < end, "{what}: {folder} has {s:?}");
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
#[ignore]
fn real_mailcow_invitations_between_two_users() {
    let Some(m) = Mailcow::from_env() else { return };
    clean(&m);
    let (ann_email, _) = m.user(0);
    let (bob_email, _) = m.user(1);
    let web = format!("{}/", m.web);
    let target = m.web.trim_start_matches("http://").to_string();

    // bob's MailCraft: his calendar server given, but not answering yet.
    let (port, gate) = proxy(&target);
    let (mut bob, bid) = App::new(&m, 1, "invitations");
    bob.s.run("account.dav", &json!({"account": bid, "caldav": format!("http://127.0.0.1:{port}/")})).unwrap();
    bob.s.run("options.answeredInvites", &json!({"after": "delete"})).unwrap();
    bob.pump(120, "bob online", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(bob.s.calendars_pending(&bid));

    // ann's MailCraft invites bob, with an online meeting.
    let (mut ann, aid) = App::new(&m, 0, "invitations");
    ann.s.run("account.dav", &json!({"account": aid, "caldav": web})).unwrap();
    ann.s.run("meetings.addRoom", &json!({"name": "BBB", "url": "https://meet.example.edu/rooms/mc3-1in-vit", "code": "2468"})).unwrap();
    ann.sync_dav(&aid);
    let tag = std::process::id();
    let first = format!("{MARK} {tag} one");
    let uid = ann
        .s
        .run("event.create", &json!({"account": aid, "summary": first, "start": "2026-11-18T10:00", "attendees": format!("Bob Tester <{bob_email}>"), "onlineMeeting": "BBB"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    ann.pump(120, "ann's meeting stored and sent", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    ann.settle(3);
    let (_, stored) = ann.s.find_event(&uid).unwrap();
    eprintln!("ann's meeting is at {}", stored.href);

    // What reaches bob: MailCraft's invitation, and SOGo's own (it schedules for its users).
    let inbox = wait_subjects(&m, 1, "INBOX", "the invitations", |s| s.iter().any(|x| x.contains(&first)));
    std::thread::sleep(Duration::from_secs(5));
    let inbox = {
        let again = m.imap_as(1).subjects("INBOX");
        if again.len() > inbox.len() { again } else { inbox }
    };
    eprintln!("bob's Inbox: {inbox:?}");
    let in_sogo: Vec<(String, String)> =
        m.calendar(1, &format!("{}personal/", m.dav_home(1))).into_iter().filter(|(_, d)| d.contains(&uid)).collect();
    eprintln!("SOGo put it in bob's calendar itself: {:?}", in_sogo.iter().map(|x| &x.0).collect::<Vec<_>>());

    // bob's MailCraft holds them while his calendar can't be reached.
    bob.pool.submit(Job::SyncFolder { account: bid.clone(), folder: "INBOX".into() });
    bob.pump(120, "the invitation held", |s, _| !s.pim[&bid].held.is_empty());
    bob.settle(3);
    eprintln!("held: {}", bob.s.pim[&bid].held.len());
    assert!(bob.s.find_event(&uid).is_none(), "held, not on this computer");

    // The calendar answers: the meeting goes to bob's own calendar on SOGo, once.
    *gate.lock().unwrap() = Some(0);
    let sync = bob.sync_dav(&bid);
    eprintln!(
        "bob's first sync: errors {:?} pushed {:?}",
        sync.errors,
        sync.pushed_events.iter().map(|e| (&e.calendar, &e.href)).collect::<Vec<_>>()
    );
    eprintln!("bob's calendars: {:?}", sync.calendars.as_ref().map(|c| c.iter().map(|c| (c.id.clone(), c.foreign)).collect::<Vec<_>>()));
    bob.settle(3);
    let sync = bob.sync_dav(&bid);
    eprintln!("bob's next sync: errors {:?}", sync.errors);
    for x in bob.s.pim[&bid].events.iter().filter(|x| x.event.uid == uid) {
        eprintln!(
            "  bob has {} {} dirty {} partstat {:?}",
            x.calendar,
            x.href,
            x.dirty,
            x.event.attendees.iter().map(|a| a.partstat.clone()).collect::<Vec<_>>()
        );
    }
    let (_, e) = bob.s.find_event(&uid).expect("the meeting in bob's calendar");
    eprintln!("bob's copy: {} {} dirty {}", e.calendar, e.href, e.dirty);
    assert!(e.calendar.ends_with(&format!("/{bob_email}/Calendar/personal/")), "{}", e.calendar);
    assert!(bob.s.pim[&bid].held.is_empty());
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert!(!e.dirty && !e.href.is_empty(), "stored: {e:?}");
    let copies = bob.s.pim[&bid].events.iter().filter(|x| x.event.uid == uid).count();
    assert_eq!(copies, 1, "one meeting, not one per invitation");
    let in_sogo: Vec<(String, String)> =
        m.calendar(1, &format!("{}personal/", m.dav_home(1))).into_iter().filter(|(_, d)| d.contains(&uid)).collect();
    assert_eq!(in_sogo.len(), 1, "{in_sogo:?}");
    assert!(in_sogo[0].1.contains("CONFERENCE") && in_sogo[0].1.contains("meet.example.edu/rooms/mc3-1in-vit"), "{}", in_sogo[0].1);

    // Answered in SOGo's message (SOGo stamps the copy it keeps later than the mail): accepted,
    // out of the Inbox to Trash on Mailcow, with MailCraft's own invitation for it.
    let key = bob
        .s
        .folder_state(&bid, "INBOX")
        .messages
        .iter()
        .find(|x| x.subject.contains(&first) && x.subject.starts_with("Event Invitation"))
        .map(|x| x.key.to_id())
        .expect("SOGo's invitation");
    bob.s.run("folder.open", &json!({"account": bid, "folder": "INBOX"})).unwrap();
    bob.s.run("list.open", &json!({"key": key})).unwrap();
    let r = bob.s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    eprintln!("bob accepts: {r}");
    bob.pump(120, "bob's answer stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));
    bob.settle(3);
    let trash = wait_subjects(&m, 1, "Trash", "the answered invitation in Trash", |s| s.iter().any(|x| x.contains(&first)));
    let inbox = m.imap_as(1).subjects("INBOX");
    eprintln!("bob's Inbox {inbox:?}, Trash {trash:?}");
    assert!(!inbox.iter().any(|x| x.contains(&first)), "every invitation for it left the Inbox: {inbox:?}");
    let bobs = m.calendar(1, &format!("{}personal/", m.dav_home(1))).into_iter().find(|(_, d)| d.contains(&uid)).map(|x| x.1).unwrap_or_default();
    assert!(bobs.contains("PARTSTAT=ACCEPTED"), "{bobs}");

    // ann's MailCraft takes bob's answer in.
    let replies = wait_subjects(&m, 0, "INBOX", "bob's answer", |s| s.iter().any(|x| x.contains(&first)));
    eprintln!("ann's Inbox: {replies:?}");
    ann.pool.submit(Job::SyncFolder { account: aid.clone(), folder: "INBOX".into() });
    ann.settle(4);
    let key = ann.s.folder_state(&aid, "INBOX").messages.iter().find(|x| x.subject.contains(&first)).map(|x| x.key.to_id()).expect("bob's answer");
    ann.s.run("folder.open", &json!({"account": aid, "folder": "INBOX"})).unwrap();
    ann.s.run("list.open", &json!({"key": key})).unwrap();
    let r = ann.s.run("invite.apply", &json!({}));
    eprintln!("ann applies: {r:?}");
    let (_, e) = ann.s.find_event(&uid).unwrap();
    let bob_said = e.event.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(bob_email)).map(|a| a.partstat.clone()).unwrap_or_default();
    eprintln!("ann's event says bob: {bob_said}");
    assert_eq!(bob_said, "ACCEPTED");
    let anns = m.calendar(0, &format!("{}personal/", m.dav_home(0))).into_iter().find(|(_, d)| d.contains(&uid)).map(|x| x.1).unwrap_or_default();
    eprintln!("--- ann's copy on SOGo:\n{anns}");
    assert!(anns.contains("PARTSTAT=ACCEPTED;CN=Bob") || anns.contains("PARTSTAT=ACCEPTED:mailto:bob"), "SOGo has bob's answer: {anns}");
    // SOGo wrote bob's answer into ann's copy itself: her change, made to the copy before, is
    // refused once (412) and SOGo's kept, not sent again at every sync.
    ann.sync_dav(&aid);
    let again = ann.sync_dav(&aid);
    assert!(again.errors.is_empty(), "{:?}", again.errors);
    assert!(ann.s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));

    // A second meeting, answered from bob's calendar: tentative, its invitations leave the Inbox.
    let second = format!("{MARK} {tag} two");
    let uid2 =
        ann.s.run("event.create", &json!({"account": aid, "summary": second, "start": "2026-11-19T10:00", "attendees": bob_email})).unwrap()["uid"]
            .as_str()
            .unwrap()
            .to_string();
    ann.pump(120, "the second meeting stored", |s, _| s.find_event(&uid2).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    wait_subjects(&m, 1, "INBOX", "the second invitation", |s| s.iter().any(|x| x.contains(&second)));
    std::thread::sleep(Duration::from_secs(5));
    bob.pool.submit(Job::SyncFolder { account: bid.clone(), folder: "INBOX".into() });
    bob.pump(120, "the second meeting in bob's calendar", |s, _| s.find_event(&uid2).is_some());
    bob.settle(3);
    bob.sync_dav(&bid);
    let r = bob.s.run("event.respond", &json!({"uid": uid2, "answer": "tentative"})).unwrap();
    eprintln!("bob answers from the calendar: {r}");
    bob.pump(120, "the second answer stored", |s, _| s.find_event(&uid2).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    bob.settle(3);
    let trash = wait_subjects(&m, 1, "Trash", "the second invitation in Trash", |s| s.iter().any(|x| x.contains(&second)));
    let inbox = m.imap_as(1).subjects("INBOX");
    eprintln!("bob's Inbox {inbox:?}, Trash {trash:?}");
    assert!(!inbox.iter().any(|x| x.contains(&second)), "{inbox:?}");
    let in_sogo: Vec<String> =
        m.calendar(1, &format!("{}personal/", m.dav_home(1))).into_iter().filter(|(_, d)| d.contains(&uid2)).map(|x| x.1).collect();
    assert_eq!(in_sogo.len(), 1, "{in_sogo:?}");
    assert!(in_sogo[0].contains("PARTSTAT=TENTATIVE"), "{}", in_sogo[0]);
    let _ = ann_email;
    clean(&m);
}
