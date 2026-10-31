//! Who mails a meeting's invitations, changes, cancellations and answers when the calendar
//! server could too (CalDAV scheduling, RFC 6638): MailCraft, once each, whatever the server.
//! Counted in every mailbox they reach: a guest on the organizer's server, a guest on another
//! server, one outside (GreenMail), and the organizer for the answers; and what lands in each
//! calendar. Never a real account; run by hand, one server at a time or together:
//!
//! ```text
//! MAILCRAFT_REAL_MAILCOW="127.0.0.1:22143 127.0.0.1:22588 http://127.0.0.1:22080 \
//!   ann@mc31.test:<password> bob@mc31.test:<password> carol@mc31.test:<password>" \
//! MAILCRAFT_REAL_STALWART="127.0.0.1:23143 127.0.0.1:23025 http://127.0.0.1:23080 ann@st.test:ann:pw bob@st.test:bob:pw" \
//! MAILCRAFT_REAL_OUTSIDE="127.0.0.1:24143 ext@outside.example" \
//! MAILCRAFT_REAL_RADICALE="http://127.0.0.1:25232/" \
//!   cargo test -p mailcraft-cli --test real_scheduling -- --ignored --nocapture --test-threads 1
//! ```
//!
//! With SOGo's `SOGoAppointmentSendEMailNotifications = NO` (stock SOGo; Mailcow sets YES), add
//! `MAILCRAFT_SOGO_NOTIFICATIONS=off`: SOGo then mails a colleague's answer to nobody, and writes
//! it in the organizer's calendar only.
//!
//! The servers, on this computer and one Docker network:
//! - Mailcow as `real_mailcow_mail.rs` says, with two transports (Mailcow's API,
//!   `POST /api/v1/add/transport`): `outside.example` to GreenMail's SMTP port, `st.test` to
//!   Stalwart's port 25.
//! - Stalwart v0.13 (`stalwartlabs/stalwart`), HTTP on 23080, IMAP on 23143, SMTP on 23025;
//!   domain `st.test`, users `ann` and `bob`; through its settings API: plain-text sign-in
//!   allowed (`imap.auth.allow-plain-text`, `session.auth.allow-plain-text`,
//!   `session.auth.mechanisms`, `session.auth.directory = '*'`, relaying for those signed in),
//!   and routes (`queue.route.*` relays, `queue.strategy.route`) for `mc31.test` to Mailcow's
//!   Postfix and `outside.example` to GreenMail.
//! - GreenMail (`greenmail/standalone`, `-Dgreenmail.setup.test.all -Dgreenmail.auth.disabled`),
//!   IMAP on 24143: the outside domain.
//! - Radicale (`tomsquest/docker-radicale`, `[auth] type = none`) on 25232, with a calendar
//!   `cal/` made for `ann@mc31.test` (`MKCALENDAR`): a server that doesn't schedule.

#[path = "support/mailcow.rs"]
mod mailcow;

use std::time::{Duration, Instant};

use mailcow::{App, Imap, Mailcow};
use mailcraft_engine::model::{Event, Job, SyncStatus};
use serde_json::json;

/// A mailbox the meeting's mail is counted in.
struct Mailbox {
    imap: String,
    user: String,
    password: String,
}

impl Mailbox {
    fn new(imap: &str, user: &str, password: &str) -> Mailbox {
        Mailbox { imap: imap.into(), user: user.into(), password: password.into() }
    }

    /// The subjects (as sent, unfolded) of the messages naming `token` in its Inbox, Junk and
    /// deleted mail (an answered invitation goes there).
    fn about(&self, token: &str) -> Vec<String> {
        let mut c = Imap::connect(&self.imap, &self.user, &self.password);
        let mut out = Vec::new();
        for folder in ["INBOX", "Junk", "Trash", "Deleted Items"] {
            let tag = format!("t{} OK", c.n + 1);
            if !c.try_cmd(&format!("EXAMINE \"{folder}\"")).lines().any(|l| l.starts_with(&tag)) {
                continue;
            }
            let r = c.try_cmd("UID FETCH 1:* (BODY.PEEK[HEADER.FIELDS (SUBJECT)])").replace("\r\n ", " ").replace("\r\n\t", " ");
            out.extend(r.lines().filter_map(|l| l.strip_prefix("Subject: ")).filter(|s| s.contains(token)).map(str::to_string));
        }
        out
    }

    /// Take out what an earlier run left.
    fn clear(&self) {
        let mut c = Imap::connect(&self.imap, &self.user, &self.password);
        for folder in ["INBOX", "Junk", "Trash", "Deleted Items"] {
            let tag = format!("t{} OK", c.n + 1);
            if c.try_cmd(&format!("SELECT \"{folder}\"")).lines().any(|l| l.starts_with(&tag)) {
                c.try_cmd("UID STORE 1:* +FLAGS.SILENT (\\Deleted)");
                c.try_cmd("EXPUNGE");
            }
        }
    }
}

/// Wait until each mailbox has as many messages about `token` as said, then a little more (a
/// second copy comes late), and check it has exactly that many. Returns what each has.
fn counted(token: &str, want: &[(&str, &Mailbox, usize)]) -> Vec<Vec<String>> {
    let end = Instant::now() + Duration::from_secs(90);
    loop {
        let got: Vec<Vec<String>> = want.iter().map(|(_, b, _)| b.about(token)).collect();
        if want.iter().zip(&got).all(|((_, _, n), g)| g.len() >= *n) {
            break;
        }
        assert!(Instant::now() < end, "still waiting: {:?}", want.iter().zip(&got).map(|((w, _, n), g)| (w, n, g)).collect::<Vec<_>>());
        std::thread::sleep(Duration::from_secs(2));
    }
    std::thread::sleep(Duration::from_secs(12));
    let got: Vec<Vec<String>> = want.iter().map(|(_, b, _)| b.about(token)).collect();
    for ((who, _, n), g) in want.iter().zip(&got) {
        eprintln!("  {who}: {} {g:?}", g.len());
        assert_eq!(g.len(), *n, "{who} got {g:?}");
    }
    got
}

fn online(app: &mut App) {
    app.pump(120, "online", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
}

/// A MailCraft online with its calendar at `caldav`, synced.
fn app(imap: &str, smtp: &str, email: &str, login: &str, password: &str, caldav: &str, tag: &str) -> (App, String) {
    let (mut a, id) = App::at(imap, smtp, email, login, password, tag);
    a.s.run("account.dav", &json!({"account": id, "caldav": caldav})).unwrap();
    online(&mut a);
    a.sync_dav(&id);
    a.settle(2);
    (a, id)
}

/// The user's meeting stored and its mail sent (nothing waiting for the server).
fn sent(app: &mut App, id: &str, uid: &str) {
    let (id, uid) = (id.to_string(), uid.to_string());
    app.pump(120, "the meeting stored and its mail sent", |s, _| {
        s.find_event(&uid).is_none_or(|(_, e)| !e.dirty && !e.href.is_empty()) && s.pim[&id].notices.is_empty() && s.pim[&id].deleted.is_empty()
    });
    app.settle(3);
}

/// The newest message whose subject is `subject`, taken in.
fn newest(app: &mut App, id: &str, subject: &str, after: u32) -> mailcraft_engine::model::MessageKey {
    let end = Instant::now() + Duration::from_secs(120);
    loop {
        let found = app
            .s
            .folder_state(id, "INBOX")
            .messages
            .iter()
            .filter(|m| m.subject == subject && m.key.uid > after)
            .map(|m| m.key.clone())
            .max_by_key(|k| k.uid);
        if let Some(k) = found {
            return k;
        }
        assert!(Instant::now() < end, "{subject:?} didn't come");
        app.pool.submit(Job::SyncFolder { account: id.to_string(), folder: "INBOX".into() });
        app.settle(2);
    }
}

/// Open an invitation and answer it; it isn't out of date (SOGo stores a higher SEQUENCE than
/// the one sent: MailCraft's mail carries the server's).
fn answer_mail(app: &mut App, id: &str, key: &mailcraft_engine::model::MessageKey, answer: &str) {
    app.sync_dav(id);
    app.s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    app.s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    let inv = app.s.invite().expect("an invitation");
    assert!(inv.problem.is_none(), "{:?}: the calendar has {:?}", inv.problem, app.s.find_event(&inv.event.uid).map(|(_, e)| e.event.sequence));
    app.s.run("invite.respond", &json!({"answer": answer})).unwrap();
    settled(app, id, &inv.event.uid);
}

/// The user's copy of a meeting stored, and any answer sent.
fn settled(app: &mut App, id: &str, uid: &str) {
    let (id, uid) = (id.to_string(), uid.to_string());
    app.pump(120, "the answer stored and sent", |s, _| {
        s.find_event(&uid).is_none_or(|(_, e)| !e.dirty) && s.pim[&id].notices.is_empty() && s.pim[&id].deleted.is_empty()
    });
    app.settle(3);
}

fn create(app: &mut App, id: &str, title: &str, attendees: &str) -> String {
    let uid =
        app.s.run("event.create", &json!({"account": id, "summary": title, "start": "2026-12-01T10:00", "attendees": attendees})).unwrap()["uid"]
            .as_str()
            .unwrap()
            .to_string();
    sent(app, id, &uid);
    uid
}

fn partstat(data: &str, who: &str) -> String {
    data.lines()
        .find(|l| l.starts_with("ATTENDEE") && l.to_ascii_lowercase().contains(&format!("mailto:{who}")))
        .and_then(|l| l.split(';').find_map(|p| p.strip_prefix("PARTSTAT=")))
        .map(|p| p.split(':').next().unwrap_or(p).to_string())
        .unwrap_or_default()
}

/// The outside domain from `MAILCRAFT_REAL_OUTSIDE`.
fn outside() -> Option<(Mailbox, String)> {
    let v = std::env::var("MAILCRAFT_REAL_OUTSIDE").ok()?;
    let (imap, user) = v.split_once(' ')?;
    Some((Mailbox::new(imap, user.trim(), user.trim()), user.trim().to_string()))
}

/// Stalwart from `MAILCRAFT_REAL_STALWART`: its IMAP, SMTP, web addresses and two users
/// (address, sign-in, password).
struct Stalwart {
    imap: String,
    smtp: String,
    web: String,
    users: Vec<(String, String, String)>,
}

fn stalwart() -> Option<Stalwart> {
    let v = std::env::var("MAILCRAFT_REAL_STALWART").ok()?;
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, smtp, web, users @ ..] = p.as_slice() else { return None };
    let users = users
        .iter()
        .filter_map(|u| {
            let mut x = u.splitn(3, ':');
            Some((x.next()?.to_string(), x.next()?.to_string(), x.next()?.to_string()))
        })
        .collect();
    Some(Stalwart { imap: imap.to_string(), smtp: smtp.to_string(), web: web.trim_end_matches('/').to_string(), users })
}

impl Stalwart {
    fn mailbox(&self, n: usize) -> Mailbox {
        Mailbox::new(&self.imap, &self.users[n].1, &self.users[n].2)
    }
    fn app(&self, n: usize, tag: &str) -> (App, String) {
        let (e, l, p) = &self.users[n];
        app(&self.imap, &self.smtp, e, l, p, &format!("{}/", self.web), tag)
    }
    /// The user's calendar items on Stalwart: (href, data).
    fn calendar(&self, n: usize) -> Vec<(String, String)> {
        let (_, l, p) = &self.users[n];
        let auth = format!("Basic {}", mailcow::base64(format!("{l}:{p}").as_bytes()));
        let url = format!("{}/dav/cal/{l}/default/", self.web);
        let body = "<c:calendar-query xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><c:calendar-data/></d:prop><c:filter><c:comp-filter name=\"VCALENDAR\"/></c:filter></c:calendar-query>";
        let r = mailcow::http("REPORT", &url, &[("Authorization", &auth), ("Depth", "1"), ("Content-Type", "application/xml")], body.as_bytes());
        let text = r.text();
        text.split("response>")
            .filter_map(|x| {
                let href = mailcow::between(x, "href>", "</")?;
                let data = mailcow::between(x, "calendar-data>", "</")?;
                Some((href, mailcow::unescape(&data).replace("&#13;", "").replace("\r\n ", "").replace("\n ", "")))
            })
            .collect()
    }
}

fn notifications_off() -> bool {
    std::env::var("MAILCRAFT_SOGO_NOTIFICATIONS").is_ok_and(|v| v.eq_ignore_ascii_case("off"))
}

/// ann's MailCraft organizes on Mailcow's SOGo, which schedules (and, as Mailcow sets it,
/// mails): a colleague (bob, MailCraft too) and a guest outside each get one invitation, one
/// change and one cancellation, all from MailCraft, and bob's calendar has the meeting from SOGo
/// as well, the same version (bob's MailCraft doesn't find the invitation out of date). bob's
/// answer, given in a meeting SOGo put in his calendar, is SOGo's to take to ann: one, by mail
/// or (with SOGo's mail off) in her calendar. The outside guest's answer, taken in, changes
/// nothing for the others.
#[test]
#[ignore]
fn real_scheduling_on_sogo() {
    let Some(m) = Mailcow::from_env() else { return };
    let Some((ext, ext_addr)) = outside() else { return eprintln!("MAILCRAFT_REAL_OUTSIDE isn't set: skipped") };
    let mb = |n: usize| Mailbox::new(&m.imap, m.user(n).0, m.user(n).1);
    let (ann_box, bob_box) = (mb(0), mb(1));
    for b in [&ann_box, &bob_box, &ext] {
        b.clear();
    }
    let web = format!("{}/", m.web);
    let tag = format!("SchedSogo{}", std::process::id());
    let (mut ann, aid) = app(&m.imap, &m.smtp, m.user(0).0, m.user(0).0, m.user(0).1, &web, "sched-sogo");
    let (mut bob, bid) = app(&m.imap, &m.smtp, m.user(1).0, m.user(1).0, m.user(1).1, &web, "sched-sogo");
    assert!(ann.s.pim[&aid].auto_schedule, "SOGo says calendar-auto-schedule");
    let bob_addr = m.user(1).0.to_string();

    eprintln!("ann invites bob and {ext_addr}");
    let uid = create(&mut ann, &aid, &tag, &format!("Bob <{bob_addr}>, Ext <{ext_addr}>"));
    counted(&tag, &[("ann", &ann_box, 0), ("bob", &bob_box, 1), ("outside", &ext, 1)]);
    let bobs_copy = || m.calendar(1, &format!("{}personal/", m.dav_home(1))).into_iter().find(|(_, d)| d.contains(&uid));
    let (_, data) = bobs_copy().expect("SOGo put the meeting in bob's calendar");
    let anns = ann.s.find_event(&uid).map(|(_, e)| e.event.sequence).unwrap_or_default();
    assert!(data.contains(&format!("SEQUENCE:{anns}")) || anns == 0, "the version ann's MailCraft has (and mailed): {anns}\n{data}");

    eprintln!("bob accepts in MailCraft's invitation");
    let key = newest(&mut bob, &bid, &format!("Invitation: {tag}"), 0);
    answer_mail(&mut bob, &bid, &key, "accepted");
    let anns_copy =
        || m.calendar(0, &format!("{}personal/", m.dav_home(0))).into_iter().find(|(_, d)| d.contains(&uid)).map(|x| x.1).unwrap_or_default();
    if notifications_off() {
        counted(&tag, &[("ann", &ann_box, 0), ("bob", &bob_box, 1), ("outside", &ext, 1)]);
        let end = Instant::now() + Duration::from_secs(30);
        while partstat(&anns_copy(), &bob_addr) != "ACCEPTED" {
            assert!(Instant::now() < end, "bob's answer in ann's calendar: {}", anns_copy());
            std::thread::sleep(Duration::from_secs(2));
        }
    } else {
        let got = counted(&tag, &[("ann", &ann_box, 1), ("bob", &bob_box, 1), ("outside", &ext, 1)]);
        assert!(!got[0][0].starts_with("Accepted: "), "SOGo's, not MailCraft's too: {:?}", got[0]);
    }
    assert_eq!(partstat(&bobs_copy().map(|x| x.1).unwrap_or_default(), &bob_addr), "ACCEPTED");

    eprintln!("bob says tentative in his calendar: SOGo's copy, SOGo's to answer");
    bob.s.run("event.respond", &json!({"uid": uid, "answer": "tentative"})).unwrap();
    settled(&mut bob, &bid, &uid);
    let sogos = usize::from(!notifications_off());
    counted(&tag, &[("ann", &ann_box, 2 * sogos), ("bob", &bob_box, 1), ("outside", &ext, 1)]);
    let end = Instant::now() + Duration::from_secs(30);
    while partstat(&anns_copy(), &bob_addr) != "TENTATIVE" {
        assert!(Instant::now() < end, "bob's answer in ann's calendar: {}", anns_copy());
        std::thread::sleep(Duration::from_secs(2));
    }

    eprintln!("the outside guest's answer reaches ann's Inbox; ann takes it in");
    let mut reply = ann.s.find_event(&uid).unwrap().1.event.clone();
    reply.attendees.retain(|a| a.email.eq_ignore_ascii_case(&ext_addr));
    reply.attendees[0].partstat = "TENTATIVE".into();
    reply.client_scheduled = false;
    let raw = itip_mail(&ext_addr, m.user(0).0, &format!("Tentative: {tag}"), "REPLY", &reply);
    m.imap_as(0).append("INBOX", "", &raw);
    let key = newest(&mut ann, &aid, &format!("Tentative: {tag}"), 0);
    ann.s.run("folder.open", &json!({"account": aid, "folder": "INBOX"})).unwrap();
    ann.s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    ann.s.run("invite.apply", &json!({})).unwrap();
    sent(&mut ann, &aid, &uid);
    counted(&tag, &[("ann", &ann_box, 2 * sogos + 1), ("bob", &bob_box, 1), ("outside", &ext, 1)]);
    assert_eq!(partstat(&anns_copy(), &ext_addr), "TENTATIVE");

    eprintln!("ann moves it");
    let before = bob.s.folder_state(&bid, "INBOX").messages.iter().map(|x| x.key.uid).max().unwrap_or(0);
    ann.s.run("event.update", &json!({"uid": uid, "start": "2026-12-01T11:00"})).unwrap();
    sent(&mut ann, &aid, &uid);
    counted(&tag, &[("bob", &bob_box, 2), ("outside", &ext, 2)]);
    let key = newest(&mut bob, &bid, &format!("Invitation: {tag}"), before);
    bob.sync_dav(&bid);
    bob.s.run("folder.open", &json!({"account": bid, "folder": "INBOX"})).unwrap();
    bob.s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    let inv = bob.s.invite().unwrap();
    assert!(inv.problem.is_none(), "the change isn't out of date in bob's calendar: {:?}", inv.problem);

    eprintln!("ann calls it off");
    ann.s.run("event.delete", &json!({"uid": uid})).unwrap();
    sent(&mut ann, &aid, &uid);
    counted(&tag, &[("bob", &bob_box, 3), ("outside", &ext, 3)]);
    assert!(bobs_copy().is_none(), "SOGo took it out of bob's calendar");
}

/// A message carrying a calendar, as another program sends it.
fn itip_mail(from: &str, to: &str, subject: &str, method: &str, e: &mailcraft_engine::model::ical::CalEvent) -> Vec<u8> {
    let ics = mailcraft_engine::model::ical::write(std::slice::from_ref(e), Some(method), mailcow::now());
    format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{}.{method}@outside.example>\r\nDate: Sat, 31 Oct 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/calendar; method={method}; charset=utf-8\r\n\r\n{ics}",
        mailcow::now()
    )
    .into_bytes()
}

/// ann's MailCraft organizes on Stalwart, which schedules and honours `SCHEDULE-AGENT`: her
/// colleague on Stalwart, a guest on Mailcow's SOGo (an organizer on another server for him)
/// and one outside each get one invitation, change and cancellation. Their MailCraft answers,
/// from the message and from the calendar, reach ann once each: SOGo, which answers for any
/// change to a guest's copy, isn't let to (bob's copy there is stored again as new).
#[test]
#[ignore]
fn real_scheduling_on_stalwart_and_from_another_server() {
    let Some(st) = stalwart() else { return eprintln!("MAILCRAFT_REAL_STALWART isn't set: skipped") };
    let Some(m) = Mailcow::from_env() else { return };
    let Some((ext, ext_addr)) = outside() else { return eprintln!("MAILCRAFT_REAL_OUTSIDE isn't set: skipped") };
    let (ann_box, colleague_box) = (st.mailbox(0), st.mailbox(1));
    let sogo_box = Mailbox::new(&m.imap, m.user(1).0, m.user(1).1);
    for b in [&ann_box, &colleague_box, &sogo_box, &ext] {
        b.clear();
    }
    let tag = format!("SchedStalwart{}", std::process::id());
    let (mut ann, aid) = st.app(0, "sched-st");
    let (mut col, cid) = st.app(1, "sched-st");
    let web = format!("{}/", m.web);
    let (mut bob, bid) = app(&m.imap, &m.smtp, m.user(1).0, m.user(1).0, m.user(1).1, &web, "sched-st");
    assert!(ann.s.pim[&aid].auto_schedule, "Stalwart says calendar-auto-schedule");
    let (col_addr, bob_addr) = (st.users[1].0.clone(), m.user(1).0.to_string());

    eprintln!("ann invites her colleague, bob on SOGo and {ext_addr}");
    let uid = create(&mut ann, &aid, &tag, &format!("{col_addr}, {bob_addr}, {ext_addr}"));
    counted(&tag, &[("ann", &ann_box, 0), ("colleague", &colleague_box, 1), ("bob on SOGo", &sogo_box, 1), ("outside", &ext, 1)]);
    let anns = st.calendar(0).into_iter().find(|(_, d)| d.contains(&uid)).map(|x| x.1).unwrap_or_default();
    assert!(anns.contains("SCHEDULE-AGENT=CLIENT"), "{anns}");
    assert!(!st.calendar(1).iter().any(|(_, d)| d.contains(&uid)), "Stalwart didn't schedule it");

    eprintln!("bob on SOGo accepts in the message, then says tentative in his calendar");
    let key = newest(&mut bob, &bid, &format!("Invitation: {tag}"), 0);
    answer_mail(&mut bob, &bid, &key, "accepted");
    counted(&tag, &[("ann", &ann_box, 1)]);
    let bobs = m.calendar(1, &format!("{}personal/", m.dav_home(1))).into_iter().find(|(_, d)| d.contains(&uid)).map(|x| x.1).unwrap_or_default();
    assert!(bobs.contains(mailcraft_engine::model::ical::GUEST_COPY) && bobs.contains("SCHEDULE-AGENT=CLIENT"), "{bobs}");
    bob.s.run("event.respond", &json!({"uid": uid, "answer": "tentative"})).unwrap();
    settled(&mut bob, &bid, &uid);
    // SOGo, mailing (as Mailcow has it), sends one of its own for an answer changed in a copy
    // it has, whatever the copy says: the one duplicate left (see `ical::scheduling`).
    let sogos = usize::from(!notifications_off());
    counted(&tag, &[("ann", &ann_box, 2 + sogos)]);
    let bobs = m.calendar(1, &format!("{}personal/", m.dav_home(1))).into_iter().find(|(_, d)| d.contains(&uid)).map(|x| x.1).unwrap_or_default();
    assert_eq!(partstat(&bobs, &bob_addr), "TENTATIVE", "{bobs}");

    eprintln!("the colleague accepts in the message, then declines in the calendar");
    let key = newest(&mut col, &cid, &format!("Invitation: {tag}"), 0);
    answer_mail(&mut col, &cid, &key, "accepted");
    counted(&tag, &[("ann", &ann_box, 3 + sogos)]);
    col.s.run("event.respond", &json!({"uid": uid, "answer": "declined"})).unwrap();
    settled(&mut col, &cid, &uid);
    counted(&tag, &[("ann", &ann_box, 4 + sogos), ("colleague", &colleague_box, 1), ("bob on SOGo", &sogo_box, 1), ("outside", &ext, 1)]);

    eprintln!("ann moves it, then calls it off");
    ann.s.run("event.update", &json!({"uid": uid, "start": "2026-12-01T11:00"})).unwrap();
    sent(&mut ann, &aid, &uid);
    counted(&tag, &[("colleague", &colleague_box, 2), ("bob on SOGo", &sogo_box, 2), ("outside", &ext, 2)]);
    ann.s.run("event.delete", &json!({"uid": uid})).unwrap();
    sent(&mut ann, &aid, &uid);
    counted(&tag, &[("ann", &ann_box, 4 + sogos), ("colleague", &colleague_box, 3), ("bob on SOGo", &sogo_box, 3), ("outside", &ext, 3)]);
}

/// A server that doesn't schedule (Radicale): MailCraft mails at once, as before, and stores
/// nothing about who schedules.
#[test]
#[ignore]
fn real_scheduling_without_a_scheduling_server() {
    let Ok(radicale) = std::env::var("MAILCRAFT_REAL_RADICALE") else { return eprintln!("MAILCRAFT_REAL_RADICALE isn't set: skipped") };
    let Some(m) = Mailcow::from_env() else { return };
    let Some((ext, ext_addr)) = outside() else { return };
    let bob_box = Mailbox::new(&m.imap, m.user(1).0, m.user(1).1);
    for b in [&bob_box, &ext] {
        b.clear();
    }
    let tag = format!("SchedRadicale{}", std::process::id());
    let (mut ann, aid) = app(&m.imap, &m.smtp, m.user(0).0, m.user(0).0, m.user(0).1, &radicale, "sched-radicale");
    assert!(!ann.s.pim[&aid].auto_schedule);
    let uid = create(&mut ann, &aid, &tag, &format!("{}, {ext_addr}", m.user(1).0));
    let (_, e) = ann.s.find_event(&uid).unwrap();
    assert!(e.calendar.starts_with(&radicale) && !e.event.client_scheduled, "{e:?}");
    let r = mailcow::http("GET", &e.href, &[("Authorization", &format!("Basic {}", mailcow::base64(b"ann@mc31.test:x")))], b"");
    assert!(r.status == 200 && !r.text().contains("SCHEDULE-AGENT"), "{}", r.text());
    counted(&tag, &[("bob", &bob_box, 1), ("outside", &ext, 1)]);
    ann.s.run("event.delete", &json!({"uid": uid})).unwrap();
    sent(&mut ann, &aid, &uid);
    counted(&tag, &[("bob", &bob_box, 2), ("outside", &ext, 2)]);
}
