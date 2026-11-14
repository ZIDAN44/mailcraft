//! What every real-server test shares, whatever the servers: a tiny IMAP, SMTP and HTTP client
//! for the other side, a proxy that can hold a server back, and MailCraft's session and network
//! layer together, as in the app. Taken in by `#[path]` (`real_*.rs`; the Mailcow tests through
//! `support/mailcow.rs`).

#![allow(dead_code)]

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mailcraft_engine::Session;
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::model::job::Secret;
use mailcraft_engine::model::{Event, Job, MessageKey};
use mailcraft_engine::store::Store;
use mailcraft_net::worker::{MemoryCredentials, Pool};
use serde_json::{Value, json};

/// A tiny IMAP client for the other side: put mail in, see what a folder holds.
pub struct Imap {
    pub s: std::net::TcpStream,
    pub n: u32,
}

/// A message in a folder, as the server has it.
#[derive(Debug)]
pub struct Held {
    pub uid: u32,
    pub flags: Vec<String>,
    pub subject: String,
    /// Its whole `FETCH` line (the day it came is there, as `INTERNALDATE`).
    pub line: String,
}

/// A folder's name or a user's as an IMAP quoted string.
pub fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Imap {
    pub fn connect(addr: &str, user: &str, password: &str) -> Imap {
        let s = std::net::TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut c = Imap { s, n: 0 };
        c.read_until("* OK");
        c.cmd(&format!("LOGIN {} {}", quote(user), quote(password)));
        c
    }

    pub fn read_until(&mut self, end: &str) -> String {
        let mut out = Vec::new();
        let mut b = [0u8; 8192];
        while !String::from_utf8_lossy(&out).lines().any(|l| l.starts_with(end)) {
            let n = self.s.read(&mut b).unwrap();
            assert!(n > 0, "the IMAP server closed");
            out.extend_from_slice(&b[..n]);
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// A command and its whole answer, whatever it says.
    pub fn try_cmd(&mut self, c: &str) -> String {
        self.n += 1;
        let tag = format!("t{}", self.n);
        write!(self.s, "{tag} {c}\r\n").unwrap();
        self.read_until(&format!("{tag} "))
    }

    pub fn cmd(&mut self, c: &str) -> String {
        let r = self.try_cmd(c);
        let tag = format!("t{} OK", self.n);
        assert!(r.lines().any(|l| l.starts_with(&tag)), "{c}: {r}");
        r
    }

    /// Make a folder; one already there is left as it is.
    pub fn create(&mut self, folder: &str) {
        self.try_cmd(&format!("CREATE {}", quote(folder)));
    }

    /// Put a message in a folder, with these flags (`\Seen $label1`, or none).
    pub fn append(&mut self, folder: &str, flags: &str, raw: &[u8]) {
        self.n += 1;
        let tag = format!("t{}", self.n);
        write!(self.s, "{tag} APPEND {} ({flags}) {{{}+}}\r\n", quote(folder), raw.len()).unwrap();
        self.s.write_all(raw).unwrap();
        self.s.write_all(b"\r\n").unwrap();
        let r = self.read_until(&format!("{tag} "));
        assert!(r.contains(&format!("{tag} OK")), "{r}");
    }

    /// Each message of a folder, looked at without changing it (`EXAMINE`: nothing marked
    /// deleted is taken away by looking).
    pub fn messages(&mut self, folder: &str) -> Vec<Held> {
        self.cmd(&format!("EXAMINE {}", quote(folder)));
        let r = self.try_cmd("UID FETCH 1:* (FLAGS INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT)])");
        self.cmd("CLOSE");
        let mut out = Vec::new();
        let mut cur: Option<(u32, Vec<String>, String)> = None;
        for l in r.lines() {
            if l.starts_with("* ") && l.contains("FETCH (") {
                let uid = between(l, "UID ", " ").or_else(|| between(l, "UID ", ")")).and_then(|u| u.parse().ok()).unwrap_or(0);
                let flags = between(l, "FLAGS (", ")").unwrap_or_default().split_whitespace().map(str::to_string).collect();
                cur = Some((uid, flags, l.to_string()));
            } else if let Some(s) = l.strip_prefix("Subject: ")
                && let Some((uid, flags, line)) = cur.take()
            {
                out.push(Held { uid, flags, subject: s.trim().to_string(), line });
            }
        }
        out
    }

    pub fn subjects(&mut self, folder: &str) -> Vec<String> {
        self.messages(folder).into_iter().map(|m| m.subject).collect()
    }

    /// The flags of the message with this subject.
    pub fn flags(&mut self, folder: &str, subject: &str) -> Vec<String> {
        self.messages(folder).into_iter().find(|m| m.subject == subject).map(|m| m.flags).unwrap_or_else(|| panic!("no {subject:?} in {folder}"))
    }

    /// The text of the newest message in a folder (the server's whole answer).
    pub fn newest_text(&mut self, folder: &str) -> String {
        let r = self.cmd(&format!("EXAMINE {}", quote(folder)));
        let n = r.lines().find_map(|l| l.strip_prefix("* ").and_then(|l| l.strip_suffix(" EXISTS"))).unwrap().to_string();
        let r = self.cmd(&format!("FETCH {n} BODY.PEEK[TEXT]"));
        self.cmd("CLOSE");
        r
    }
}

/// Send through SMTP with a password (AUTH PLAIN, no TLS: a server on this computer).
pub fn smtp_send(addr: &str, user: &str, password: &str, to: &[&str], raw: &[u8]) {
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let read = |s: &mut std::net::TcpStream| {
        let mut out = String::new();
        let mut b = [0u8; 4096];
        loop {
            let n = s.read(&mut b).unwrap();
            assert!(n > 0, "the SMTP server closed: {out}");
            out.push_str(&String::from_utf8_lossy(&b[..n]));
            // The last line of a reply has a space after its code.
            if out.ends_with("\r\n") && out.trim_end().lines().last().is_some_and(|l| l.as_bytes().get(3) == Some(&b' ')) {
                return out;
            }
        }
    };
    let say = |s: &mut std::net::TcpStream, line: &str, want: &str| {
        s.write_all(line.as_bytes()).unwrap();
        let r = read(s);
        assert!(r.trim_end().lines().last().is_some_and(|l| l.starts_with(want)), "{line:?}: {r}");
    };
    let hello = read(&mut s);
    assert!(hello.starts_with("220"), "{hello}");
    say(&mut s, "EHLO test.local\r\n", "250");
    say(&mut s, &format!("AUTH PLAIN {}\r\n", base64(format!("\0{user}\0{password}").as_bytes())), "235");
    say(&mut s, &format!("MAIL FROM:<{user}>\r\n"), "250");
    for t in to {
        say(&mut s, &format!("RCPT TO:<{t}>\r\n"), "250");
    }
    say(&mut s, "DATA\r\n", "354");
    let mut body = Vec::new();
    for line in raw.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.first() == Some(&b'.') {
            body.push(b'.');
        }
        body.extend_from_slice(line);
        body.extend_from_slice(b"\r\n");
    }
    s.write_all(&body).unwrap();
    say(&mut s, ".\r\n", "250");
    let _ = s.write_all(b"QUIT\r\n");
}

/// An HTTP answer.
pub struct Answer {
    pub status: u16,
    pub head: String,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The `name=value` of each `Set-Cookie`.
    pub fn cookies(&self) -> Vec<String> {
        self.head
            .lines()
            .filter_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().to_string()))
            .filter_map(|v| v.split(';').next().map(str::to_string))
            .collect()
    }
}

/// One HTTP/1.1 request over plain http (`http://host:port/path`); chunked answers are joined.
pub fn http(method: &str, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Answer {
    let rest = url.trim_start_matches("http://");
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let mut s = std::net::TcpStream::connect(host).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let mut req = format!("{method} /{path} HTTP/1.1\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        req.push_str(&format!("Host: {host}\r\n"));
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    s.write_all(body).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").map_or(out.len(), |i| i + 4);
    let head = String::from_utf8_lossy(&out[..split]).into_owned();
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let mut body = out[split.min(out.len())..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        let mut joined = Vec::new();
        let mut rest = body.as_slice();
        while let Some(eol) = rest.windows(2).position(|w| w == b"\r\n") {
            let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..eol]).split(';').next().unwrap_or("0").trim(), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            let start = eol + 2;
            joined.extend_from_slice(&rest[start..(start + size).min(rest.len())]);
            rest = &rest[(start + size + 2).min(rest.len())..];
        }
        body = joined;
    }
    Answer { status, head, body }
}

/// `Basic` authorization for `user:password`.
pub fn basic(user_password: &str) -> String {
    format!("Basic {}", base64(user_password.as_bytes()))
}

/// A request with a password (`user:password`) and a body of this type.
pub fn http_as(method: &str, url: &str, user_password: &str, content_type: &str, body: &[u8]) -> Answer {
    http(method, url, &[("Authorization", &basic(user_password)), ("Content-Type", content_type)], body)
}

pub fn base64(data: &[u8]) -> String {
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

pub fn between(s: &str, a: &str, b: &str) -> Option<String> {
    let i = s.find(a)? + a.len();
    let j = s.get(i..)?.find(b)? + i;
    s.get(i..j).map(str::to_string)
}

pub fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#13;", "").replace("&amp;", "&")
}

/// MailCraft: the session and the network layer together, as in the app (a Start gets the
/// password, as the keychain would give it). Its files are in a folder of its own, taken away
/// when it's dropped.
pub struct App {
    pub s: Session,
    pub pool: Pool,
    pub password: String,
    pub dir: std::path::PathBuf,
    /// Shown each event before the session takes it in (a test's own logging or checks).
    pub watch: fn(&Event),
}

impl App {
    /// A MailCraft with no account yet.
    pub fn open(tag: &str, password: &str) -> App {
        let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("real-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let s = Session::open(Some(store.clone()), Clock::new(now(), 0));
        App { s, pool: Pool::new(store, Arc::new(MemoryCredentials::default())), password: password.to_string(), dir, watch: |_| {} }
    }

    /// A new MailCraft for `email` (signing in as `username`), its mail on these IMAP and SMTP
    /// servers without TLS, on this computer; returns the account's id.
    pub fn at(imap: &str, smtp: &str, email: &str, username: &str, password: &str, tag: &str) -> (App, String) {
        let name = email.split('@').next().unwrap_or_default();
        let mut app = App::open(&format!("{tag}-{name}"), password);
        let id = app.add_plain(imap, smtp, json!({"email": email, "username": username, "password": password}));
        (app, id)
    }

    /// Add an account (`account.add`'s params); returns its id.
    pub fn add(&mut self, params: Value) -> String {
        self.s.run("account.add", &params).unwrap()["account"].as_str().unwrap().to_string()
    }

    /// Add an account on these IMAP and SMTP servers (`host:port`, without TLS), with the rest
    /// of `account.add`'s params from `more`; returns its id.
    pub fn add_plain(&mut self, imap: &str, smtp: &str, more: Value) -> String {
        let (ih, ip) = imap.rsplit_once(':').unwrap();
        let (sh, sp) = smtp.rsplit_once(':').unwrap();
        let mut params = json!({"imapHost": ih, "imapPort": ip.parse::<u16>().unwrap(), "imapSecurity": "none",
                                "smtpHost": sh, "smtpPort": sp.parse::<u16>().unwrap(), "smtpSecurity": "none"});
        if let (Some(p), Value::Object(more)) = (params.as_object_mut(), more) {
            p.extend(more);
        }
        self.add(params)
    }

    fn submit_jobs(&mut self) {
        for j in self.s.take_jobs() {
            let j = match j {
                Job::Start { account, password: None } => Job::Start { account, password: Some(Secret(self.password.clone())) },
                j => j,
            };
            self.pool.submit(j);
        }
    }

    pub fn pump(&mut self, secs: u64, what: &str, mut done: impl FnMut(&mut Session, &Event) -> bool) {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            self.submit_jobs();
            assert!(Instant::now() < end, "timed out waiting for {what}");
            let Some(e) = self.pool.wait(Duration::from_millis(200)) else { continue };
            match &e {
                Event::Notice { message, .. } => eprintln!("note: {message}"),
                Event::OpFailed { error, .. } => eprintln!("refused: {error}"),
                Event::DavSynced { sync, .. } if !sync.errors.is_empty() => eprintln!("calendar sync: {:?}", sync.errors),
                _ => {}
            }
            (self.watch)(&e);
            self.s.apply(e.clone());
            self.s.tick();
            if done(&mut self.s, &e) {
                self.submit_jobs();
                return;
            }
        }
    }

    /// Keep going for `secs` (what's queued reaches the servers).
    pub fn settle(&mut self, secs: u64) {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            self.submit_jobs();
            if let Some(e) = self.pool.wait(Duration::from_millis(200)) {
                if let Event::DavSynced { sync, .. } = &e
                    && !sync.errors.is_empty()
                {
                    eprintln!("calendar sync: {:?}", sync.errors);
                }
                (self.watch)(&e);
                self.s.apply(e);
                self.s.tick();
            }
        }
    }

    /// Sync the account's calendars and wait for that sync: one already under way when asked
    /// (begun before `t0`) may have read the server before the change looked for.
    pub fn sync_dav(&mut self, id: &str) -> mailcraft_engine::model::pim::DavSync {
        self.submit_jobs();
        let t0 = now();
        self.pool.submit(Job::SyncDav { account: id.to_string() });
        let mut got = None;
        self.pump(180, "a calendar sync", |_, e| match e {
            Event::DavSynced { sync, .. } if sync.checked >= t0 => {
                got = Some((**sync).clone());
                true
            }
            _ => false,
        });
        got.unwrap_or_default()
    }

    /// Wait for a message in the Inbox (syncing it now and then).
    pub fn wait_for_mail(&mut self, id: &str, subject: &str) -> MessageKey {
        let end = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(k) = self.s.folder_state(id, "INBOX").messages.iter().find(|m| m.subject == subject).map(|m| m.key.clone()) {
                return k;
            }
            assert!(Instant::now() < end, "{subject:?} didn't come");
            self.pool.submit(Job::SyncFolder { account: id.to_string(), folder: "INBOX".into() });
            self.settle(2);
        }
    }

    /// The message with this subject in a folder here.
    pub fn key(&mut self, account: &str, folder: &str, subject: &str) -> MessageKey {
        self.s
            .folder_state(account, folder)
            .messages
            .iter()
            .find(|m| m.subject == subject)
            .map(|m| m.key.clone())
            .unwrap_or_else(|| panic!("no {subject:?} in {folder}"))
    }

    /// Open a folder and select the message with this subject in it.
    pub fn select(&mut self, account: &str, folder: &str, subject: &str) {
        self.s.run("folder.open", &json!({"account": account, "folder": folder})).unwrap();
        let key = self.key(account, folder, subject);
        self.s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    }

    pub fn subject(&self, k: &MessageKey) -> String {
        self.s.summary(k).map(|m| m.subject.clone()).unwrap_or_default()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Now, in Unix seconds.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A proxy in front of a server, on a port of its own: it refuses connections (`None`) or
/// passes them on after so many seconds. Returns its port and its switch.
pub fn proxy(target: &str) -> (u16, Arc<std::sync::Mutex<Option<u64>>>) {
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
