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

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mailcraft_engine::Session;
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::model::job::Secret;
use mailcraft_engine::model::{Event, Job};
use mailcraft_engine::store::Store;
use mailcraft_net::worker::{MemoryCredentials, Pool};
use serde_json::json;

/// A tiny IMAP client for the other side: put mail in, see what a folder holds.
struct Imap {
    s: std::net::TcpStream,
    n: u32,
}

impl Imap {
    fn connect(addr: &str, user: &str) -> Imap {
        let s = std::net::TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        let mut c = Imap { s, n: 0 };
        c.read_until("* OK");
        c.cmd(&format!("LOGIN \"{user}\" pw"));
        c
    }

    fn read_until(&mut self, end: &str) -> String {
        let mut out = Vec::new();
        let mut b = [0u8; 4096];
        while !String::from_utf8_lossy(&out).lines().any(|l| l.starts_with(end)) {
            let n = self.s.read(&mut b).unwrap();
            assert!(n > 0, "the IMAP server closed");
            out.extend_from_slice(&b[..n]);
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// A command; its answer, whether or not it's OK.
    fn try_cmd(&mut self, c: &str) -> String {
        self.n += 1;
        let tag = format!("t{}", self.n);
        write!(self.s, "{tag} {c}\r\n").unwrap();
        self.read_until(&format!("{tag} "))
    }

    fn cmd(&mut self, c: &str) -> String {
        let r = self.try_cmd(c);
        assert!(r.lines().any(|l| l.starts_with(&format!("t{} OK", self.n))), "{c}: {r}");
        r
    }

    fn append(&mut self, folder: &str, flags: &str, raw: &[u8]) {
        self.n += 1;
        let tag = format!("t{}", self.n);
        write!(self.s, "{tag} APPEND {folder} ({flags}) {{{}+}}\r\n", raw.len()).unwrap();
        self.s.write_all(raw).unwrap();
        self.s.write_all(b"\r\n").unwrap();
        let r = self.read_until(&format!("{tag} "));
        assert!(r.contains(&format!("{tag} OK")), "{r}");
    }

    /// What a folder holds: each message's subject after its `FETCH` line (flags, the day it came).
    fn messages(&mut self, folder: &str) -> Vec<(String, String)> {
        self.cmd(&format!("SELECT {folder}"));
        let r = self.try_cmd("FETCH 1:* (FLAGS INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT)])");
        self.try_cmd("CLOSE");
        let mut out = Vec::new();
        let mut line = String::new();
        for l in r.lines() {
            if l.starts_with("* ") && l.contains("FETCH") {
                line = l.to_string();
            } else if let Some(s) = l.strip_prefix("Subject: ") {
                out.push((s.trim().to_string(), line.clone()));
            }
        }
        out
    }

    fn subjects(&mut self, folder: &str) -> Vec<String> {
        self.messages(folder).into_iter().map(|(s, _)| s).collect()
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn message(n: u32) -> Vec<u8> {
    format!(
        "From: Ana <ana@x.example>\r\nTo: kim@test.example\r\nSubject: Note {n}\r\nDate: Mon, 2 Nov 2026 10:{n:02}:00 +0000\r\n\
         Message-ID: <awkward-{n}.{}@x.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nNote {n}.\r\n",
        std::process::id()
    )
    .into_bytes()
}

/// The session and the network layer together, as in the app.
struct App {
    s: Session,
    pool: Pool,
}

impl App {
    fn pump(&mut self, secs: u64, what: &str, mut done: impl FnMut(&mut Session, &Event) -> bool) {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            for j in self.s.take_jobs() {
                let j = match j {
                    Job::Start { account, password: None } => Job::Start { account, password: Some(Secret("pw".into())) },
                    j => j,
                };
                self.pool.submit(j);
            }
            assert!(Instant::now() < end, "timed out waiting for {what}");
            let Some(e) = self.pool.wait(Duration::from_millis(200)) else { continue };
            if let Event::Notice { message, .. } = &e {
                eprintln!("note: {message}");
            }
            self.s.apply(e.clone());
            self.s.tick();
            if done(&mut self.s, &e) {
                for j in self.s.take_jobs() {
                    self.pool.submit(j);
                }
                return;
            }
        }
    }

    /// Keep going until the servers have what's queued (a few seconds of quiet).
    fn settle(&mut self, secs: u64) {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            for j in self.s.take_jobs() {
                self.pool.submit(j);
            }
            if let Some(e) = self.pool.wait(Duration::from_millis(200)) {
                self.s.apply(e);
                self.s.tick();
            }
        }
    }

    fn select(&mut self, account: &str, folder: &str, subject: &str) {
        self.s.run("folder.open", &json!({"account": account, "folder": folder})).unwrap();
        let key = self
            .s
            .folder_state(account, folder)
            .messages
            .iter()
            .find(|m| m.subject == subject)
            .map(|m| m.key.clone())
            .unwrap_or_else(|| panic!("no {subject:?} in {folder}"));
        self.s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    }
}

#[test]
#[ignore]
fn real_awkward_names_and_mail_between_accounts() {
    let Ok(imap) = std::env::var("MAILCRAFT_REAL_AWKWARD") else {
        eprintln!("MAILCRAFT_REAL_AWKWARD isn't set: skipped");
        return;
    };
    let imap = imap.trim();
    let pid = std::process::id();
    let (kim, lee) = (format!("kim{pid}"), format!("lee{pid}"));
    let mut other = Imap::connect(imap, &kim);
    // As a server names them: a backslash, quotes, an ampersand (modified UTF-7's own).
    for f in [r#""Back\\slash""#, r#""Say \"hi\"""#, "R&-D", "Trash"] {
        other.try_cmd(&format!("CREATE {f}"));
    }
    other.append("INBOX", "", &message(1));
    other.append("INBOX", "\\Seen \\Flagged to=20do", &message(2));
    other.append("INBOX", "", &message(3));
    let mut theirs = Imap::connect(imap, &lee);

    let dir = std::env::temp_dir().join(format!("mailcraft-real-awkward-{pid}"));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut s = Session::open(Some(store.clone()), Clock::new(now(), 0));
    let (host, port) = imap.rsplit_once(':').unwrap();
    let add = |s: &mut Session, user: &str| {
        s.run(
            "account.add",
            &json!({"email": format!("{user}@test.example"), "username": user, "name": "Kim", "password": "pw", "imapHost": host,
                "imapPort": port.parse::<u16>().unwrap(), "imapSecurity": "none", "smtpHost": host, "smtpPort": 1, "smtpSecurity": "none"}),
        )
        .unwrap()["account"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let a = add(&mut s, &kim);
    let b = add(&mut s, &lee);
    let mut app = App { s, pool: Pool::new(store.clone(), Arc::new(MemoryCredentials::default())) };
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
    assert_eq!(other.subjects(r#""Back\\slash""#), ["Note 1"]);
    app.select(&a, "INBOX", "Note 3");
    app.s.run("mail.move", &json!({"folder": r#"Say "hi""#})).unwrap();
    app.settle(6);
    assert_eq!(other.subjects(r#""Say \"hi\"""#), ["Note 3"]);
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
    let (subject, line) = &there[0];
    assert_eq!(subject, "Note 2");
    assert!(line.contains("\\Seen") && line.contains("\\Flagged") && line.contains("to=20do"), "{line}");
    assert!(line.contains("02-Nov-2026"), "the day it came: {line}");
    assert!(!other.subjects("INBOX").contains(&"Note 2".to_string()), "moved, not copied");
    let _ = std::fs::remove_dir_all(&dir);
}
