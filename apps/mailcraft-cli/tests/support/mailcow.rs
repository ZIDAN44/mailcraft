//! What the Mailcow tests share (`real_mailcow_*.rs`): the servers and users from the
//! environment, a tiny IMAP and SMTP client and HTTP client for the other side, SOGo's web
//! endpoints signed in through Mailcow's own sign-in page, and MailCraft's session and network
//! layer together, as in the app, one per user.
//!
//! `MAILCRAFT_REAL_MAILCOW="<imap> <submission> <web> <user>:<password> <user>:<password> <user>:<password>"`:
//! Dovecot's IMAP port without TLS (Mailcow's `data/conf/dovecot/extra.conf` with
//! `disable_plaintext_auth = no`), Postfix's port 588 (SOGo's own submission, which takes a
//! password without TLS), and Mailcow's web server over plain http (`HTTP_REDIRECT=n`), all on
//! this computer: MailCraft trusts only real certificates, and a test server has none. Three
//! mailboxes of one domain: the owner of the shared mailbox and calendar (ann), a colleague who
//! may change them (bob) and one who may only read (carol).

#![allow(dead_code)]

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mailcraft_engine::Session;
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::model::job::Secret;
use mailcraft_engine::model::{Event, Job};
use mailcraft_engine::store::Store;
use mailcraft_net::worker::{MemoryCredentials, Pool};
use serde_json::{Value, json};

/// The servers and users from `MAILCRAFT_REAL_MAILCOW`, or `None` (the test is skipped).
pub struct Mailcow {
    pub imap: String,
    pub smtp: String,
    pub web: String,
    pub users: Vec<(String, String)>,
}

impl Mailcow {
    pub fn from_env() -> Option<Mailcow> {
        let Ok(v) = std::env::var("MAILCRAFT_REAL_MAILCOW") else {
            eprintln!("MAILCRAFT_REAL_MAILCOW isn't set: skipped");
            return None;
        };
        let p: Vec<&str> = v.split_whitespace().collect();
        let [imap, smtp, web, users @ ..] = p.as_slice() else { panic!("MAILCRAFT_REAL_MAILCOW: imap smtp web user:password…") };
        let users: Vec<(String, String)> =
            users.iter().map(|u| u.split_once(':').map(|(a, b)| (a.to_string(), b.to_string())).expect("user:password")).collect();
        assert!(users.len() >= 3, "three users: the owner, one who may write, one who may read");
        Some(Mailcow { imap: imap.to_string(), smtp: smtp.to_string(), web: web.trim_end_matches('/').to_string(), users })
    }

    pub fn user(&self, n: usize) -> (&str, &str) {
        let (u, p) = &self.users[n];
        (u.as_str(), p.as_str())
    }

    /// A user's address without its domain (`ann`).
    pub fn name(&self, n: usize) -> &str {
        self.users[n].0.split('@').next().unwrap_or_default()
    }

    pub fn imap_as(&self, n: usize) -> Imap {
        let (u, p) = self.user(n);
        Imap::connect(&self.imap, u, p)
    }

    /// Send a message as user `n` through Postfix.
    pub fn send(&self, n: usize, to: &[&str], raw: &[u8]) {
        let (u, p) = self.user(n);
        smtp_send(&self.smtp, u, p, to, raw);
    }

    /// The user's calendar home on SOGo.
    pub fn dav_home(&self, n: usize) -> String {
        format!("{}/SOGo/dav/{}/Calendar/", self.web, self.user(n).0)
    }

    pub fn sogo(&self, n: usize) -> Sogo {
        let (u, p) = self.user(n);
        Sogo::login(&self.web, u, p)
    }

    /// A raw CalDAV request as user `n`: the answer's status and body.
    pub fn dav(&self, n: usize, method: &str, url: &str, depth: Option<&str>, body: &str) -> (u16, String) {
        let (u, p) = self.user(n);
        let auth = format!("Basic {}", base64(format!("{u}:{p}").as_bytes()));
        let mut h = vec![("Authorization", auth.as_str()), ("Content-Type", "application/xml; charset=utf-8")];
        if let Some(d) = depth {
            h.push(("Depth", d));
        }
        let r = http(method, url, &h, body.as_bytes());
        (r.status, String::from_utf8_lossy(&r.body).into_owned())
    }

    /// Every item of a calendar as user `n` sees it: (href, calendar data).
    pub fn calendar(&self, n: usize, url: &str) -> Vec<(String, String)> {
        let (_, list) =
            self.dav(n, "PROPFIND", url, Some("1"), "<d:propfind xmlns:d=\"DAV:\"><d:prop><d:getetag/><d:resourcetype/></d:prop></d:propfind>");
        let base = url.trim_start_matches("http://").split_once('/').map(|(_, p)| format!("/{p}")).unwrap_or_default();
        let hrefs: Vec<String> = hrefs_in(&list).into_iter().filter(|h| h.trim_end_matches('/') != base.trim_end_matches('/')).collect();
        if hrefs.is_empty() {
            return Vec::new();
        }
        let refs: String = hrefs.iter().map(|h| format!("<d:href>{h}</d:href>")).collect();
        let (_, got) = self.dav(
            n,
            "REPORT",
            url,
            Some("1"),
            &format!(
                "<c:calendar-multiget xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><d:getetag/><c:calendar-data/></d:prop>{refs}</c:calendar-multiget>"
            ),
        );
        let mut out = Vec::new();
        for resp in got.split("<D:response>").skip(1) {
            let href = between(resp, "<D:href>", "</D:href>").unwrap_or_default();
            let data = between(resp, "<C:calendar-data>", "</C:calendar-data>").unwrap_or_default();
            out.push((href, unescape(&data).replace("&#13;", "").replace("\r\n ", "").replace("\n ", "")));
        }
        out
    }
}

pub fn between(s: &str, a: &str, b: &str) -> Option<String> {
    let i = s.find(a)? + a.len();
    let j = s.get(i..)?.find(b)? + i;
    s.get(i..j).map(str::to_string)
}

fn hrefs_in(xml: &str) -> Vec<String> {
    xml.split("<D:href>").skip(1).filter_map(|x| x.split_once("</D:href>").map(|(h, _)| h.to_string())).collect()
}

pub fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#13;", "").replace("&amp;", "&")
}

/// A tiny IMAP client for the other side.
pub struct Imap {
    pub s: std::net::TcpStream,
    pub n: u32,
}

impl Imap {
    pub fn connect(addr: &str, user: &str, password: &str) -> Imap {
        let s = std::net::TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut c = Imap { s, n: 0 };
        c.read_until("* OK");
        c.cmd(&format!("LOGIN \"{user}\" \"{password}\""));
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

    pub fn append(&mut self, folder: &str, flags: &str, raw: &[u8]) {
        self.n += 1;
        let tag = format!("t{}", self.n);
        write!(self.s, "{tag} APPEND \"{folder}\" ({flags}) {{{}+}}\r\n", raw.len()).unwrap();
        self.s.write_all(raw).unwrap();
        self.s.write_all(b"\r\n").unwrap();
        let r = self.read_until(&format!("{tag} "));
        assert!(r.contains(&format!("{tag} OK")), "{r}");
    }

    /// Each message of a folder: its uid, flags and subject.
    pub fn messages(&mut self, folder: &str) -> Vec<(u32, Vec<String>, String)> {
        self.cmd(&format!("EXAMINE \"{folder}\""));
        let r = self.try_cmd("UID FETCH 1:* (FLAGS BODY.PEEK[HEADER.FIELDS (SUBJECT)])");
        self.cmd("UNSELECT");
        let mut out = Vec::new();
        let mut cur: Option<(u32, Vec<String>)> = None;
        for l in r.lines() {
            if l.starts_with("* ") && l.contains("FETCH (") {
                let uid = between(l, "UID ", " ").or_else(|| between(l, "UID ", ")")).and_then(|u| u.parse().ok()).unwrap_or(0);
                let flags = between(l, "FLAGS (", ")").unwrap_or_default().split_whitespace().map(str::to_string).collect();
                cur = Some((uid, flags));
            } else if let Some(s) = l.strip_prefix("Subject: ")
                && let Some((uid, flags)) = cur.take()
            {
                out.push((uid, flags, s.trim().to_string()));
            }
        }
        out
    }

    pub fn subjects(&mut self, folder: &str) -> Vec<String> {
        self.messages(folder).into_iter().map(|m| m.2).collect()
    }

    /// The flags of the message with this subject.
    pub fn flags(&mut self, folder: &str, subject: &str) -> Vec<String> {
        self.messages(folder).into_iter().find(|m| m.2 == subject).map(|m| m.1).unwrap_or_else(|| panic!("no {subject:?} in {folder}"))
    }
}

/// Send through SMTP with a password (AUTH PLAIN, no TLS: Postfix's port 588 on this computer).
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

fn form(v: &str) -> String {
    v.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') { char::from(b).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// SOGo's web endpoints, as its web pages call them, signed in through Mailcow's sign-in page
/// (Mailcow's nginx lets SOGo's pages in only with that session).
pub struct Sogo {
    web: String,
    cookie: String,
    pub user: String,
}

impl Sogo {
    pub fn login(web: &str, user: &str, password: &str) -> Sogo {
        let page = http("GET", &format!("{web}/"), &[], b"");
        let cookie = page.cookies().join("; ");
        let text = page.text();
        let csrf =
            text.split("value=\"").skip(1).filter_map(|x| x.split('"').next()).find(|v| v.len() == 64).expect("the sign-in page's token").to_string();
        let body = format!("login_user={}&pass_user={}&csrf_token={csrf}", form(user), form(password));
        let r = http("POST", &format!("{web}/"), &[("Cookie", &cookie), ("Content-Type", "application/x-www-form-urlencoded")], body.as_bytes());
        assert!(r.status == 302 && r.head.contains("/SOGo/so/"), "signing in to Mailcow as {user}: {}", r.head);
        // A new session after signing in replaces the one before.
        let mut jar: Vec<String> = page.cookies();
        for c in r.cookies() {
            let name = c.split('=').next().unwrap_or_default().to_string();
            jar.retain(|x| x.split('=').next() != Some(name.as_str()));
            jar.push(c);
        }
        Sogo { web: web.to_string(), cookie: jar.join("; "), user: user.to_string() }
    }

    /// `path` under `/SOGo/so/<user>/`.
    pub fn get(&self, path: &str) -> Answer {
        let url = format!("{}/SOGo/so/{}/{path}", self.web, self.user);
        http("GET", &url, &[("Cookie", &self.cookie)], b"")
    }

    pub fn post(&self, path: &str, body: &Value) -> Answer {
        let url = format!("{}/SOGo/so/{}/{path}", self.web, self.user);
        let r = http("POST", &url, &[("Cookie", &self.cookie), ("Content-Type", "application/json")], body.to_string().as_bytes());
        assert!((200..300).contains(&r.status), "{path}: {} {}", r.status, r.text());
        r
    }

    pub fn json(&self, path: &str) -> Value {
        let r = self.get(path);
        assert!((200..300).contains(&r.status), "{path}: {} {}", r.status, r.text());
        serde_json::from_slice(&r.body).unwrap_or(Value::Null)
    }

    /// The user's SOGo mail labels (keyword → name), after adding these.
    pub fn add_labels(&self, labels: &[(&str, &str, &str)]) -> Value {
        let mut d = self.json("Preferences/jsonDefaults");
        for (key, name, color) in labels {
            d["SOGoMailLabelsColors"][key] = json!([name, color]);
        }
        self.post("Preferences/save", &json!({ "defaults": d }));
        self.json("Preferences/jsonDefaults")["SOGoMailLabelsColors"].clone()
    }

    /// The uids SOGo lists in a mail folder (`folderINBOX`), newest first, and its rows.
    pub fn mail_list(&self, folder: &str) -> Value {
        let r = self.post(&format!("Mail/0/{folder}/view"), &json!({"sortingAttributes": {"sort": "arrival", "asc": 0}}));
        serde_json::from_slice(&r.body).unwrap_or(Value::Null)
    }

    /// Add or remove a label on messages, as SOGo's message list does.
    pub fn label(&self, folder: &str, uids: &[u32], label: &str, add: bool) {
        let op = if add { "add" } else { "remove" };
        self.post(&format!("Mail/0/{folder}/addOrRemoveLabel"), &json!({"operation": op, "msgUIDs": uids, "flags": label}));
    }

    /// Give another user rights on a folder (mail: `folderINBOX`; calendar: `Calendar/personal`),
    /// as SOGo's sharing dialog does, subscribing them when asked.
    pub fn share(&self, folder: &str, who: &str, rights: Value, subscribe: bool) {
        let r = self.get(&format!("{folder}/addUserInAcls?uid={who}"));
        assert!((200..300).contains(&r.status), "{}", r.text());
        self.post(&format!("{folder}/saveUserRights"), &json!([{"uid": who, "rights": rights}]));
        if subscribe {
            let r = self.get(&format!("{folder}/subscribeUsers?uids={who}"));
            assert!((200..300).contains(&r.status), "{}", r.text());
        }
    }
}

/// One user's MailCraft: the session and the network layer together, as in the app (a Start
/// gets the user's password, as the keychain would give it).
pub struct App {
    pub s: Session,
    pub pool: Pool,
    pub password: String,
    pub dir: std::path::PathBuf,
}

impl App {
    /// A new MailCraft for `email`, its mail on Mailcow (IMAP and SMTP without TLS, on this
    /// computer); returns the account's id.
    pub fn new(m: &Mailcow, n: usize, tag: &str) -> (App, String) {
        let (email, password) = m.user(n);
        let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("mailcow-{tag}-{}-{}", m.name(n), std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let mut s = Session::open(Some(store.clone()), Clock::new(now(), 0));
        let (ih, ip) = m.imap.rsplit_once(':').unwrap();
        let (sh, sp) = m.smtp.rsplit_once(':').unwrap();
        let id = s
            .run(
                "account.add",
                &json!({"email": email, "password": password, "imapHost": ih, "imapPort": ip.parse::<u16>().unwrap(), "imapSecurity": "none",
                        "smtpHost": sh, "smtpPort": sp.parse::<u16>().unwrap(), "smtpSecurity": "none"}),
            )
            .unwrap()["account"]
            .as_str()
            .unwrap()
            .to_string();
        let app = App { s, pool: Pool::new(store, Arc::new(MemoryCredentials::default())), password: password.to_string(), dir };
        (app, id)
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
    pub fn wait_for_mail(&mut self, id: &str, subject: &str) -> mailcraft_engine::model::MessageKey {
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

/// A plain message.
pub fn message(from: &str, to: &str, subject: &str, id: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{id}>\r\nDate: Sat, 31 Oct 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{subject}\r\n"
    )
    .into_bytes()
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
