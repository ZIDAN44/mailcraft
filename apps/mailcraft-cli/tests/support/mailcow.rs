//! What the Mailcow tests share (`real_mailcow_*.rs`): the servers and users from the
//! environment and SOGo's web endpoints signed in through Mailcow's own sign-in page, with what
//! every real-server test shares (`servers.rs`: the IMAP, SMTP and HTTP clients for the other
//! side, MailCraft's session and network layer together, as in the app, one per user).
//!
//! `MAILCRAFT_REAL_MAILCOW="<imap> <submission> <web> <user>:<password> <user>:<password> <user>:<password>"`:
//! Dovecot's IMAP port without TLS (Mailcow's `data/conf/dovecot/extra.conf` with
//! `disable_plaintext_auth = no`), Postfix's port 588 (SOGo's own submission, which takes a
//! password without TLS), and Mailcow's web server over plain http (`HTTP_REDIRECT=n`), all on
//! this computer: MailCraft trusts only real certificates, and a test server has none. Three
//! mailboxes of one domain: the owner of the shared mailbox and calendar (ann), a colleague who
//! may change them (bob) and one who may only read (carol).

#![allow(dead_code)]

#[path = "servers.rs"]
mod servers;

use serde_json::{Value, json};
pub use servers::*;

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

    /// Take out of users `who`'s personal calendars every item titled from `mark` on: what a run
    /// before left there, passed or failed (called as a test starts, and as it ends).
    pub fn clean_calendars(&self, who: &[usize], mark: &str) {
        for &n in who {
            for (href, data) in self.calendar(n, &format!("{}personal/", self.dav_home(n))) {
                if data.contains(&format!("SUMMARY:{mark}")) {
                    let (st, _) = self.dav(n, "DELETE", &format!("{}{href}", self.web), None, "");
                    eprintln!("cleaned {href}: {st}");
                }
            }
        }
    }
}

fn hrefs_in(xml: &str) -> Vec<String> {
    xml.split("<D:href>").skip(1).filter_map(|x| x.split_once("</D:href>").map(|(h, _)| h.to_string())).collect()
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

impl App {
    /// A new MailCraft for Mailcow's user `n`, its mail on Mailcow (IMAP and SMTP without TLS,
    /// on this computer); returns the account's id.
    pub fn new(m: &Mailcow, n: usize, tag: &str) -> (App, String) {
        let (email, password) = m.user(n);
        App::at(&m.imap, &m.smtp, email, email, password, &format!("mailcow-{tag}"))
    }
}

/// A plain message.
pub fn message(from: &str, to: &str, subject: &str, id: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{id}>\r\nDate: Sat, 31 Oct 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{subject}\r\n"
    )
    .into_bytes()
}
