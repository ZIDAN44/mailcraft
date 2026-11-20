//! An Inbox's mail through a real IMAP server on this computer (never a real account): a heading
//! of the list folded away (issue #189), the folder marked read (issue #197), and what's
//! announced of new mail with the Focused Inbox on (issue #199). Run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_FOLDING="127.0.0.1:31143" \
//!   cargo test -p mailcraft-cli --test real_folding -- --ignored --nocapture
//! ```
//!
//! Dovecot (any user, password `pw`, cleartext allowed; `servers.md` in the issues skill has the
//! line). Today's mail folded away stays hidden and counted as more arrives on the server, read
//! or unread; the older mail stays shown; the arrows step over it; shown again, it's all there.
//! A folder marked read is read on the server, mail not kept here too. With the Focused Inbox
//! on, a newsletter arriving isn't announced and a person's message is.

#[path = "support/servers.rs"]
mod servers;

use mailcraft_engine::Row;
use serde_json::json;
use servers::{App, Imap};

/// `t` (Unix seconds) as a message's `Date:`.
fn date(t: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (y, m, d) = mailcraft_engine::model::dates::civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    let month = MONTHS.get(usize::try_from(m).unwrap_or(1).saturating_sub(1)).copied().unwrap_or("Jan");
    format!("{d} {month} {y} {:02}:{:02}:{:02} +0000", s / 3600, s % 3600 / 60, s % 60)
}

fn message(subject: &str, when: &str) -> Vec<u8> {
    format!(
        "From: Ana <ana@x.example>\r\nTo: kim@test.example\r\nSubject: {subject}\r\nDate: {when}\r\n\
         Message-ID: <{}.{}@x.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{subject}.\r\n",
        subject.replace(' ', "-"),
        std::process::id()
    )
    .into_bytes()
}

/// The subjects of the message rows shown, in order.
fn shown(s: &mailcraft_engine::Session) -> Vec<String> {
    s.rows()
        .iter()
        .filter_map(|r| match r {
            Row::Message { index, .. } => s.list.get(*index).map(|m| m.subject.clone()),
            _ => None,
        })
        .collect()
}

fn folded(s: &mailcraft_engine::Session) -> Option<(usize, usize)> {
    s.rows().iter().find_map(|r| if let Row::Folded { count, unread } = r { Some((*count, *unread)) } else { None })
}

#[test]
#[ignore = "needs a real IMAP server in MAILCRAFT_REAL_FOLDING"]
fn real_todays_mail_folded_away_stays_so_as_more_comes() {
    let Ok(imap) = std::env::var("MAILCRAFT_REAL_FOLDING") else {
        eprintln!("MAILCRAFT_REAL_FOLDING isn't set: skipped");
        return;
    };
    let imap = imap.trim();
    let user = format!("kim{}", std::process::id());
    let mut server = Imap::connect(imap, &user, "pw");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    // Today's at its start (in UTC, as the app's clock here), so it's today whenever it runs.
    let morning = now - now.rem_euclid(86_400) + 60;
    server.append("INBOX", "", &message("Old note", "Mon, 2 Nov 2020 10:00:00 +0000"));
    server.append("INBOX", "\\Seen", &message("Morning one", &date(morning)));
    server.append("INBOX", "", &message("Morning two", &date(morning + 60)));

    let mut app = App::open("folding", "pw");
    let smtp = format!("{}:1", imap.rsplit_once(':').unwrap().0);
    let a = app.add_plain(imap, &smtp, json!({"email": format!("{user}@test.example"), "username": user, "password": "pw"}));
    app.wait_for_mail(&a, "Morning two");
    app.s.run("folder.open", &json!({"account": a, "folder": "INBOX"})).unwrap();
    assert_eq!(shown(&app.s), ["Morning two", "Morning one", "Old note"]);

    app.s.run("list.foldGroup", &json!({"heading": "Today"})).unwrap();
    assert_eq!(shown(&app.s), ["Old note"], "today's folded away");
    assert_eq!(folded(&app.s), Some((2, 1)), "two, one unread");

    // More today, on the server: hidden too, counted.
    server.append("INBOX", "", &message("Morning three", &date(morning + 120)));
    app.wait_for_mail(&a, "Morning three");
    app.settle(2);
    assert_eq!(shown(&app.s), ["Old note"], "still folded");
    assert_eq!(folded(&app.s), Some((3, 2)));
    // One read elsewhere: the count follows.
    let uid = server.messages("INBOX").into_iter().find(|m| m.subject == "Morning two").map(|m| m.uid).unwrap();
    server.cmd("SELECT INBOX");
    server.cmd(&format!("UID STORE {uid} +FLAGS (\\Seen)"));
    app.pool.submit(mailcraft_engine::model::Job::SyncFolder { account: a.clone(), folder: "INBOX".into() });
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while folded(&app.s) != Some((3, 1)) && std::time::Instant::now() < end {
        app.settle(1);
    }
    assert_eq!(folded(&app.s), Some((3, 1)), "read on the server");

    // The arrows reach only what's shown.
    let focused = |s: &mailcraft_engine::Session| s.focus.as_ref().and_then(|k| s.summary(k)).map(|m| m.subject.clone());
    app.s.run("list.first", &json!({})).unwrap();
    assert_eq!(focused(&app.s).as_deref(), Some("Old note"));
    app.s.run("list.prev", &json!({})).unwrap();
    assert_eq!(focused(&app.s).as_deref(), Some("Old note"), "nothing above it shown");

    // Shown again: all of it.
    app.s.run("list.foldGroup", &json!({"heading": "Today"})).unwrap();
    assert_eq!(shown(&app.s), ["Morning three", "Morning two", "Morning one", "Old note"]);
    assert_eq!(folded(&app.s), None);
}

/// Note `n`, for marking read.
fn note(n: u32) -> Vec<u8> {
    format!(
        "From: Ana <ana@x.example>\r\nTo: kim@test.example\r\nSubject: Note {n}\r\nDate: Mon, {n} Nov 2026 10:00:00 +0000\r\n\
         Message-ID: <note{n}.{}@x.example>\r\n\r\nNote {n}.\r\n",
        std::process::id()
    )
    .into_bytes()
}

/// Issue #197: Mark All as Read on a folder of six unread messages, two of them kept here: all
/// six are read on the server, the folder's count with them, and none comes back unread at the
/// next sync.
#[test]
#[ignore = "needs a real IMAP server in MAILCRAFT_REAL_FOLDING"]
fn real_a_folder_marked_read_past_what_is_kept() {
    let Ok(imap) = std::env::var("MAILCRAFT_REAL_FOLDING") else {
        eprintln!("MAILCRAFT_REAL_FOLDING isn't set: skipped");
        return;
    };
    let imap = imap.trim();
    // A mailbox of its own: the folding test above has its own in this run.
    let user = format!("lee{}", std::process::id());
    let mut server = Imap::connect(imap, &user, "pw");
    for n in 1..=6 {
        server.append("INBOX", "", &note(n));
    }
    let mut app = App::open("mark-all", "pw");
    // The newest two of each folder kept here.
    app.s.run("account.keepMail", &json!({"messages": 2})).unwrap();
    let smtp = format!("{}:1", imap.rsplit_once(':').unwrap().0);
    let a = app.add_plain(imap, &smtp, json!({"email": format!("{user}@test.example"), "username": user, "password": "pw"}));
    app.wait_for_mail(&a, "Note 6");
    app.s.run("folder.open", &json!({"account": a, "folder": "INBOX"})).unwrap();
    let kept = app.s.list.len();
    let counted = app.s.folder(&a, "INBOX").map(|f| f.unread);
    eprintln!("kept {kept}, the server's count {counted:?}");
    assert!(kept < 6, "not all kept: {kept}");
    assert_eq!(counted, Some(6));

    let r = app.s.run("mail.markAllRead", &json!({})).unwrap();
    assert_eq!(r["count"], 6);
    assert_eq!(app.s.folder(&a, "INBOX").map(|f| f.unread), Some(0), "at once");
    app.settle(3);
    let unread: Vec<String> = (1..=6).map(|n| format!("Note {n}")).filter(|s| !server.flags("INBOX", s).iter().any(|f| f == "\\Seen")).collect();
    assert!(unread.is_empty(), "still unread on the server: {unread:?}");
    // Synced again: still none.
    app.pool.submit(mailcraft_engine::model::Job::SyncFolder { account: a.clone(), folder: "INBOX".into() });
    app.settle(3);
    assert_eq!(app.s.folder(&a, "INBOX").map(|f| f.unread), Some(0), "after a sync");
}

/// Issue #199: with the Focused Inbox on, a newsletter (List-Unsubscribe, Precedence: bulk) and
/// a person's message arriving on the server: only the person's is announced; asked to, the
/// next newsletter is too.
#[test]
#[ignore = "needs a real IMAP server in MAILCRAFT_REAL_FOLDING"]
fn real_only_focused_mail_is_announced() {
    let Ok(imap) = std::env::var("MAILCRAFT_REAL_FOLDING") else {
        eprintln!("MAILCRAFT_REAL_FOLDING isn't set: skipped");
        return;
    };
    let imap = imap.trim();
    // A mailbox of its own.
    let user = format!("fay{}", std::process::id());
    let mut server = Imap::connect(imap, &user, "pw");
    let mail = |subject: &str, bulk: bool| {
        let (from, extra) = if bulk {
            ("Shop <news@shop.example>", "List-Unsubscribe: <mailto:leave@shop.example>\r\nPrecedence: bulk\r\n")
        } else {
            ("Ana <ana@x.example>", "")
        };
        format!(
            "From: {from}\r\nTo: fay@test.example\r\nSubject: {subject}\r\n{extra}Date: {}\r\nMessage-ID: <{}.{}@x.example>\r\n\r\n{subject}.\r\n",
            date(servers::now()),
            subject.replace(' ', "-"),
            std::process::id()
        )
        .into_bytes()
    };
    server.append("INBOX", "", &mail("Before", false));
    let mut app = App::open("focused-notify", "pw");
    let smtp = format!("{}:1", imap.rsplit_once(':').unwrap().0);
    let a = app.add_plain(imap, &smtp, json!({"email": format!("{user}@test.example"), "username": user, "password": "pw"}));
    app.wait_for_mail(&a, "Before");
    app.s.run("view.focused", &json!({"on": true})).unwrap();
    app.s.take_new_mail();
    let arrive = |app: &mut App, server: &mut Imap, msgs: &[(&str, bool)]| -> Vec<String> {
        for (subject, bulk) in msgs {
            server.append("INBOX", "", &mail(subject, *bulk));
        }
        for (subject, _) in msgs {
            app.wait_for_mail(&a, subject);
        }
        app.settle(2);
        app.s.take_new_mail().into_iter().map(|m| m.subject).collect()
    };
    let said = arrive(&mut app, &mut server, &[("Spring sale", true), ("Lunch on Friday", false)]);
    assert_eq!(said, ["Lunch on Friday"], "the newsletter isn't announced");
    app.s.run("view.notifyOther", &json!({"on": true})).unwrap();
    let said = arrive(&mut app, &mut server, &[("Summer sale", true)]);
    assert_eq!(said, ["Summer sale"], "asked to, Other's too");
}
