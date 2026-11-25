//! A series on a real Mailcow on this computer, its organizer a SOGo user (issue #101): the guest,
//! in MailCraft, declines one occurrence the organizer moved. Never a real account; run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_MAILCOW="127.0.0.1:22143 127.0.0.1:22588 http://127.0.0.1:22080 \
//!   ann@mc31.test:<password> bob@mc31.test:<password> carol@mc31.test:<password>" \
//!   cargo test -p mailcraft-cli --test real_mailcow_series -- --ignored --nocapture --test-threads 1
//! ```
//!
//! See `real_mailcow_mail.rs` for the Mailcow it needs.

#[path = "support/mailcow.rs"]
mod mailcow;

use mailcow::{App, Mailcow};
use mailcraft_engine::model::ical::{self, CalEvent};
use mailcraft_engine::model::{Event, SyncStatus};
use serde_json::json;

const MARK: &str = "Mailcow series";

fn online(app: &mut App) {
    app.pump(120, "online", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
}

/// The parts of a meeting in a user's SOGo calendar: its item's address, and its events.
fn on_sogo(m: &Mailcow, n: usize, uid: &str) -> Option<(String, Vec<CalEvent>)> {
    let (href, data) = m.calendar(n, &format!("{}personal/", m.dav_home(n))).into_iter().find(|(_, d)| d.contains(uid))?;
    Some((href, ical::parse(&data, 0).events))
}

fn answer_of(e: &CalEvent, who: &str) -> String {
    e.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(who)).map(|a| a.partstat.clone()).unwrap_or_default()
}

/// bob (a SOGo user, in MailCraft) invites ann to a daily series; ann accepts in MailCraft. bob
/// moves the third occurrence, and his calendar and mail tell ann. ann declines that occurrence
/// in the update: SOGo takes it (it refused the occurrence taken out, HTTP 403), bob's copy
/// says she declined it, the rest stays accepted, and the update leaves her Inbox only then.
#[test]
#[ignore = "needs a Mailcow on this computer in MAILCRAFT_REAL_MAILCOW"]
fn real_mailcow_one_occurrence_declined_as_a_guest() {
    let Some(m) = Mailcow::from_env() else { return };
    let mark = format!("{MARK} decline");
    m.clean_calendars(&[0, 1], &mark);
    let web = format!("{}/", m.web);
    let (ann_email, bob_email) = (m.user(0).0.to_string(), m.user(1).0.to_string());
    let title = format!("{mark} {}", std::process::id());

    let (mut bob, bid) = App::new(&m, 1, "series-bob");
    bob.s.run("account.dav", &json!({"account": bid, "caldav": web})).unwrap();
    online(&mut bob);
    bob.sync_dav(&bid);
    let uid = bob
        .s
        .run(
            "event.create",
            &json!({"account": bid, "summary": title, "start": "2026-12-14T09:00", "repeat": "FREQ=DAILY;COUNT=4", "attendees": ann_email}),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    bob.pump(120, "the series stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    bob.settle(5);

    let (mut ann, aid) = App::new(&m, 0, "series-ann");
    ann.s.run("account.dav", &json!({"account": aid, "caldav": web})).unwrap();
    online(&mut ann);
    ann.sync_dav(&aid);
    ann.s.run("options.answeredInvites", &json!({"after": "delete"})).unwrap();
    let invitation = ann.wait_for_mail(&aid, &format!("Invitation: {title}"));
    ann.s.run("folder.open", &json!({"account": aid, "folder": "INBOX"})).unwrap();
    ann.s.run("list.open", &json!({"key": invitation.to_id()})).unwrap();
    ann.s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    ann.pump(120, "ann's answer stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));
    ann.settle(5);

    // bob moves the third occurrence an hour later, in his calendar on SOGo, and mails ann the
    // update as his calendar program would.
    let (href, mut parts) = on_sogo(&m, 1, &uid).expect("bob's series on SOGo");
    let master = parts.iter().find(|e| e.recurrence_id.is_none()).cloned().unwrap();
    let third = ical::starts(&master, i64::MAX).get(2).copied().unwrap();
    let moved = CalEvent {
        recurrence_id: Some(third),
        start: third + 3600,
        end: third + 7200,
        rrule: None,
        exdates: vec![],
        sequence: master.sequence + 1,
        ..master.clone()
    };
    parts.push(moved.clone());
    let (st, _) = m.dav(1, "PUT", &format!("{}{href}", m.web), None, &ical::write_for_server(&parts, mailcow::now()));
    assert!((200..300).contains(&st), "bob's change on SOGo: {st}");
    // As SOGo stored it (it raises the occurrence's version): what its mail would carry.
    let (_, stored) = on_sogo(&m, 1, &uid).unwrap();
    let moved = stored.into_iter().find(|e| e.recurrence_id == Some(third)).expect("bob's moved occurrence on SOGo");
    let update = format!(
        "From: <{bob_email}>\r\nTo: <{ann_email}>\r\nSubject: Updated: {title}\r\nMessage-ID: <{uid}.update@mc31.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/calendar; charset=utf-8; method=REQUEST\r\n\r\n{}",
        ical::write(std::slice::from_ref(&moved), Some("REQUEST"), mailcow::now())
    );
    m.send(1, &[&ann_email], update.as_bytes());

    // ann declines that occurrence, in the update.
    let k = ann.wait_for_mail(&aid, &format!("Updated: {title}"));
    ann.sync_dav(&aid);
    ann.s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    let r = ann.s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
    assert_eq!(r["moved"], 0, "not before SOGo has the answer: {r}");
    let mut refused = Vec::new();
    let mut errors = Vec::new();
    ann.pump(120, "ann's answer stored", |s, e| {
        if let Event::DavSynced { sync, .. } = e {
            refused.extend(sync.refused.clone());
            errors.extend(sync.errors.clone());
        }
        s.find_event(&uid).is_some_and(|_| s.pim.values().flat_map(|p| p.events.iter()).filter(|e| e.event.uid == uid).all(|e| !e.dirty))
    });
    ann.settle(5);
    assert!(refused.is_empty() && errors.is_empty(), "SOGo took it: {refused:?} {errors:?}");

    let (_, anns) = on_sogo(&m, 0, &uid).expect("ann's copy on SOGo");
    let declined = anns.iter().find(|e| e.recurrence_id == Some(third)).expect("the occurrence stays in ann's copy");
    assert_eq!(answer_of(declined, &ann_email), "DECLINED");
    let series = anns.iter().find(|e| e.recurrence_id.is_none()).unwrap();
    assert_eq!(answer_of(series, &ann_email), "ACCEPTED", "the rest stays accepted");
    assert!(series.exdates.is_empty(), "not taken out: {:?}", series.exdates);
    let (_, bobs) = on_sogo(&m, 1, &uid).expect("bob's copy");
    let theirs = bobs.iter().find(|e| e.recurrence_id == Some(third)).expect("bob's moved occurrence");
    eprintln!("bob's copy says ann: {}", answer_of(theirs, &ann_email));
    // Not in ann's calendar any more, and the update out of her Inbox.
    let shown = ann.s.agenda(third - 86_400, third + 86_400);
    assert!(!shown.iter().any(|a| a.uid == uid && a.start == third + 3600), "{shown:?}");
    assert!(ann.s.folder_state(&aid, "Trash").messages.iter().any(|x| x.subject == format!("Updated: {title}")));
    m.clean_calendars(&[0, 1], &mark);
}

/// Issue #234: ann, a guest of bob's daily series (bob a SOGo user), forwards one occurrence of it
/// to carol from MailCraft: carol gets that occurrence alone (the whole series went, every day);
/// accepted in MailCraft, her SOGo calendar has that day alone, and ann's copy is as it was.
#[test]
#[ignore = "needs a Mailcow on this computer in MAILCRAFT_REAL_MAILCOW"]
fn real_mailcow_one_occurrence_forwarded() {
    let Some(m) = Mailcow::from_env() else { return };
    let mark = format!("{MARK} forward");
    m.clean_calendars(&[0, 1, 2], &mark);
    let web = format!("{}/", m.web);
    let (ann_email, carol_email) = (m.user(0).0.to_string(), m.user(2).0.to_string());
    let title = format!("{mark} {}", std::process::id());

    let (mut bob, bid) = App::new(&m, 1, "forward-bob");
    bob.s.run("account.dav", &json!({"account": bid, "caldav": web})).unwrap();
    online(&mut bob);
    bob.sync_dav(&bid);
    let uid = bob
        .s
        .run(
            "event.create",
            &json!({"account": bid, "summary": title, "start": "2026-12-14T11:00", "repeat": "FREQ=DAILY;COUNT=5", "attendees": ann_email}),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    bob.pump(120, "the series stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    bob.settle(5);

    let (mut ann, aid) = App::new(&m, 0, "forward-ann");
    ann.s.run("account.dav", &json!({"account": aid, "caldav": web})).unwrap();
    online(&mut ann);
    ann.sync_dav(&aid);
    let invitation = ann.wait_for_mail(&aid, &format!("Invitation: {title}"));
    ann.s.run("folder.open", &json!({"account": aid, "folder": "INBOX"})).unwrap();
    ann.s.run("list.open", &json!({"key": invitation.to_id()})).unwrap();
    ann.s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    ann.pump(120, "ann's answer stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));
    ann.settle(5);
    let (_, before) = on_sogo(&m, 0, &uid).expect("ann's copy on SOGo");
    let master = before.iter().find(|e| e.recurrence_id.is_none()).cloned().unwrap();
    let third = ical::starts(&master, i64::MAX).get(2).copied().unwrap();
    let r = ann.s.run("invite.forward", &json!({"uid": uid, "to": carol_email, "occurrence": third})).unwrap();
    assert_eq!(r["forwarded"], 1, "{r}");
    ann.settle(10);
    let (_, after) = on_sogo(&m, 0, &uid).expect("ann's copy on SOGo");
    assert_eq!(after.len(), before.len(), "ann's copy as it was");

    let (mut carol, cid) = App::new(&m, 2, "forward-carol");
    carol.s.run("account.dav", &json!({"account": cid, "caldav": web})).unwrap();
    online(&mut carol);
    carol.sync_dav(&cid);
    let fwd = carol.wait_for_mail(&cid, &format!("Fwd: {title}"));
    let cal = carol.s.message_calendar(&fwd).expect("a meeting");
    assert_eq!(cal.events.iter().map(|e| (e.recurrence_id, e.rrule.is_some())).collect::<Vec<_>>(), [(Some(third), false)], "that occurrence alone");
    carol.s.run("folder.open", &json!({"account": cid, "folder": "INBOX"})).unwrap();
    carol.s.run("list.open", &json!({"key": fwd.to_id()})).unwrap();
    carol.s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    carol.pump(120, "carol's answer stored", |s, _| {
        s.pim.values().flat_map(|p| p.events.iter()).filter(|e| e.event.uid == uid).all(|e| !e.dirty && !e.href.is_empty())
    });
    carol.settle(5);
    let (_, carols) = on_sogo(&m, 2, &uid).expect("carol's copy on SOGo");
    eprintln!("carol's copy on SOGo: {:?}", carols.iter().map(|e| (e.recurrence_id, e.start, e.rrule.clone())).collect::<Vec<_>>());
    assert!(carols.iter().all(|e| e.rrule.is_none()), "no series in carol's calendar");
    assert!(carols.iter().any(|e| e.start == third && answer_of(e, &carol_email) == "ACCEPTED"), "that day, accepted");
    m.clean_calendars(&[0, 1, 2], &mark);
}
