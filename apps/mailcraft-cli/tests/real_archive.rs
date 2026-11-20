//! Old mail archived to an Outlook data file on this computer (issue #179), through a real IMAP
//! server on this computer (never a real account), run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_ARCHIVE="127.0.0.1:31143" \
//!   cargo test -p mailcraft-cli --test real_archive -- --ignored --nocapture
//! ```
//!
//! Dovecot (any user, password `pw`, cleartext allowed; `servers.md` in the issues skill has the
//! line). The account keeps only the newest message of each folder here, none downloaded ahead,
//! as a big mailbox would: the old mail is found on the server and downloaded. The Inbox's old
//! mail and its folder's go into the file, in folders of the same names; they're deleted on the
//! server only then; the new mail stays; and the file opens in the folder list. Archived again
//! into the file open by then, what's new is read into it. AutoArchive (issue #180) archives each
//! folder by its own settings: one to the file, one deleted for good, one left alone.

#[path = "support/servers.rs"]
mod servers;

use mailcraft_engine::model::Event;
use serde_json::json;
use servers::{App, Imap};

fn message(subject: &str, date: &str) -> Vec<u8> {
    format!(
        "From: Ana <ana@x.example>\r\nTo: kim@test.example\r\nSubject: {subject}\r\nDate: {date}\r\n\
         Message-ID: <{}.{}@x.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{subject}.\r\n",
        subject.replace(' ', "-"),
        std::process::id()
    )
    .into_bytes()
}

/// Every message of every folder of `account` here, by subject, in order.
fn subjects_here(s: &mut mailcraft_engine::Session, account: &str) -> Vec<String> {
    let folders: Vec<String> = s.account(account).map(|a| a.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default();
    let mut all = Vec::new();
    for f in &folders {
        all.extend(s.folder_state(account, f).messages.iter().map(|m| m.subject.clone()));
    }
    all.sort();
    all
}

#[test]
#[ignore = "needs a real IMAP server in MAILCRAFT_REAL_ARCHIVE"]
fn real_old_mail_is_archived_to_a_data_file() {
    let Ok(imap) = std::env::var("MAILCRAFT_REAL_ARCHIVE") else {
        eprintln!("MAILCRAFT_REAL_ARCHIVE isn't set: skipped");
        return;
    };
    let imap = imap.trim();
    let user = format!("kim{}", std::process::id());
    let mut server = Imap::connect(imap, &user, "pw");
    // The server's own separator: Projects under the Inbox.
    let list = server.cmd(r#"LIST "" """#);
    let sep = list.split('"').nth(1).filter(|s| s.chars().count() == 1).unwrap_or("/").to_string();
    let projects = format!("INBOX{sep}Projects");
    server.create(&projects);
    server.create("Kept");
    server.append("INBOX", "", &message("Old one", "Mon, 2 Nov 2020 10:00:00 +0000"));
    server.append("INBOX", "\\Seen \\Flagged", &message("Old two", "Tue, 3 Nov 2020 10:00:00 +0000"));
    server.append("INBOX", "", &message("New one", "Mon, 2 Nov 2026 10:00:00 +0000"));
    server.append(&projects, "", &message("Old plan", "Wed, 4 Nov 2020 10:00:00 +0000"));
    server.append("Kept", "", &message("Old kept", "Wed, 4 Nov 2020 10:00:00 +0000"));

    let mut app = App::open("archive", "pw");
    app.s.run("account.keepMail", &json!({"messages": 1, "offline": 0})).unwrap();
    let smtp = format!("{}:1", imap.rsplit_once(':').unwrap().0);
    let a = app.add_plain(imap, &smtp, json!({"email": format!("{user}@test.example"), "username": user, "password": "pw"}));
    app.pump(120, "the Inbox", |s, _| !s.folder_state(&a, "INBOX").messages.is_empty());
    app.settle(4);
    let listed = app.s.folder_state(&a, "INBOX").messages.iter().map(|m| m.subject.clone()).collect::<Vec<_>>();
    assert_eq!(listed, ["New one"], "only the newest kept here");

    // The Inbox and its folder, before 2024: into the file; Kept and the new one stay.
    let file = app.dir.join("archive.pst");
    let before = 1_704_067_200; // 2024-01-01
    let path = file.to_string_lossy().to_string();
    app.s.run("dataFile.archive", &json!({"account": a, "path": path, "before": before, "folder": "INBOX"})).unwrap();
    app.pump(180, "the archive opened", |_, e| matches!(e, Event::Imported { done: true, .. }));
    app.settle(6);
    assert!(file.is_file());
    let archive = app.s.accounts.iter().find(|x| x.config.name == "archive").map(|x| x.config.id.clone()).expect("the archive in the folder list");
    assert_eq!(subjects_here(&mut app.s, &archive), ["Old one", "Old plan", "Old two"]);
    let plan_folder =
        app.s.account(&archive).unwrap().folders.iter().find(|f| f.path.ends_with("Projects")).map(|f| f.path.clone()).expect("its Projects");
    assert_eq!(app.s.folder_state(&archive, &plan_folder).messages.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Old plan"]);
    let two = app.s.folder_state(&archive, "INBOX").messages.iter().find(|m| m.subject == "Old two").map(|m| m.flags.clone()).expect("Old two");
    assert!(two.seen && two.flagged, "as it was marked: {two:?}");
    // Gone from the server; the rest stays.
    assert_eq!(server.subjects("INBOX"), ["New one"]);
    assert!(server.subjects(&projects).is_empty());
    assert_eq!(server.subjects("Kept"), ["Old kept"]);

    // Again, into the file open by then: Kept's old mail is read into it.
    app.s.run("dataFile.archive", &json!({"account": a, "path": path, "before": before, "folder": "Kept"})).unwrap();
    app.pump(180, "the archive read again", |s, _| subjects_here(s, &archive).contains(&"Old kept".to_string()));
    app.settle(6);
    assert!(server.subjects("Kept").is_empty());
    assert_eq!(subjects_here(&mut app.s, &archive), ["Old kept", "Old one", "Old plan", "Old two"]);
    assert_eq!(app.s.accounts.iter().filter(|x| x.config.name == "archive").count(), 1, "the same one, not opened twice");

    // Nothing old left: said, and nothing changes.
    app.s.run("dataFile.archive", &json!({"account": a, "path": path, "before": before})).unwrap();
    app.pump(60, "nothing to archive", |s, _| s.archiving_now().is_none());
    assert!(app.s.notices.iter().any(|n| n.text.contains("No mail old enough")), "{:?}", app.s.notices);
    assert_eq!(server.subjects("INBOX"), ["New one"]);
}

/// More old mail in a folder than a server finds at a time: archived in rounds, all of it.
#[test]
#[ignore = "needs a real IMAP server in MAILCRAFT_REAL_ARCHIVE"]
fn real_a_big_folder_is_archived_in_rounds() {
    let Ok(imap) = std::env::var("MAILCRAFT_REAL_ARCHIVE") else {
        eprintln!("MAILCRAFT_REAL_ARCHIVE isn't set: skipped");
        return;
    };
    let imap = imap.trim();
    let user = format!("bulk{}", std::process::id());
    let mut server = Imap::connect(imap, &user, "pw");
    server.create("Bulk");
    for n in 0..520 {
        server.append("Bulk", "\\Seen", &message(&format!("Bulk {n}"), "Mon, 2 Nov 2020 10:00:00 +0000"));
    }
    server.append("Bulk", "", &message("Bulk new", "Mon, 2 Nov 2026 10:00:00 +0000"));

    let mut app = App::open("archive-bulk", "pw");
    app.s.run("account.keepMail", &json!({"messages": 20, "offline": 0})).unwrap();
    let smtp = format!("{}:1", imap.rsplit_once(':').unwrap().0);
    let a = app.add_plain(imap, &smtp, json!({"email": format!("{user}@test.example"), "username": user, "password": "pw"}));
    app.pump(120, "Bulk", |s, _| s.folder_state(&a, "Bulk").messages.len() >= 20);
    let file = app.dir.join("bulk.pst");
    let path = file.to_string_lossy().to_string();
    app.s.run("dataFile.archive", &json!({"account": a, "path": path, "before": 1_704_067_200})).unwrap();
    app.pump(600, "the archive opened", |_, e| matches!(e, Event::Imported { done: true, .. }));
    app.settle(8);
    assert!(
        app.s.notices.iter().any(|n| n.text.contains("520 messages archived")),
        "{:?}",
        app.s.notices.iter().map(|n| &n.text).collect::<Vec<_>>()
    );
    assert_eq!(server.subjects("Bulk"), ["Bulk new"]);
    let archive = app.s.accounts.iter().find(|x| x.config.name == "bulk").map(|x| x.config.id.clone()).expect("the archive in the folder list");
    let mut there = subjects_here(&mut app.s, &archive);
    there.dedup();
    assert_eq!(there.len(), 520, "each once");
}

/// AutoArchive run: the Inbox by the default settings (into the file), Projects by its own
/// (deleted for good), Kept never archived.
#[test]
#[ignore = "needs a real IMAP server in MAILCRAFT_REAL_ARCHIVE"]
fn real_autoarchive_goes_by_each_folder_s_settings() {
    let Ok(imap) = std::env::var("MAILCRAFT_REAL_ARCHIVE") else {
        eprintln!("MAILCRAFT_REAL_ARCHIVE isn't set: skipped");
        return;
    };
    let imap = imap.trim();
    let user = format!("auto{}", std::process::id());
    let mut server = Imap::connect(imap, &user, "pw");
    server.create("Projects");
    server.create("Kept");
    server.append("INBOX", "", &message("Auto old", "Mon, 2 Nov 2020 10:00:00 +0000"));
    server.append("INBOX", "", &message("Auto new", "Mon, 2 Nov 2026 10:00:00 +0000"));
    server.append("Projects", "", &message("Auto plan", "Mon, 2 Nov 2020 10:00:00 +0000"));
    server.append("Kept", "", &message("Auto kept", "Mon, 2 Nov 2020 10:00:00 +0000"));

    let mut app = App::open("autoarchive", "pw");
    app.s.run("account.keepMail", &json!({"messages": 1, "offline": 0})).unwrap();
    let smtp = format!("{}:1", imap.rsplit_once(':').unwrap().0);
    let a = app.add_plain(imap, &smtp, json!({"email": format!("{user}@test.example"), "username": user, "password": "pw"}));
    app.pump(120, "the folders", |s, _| s.account(&a).is_some_and(|x| x.folders.iter().any(|f| f.path == "Kept")));
    app.settle(3);
    let file = app.dir.join("auto.pst").to_string_lossy().to_string();
    app.s.run("autoArchive.set", &json!({"everyDays": 14, "path": file})).unwrap();
    app.s.run("autoArchive.folder", &json!({"account": a, "folder": "Projects", "archive": "own", "delete": true})).unwrap();
    app.s.run("autoArchive.folder", &json!({"account": a, "folder": "Kept", "archive": "off"})).unwrap();
    app.s.run("autoArchive.run", &json!({})).unwrap();
    app.pump(300, "AutoArchive done", |s, _| !s.auto_archive_running() && s.archiving_now().is_none());
    app.settle(8);
    assert_eq!(server.subjects("INBOX"), ["Auto new"]);
    assert!(server.subjects("Projects").is_empty(), "deleted for good");
    assert_eq!(server.subjects("Kept"), ["Auto kept"], "never archived");
    let archive = app.s.accounts.iter().find(|x| x.config.name == "auto").map(|x| x.config.id.clone()).expect("the file in the folder list");
    assert_eq!(subjects_here(&mut app.s, &archive), ["Auto old"], "only what was archived, not what was deleted");
}
