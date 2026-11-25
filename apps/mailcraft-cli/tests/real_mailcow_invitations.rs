//! Meeting invitations between two users of a real Mailcow on this computer, both in MailCraft:
//! ann invites bob (with an online meeting, issue #25) through Postfix; SOGo, which schedules
//! for its own users itself, puts the meeting in bob's calendar, and leaves the mail to
//! MailCraft (see `real_scheduling.rs`). bob's calendar
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

/// 10:00 (UTC) `days` from now: the meetings are to come, as SOGo mails no answer to a meeting
/// that's over (they were on fixed days, gone by, and bob's answer never came).
fn ahead(days: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let (y, m, d) = mailcraft_engine::model::dates::civil_from_days(now / 86_400 + days);
    format!("{y:04}-{m:02}-{d:02}T10:00")
}

/// Take out what an earlier run left in ann's and bob's calendars and Inboxes.
fn clean(m: &Mailcow) {
    m.clean_calendars(&[0, 1], MARK);
    for n in [0, 1] {
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
#[ignore = "needs a Mailcow on this computer in MAILCRAFT_REAL_MAILCOW"]
fn real_mailcow_invitations_between_two_users() {
    let Some(m) = Mailcow::from_env() else { return };
    clean(&m);
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
        .run(
            "event.create",
            &json!({"account": aid, "summary": first, "start": ahead(30), "attendees": format!("Bob Tester <{bob_email}>"), "onlineMeeting": "BBB"}),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    ann.pump(120, "ann's meeting stored and sent", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    ann.settle(3);
    let (_, stored) = ann.s.find_event(&uid).unwrap();
    eprintln!("ann's meeting is at {}", stored.href);

    // What reaches bob: MailCraft's invitation only (SOGo is told MailCraft sends them).
    let inbox = wait_subjects(&m, 1, "INBOX", "the invitations", |s| s.iter().any(|x| x.contains(&first)));
    std::thread::sleep(Duration::from_secs(5));
    let inbox = {
        let again = m.imap_as(1).subjects("INBOX");
        if again.len() > inbox.len() { again } else { inbox }
    };
    eprintln!("bob's Inbox: {inbox:?}");
    assert_eq!(inbox.iter().filter(|x| x.contains(&first)).count(), 1, "one invitation: {inbox:?}");
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
    // Everything with ann (issue #178): the meeting she organises, from bob's calendar on SOGo.
    let with_ann = bob.s.person_activity(m.user(0).0);
    assert!(with_ann.meetings.iter().any(|x| x.uid == uid && x.theirs), "{:?}", with_ann.meetings);
    let in_sogo: Vec<(String, String)> =
        m.calendar(1, &format!("{}personal/", m.dav_home(1))).into_iter().filter(|(_, d)| d.contains(&uid)).collect();
    assert_eq!(in_sogo.len(), 1, "{in_sogo:?}");
    assert!(in_sogo[0].1.contains("CONFERENCE") && in_sogo[0].1.contains("meet.example.edu/rooms/mc3-1in-vit"), "{}", in_sogo[0].1);

    // Answered in the message (SOGo stamps the copy it keeps later than the mail): accepted, out
    // of the Inbox to Trash on Mailcow.
    let key = bob
        .s
        .folder_state(&bid, "INBOX")
        .messages
        .iter()
        .find(|x| x.subject.contains(&first) && x.subject.starts_with("Invitation:"))
        .map(|x| x.key.to_id())
        .expect("MailCraft's invitation");
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

    // ann's MailCraft takes bob's answer in: SOGo's, as bob answered in the copy SOGo put in his
    // calendar (its subject's words encoded one by one).
    let end = Instant::now() + Duration::from_secs(60);
    let key = loop {
        ann.pool.submit(Job::SyncFolder { account: aid.clone(), folder: "INBOX".into() });
        ann.settle(3);
        let replies: Vec<_> = ann
            .s
            .folder_state(&aid, "INBOX")
            .messages
            .iter()
            .filter(|x| x.subject.contains(&first))
            .map(|x| (x.key.to_id(), x.subject.clone()))
            .collect();
        if let Some((k, _)) = replies.first() {
            std::thread::sleep(Duration::from_secs(5));
            assert_eq!(replies.len(), 1, "one answer: {replies:?}");
            break k.clone();
        }
        assert!(Instant::now() < end, "bob's answer didn't come");
    };
    ann.s.run("folder.open", &json!({"account": aid, "folder": "INBOX"})).unwrap();
    ann.s.run("list.open", &json!({"key": key})).unwrap();
    let r = ann.s.run("invite.apply", &json!({}));
    eprintln!("ann applies: {r:?}");
    let (_, e) = ann.s.find_event(&uid).unwrap();
    let bob_said = e.event.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(bob_email)).map(|a| a.partstat.clone()).unwrap_or_default();
    eprintln!("ann's event says bob: {bob_said}");
    assert_eq!(bob_said, "ACCEPTED");
    // Everything with bob, on ann's side: her meeting he's invited to.
    let with_bob = ann.s.person_activity(bob_email);
    assert!(with_bob.meetings.iter().any(|x| x.uid == uid && !x.theirs), "{:?}", with_bob.meetings);
    let anns = m.calendar(0, &format!("{}personal/", m.dav_home(0))).into_iter().find(|(_, d)| d.contains(&uid)).map(|x| x.1).unwrap_or_default();
    eprintln!("--- ann's copy on SOGo:\n{anns}");
    let bobs_line = anns.lines().find(|l| l.starts_with("ATTENDEE") && l.contains(&format!("mailto:{bob_email}"))).unwrap_or_default();
    assert!(bobs_line.contains("PARTSTAT=ACCEPTED"), "SOGo has bob's answer: {anns}");
    // SOGo wrote bob's answer into ann's copy itself: her change, made to the copy before, is
    // refused once (412) and SOGo's kept, not sent again at every sync.
    ann.sync_dav(&aid);
    let again = ann.sync_dav(&aid);
    assert!(again.errors.is_empty(), "{:?}", again.errors);
    assert!(ann.s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));

    // A second meeting, answered from bob's calendar: tentative, its invitations leave the Inbox.
    let second = format!("{MARK} {tag} two");
    let uid2 = ann.s.run("event.create", &json!({"account": aid, "summary": second, "start": ahead(31), "attendees": bob_email})).unwrap()["uid"]
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
    clean(&m);
}

/// The SOGo copy of `uid` in user `n`'s calendar, unfolded (empty: none).
fn sogo_copy(m: &Mailcow, n: usize, uid: &str) -> String {
    let copy = m.calendar(n, &format!("{}personal/", m.dav_home(n))).into_iter().find(|(_, d)| d.contains(uid)).map(|x| x.1).unwrap_or_default();
    copy.replace("\r\n ", "").replace("\n ", "")
}

/// A user's `ATTENDEE` line for `email` in a copy.
fn guest_line<'a>(copy: &'a str, email: &str) -> &'a str {
    copy.lines()
        .find(|l| l.starts_with("ATTENDEE") && l.to_ascii_lowercase().ends_with(&format!(":mailto:{}", email.to_ascii_lowercase())))
        .unwrap_or_default()
}

/// Issue #254: ann's meeting, which SOGo put in bob's calendar, delegated by bob to carol in
/// MailCraft: carol gets one invitation, as bob's delegate; ann's copy (SOGo's and MailCraft's)
/// has bob as delegated to carol, and carol, then carol's answer.
#[test]
#[ignore = "needs a Mailcow on this computer in MAILCRAFT_REAL_MAILCOW"]
fn real_mailcow_meeting_delegated_to_a_colleague() {
    let Some(m) = Mailcow::from_env() else { return };
    m.clean_calendars(&[0, 1, 2], MARK);
    for n in [0, 1, 2] {
        let mut imap = m.imap_as(n);
        for folder in ["INBOX", "Trash"] {
            imap.cmd(&format!("SELECT {folder}"));
            imap.cmd("UID STORE 1:* +FLAGS.SILENT (\\Deleted)");
            imap.cmd("EXPUNGE");
        }
    }
    let (bob_email, _) = m.user(1);
    let (carol_email, _) = m.user(2);
    let web = format!("{}/", m.web);
    let tag = std::process::id();
    let title = format!("{MARK} {tag} delegated");

    // ann invites bob; SOGo puts it in bob's calendar.
    let (mut ann, aid) = App::new(&m, 0, "delegate");
    ann.s.run("account.dav", &json!({"account": aid, "caldav": web})).unwrap();
    ann.sync_dav(&aid);
    let uid = ann
        .s
        .run("event.create", &json!({"account": aid, "summary": title, "start": ahead(32), "attendees": format!("Bob Tester <{bob_email}>")}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    ann.pump(120, "ann's meeting stored and sent", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    wait_subjects(&m, 1, "INBOX", "the invitation", |s| s.iter().any(|x| x.contains(&title)));

    // bob's MailCraft: the meeting in his calendar, delegated to carol.
    let (mut bob, bid) = App::new(&m, 1, "delegate");
    bob.s.run("account.dav", &json!({"account": bid, "caldav": web})).unwrap();
    bob.pump(120, "bob online", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    bob.sync_dav(&bid);
    bob.pool.submit(Job::SyncFolder { account: bid.clone(), folder: "INBOX".into() });
    bob.pump(120, "the meeting in bob's calendar", |s, _| s.find_event(&uid).is_some());
    bob.settle(3);
    let r = bob.s.run("invite.delegate", &json!({"uid": uid, "to": format!("Carol Tester <{carol_email}>")})).unwrap();
    eprintln!("bob delegates: {r}");
    bob.pump(120, "bob's copy stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));
    bob.settle(5);
    let bobs = sogo_copy(&m, 1, &uid);
    eprintln!(
        "--- bob's copy on SOGo:
{bobs}"
    );

    // carol: one invitation, as bob's delegate.
    wait_subjects(&m, 2, "INBOX", "carol's invitation", |s| s.iter().any(|x| x.contains(&title)));
    std::thread::sleep(Duration::from_secs(10));
    let inbox = m.imap_as(2).subjects("INBOX");
    eprintln!("carol's Inbox: {inbox:?}");
    eprintln!(
        "--- carol's copy on SOGo:
{}",
        sogo_copy(&m, 2, &uid)
    );
    let anns_inbox = m.imap_as(0).subjects("INBOX");
    eprintln!("ann's Inbox: {anns_inbox:?}");
    let anns = sogo_copy(&m, 0, &uid);
    eprintln!(
        "--- ann's copy on SOGo:
{anns}"
    );
    assert_eq!(inbox.iter().filter(|x| x.contains(&title)).count(), 1, "one invitation: {inbox:?}");
    assert!(guest_line(&anns, bob_email).contains("PARTSTAT=DELEGATED"), "{anns}");
    assert!(
        guest_line(&anns, carol_email).to_ascii_lowercase().contains(&format!("delegated-from=\"mailto:{}\"", bob_email.to_ascii_lowercase())),
        "{anns}"
    );

    // carol's MailCraft: bob's place; she accepts.
    let (mut carol, cid) = App::new(&m, 2, "delegate");
    carol.s.run("account.dav", &json!({"account": cid, "caldav": web})).unwrap();
    carol.pump(120, "carol online", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    carol.sync_dav(&cid);
    carol.pool.submit(Job::SyncFolder { account: cid.clone(), folder: "INBOX".into() });
    carol.pump(120, "carol's invitation", |s, _| s.folder_state(&cid, "INBOX").messages.iter().any(|x| x.subject.contains(&title)));
    carol.settle(3);
    let key = carol.s.folder_state(&cid, "INBOX").messages.iter().find(|x| x.subject.contains(&title)).map(|x| x.key.to_id()).unwrap();
    carol.s.run("folder.open", &json!({"account": cid, "folder": "INBOX"})).unwrap();
    carol.s.run("list.open", &json!({"key": key})).unwrap();
    let inv = carol.s.invite().expect("an invitation");
    eprintln!("carol's card: handed by {:?}, answer {}, problem {:?}", inv.handed_by, inv.answer, inv.problem);
    assert!(inv.handed_by.is_some(), "{inv:?}");
    carol.s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    carol.pump(120, "carol's answer stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));
    carol.settle(5);

    // ann: bob delegated to carol, carol accepted, in SOGo's copy and in MailCraft's.
    let end = Instant::now() + Duration::from_secs(90);
    loop {
        let anns = sogo_copy(&m, 0, &uid);
        if guest_line(&anns, carol_email).contains("PARTSTAT=ACCEPTED") {
            eprintln!(
                "--- ann's copy on SOGo at last:
{anns}"
            );
            break;
        }
        assert!(Instant::now() < end, "carol's answer didn't reach ann's copy: {anns}");
        std::thread::sleep(Duration::from_secs(2));
    }
    let anns_inbox = m.imap_as(0).subjects("INBOX");
    eprintln!("ann's Inbox: {anns_inbox:?}");
    ann.pool.submit(Job::SyncFolder { account: aid.clone(), folder: "INBOX".into() });
    ann.settle(5);
    ann.sync_dav(&aid);
    ann.settle(3);
    let (_, e) = ann.s.find_event(&uid).unwrap();
    let said = |email: &str| {
        e.event
            .attendees
            .iter()
            .find(|a| a.email.eq_ignore_ascii_case(email))
            .map(|a| (a.partstat.clone(), a.delegated_to.clone(), a.delegated_from.clone()))
    };
    eprintln!("ann's MailCraft: bob {:?}, carol {:?}", said(bob_email), said(carol_email));
    assert_eq!(said(bob_email).map(|x| x.0), Some("DELEGATED".to_string()));
    assert_eq!(said(carol_email).map(|x| x.0), Some("ACCEPTED".to_string()));
    m.clean_calendars(&[0, 1, 2], MARK);
}

/// Issue #254, the other way: bob delegates ann's meeting to carol in SOGo's web mail (its own
/// call, `rsvpAppointment` with a delegate); carol's MailCraft offers her bob's place to answer,
/// and ann's MailCraft has bob as delegated to carol, then carol's answer.
#[test]
#[ignore = "needs a Mailcow on this computer in MAILCRAFT_REAL_MAILCOW"]
fn real_mailcow_meeting_delegated_in_sogo() {
    let Some(m) = Mailcow::from_env() else { return };
    m.clean_calendars(&[0, 1, 2], MARK);
    let (bob_email, _) = m.user(1);
    let (carol_email, _) = m.user(2);
    let web = format!("{}/", m.web);
    let tag = std::process::id();
    let title = format!("{MARK} {tag} delegated in SOGo");

    let (mut ann, aid) = App::new(&m, 0, "sogo-delegate");
    ann.s.run("account.dav", &json!({"account": aid, "caldav": web})).unwrap();
    ann.sync_dav(&aid);
    let uid = ann
        .s
        .run("event.create", &json!({"account": aid, "summary": title, "start": ahead(33), "attendees": format!("Bob Tester <{bob_email}>")}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    ann.pump(120, "ann's meeting stored and sent", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));

    // bob, in SOGo: his copy's place handed to carol.
    let end = Instant::now() + Duration::from_secs(60);
    let href = loop {
        if let Some((href, _)) = m.calendar(1, &format!("{}personal/", m.dav_home(1))).into_iter().find(|(_, d)| d.contains(&uid)) {
            break href;
        }
        assert!(Instant::now() < end, "the meeting never reached bob's calendar");
        std::thread::sleep(Duration::from_secs(1));
    };
    let name = href.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string();
    let sogo = m.sogo(1);
    let r =
        sogo.post(&format!("Calendar/personal/{name}/rsvpAppointment"), &json!({"reply": 4, "delegatedTo": carol_email, "receiveUpdates": false}));
    eprintln!("SOGo delegates: {} {}", r.status, r.text());
    std::thread::sleep(Duration::from_secs(10));
    eprintln!(
        "--- bob's copy on SOGo:
{}",
        sogo_copy(&m, 1, &uid)
    );
    eprintln!(
        "--- ann's copy on SOGo:
{}",
        sogo_copy(&m, 0, &uid)
    );
    eprintln!(
        "--- carol's copy on SOGo:
{}",
        sogo_copy(&m, 2, &uid)
    );
    eprintln!("ann's Inbox: {:?}", m.imap_as(0).subjects("INBOX"));
    eprintln!("carol's Inbox: {:?}", m.imap_as(2).subjects("INBOX"));

    // carol's MailCraft: bob's place to answer.
    let (mut carol, cid) = App::new(&m, 2, "sogo-delegate");
    carol.s.run("account.dav", &json!({"account": cid, "caldav": web})).unwrap();
    carol.pump(120, "carol online", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    carol.sync_dav(&cid);
    let (_, copy) = carol.s.find_event(&uid).expect("in carol's calendar");
    let me = copy.event.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(carol_email)).cloned().unwrap_or_default();
    eprintln!("carol's copy: {me:?}");
    assert!(me.delegated_from.eq_ignore_ascii_case(bob_email), "{:?}", copy.event.attendees);
    carol.s.run("event.respond", &json!({"uid": uid, "answer": "accepted"})).unwrap();
    carol.pump(120, "carol's answer stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));

    // ann: bob delegated to carol, carol accepted.
    let end = Instant::now() + Duration::from_secs(90);
    loop {
        ann.pool.submit(Job::SyncFolder { account: aid.clone(), folder: "INBOX".into() });
        ann.settle(3);
        ann.sync_dav(&aid);
        let (_, e) = ann.s.find_event(&uid).unwrap();
        let said = |email: &str| {
            e.event.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(email)).map(|a| (a.partstat.clone(), a.delegated_to.clone()))
        };
        eprintln!("ann's MailCraft: bob {:?}, carol {:?}", said(bob_email), said(carol_email));
        if said(carol_email).is_some_and(|x| x.0 == "ACCEPTED") {
            assert_eq!(said(bob_email), Some(("DELEGATED".to_string(), carol_email.to_string())));
            break;
        }
        assert!(Instant::now() < end, "carol's answer never reached ann");
        std::thread::sleep(Duration::from_secs(3));
    }
    m.clean_calendars(&[0, 1, 2], MARK);
}

/// Issue #235: ann's MailCraft in German. Her meeting goes to bob as "Einladung:"; deleted with
/// a message, its cancellation waits for SOGo to have the meeting gone, then reaches bob as
/// "Abgesagt:", in German, with her words.
#[test]
#[ignore = "needs a Mailcow on this computer in MAILCRAFT_REAL_MAILCOW"]
fn real_mailcow_cancellation_in_the_windows_language() {
    let Some(m) = Mailcow::from_env() else { return };
    clean(&m);
    let (bob_email, _) = m.user(1);
    let web = format!("{}/", m.web);
    let title = format!("{MARK} {} abgesagt", std::process::id());

    let (mut ann, aid) = App::new(&m, 0, "cancel-words");
    ann.s.mail_words = mailcraft_engine::MailWords {
        invitation: "Einladung: {summary}".into(),
        cancelled: "Abgesagt: {summary}".into(),
        cancelled_body: "{summary} wurde abgesagt.".into(),
        when: "Wann: {when}".into(),
        ..Default::default()
    };
    ann.s.run("account.dav", &json!({"account": aid, "caldav": web})).unwrap();
    ann.sync_dav(&aid);
    let uid = ann
        .s
        .run("event.create", &json!({"account": aid, "summary": title, "start": ahead(33), "attendees": format!("Bob Tester <{bob_email}>")}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    ann.pump(120, "ann's meeting stored and sent", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    wait_subjects(&m, 1, "INBOX", "the invitation", |s| s.iter().any(|x| *x == format!("Einladung: {title}")));

    ann.s.run("event.delete", &json!({"uid": uid, "message": "Raum 4 ist belegt, bitte naechste Woche."})).unwrap();
    ann.pump(120, "the meeting gone from SOGo", |s, _| s.pim.get(&aid).is_none_or(|p| p.notices.iter().all(|n| n.uid != uid)));
    wait_subjects(&m, 1, "INBOX", "the cancellation", |s| s.iter().any(|x| *x == format!("Abgesagt: {title}")));
    let text = m.imap_as(1).newest_text("INBOX");
    eprintln!("--- the cancellation bob has:\n{text}");
    assert!(text.contains("wurde abgesagt.") && text.contains("Raum 4 ist belegt, bitte naechste Woche."), "{text}");
    clean(&m);
}
