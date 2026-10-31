//! Answers to meetings crossing on a real Mailcow on this computer, where SOGo writes them into
//! the calendars itself: an answer given on another device isn't undone by the invitation's mail
//! taken in later, and an organizer's change made while a guest answered keeps both. Never a
//! real account; run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_MAILCOW="127.0.0.1:22143 127.0.0.1:22588 http://127.0.0.1:22080 \
//!   ann@mc31.test:<password> bob@mc31.test:<password> carol@mc31.test:<password>" \
//!   cargo test -p mailcraft-cli --test real_mailcow_answers -- --ignored --nocapture --test-threads 1
//! ```
//!
//! See `real_mailcow_mail.rs` for the Mailcow it needs.

#[path = "support/mailcow.rs"]
mod mailcow;

use std::time::{Duration, Instant};

use mailcow::{App, Mailcow};
use mailcraft_engine::model::{Event, Job, SyncStatus};
use serde_json::json;

const MARK: &str = "Mailcow answers";

fn online(app: &mut App) {
    app.pump(120, "online", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
}

/// A meeting ann's MailCraft organizes with bob, stored in her SOGo calendar; its uid.
fn invite_bob(m: &Mailcow, ann: &mut App, aid: &str, title: &str, start: &str) -> String {
    let uid = ann.s.run("event.create", &json!({"account": aid, "summary": title, "start": start, "attendees": m.user(1).0})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    ann.pump(120, "the meeting stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    ann.settle(5);
    uid
}

/// bob's copy of a meeting in his SOGo calendar: its address and data.
fn bobs_copy(m: &Mailcow, uid: &str) -> Option<(String, String)> {
    m.calendar(1, &format!("{}personal/", m.dav_home(1))).into_iter().find(|(_, d)| d.contains(uid))
}

/// SOGo stores a new item over one of the same name, though asked not to, and keeps the meeting
/// in bob's calendar under the name MailCraft gives it by its uid. bob accepts on his phone while
/// his laptop sleeps; the laptop wakes, takes the invitation in from the mail and used to store it
/// over the calendar's copy: back to "needs action".
#[test]
#[ignore]
fn real_mailcow_an_answer_from_another_device_stays() {
    let Some(m) = Mailcow::from_env() else { return };
    let web = format!("{}/", m.web);
    let (mut bob, bid) = App::new(&m, 1, "answers-phone");
    bob.s.run("account.dav", &json!({"account": bid, "caldav": web})).unwrap();
    online(&mut bob);
    bob.sync_dav(&bid);
    bob.settle(3);
    // bob's MailCraft sleeps (nothing taken in) from here.
    let (mut ann, aid) = App::new(&m, 0, "answers-phone");
    ann.s.run("account.dav", &json!({"account": aid, "caldav": web})).unwrap();
    ann.sync_dav(&aid);
    let title = format!("{MARK} {} phone", std::process::id());
    let uid = invite_bob(&m, &mut ann, &aid, &title, "2026-12-21T10:00");
    let (href, data) = bobs_copy(&m, &uid).expect("SOGo's copy in bob's calendar");
    let (u, p) = m.user(1);
    let auth = format!("Basic {}", mailcow::base64(format!("{u}:{p}").as_bytes()));
    let accepted = data.replace("PARTSTAT=NEEDS-ACTION", "PARTSTAT=ACCEPTED");
    let headers = [("Authorization", auth.as_str()), ("Content-Type", "text/calendar; charset=utf-8")];
    let st = mailcow::http("PUT", &format!("{}{href}", m.web), &headers, accepted.as_bytes()).status;
    assert!((200..300).contains(&st), "accepted on the phone: {st}");

    // The laptop wakes: the mail, and the calendar.
    let end = Instant::now() + Duration::from_secs(120);
    while bob.s.find_event(&uid).is_none_or(|(_, e)| e.dirty) {
        assert!(Instant::now() < end, "the meeting didn't settle in bob's MailCraft");
        bob.pool.submit(Job::SyncFolder { account: bid.clone(), folder: "INBOX".into() });
        bob.settle(3);
    }
    bob.sync_dav(&bid);
    let (_, now) = bobs_copy(&m, &uid).expect("still there");
    eprintln!("bob's copy on SOGo:\n{now}");
    assert!(now.contains("PARTSTAT=ACCEPTED") && !now.contains("PARTSTAT=NEEDS-ACTION"), "the answer given on the phone stays: {now}");
    let local = bob.s.find_event(&uid).map(|(_, e)| e.event.attendees.iter().map(|a| a.partstat.clone()).collect::<Vec<_>>());
    assert_eq!(local, Some(vec!["ACCEPTED".to_string()]));
    assert_eq!(m.calendar(1, &format!("{}personal/", m.dav_home(1))).iter().filter(|(_, d)| d.contains(&uid)).count(), 1);
    for n in [0, 1] {
        for (h, d) in m.calendar(n, &format!("{}personal/", m.dav_home(n))) {
            if d.contains(MARK) {
                m.dav(n, "DELETE", &format!("{}{h}", m.web), None, "");
            }
        }
    }
}

/// SOGo writes bob's answer into ann's copy itself. ann's MailCraft takes the same answer in from
/// his mail and, before its calendar syncs, she writes the agenda: the change was refused (412)
/// and SOGo's copy kept, the agenda gone without a word. Both are on SOGo now.
#[test]
#[ignore]
fn real_mailcow_an_agenda_written_as_a_guest_answers_stays() {
    let Some(m) = Mailcow::from_env() else { return };
    let web = format!("{}/", m.web);
    let (mut ann, aid) = App::new(&m, 0, "answers-agenda");
    ann.s.run("account.dav", &json!({"account": aid, "caldav": web})).unwrap();
    online(&mut ann);
    ann.sync_dav(&aid);
    let title = format!("{MARK} {} agenda", std::process::id());
    let uid = invite_bob(&m, &mut ann, &aid, &title, "2026-12-22T10:00");
    let (mut bob, bid) = App::new(&m, 1, "answers-agenda");
    bob.s.run("account.dav", &json!({"account": bid, "caldav": web})).unwrap();
    bob.sync_dav(&bid);
    bob.s.run("event.respond", &json!({"uid": uid, "answer": "accepted"})).unwrap();
    bob.pump(120, "bob's answer stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));
    bob.settle(5);

    let end = Instant::now() + Duration::from_secs(120);
    let key = loop {
        ann.pool.submit(Job::SyncFolder { account: aid.clone(), folder: "INBOX".into() });
        ann.settle(3);
        let reply = ann.s.folder_state(&aid, "INBOX").messages.iter().find(|x| x.subject.contains(&title) && x.subject.starts_with("Accepted:"));
        if let Some(k) = reply.map(|x| x.key.to_id()) {
            break k;
        }
        assert!(Instant::now() < end, "bob's answer didn't come");
    };
    ann.s.run("folder.open", &json!({"account": aid, "folder": "INBOX"})).unwrap();
    ann.s.run("list.open", &json!({"key": key})).unwrap();
    ann.s.run("invite.apply", &json!({})).unwrap();
    ann.s.run("event.update", &json!({"uid": uid, "description": "Agenda: budget"})).unwrap();
    let t0 = mailcow::now();
    let mut errors = Vec::new();
    ann.pump(120, "a sync after the change", |s, e| match e {
        Event::DavSynced { sync, .. } => {
            errors.extend(sync.errors.clone());
            sync.checked >= t0 && s.find_event(&uid).is_some_and(|(_, e)| !e.dirty)
        }
        _ => false,
    });
    ann.settle(3);
    assert!(errors.is_empty(), "{errors:?}");
    let (_, local) = ann.s.find_event(&uid).unwrap();
    assert_eq!(local.event.description, "Agenda: budget");
    let (_, stored) = m.dav(0, "GET", &local.href.clone(), None, "");
    eprintln!("ann's copy on SOGo:\n{stored}");
    assert!(stored.contains("Agenda: budget") && stored.contains("PARTSTAT=ACCEPTED"), "both: {stored}");
    for n in [0, 1] {
        for (h, d) in m.calendar(n, &format!("{}personal/", m.dav_home(n))) {
            if d.contains(MARK) {
                m.dav(n, "DELETE", &format!("{}{h}", m.web), None, "");
            }
        }
    }
}
