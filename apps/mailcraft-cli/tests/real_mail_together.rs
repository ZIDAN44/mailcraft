//! The mail side of v0.31 together, through real servers on this computer (never a real
//! account), run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_TOGETHER="127.0.0.1:21710 127.0.0.1:21725 127.0.0.1:21743" \
//! MAILCRAFT_REAL_TOGETHER_JMAP="http://127.0.0.1:21780 riley pw" \
//!   cargo test -p mailcraft-cli --test real_mail_together -- --ignored --nocapture
//! ```
//!
//! Dovecot (any user, password `pw`, cleartext allowed) for IMAP, GreenMail (any password) for
//! SMTP and a second account's IMAP; then Stalwart over JMAP. In one mailbox: the sender check
//! and External (#29), other programs' keywords as categories and a server search for one (#28),
//! the message source (#27), Shift selections (#26), an invitation answered while it's read in
//! the middle of the list (#31) and named signatures following From (#11).

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mailcraft_engine::Session;
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::model::job::Secret;
use mailcraft_engine::model::{Event, Job, MessageKey};
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

    /// Each message of a folder: its subject and its flags line.
    fn messages(&mut self, folder: &str) -> Vec<(String, String)> {
        self.cmd(&format!("SELECT {folder}"));
        let r = self.try_cmd("FETCH 1:* (FLAGS BODY.PEEK[HEADER.FIELDS (SUBJECT)])");
        self.try_cmd("CLOSE");
        let mut out = Vec::new();
        let mut flags = String::new();
        for l in r.lines() {
            if l.starts_with("* ") && l.contains("FETCH") {
                flags = l.to_string();
            } else if let Some(s) = l.strip_prefix("Subject: ") {
                out.push((s.trim().to_string(), flags.clone()));
            }
        }
        out
    }

    fn subjects(&mut self, folder: &str) -> Vec<String> {
        self.messages(folder).into_iter().map(|(s, _)| s).collect()
    }

    /// The text of the newest message in a folder.
    fn newest_text(&mut self, folder: &str) -> String {
        let r = self.cmd(&format!("SELECT {folder}"));
        let n = r.lines().find_map(|l| l.strip_prefix("* ").and_then(|l| l.strip_suffix(" EXISTS"))).unwrap().to_string();
        let r = self.cmd(&format!("FETCH {n} BODY.PEEK[TEXT]"));
        self.try_cmd("CLOSE");
        r
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// `ago` minutes before now as a `Date:` header (RFC 5322, in UTC).
fn date(ago: i64) -> String {
    let t = now() - ago * 60;
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // Days since 1970 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d} {} {y} {:02}:{:02}:{:02} +0000", MONTHS[(m - 1) as usize], secs / 3600, secs / 60 % 60, secs % 60)
}

/// A plain message (`ago`: minutes before now).
fn message(from: &str, to: &str, subject: &str, ago: i64) -> Vec<u8> {
    let date = date(ago);
    format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <{}.{}@together.example>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{subject}.\r\n",
        subject.replace(' ', "-"),
        std::process::id()
    )
    .into_bytes()
}

/// An invitation from Zoe (version `sequence` of meeting `uid`).
fn invitation(user: &str, uid: &str, sequence: u32, subject: &str, ago: i64) -> Vec<u8> {
    let date = date(ago);
    format!(
        "From: Zoe Park <zoe@studio.example>\r\nTo: Riley <{user}>\r\nSubject: {subject}\r\nMessage-ID: <{uid}.{sequence}@studio.example>\r\n\
         Date: {date}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nGrade review\r\n--b\r\nContent-Type: text/calendar; charset=utf-8; method=REQUEST\r\n\r\n\
         BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//studio.example//EN\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261031T090000Z\r\n\
         DTSTART:20261110T100000Z\r\nDURATION:PT1H\r\nSUMMARY:Grade review\r\nORGANIZER;CN=Zoe Park:mailto:zoe@studio.example\r\n\
         ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{user}\r\nSEQUENCE:{sequence}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n--b--\r\n"
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

    fn key(&mut self, account: &str, folder: &str, subject: &str) -> MessageKey {
        self.s
            .folder_state(account, folder)
            .messages
            .iter()
            .find(|m| m.subject == subject)
            .map(|m| m.key.clone())
            .unwrap_or_else(|| panic!("no {subject:?} in {folder}"))
    }

    fn subject(&self, k: &MessageKey) -> String {
        self.s.summary(k).map(|m| m.subject.clone()).unwrap_or_default()
    }
}

#[test]
#[ignore]
fn real_mail_side_together() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_TOGETHER") else {
        eprintln!("MAILCRAFT_REAL_TOGETHER isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, smtp, gm_imap] = p.as_slice() else { panic!("MAILCRAFT_REAL_TOGETHER: imap smtp greenmail-imap") };
    let pid = std::process::id();
    let user = format!("riley{pid}@studio.example");
    let other_user = format!("riley{pid}@studio.test");
    let mut other = Imap::connect(imap, &user);
    for f in ["Sent", "Trash"] {
        other.try_cmd(&format!("CREATE {f}"));
    }
    // Written to Zoe before: her name is known; the rest of the Inbox.
    other.append("Sent", "\\Seen", &message(&format!("Riley <{user}>"), "Zoe Park <zoe@studio.example>", "Notes for Zoe", 600));
    other.append("INBOX", "", &message("Zoe Park <zoe@studio.example>", &user, "From the real Zoe", 90));
    other.append("INBOX", "", &message("Zoe Park <zoe.park.director@gmail.com>", &user, "Quick favour", 80));
    other.append("INBOX", "$label1 projekt=20x JunkRecorded $MDNSent", &message("Bo <bo@vendor.example>", &user, "Tagged by others", 70));
    other.append("INBOX", "", &message(&format!("Riley <{user}>"), &user, "Note to self", 60));
    let uid = format!("grade-{pid}@studio.example");
    other.append("INBOX", "", &invitation(&user, &uid, 0, "Invitation v0", 50));
    other.append("INBOX", "", &message("Bo <bo@vendor.example>", &user, "Between", 40));
    other.append("INBOX", "", &invitation(&user, &uid, 1, "Invitation v1", 30));
    other.append("INBOX", "", &message("Bo <bo@vendor.example>", &user, "Newest", 20));

    let dir = std::env::temp_dir().join(format!("mailcraft-real-together-{pid}"));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut s = Session::open(Some(store.clone()), Clock::new(now(), 0));
    let (host, port) = imap.rsplit_once(':').unwrap();
    let (smtp_host, smtp_port) = smtp.rsplit_once(':').unwrap();
    let id = s
        .run(
            "account.add",
            &json!({"email": user, "name": "Riley", "password": "pw", "imapHost": host, "imapPort": port.parse::<u16>().unwrap(), "imapSecurity": "none",
                "smtpHost": smtp_host, "smtpPort": smtp_port.parse::<u16>().unwrap(), "smtpSecurity": "none"}),
        )
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let (gm_host, gm_port) = gm_imap.rsplit_once(':').unwrap();
    let id2 = s
        .run(
            "account.add",
            &json!({"email": other_user, "name": "Riley", "password": "pw", "imapHost": gm_host, "imapPort": gm_port.parse::<u16>().unwrap(), "imapSecurity": "none",
                "smtpHost": smtp_host, "smtpPort": smtp_port.parse::<u16>().unwrap(), "smtpSecurity": "none"}),
        )
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.run("options.answeredInvites", &json!({"after": "delete"})).unwrap();
    s.run("view.externalTag", &json!({"on": true})).unwrap();
    let mut app = App { s, pool: Pool::new(store.clone(), Arc::new(MemoryCredentials::default())) };
    app.pump(120, "the Inbox", |s, _| s.folder_state(&id, "INBOX").messages.len() >= 8);
    app.pump(120, "Sent", |s, _| !s.folder_state(&id, "Sent").messages.is_empty());
    // The people written to are counted again once the check is ten minutes old (it was
    // gathered before Sent came).
    app.s.clock.now += 601;
    app.s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();

    // #29 and #28 in one list: the warning and the address, External, other programs' keywords
    // as categories (not Apple's or the IANA's own).
    let r = app.s.run("list.messages", &json!({"limit": 50})).unwrap();
    let row = |subject: &str| r["messages"].as_array().unwrap().iter().find(|m| m["subject"] == subject).cloned().unwrap();
    eprintln!("{}", serde_json::to_string_pretty(&r).unwrap());
    let favour = row("Quick favour");
    assert_eq!(
        (favour["senderWarning"].as_bool(), favour["fromAddress"].as_bool(), favour["external"].as_bool()),
        (Some(true), Some(true), Some(true))
    );
    let real = row("From the real Zoe");
    assert_eq!((real["senderWarning"].as_bool(), real["external"].as_bool()), (Some(false), Some(false)));
    let own = row("Note to self");
    assert_eq!((own["senderWarning"].as_bool(), own["external"].as_bool()), (Some(false), Some(false)), "own mail");
    let tagged = row("Tagged by others");
    assert_eq!(tagged["external"].as_bool(), Some(true));
    let cats: Vec<&str> = tagged["categories"].as_array().unwrap().iter().filter_map(|c| c.as_str()).collect();
    assert!(cats.contains(&"Important") && cats.iter().any(|c| c.eq_ignore_ascii_case("projekt x")), "{cats:?}");
    assert!(!cats.iter().any(|c| c.contains("Junk") || c.contains("MDN")), "{cats:?}");

    // #27: the source of the one that doesn't add up, downloaded.
    let favour = app.key(&id, "INBOX", "Quick favour");
    let src = app.s.run("message.source", &json!({"key": favour.to_id(), "headers": true})).unwrap();
    if src["pending"].as_bool() == Some(true) {
        app.pump(60, "the source", |s, _| s.sources.iter().any(|v| v.key == favour && !v.pending));
    }
    let src = app.s.run("message.source", &json!({"key": favour.to_id(), "headers": true})).unwrap();
    assert!(src["text"].as_str().unwrap().contains("From: Zoe Park <zoe.park.director@gmail.com>"), "{src}");

    // #28 on the server: a search for the Thunderbird tag by name finds it there.
    app.s.run("search.run", &json!({"query": "category:\"Projekt X\"", "scope": "mailbox"})).unwrap();
    app.s.run("search.server", &json!({})).unwrap();
    let mut asked = (String::new(), 0, None);
    app.pump(60, "the server's search", |_, e| match e {
        Event::ServerSearched { query, keys, error, done, .. } => {
            asked = (query.clone(), asked.1 + keys.len(), error.clone());
            *done
        }
        _ => false,
    });
    assert!(asked.0.contains("keyword:projekt=20x") && asked.1 >= 1 && asked.2.is_none(), "the server asked by keyword: {asked:?}");
    let found: Vec<String> = app.s.list.iter().map(|m| m.subject.clone()).collect();
    assert_eq!(found, vec!["Tagged by others".to_string()], "{found:?}");
    app.s.run("search.clear", &json!({})).ok();
    app.s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();

    // #26 then #31: the newest invitation read, between plain mail; answered, it and the earlier
    // version go to Deleted Items on the server, and the list goes on to the next message, from
    // which Shift+Down selects.
    let v1 = app.key(&id, "INBOX", "Invitation v1");
    let v0 = app.key(&id, "INBOX", "Invitation v0");
    for k in [&v0, &v1] {
        app.s.run("list.open", &json!({"key": k.to_id()})).unwrap();
        let k = k.clone();
        app.pump(60, "the invitation's body", |s, _| s.reading.as_ref().is_some_and(|r| r.key == k && r.parsed.is_some()));
    }
    let rows = app.s.row_keys();
    let at = rows.iter().position(|k| *k == v1).unwrap();
    let after = rows[at + 1].clone();
    let r = app.s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 2, "{r}");
    assert_eq!(app.s.reading.as_ref().map(|r| app.subject(&r.key)), Some(app.subject(&after)), "the next one is read");
    app.s.run("list.extendNext", &json!({})).unwrap();
    let picked: Vec<String> = app.s.selection.iter().map(|k| app.subject(k)).collect();
    eprintln!("selected after answering: {picked:?}");
    assert_eq!(picked.len(), 2);
    assert!(!picked.iter().any(|p| p.starts_with("Invitation")), "{picked:?}");
    // Ctrl+A, then a category on all of them: kept on the server as its keyword.
    app.s.run("list.selectAll", &json!({})).unwrap();
    assert!(app.s.reading.is_none(), "several selected: none read");
    app.s.run("mail.categorize", &json!({"category": "Work"})).unwrap();
    app.settle(5);
    let trash = other.subjects("Trash");
    let inbox = other.messages("INBOX");
    eprintln!("Trash: {trash:?}\nInbox: {inbox:?}");
    assert!(trash.contains(&"Invitation v0".to_string()) && trash.contains(&"Invitation v1".to_string()));
    assert!(!inbox.iter().any(|(s, _)| s.starts_with("Invitation")));
    let work = app.s.category("Work").map(|c| c.keyword.clone()).unwrap();
    assert!(inbox.iter().all(|(_, f)| f.to_ascii_lowercase().contains(&work.to_ascii_lowercase())), "{inbox:?}");

    // #11: each account's signature; From changed, the other's follows, and it's what's sent.
    app.s.run("account.setSignature", &json!({"account": id, "name": "Dovecot sig", "signature": "Riley at the Dovecot desk"})).unwrap();
    app.s.run("account.setSignature", &json!({"account": id2, "name": "GreenMail sig", "signature": "Riley at the GreenMail desk"})).unwrap();
    app.s.run("mail.new", &json!({"account": id})).unwrap();
    assert!(app.s.compose.as_ref().unwrap().body.contains("Dovecot desk"));
    app.s.run("compose.update", &json!({"to": other_user, "subject": format!("Signed {pid}"), "account": id2})).unwrap();
    let body = app.s.compose.as_ref().unwrap().body.clone();
    assert!(body.contains("GreenMail desk") && !body.contains("Dovecot desk"), "{body}");
    app.s.run("compose.send", &json!({})).unwrap();
    app.pump(60, "sent", |_, e| matches!(e, Event::Sent { .. }));
    let mut gm = Imap::connect(gm_imap, &other_user);
    let end = Instant::now() + Duration::from_secs(30);
    while !gm.subjects("INBOX").contains(&format!("Signed {pid}")) {
        assert!(Instant::now() < end, "not delivered");
        std::thread::sleep(Duration::from_millis(500));
    }
    let text = gm.newest_text("INBOX");
    assert!(text.contains("GreenMail desk") && !text.contains("Dovecot desk"), "{text}");
    eprintln!("all of it on Dovecot and GreenMail");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An HTTP/1.1 request with a password; its answer's body (`http://host:port/path`).
fn http(method: &str, url: &str, auth: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let rest = url.trim_start_matches("http://");
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let mut s = std::net::TcpStream::connect(host).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let head = format!(
        "{method} /{path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Basic {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        base64(auth.as_bytes()),
        body.len()
    );
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(body).unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).unwrap();
    let at = out.windows(4).position(|w| w == b"\r\n\r\n").map_or(0, |i| i + 4);
    out.split_off(at)
}

fn base64(data: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The same over JMAP (Stalwart: `MAILCRAFT_REAL_TOGETHER_JMAP="http://127.0.0.1:21780 riley
/// pw"`): another program's keyword shows as a category and a search for it by name asks the
/// server for that keyword; the source of a message is downloaded as it came.
#[test]
#[ignore]
fn real_jmap_categories_and_source() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_TOGETHER_JMAP") else {
        eprintln!("MAILCRAFT_REAL_TOGETHER_JMAP isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [base, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_TOGETHER_JMAP: base user password") };
    let auth = format!("{user}:{password}");
    let pid = std::process::id();
    // Put in by another program, with its tag and a junk filter's note.
    let session: serde_json::Value = serde_json::from_slice(&http("GET", &format!("{base}/jmap/session"), &auth, "application/json", b"")).unwrap();
    let account = session["primaryAccounts"]["urn:ietf:params:jmap:mail"].as_str().unwrap().to_string();
    let api = session["apiUrl"].as_str().unwrap().replace("https://", "http://");
    let api = format!("{base}/{}", api.split_once("/jmap").map(|(_, rest)| format!("jmap{rest}")).unwrap_or_else(|| "jmap/".into()));
    let call = |calls: serde_json::Value| -> serde_json::Value {
        let body = json!({"using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"], "methodCalls": calls});
        serde_json::from_slice(&http("POST", &api, &auth, "application/json", body.to_string().as_bytes())).unwrap()
    };
    let r = call(json!([["Mailbox/query", {"accountId": account, "filter": {"role": "inbox"}}, "q"]]));
    let inbox = r["methodResponses"][0][1]["ids"][0].as_str().unwrap().to_string();
    let subject = format!("Tagged over JMAP {pid}");
    let raw = message("Bo <bo@vendor.example>", "riley@studio.example", &subject, 5);
    let up: serde_json::Value =
        serde_json::from_slice(&http("POST", &format!("{base}/jmap/upload/{account}/"), &auth, "message/rfc822", &raw)).unwrap();
    let blob = up["blobId"].as_str().unwrap();
    let r = call(json!([["Email/import", {"accountId": account, "emails": {"m": {"blobId": blob, "mailboxIds": {inbox.clone(): true},
        "keywords": {"projekt=20x": true, "junkrecorded": true}}}}, "i"]]));
    assert!(r["methodResponses"][0][1]["created"]["m"].is_object(), "{r}");

    let dir = std::env::temp_dir().join(format!("mailcraft-real-together-jmap-{pid}"));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut s = Session::open(Some(store.clone()), Clock::new(now(), 0));
    let id = s
        .run("account.add", &json!({"email": "riley@studio.example", "username": user, "password": password, "jmap": format!("{base}/jmap/session")}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let mut app = App { s, pool: Pool::new(store.clone(), Arc::new(MemoryCredentials::default())) };
    let folder = |s: &Session| s.account(&id).and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Inbox)).map(|f| f.path.clone());
    app.pump(120, "the Inbox", |s, _| folder(s).is_some_and(|f| s.folder_state(&id, &f).messages.iter().any(|m| m.subject == subject)));
    let inbox_path = folder(&app.s).unwrap();
    app.s.run("folder.open", &json!({"account": id, "folder": inbox_path})).unwrap();
    let key = app.key(&id, &inbox_path, &subject);
    let m = app.s.summary(&key).cloned().unwrap();
    let cats: Vec<String> = app.s.categories_of(&m).into_iter().map(|c| c.name).collect();
    assert_eq!(cats, vec!["projekt x".to_string()], "{:?}", m.flags.keywords);
    // The source, downloaded.
    let src = app.s.run("message.source", &json!({"key": key.to_id()})).unwrap();
    if src["pending"].as_bool() == Some(true) {
        app.pump(60, "the source", |s, _| s.sources.iter().any(|v| v.key == key && !v.pending));
    }
    let src = app.s.run("message.source", &json!({"key": key.to_id()})).unwrap();
    assert!(src["error"].is_null(), "{src}");
    assert!(src["text"].as_str().unwrap().contains(&format!("Subject: {subject}")), "{src}");
    // A search for the tag by name: the server is asked for its keyword, and finds it.
    app.s.run("search.run", &json!({"query": "category:\"Projekt X\"", "scope": "mailbox"})).unwrap();
    app.s.run("search.server", &json!({})).unwrap();
    let mut asked = (String::new(), 0, None);
    app.pump(60, "the server's search", |_, e| match e {
        Event::ServerSearched { query, keys, error, done, .. } => {
            asked = (query.clone(), asked.1 + keys.len(), error.clone());
            *done
        }
        _ => false,
    });
    assert!(asked.0.contains("keyword:projekt=20x") && asked.1 >= 1 && asked.2.is_none(), "the server asked by keyword: {asked:?}");
    let found: Vec<String> = app.s.list.iter().map(|m| m.subject.clone()).collect();
    assert!(found.contains(&subject), "{found:?}");
    eprintln!("JMAP: categories {cats:?}, the source and a search for the tag");
    let _ = std::fs::remove_dir_all(&dir);
}
