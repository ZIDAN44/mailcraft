//! Awkward names and mail between two accounts, through a real IMAP server on this computer
//! (never a real account), run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_AWKWARD="127.0.0.1:31143" \
//!   cargo test -p mailcraft-cli --test real_awkward -- --ignored --nocapture
//! ```
//!
//! Dovecot (any user, password `pw`, cleartext allowed). Found there: folders whose names have
//! a `"` or a `\` in them were shown, and asked for, with the server's escapes still in. Then
//! what an audit changed in moving mail to another account: its categories and the day it came
//! go with it, and it's deleted where it was once its copy is there.

#[path = "support/servers.rs"]
mod servers;

use mailcraft_engine::model::Event;
use serde_json::json;
use servers::{App, Imap};

fn message(n: u32) -> Vec<u8> {
    format!(
        "From: Ana <ana@x.example>\r\nTo: kim@test.example\r\nSubject: Note {n}\r\nDate: Mon, 2 Nov 2026 10:{n:02}:00 +0000\r\n\
         Message-ID: <awkward-{n}.{}@x.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nNote {n}.\r\n",
        std::process::id()
    )
    .into_bytes()
}

#[test]
#[ignore = "needs a real IMAP server in MAILCRAFT_REAL_AWKWARD"]
fn real_awkward_names_and_mail_between_accounts() {
    let Ok(imap) = std::env::var("MAILCRAFT_REAL_AWKWARD") else {
        eprintln!("MAILCRAFT_REAL_AWKWARD isn't set: skipped");
        return;
    };
    let imap = imap.trim();
    let pid = std::process::id();
    let (kim, lee) = (format!("kim{pid}"), format!("lee{pid}"));
    let mut other = Imap::connect(imap, &kim, "pw");
    // As a server names them: a backslash, quotes, an ampersand (modified UTF-7's own).
    for f in [r"Back\slash", r#"Say "hi""#, "R&-D", "Trash"] {
        other.create(f);
    }
    other.append("INBOX", "", &message(1));
    other.append("INBOX", "\\Seen \\Flagged to=20do", &message(2));
    other.append("INBOX", "", &message(3));
    let mut theirs = Imap::connect(imap, &lee, "pw");

    let mut app = App::open("awkward", "pw");
    let smtp = format!("{}:1", imap.rsplit_once(':').unwrap().0);
    let add = |app: &mut App, user: &str| {
        app.add_plain(imap, &smtp, json!({"email": format!("{user}@test.example"), "username": user, "name": "Kim", "password": "pw"}))
    };
    let a = add(&mut app, &kim);
    let b = add(&mut app, &lee);
    app.pump(120, "Kim's Inbox", |s, _| s.folder_state(&a, "INBOX").messages.len() >= 3);
    app.pump(120, "Lee's folders", |s, _| s.account(&b).is_some_and(|x| !x.folders.is_empty()));

    // The names as they are, their escapes read.
    let paths: Vec<String> = app.s.account(&a).unwrap().folders.iter().map(|f| f.path.clone()).collect();
    for name in [r"Back\slash", r#"Say "hi""#, "R&-D"] {
        assert!(paths.iter().any(|p| p == name), "{name} in {paths:?}");
    }
    // Mail goes into them, and comes back with Undo.
    app.select(&a, "INBOX", "Note 1");
    app.s.run("mail.move", &json!({"folder": r"Back\slash"})).unwrap();
    app.settle(6);
    assert_eq!(other.subjects(r"Back\slash"), ["Note 1"]);
    app.select(&a, "INBOX", "Note 3");
    app.s.run("mail.move", &json!({"folder": r#"Say "hi""#})).unwrap();
    app.settle(6);
    assert_eq!(other.subjects(r#"Say "hi""#), ["Note 3"]);
    app.s.run("edit.undo", &json!({})).unwrap();
    app.settle(6);
    assert!(other.subjects("INBOX").contains(&"Note 3".to_string()), "put back");
    // A folder made here with both in its name is the server's by that name.
    app.s.run("folder.create", &json!({"name": r#"New "q" \ one"#, "account": a})).unwrap();
    app.settle(6);
    assert!(other.try_cmd(r#"SELECT "New \"q\" \\ one""#).contains(" OK"), "made as named");

    // To the other account's Inbox: read, flagged and categorised as it was, come the day it
    // came; gone from where it was only then; and not new mail there.
    app.select(&a, "INBOX", "Note 2");
    let r = app.s.run("mail.move", &json!({"folder": "INBOX", "account": b})).unwrap();
    assert_eq!(r["sending"], 1, "{r}");
    app.pump(120, "the copy", |_, e| matches!(e, Event::CopiesFiled { .. }));
    app.settle(8);
    let there = theirs.messages("INBOX");
    assert_eq!(there.len(), 1, "{there:?}");
    let m = &there[0];
    assert_eq!(m.subject, "Note 2");
    assert!(["\\Seen", "\\Flagged", "to=20do"].iter().all(|f| m.flags.iter().any(|x| x == f)), "{m:?}");
    assert!(m.line.contains("02-Nov-2026"), "the day it came: {m:?}");
    assert!(!other.subjects("INBOX").contains(&"Note 2".to_string()), "moved, not copied");
}
