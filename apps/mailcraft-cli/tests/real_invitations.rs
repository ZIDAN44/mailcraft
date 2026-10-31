//! Meeting invitations through a real mail server and a real calendar server on this computer
//! (issues #30 and #31; never a real account), run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_INVITES="127.0.0.1:21510 http://127.0.0.1:21532/riley@studio.example/personal/" //! MAILCRAFT_REAL_INVITES_JMAP="http://127.0.0.1:21508 riley pw" //!   cargo test -p mailcraft-cli --test real_invitations -- --ignored --nocapture
//! ```
//!
//! An IMAP server (Dovecot: any user, password `pw`, with a folder `Trash`) and a CalDAV server
//! (Radicale, any password, with a calendar `personal` made for the user), reached through a
//! proxy here that refuses connections, then answers slowly; the calendar is read straight from
//! the server to check what was stored. Then Stalwart, mail over JMAP and its CalDAV calendar.
//! The engine and the network layer run together, as in the app.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mailcraft_engine::Session;
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::model::job::Secret;
use mailcraft_engine::model::pim::LOCAL;
use mailcraft_engine::model::{Event, Job, SyncStatus};
use mailcraft_engine::store::Store;
use mailcraft_net::worker::{MemoryCredentials, Pool};
use serde_json::json;

const USER: &str = "riley@studio.example";

/// A tiny IMAP client for the other side: put mail in, list what a folder holds.
struct Imap {
    s: std::net::TcpStream,
    n: u32,
}

impl Imap {
    fn connect(addr: &str) -> Imap {
        let s = std::net::TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        let mut c = Imap { s, n: 0 };
        c.read_until("* OK");
        c.cmd(&format!("LOGIN \"{USER}\" pw"));
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

    fn cmd(&mut self, c: &str) -> String {
        self.n += 1;
        let tag = format!("t{}", self.n);
        write!(self.s, "{tag} {c}\r\n").unwrap();
        let r = self.read_until(&format!("{tag} "));
        assert!(r.lines().any(|l| l.starts_with(&format!("{tag} OK"))), "{c}: {r}");
        r
    }

    fn append(&mut self, folder: &str, raw: &[u8]) {
        self.n += 1;
        let tag = format!("t{}", self.n);
        write!(self.s, "{tag} APPEND {folder} {{{}+}}\r\n", raw.len()).unwrap();
        self.s.write_all(raw).unwrap();
        self.s.write_all(b"\r\n").unwrap();
        let r = self.read_until(&format!("{tag} "));
        assert!(r.contains(&format!("{tag} OK")), "{r}");
    }

    /// The subjects in a folder.
    fn subjects(&mut self, folder: &str) -> Vec<String> {
        self.cmd(&format!("SELECT {folder}"));
        let r = self.cmd("FETCH 1:* (BODY.PEEK[HEADER.FIELDS (SUBJECT)])");
        let found = r.lines().filter_map(|l| l.strip_prefix("Subject: ")).map(|s| s.trim().to_string()).collect();
        self.cmd("UNSELECT");
        found
    }
}

/// A proxy in front of the calendar server, on a port of its own: it refuses connections
/// (`None`) or passes them on after so many seconds.
fn proxy(target: &str) -> (u16, Arc<std::sync::Mutex<Option<u64>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mode = Arc::new(std::sync::Mutex::new(None));
    let (m, target) = (mode.clone(), target.to_string());
    std::thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let Some(delay) = *m.lock().unwrap() else { continue };
            let target = target.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(delay));
                let Ok(server) = std::net::TcpStream::connect(&target) else { return };
                let (mut c2, mut s2) = (client.try_clone().unwrap(), server.try_clone().unwrap());
                let (mut client, mut server) = (client, server);
                let up = std::thread::spawn(move || {
                    let _ = std::io::copy(&mut c2, &mut s2);
                    let _ = s2.shutdown(std::net::Shutdown::Write);
                });
                let _ = std::io::copy(&mut server, &mut client);
                let _ = client.shutdown(std::net::Shutdown::Both);
                let _ = up.join();
            });
        }
    });
    (port, mode)
}

/// An invitation from Zoe to the user, as a calendar service would send it.
fn invitation(uid: &str, summary: &str, start: &str) -> Vec<u8> {
    format!(
        "From: Zoe Park <zoe@organizer.example>\r\nTo: Riley <{USER}>\r\nSubject: Invitation: {summary}\r\nMessage-ID: <{uid}.mail@organizer.example>\r\n\
         Date: Sat, 31 Oct 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{summary}\r\n--b\r\nContent-Type: text/calendar; charset=utf-8; method=REQUEST\r\n\r\n\
         BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//organizer.example//EN\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261031T090000Z\r\n\
         DTSTART:{start}\r\nDURATION:PT1H\r\nSUMMARY:{summary}\r\nORGANIZER;CN=Zoe Park:mailto:zoe@organizer.example\r\n\
         ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{USER}\r\nSEQUENCE:0\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n--b--\r\n"
    )
    .into_bytes()
}

/// The calendar's items, read straight from the server (no proxy).
fn stored(calendar: &str) -> String {
    let (host, path) = calendar.trim_start_matches("http://").split_once('/').unwrap();
    let body = "<?xml version=\"1.0\"?><c:calendar-query xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><c:calendar-data/></d:prop><c:filter><c:comp-filter name=\"VCALENDAR\"/></c:filter></c:calendar-query>";
    let mut s = std::net::TcpStream::connect(host).unwrap();
    write!(
        s,
        "REPORT /{path} HTTP/1.1\r\nHost: {host}\r\nDepth: 1\r\nAuthorization: Basic {}\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        base64(format!("{USER}:pw").as_bytes()),
        body.len()
    )
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

/// The session and the network layer together, as in the app: events in, jobs out (a Start
/// gets the password, as the keychain would give it).
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
            match &e {
                Event::Notice { message, .. } => eprintln!("note: {message}"),
                Event::Status { status, .. } => eprintln!("status: {status:?}"),
                Event::DavSynced { sync, .. } => {
                    eprintln!("calendars: {:?}, {} events stored", sync.calendars.as_ref().map(Vec::len), sync.pushed_events.len());
                    // Each item stored once ("changed on the server meanwhile" said a second sync
                    // sent them again).
                    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
                }
                Event::FolderChanged { folder, .. } => eprintln!("changed: {folder}"),
                Event::Body { key, .. } => eprintln!("body: {}", key.to_id()),
                _ => {}
            }
            // Asked once the event is taken in.
            self.s.apply(e.clone());
            self.s.tick();
            let hit = done(&mut self.s, &e);
            if hit {
                for j in self.s.take_jobs() {
                    self.pool.submit(j);
                }
                return;
            }
        }
    }
}

#[test]
#[ignore]
fn real_invitations_wait_for_the_calendar_and_leave_the_inbox() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_INVITES") else {
        eprintln!("MAILCRAFT_REAL_INVITES isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, calendar] = p.as_slice() else { panic!("MAILCRAFT_REAL_INVITES: imap calendar") };
    let target = calendar.trim_start_matches("http://").split('/').next().unwrap().to_string();
    let (port, mode) = proxy(&target);
    let set_mode = |m: Option<u64>| *mode.lock().unwrap() = m;
    let home = calendar.trim_end_matches('/').rsplit_once('/').unwrap().0.to_string() + "/";
    let proxied = home.replace(&target, &format!("127.0.0.1:{port}"));
    let (host, port) = imap.rsplit_once(':').unwrap();
    let dir = std::env::temp_dir().join(format!("mailcraft-real-invites-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut s = Session::open(Some(store.clone()), Clock::new(chrono_now(), 0));
    let id = s
        .run(
            "account.add",
            &json!({"email": USER, "password": "pw", "imapHost": host, "imapPort": port.parse::<u16>().unwrap(), "imapSecurity": "none", "smtpHost": "127.0.0.1", "smtpPort": 1, "smtpSecurity": "none"}),
        )
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.run("options.answeredInvites", &json!({"after": "delete"})).unwrap();
    let mut app = App { s, pool: Pool::new(store.clone(), Arc::new(MemoryCredentials::default())) };
    let mut other = Imap::connect(imap);

    // 1. No calendar server found from the address (studio.example has none): the account's
    //    calendar is on this computer, and an invitation that comes goes there, as it should.
    set_mode(None);
    app.pump(120, "calendars looked for", |_, e| matches!(e, Event::DavSynced { .. }));
    assert_eq!(app.s.pim[&id].caldav_home.as_deref(), Some(""), "none found");
    other.append("INBOX", &invitation("one@organizer.example", "Colour pass", "20261105T100000Z"));
    app.pump(120, "the first invitation", |s, _| s.find_event("one@organizer.example").is_some());
    assert_eq!(app.s.find_event("one@organizer.example").unwrap().1.calendar, LOCAL);
    eprintln!("1: the first invitation is in the calendar on this computer");

    // 2. The calendar server given by hand (issue #7), but it can't be reached yet: what
    //    arrives now is held, then after a while put on this computer too.
    app.s.run("account.dav", &json!({"account": id, "caldav": proxied})).unwrap();
    app.pump(120, "online again", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(app.s.calendars_pending(&id), "still to be found");
    other.append("INBOX", &invitation("two@organizer.example", "Grade review", "20261106T140000Z"));
    app.pump(120, "the second invitation held", |s, _| !s.pim[&id].held.is_empty());
    assert!(app.s.find_event("two@organizer.example").is_none(), "held, not on this computer's calendar");
    eprintln!("2: the second invitation is held");
    app.s.clock.now += 181;
    app.s.tick();
    assert_eq!(app.s.find_event("two@organizer.example").unwrap().1.calendar, LOCAL, "waited long enough: here");
    other.append("INBOX", &invitation("three@organizer.example", "Mix review", "20261107T150000Z"));
    app.pump(120, "the third invitation held", |s, _| !s.pim[&id].held.is_empty());

    // 3. The server answers, slowly: the held one goes to its calendar, those put here move
    //    there, and both are stored on the server.
    set_mode(Some(3));
    app.pool.submit(Job::SyncDav { account: id.clone() });
    app.pump(180, "the calendars found", |_, e| matches!(e, Event::DavSynced { sync, .. } if sync.calendars.is_some()));
    for uid in ["one@organizer.example", "two@organizer.example", "three@organizer.example"] {
        let (_, e) = app.s.find_event(uid).unwrap();
        assert!(e.calendar.ends_with("/personal/"), "{uid} in {}", e.calendar);
    }
    assert!(app.s.pim[&id].held.is_empty() && app.s.pim[&id].placed.is_empty());
    app.pump(180, "stored on the server", |s, _| s.pim[&id].events.iter().all(|e| !e.dirty && !e.href.is_empty()));
    let ics = stored(calendar);
    for summary in ["Colour pass", "Grade review", "Mix review"] {
        assert!(ics.contains(&format!("SUMMARY:{summary}")), "{summary} not on the server:\n{ics}");
    }
    eprintln!("3: all three are in the server's calendar");

    // 4. Issue #31: answered from the calendar, the invitation leaves the Inbox on the server
    //    (Deleted Items); answered in the message, likewise; the answer is stored.
    app.s.run("event.respond", &json!({"uid": "one@organizer.example", "answer": "accepted"})).unwrap();
    app.pump(120, "the move done", |s, _| s.folder_state(&id, "Trash").messages.iter().any(|m| m.subject == "Invitation: Colour pass"));
    let key = app.s.folder_state(&id, "INBOX").messages.iter().find(|m| m.subject == "Invitation: Grade review").map(|m| m.key.to_id()).unwrap();
    app.s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    app.s.run("list.open", &json!({"key": key})).unwrap();
    let r = app.s.run("invite.respond", &json!({"answer": "tentative"})).unwrap();
    assert_eq!(r["moved"], 1, "{r}");
    app.pump(120, "the answers stored", |s, _| s.pim[&id].events.iter().all(|e| !e.dirty));
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let inbox = other.subjects("INBOX");
        let trash = other.subjects("Trash");
        if trash.len() >= 2 {
            assert!(inbox.iter().all(|s| !s.contains("Colour pass") && !s.contains("Grade review")), "{inbox:?}");
            assert!(inbox.iter().any(|s| s.contains("Mix review")), "the unanswered one stays: {inbox:?}");
            eprintln!("4: on the server, Inbox {inbox:?}, Trash {trash:?}");
            break;
        }
        assert!(Instant::now() < end, "not moved on the server: Inbox {inbox:?}, Trash {trash:?}");
        let _ = app.pool.wait(Duration::from_millis(500));
    }
    let ics = stored(calendar);
    assert!(ics.contains("PARTSTAT=ACCEPTED") && ics.contains("PARTSTAT=TENTATIVE"), "{ics}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Now, in Unix seconds.
fn chrono_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// An HTTP/1.1 request with a password, its answer's body (`http://host:port/path`).
fn http(method: &str, url: &str, auth: &str, content_type: &str, body: &[u8]) -> String {
    let rest = url.trim_start_matches("http://");
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let mut s = std::net::TcpStream::connect(host).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    write!(
        s,
        "{method} /{path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Basic {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        base64(auth.as_bytes()),
        body.len()
    )
    .unwrap();
    s.write_all(body).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default()
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

/// A JMAP server's mail, from the other side: put a message in the Inbox, list a mailbox.
struct Jmap {
    api: String,
    upload: String,
    account: String,
    auth: String,
}

impl Jmap {
    fn connect(base: &str, auth: &str) -> Jmap {
        let session: serde_json::Value = serde_json::from_str(&http("GET", &format!("{base}/jmap/session"), auth, "application/json", b"")).unwrap();
        let account = session["primaryAccounts"]["urn:ietf:params:jmap:mail"].as_str().unwrap().to_string();
        let upload = session["uploadUrl"].as_str().unwrap().replace("{accountId}", &account);
        Jmap { api: session["apiUrl"].as_str().unwrap().to_string(), upload, account, auth: auth.into() }
    }

    fn call(&self, calls: serde_json::Value) -> serde_json::Value {
        let req = json!({"using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"], "methodCalls": calls});
        serde_json::from_str(&http("POST", &self.api, &self.auth, "application/json", req.to_string().as_bytes())).unwrap()
    }

    fn mailbox(&self, role: &str) -> String {
        let r = self.call(json!([["Mailbox/get", {"accountId": self.account, "ids": null}, "m"]]));
        r["methodResponses"][0][1]["list"].as_array().unwrap().iter().find(|m| m["role"] == role).unwrap()["id"].as_str().unwrap().to_string()
    }

    fn deliver(&self, raw: &[u8]) {
        let blob: serde_json::Value = serde_json::from_str(&http("POST", &self.upload, &self.auth, "message/rfc822", raw)).unwrap();
        let inbox = self.mailbox("inbox");
        let r = self.call(
            json!([["Email/import", {"accountId": self.account, "emails": {"a": {"blobId": blob["blobId"], "mailboxIds": {inbox: true}}}}, "i"]]),
        );
        assert!(r["methodResponses"][0][1]["created"]["a"].is_object(), "{r}");
    }

    fn subjects(&self, role: &str) -> Vec<String> {
        let mb = self.mailbox(role);
        let r = self.call(json!([
            ["Email/query", {"accountId": self.account, "filter": {"inMailbox": mb}}, "q"],
            ["Email/get", {"accountId": self.account, "#ids": {"resultOf": "q", "name": "Email/query", "path": "/ids"}, "properties": ["subject"]}, "g"]
        ]));
        r["methodResponses"][1][1]["list"].as_array().unwrap().iter().filter_map(|e| e["subject"].as_str().map(str::to_string)).collect()
    }
}

/// The same through JMAP and a CalDAV calendar on one server (Stalwart:
/// `MAILCRAFT_REAL_INVITES_JMAP="http://127.0.0.1:21508 riley pw"`): the invitation goes to the
/// account's calendar there; answered from the calendar, it's moved to Deleted Items on the
/// server by JMAP.
#[test]
#[ignore]
fn real_jmap_answered_invitation_leaves_the_inbox() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_INVITES_JMAP") else {
        eprintln!("MAILCRAFT_REAL_INVITES_JMAP isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [base, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_INVITES_JMAP: base user password") };
    let auth = format!("{user}:{password}");
    let other = Jmap::connect(base, &auth);
    let dir = std::env::temp_dir().join(format!("mailcraft-real-invites-jmap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut s = Session::open(Some(store.clone()), Clock::new(chrono_now(), 0));
    let id = s
        .run("account.add", &json!({"email": USER, "username": user, "password": password, "jmap": format!("{base}/jmap/session")}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.run("account.dav", &json!({"account": id, "caldav": format!("{base}/dav/cal/"), "username": user, "password": password})).unwrap();
    s.run("options.answeredInvites", &json!({"after": "delete"})).unwrap();
    let mut app = App { s, pool: Pool::new(store.clone(), Arc::new(MemoryCredentials::default())) };
    app.pump(120, "calendars found", |_, e| matches!(e, Event::DavSynced { sync, .. } if sync.calendars.is_some()));
    let calendar = app.s.pim[&id].default_calendar().map(|c| c.id.clone()).unwrap();
    eprintln!("calendars: {:?}", app.s.pim[&id].calendars.iter().map(|c| (&c.id, &c.name, c.foreign)).collect::<Vec<_>>());
    let uid = format!("jmap-{}@organizer.example", std::process::id());
    let summary = format!("Sound check {}", std::process::id());
    let subject = format!("Invitation: {summary}");
    other.deliver(&invitation(&uid, &summary, "20261108T100000Z"));
    // Stalwart pushes no change for a message imported (as one delivered would be): looked at as
    // Send/Receive would.
    let inbox = app.s.account(&id).and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Inbox)).map(|f| f.path.clone()).unwrap();
    app.pool.submit(Job::SyncFolder { account: id.clone(), folder: inbox });
    app.pump(120, "the invitation", |s, _| s.find_event(&uid).is_some());
    assert_eq!(app.s.find_event(&uid).unwrap().1.calendar, calendar);
    app.pump(120, "stored on the server", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    app.s.run("event.respond", &json!({"uid": uid, "answer": "accepted"})).unwrap();
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        for j in app.s.take_jobs() {
            app.pool.submit(j);
        }
        if let Some(e) = app.pool.wait(Duration::from_millis(300)) {
            app.s.apply(e);
        }
        let trash = other.subjects("trash");
        if trash.contains(&subject) {
            assert!(!other.subjects("inbox").contains(&subject));
            eprintln!("moved on the server: Trash {trash:?}");
            break;
        }
        assert!(Instant::now() < end, "not moved: Inbox {:?}", other.subjects("inbox"));
    }
    let (_, e) = app.s.find_event(&uid).unwrap();
    let ics = http("GET", &e.href, &auth, "text/plain", b"");
    assert!(ics.contains(&format!("SUMMARY:{summary}")) && ics.contains("PARTSTAT=ACCEPTED"), "{ics}");
    eprintln!(
        "stored on the server, accepted:
{ics}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
