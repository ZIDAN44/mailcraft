//! An in-memory IMAP server for tests: enough of RFC 3501 + UIDPLUS, MOVE and CONDSTORE to run
//! the real client and sync code against, with state the test can change between syncs.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

use mailcraft_store::Store;

use crate::imap::Client;
use crate::sync::{SyncOptions, sync_folder};

pub struct Msg {
    pub uid: u32,
    pub flags: Vec<String>,
    pub raw: Vec<u8>,
    pub modseq: u64,
}

pub struct Mailbox {
    pub name: String,
    pub attr: &'static str,
    pub validity: u32,
    pub uid_next: u32,
    pub msgs: Vec<Msg>,
    /// Expunged UIDs and the modseq of their removal (for QRESYNC's VANISHED).
    pub expunged: Vec<(u32, u64)>,
}

pub struct State {
    pub boxes: Vec<Mailbox>,
    pub modseq: u64,
    pub caps: &'static str,
    /// Every command received (tags stripped).
    pub log: Vec<String>,
    /// Addresses whose good token is taken but whose mailbox then won't open, as Microsoft
    /// answers when IMAP is off or the token is another account's.
    pub not_connected: Vec<String>,
    /// What every password sign-in is refused with, when set (Outlook.com: "Basic authentication
    /// is disabled").
    pub refuse_passwords: Option<String>,
    /// How long a text `SEARCH` takes (a server reading every message rather than an index).
    pub search_delay: std::time::Duration,
    /// A hostile server: after the next `NOOP`, this is sent over and over, forever.
    pub flood: Option<Vec<u8>>,
    /// How long each read of the connection waits (a server trickling its answer).
    pub trickle: std::time::Duration,
    /// Mail delivered to the Inbox (this subject) when a command containing this text first
    /// comes: mail arriving at a set point of a sync.
    pub deliver_on: Option<(String, String)>,
}

impl State {
    pub fn new() -> Arc<Mutex<State>> {
        Arc::new(Mutex::new(State {
            boxes: vec![
                Mailbox { name: "INBOX".into(), attr: "", validity: 100, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() },
                Mailbox { name: "Sent".into(), attr: "\\Sent", validity: 200, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() },
                Mailbox { name: "Archive".into(), attr: "\\Archive", validity: 300, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() },
                Mailbox { name: "Projects/Caf&AOk-".into(), attr: "", validity: 400, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() },
            ],
            modseq: 10,
            caps: "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA",
            log: Vec::new(),
            not_connected: Vec::new(),
            refuse_passwords: None,
            search_delay: std::time::Duration::ZERO,
            flood: None,
            trickle: std::time::Duration::ZERO,
            deliver_on: None,
        }))
    }

    pub fn mailbox(&mut self, name: &str) -> Option<&mut Mailbox> {
        self.boxes.iter_mut().find(|b| b.name == name)
    }

    /// Remove a message the way another client would (recorded for VANISHED).
    pub fn expunge(&mut self, mailbox: &str, uid: u32) {
        self.modseq += 1;
        let ms = self.modseq;
        if let Some(b) = self.mailbox(mailbox) {
            b.msgs.retain(|m| m.uid != uid);
            b.expunged.push((uid, ms));
        }
    }

    /// Set a message's flags the way another client would.
    pub fn set_flags(&mut self, mailbox: &str, uid: u32, flags: &[&str]) {
        self.modseq += 1;
        let ms = self.modseq;
        if let Some(m) = self.mailbox(mailbox).and_then(|b| b.msgs.iter_mut().find(|m| m.uid == uid)) {
            m.flags = flags.iter().map(|s| s.to_string()).collect();
            m.modseq = ms;
        }
    }

    /// Put a whole message (as sent) in a mailbox.
    pub fn add_raw(&mut self, mailbox: &str, raw: Vec<u8>) -> u32 {
        self.modseq += 1;
        let modseq = self.modseq;
        let Some(b) = self.mailbox(mailbox) else { return 0 };
        let uid = b.uid_next;
        b.uid_next += 1;
        b.msgs.push(Msg { uid, flags: Vec::new(), raw, modseq });
        uid
    }

    pub fn add(&mut self, mailbox: &str, subject: &str, flags: &[&str]) -> u32 {
        self.modseq += 1;
        let modseq = self.modseq;
        let Some(b) = self.mailbox(mailbox) else { return 0 };
        let uid = b.uid_next;
        b.uid_next += 1;
        let raw = format!(
            "From: Zoe <zoe@studio.example>\r\nTo: me@studio.example\r\nSubject: {subject}\r\nMessage-ID: <{subject}-{uid}@studio.example>\r\nDate: Wed, 14 Oct 2026 09:{:02}:00 +0000\r\n\r\nBody of {subject}.\r\n",
            uid % 60
        );
        b.msgs.push(Msg { uid, flags: flags.iter().map(|s| s.to_string()).collect(), raw: raw.into_bytes(), modseq });
        uid
    }
}

pub struct Fake {
    st: Arc<Mutex<State>>,
    selected: Option<String>,
    inbuf: Vec<u8>,
    out: VecDeque<u8>,
    literal: Option<(String, usize)>,
    /// SASL in progress: the tag, and whether the next line is the token (`true`) or the empty
    /// answer to an error challenge (`false`).
    sasl: Option<(String, bool)>,
    /// Signed in, but every command is refused (see `State::not_connected`).
    refused: bool,
    /// Sent forever once the output runs dry (see `State::flood`).
    flood: Option<Vec<u8>>,
    /// In IDLE (this tag) until `DONE`.
    idling: Option<String>,
}

/// The only access token the fake accepts.
pub const GOOD_TOKEN: &str = "good-token";

impl Fake {
    pub fn connect(st: &Arc<Mutex<State>>) -> Fake {
        let mut f = Fake {
            st: st.clone(),
            selected: None,
            inbuf: Vec::new(),
            out: VecDeque::new(),
            literal: None,
            sasl: None,
            refused: false,
            flood: None,
            idling: None,
        };
        f.say("* OK fake ready");
        f
    }

    fn say(&mut self, s: &str) {
        self.out.extend(s.as_bytes());
        self.out.extend(b"\r\n");
    }

    fn pump(&mut self) {
        loop {
            if let Some((line, n)) = self.literal.clone() {
                // literal bytes, then the rest of the command line
                if self.inbuf.len() < n {
                    return;
                }
                let rest = &self.inbuf[n..];
                let Some(end) = rest.windows(2).position(|w| w == b"\r\n") else { return };
                let data = self.inbuf[..n].to_vec();
                self.inbuf.drain(..n + end + 2);
                self.literal = None;
                self.command(&line, Some(data));
                continue;
            }
            let Some(end) = self.inbuf.windows(2).position(|w| w == b"\r\n") else { return };
            let line = String::from_utf8_lossy(&self.inbuf[..end]).to_string();
            self.inbuf.drain(..end + 2);
            if let Some(open) = line.rfind('{')
                && line.ends_with('}')
                && let Ok(n) = line[open + 1..line.len() - 1].trim_end_matches('+').parse::<usize>()
            {
                self.literal = Some((line[..open].to_string(), n));
                self.say("+ go ahead");
                continue;
            }
            if let Some(tag) = self.idling.take() {
                if line.trim().eq_ignore_ascii_case("DONE") {
                    self.say(&format!("{tag} OK idle done"));
                } else {
                    self.idling = Some(tag);
                }
                continue;
            }
            if let Some((tag, token_next)) = self.sasl.take() {
                self.sasl_step(&tag, token_next.then_some(line.as_str()));
                continue;
            }
            self.command(&line, None);
        }
    }

    /// XOAUTH2: accept the good token; anything else gets an error challenge, then NO.
    fn sasl_step(&mut self, tag: &str, token: Option<&str>) {
        match token {
            Some(t) if t.trim() == crate::oauth::xoauth2("me@studio.example", GOOD_TOKEN) => {
                let caps = self.st.lock().map(|s| s.caps).unwrap_or("");
                self.say(&format!("{tag} OK [CAPABILITY {caps}] signed in"));
            }
            Some(t) if self.st.lock().map(|s| s.not_connected.iter().any(|u| t.trim() == crate::oauth::xoauth2(u, GOOD_TOKEN))).unwrap_or(false) => {
                self.refused = true;
                self.say(&format!("{tag} OK AUTHENTICATE completed."));
            }
            Some(_) => {
                self.say("+ eyJzdGF0dXMiOiI0MDEiLCJzY2hlbWVzIjoiYmVhcmVyIn0=");
                self.sasl = Some((tag.to_string(), false));
            }
            None => self.say(&format!("{tag} NO [AUTHENTICATIONFAILED] invalid credentials")),
        }
    }

    fn command(&mut self, line: &str, literal: Option<Vec<u8>>) {
        let upper_line = line.to_ascii_uppercase();
        if let Some(i) = upper_line.find(" AUTHENTICATE XOAUTH2") {
            let tag = line.get(..i).unwrap_or("").to_string();
            match line.get(i + " AUTHENTICATE XOAUTH2".len()..).map(str::trim).filter(|r| !r.is_empty()) {
                Some(ir) => self.sasl_step(&tag, Some(ir)),
                None => {
                    self.say("+ ");
                    self.sasl = Some((tag, true));
                }
            }
            return;
        }
        let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
        if self.refused {
            self.say(&format!("{tag} BAD User is authenticated but not connected."));
            return;
        }
        if upper_line.contains(" SEARCH ") && upper_line.contains(" TEXT ") {
            let delay = self.st.lock().map(|s| s.search_delay).unwrap_or_default();
            std::thread::sleep(delay);
        }
        let st = self.st.clone();
        let mut st = st.lock().unwrap();
        st.log.push(rest.to_string());
        let upper = rest.to_ascii_uppercase();
        let ok = |s: &mut Fake, extra: &str| s.say(&format!("{tag} OK {extra}done"));
        if let Some((on, subject)) = st.deliver_on.clone()
            && rest.contains(on.as_str())
        {
            st.deliver_on = None;
            st.add("INBOX", &subject, &[]);
        }
        if upper == "IDLE" {
            // RFC 2177, without pushing anything: what came before IDLE isn't said again.
            self.idling = Some(tag.to_string());
            self.say("+ idling");
            return;
        }
        if upper == "NOOP"
            && let Some(flood) = st.flood.clone()
        {
            self.flood = Some(flood);
            return;
        }
        if upper == "CAPABILITY" {
            self.say(&format!("* CAPABILITY {}", st.caps));
            ok(self, "");
        } else if upper.starts_with("LOGIN ") {
            if let Some(why) = &st.refuse_passwords {
                let why = why.clone();
                self.say(&format!("{tag} NO {why}"));
            } else if rest.ends_with("\"pw\"") {
                ok(self, &format!("[CAPABILITY {}] ", st.caps));
            } else {
                self.say(&format!("{tag} NO [AUTHENTICATIONFAILED] wrong password"));
            }
        } else if upper.starts_with("GETQUOTAROOT") {
            let used: usize = st.boxes.iter().flat_map(|b| b.msgs.iter()).map(|m| m.raw.len()).sum::<usize>() / 1024 + 1;
            self.say("* QUOTAROOT INBOX \"\"");
            self.say(&format!("* QUOTA \"\" (STORAGE {used} 1024)"));
            ok(self, "");
        } else if upper.starts_with("ENABLE") {
            if upper.contains("QRESYNC") && st.caps.contains("QRESYNC") {
                self.say("* ENABLED QRESYNC");
            } else {
                self.say("* ENABLED CONDSTORE");
            }
            ok(self, "");
        } else if upper.starts_with("LIST") {
            for b in &st.boxes {
                self.say(&format!("* LIST (\\HasNoChildren{}{}) \"/\" \"{}\"", if b.attr.is_empty() { "" } else { " " }, b.attr, b.name));
            }
            ok(self, "");
        } else if upper.starts_with("SELECT ") || upper.starts_with("EXAMINE ") {
            let name = quoted_arg(rest);
            let modseq = st.modseq;
            // `(QRESYNC (validity modseq …))`
            let qresync: Option<(u32, u64)> = upper.split("(QRESYNC (").nth(1).and_then(|r| {
                let mut w = r.split([' ', ')']);
                Some((w.next()?.parse().ok()?, w.next()?.parse().ok()?))
            });
            match st.mailbox(&name) {
                Some(b) => {
                    let (n, v, next) = (b.msgs.len(), b.validity, b.uid_next);
                    self.say(&format!("* {n} EXISTS"));
                    self.say(&format!("* OK [UIDVALIDITY {v}] ok"));
                    self.say(&format!("* OK [UIDNEXT {next}] ok"));
                    self.say(&format!("* OK [HIGHESTMODSEQ {modseq}] ok"));
                    if let Some((qv, qm)) = qresync.filter(|(qv, _)| *qv == v) {
                        let _ = qv;
                        let gone: Vec<String> = b.expunged.iter().filter(|(_, m)| *m > qm).map(|(u, _)| u.to_string()).collect();
                        let changed: Vec<String> = b
                            .msgs
                            .iter()
                            .enumerate()
                            .filter(|(_, m)| m.modseq > qm)
                            .map(|(i, m)| format!("* {} FETCH (UID {} FLAGS ({}) MODSEQ ({}))", i + 1, m.uid, m.flags.join(" "), m.modseq))
                            .collect();
                        if !gone.is_empty() {
                            self.say(&format!("* VANISHED (EARLIER) {}", gone.join(",")));
                        }
                        for c in changed {
                            self.say(&c);
                        }
                    }
                    self.selected = Some(name);
                    ok(self, "[READ-WRITE] ");
                }
                None => {
                    self.selected = None;
                    self.say(&format!("{tag} NO no such mailbox"));
                }
            }
        } else if upper.starts_with("CREATE ") {
            let name = quoted_arg(rest);
            if st.mailbox(&name).is_some() {
                self.say(&format!("{tag} NO [ALREADYEXISTS] exists"));
            } else {
                let validity = 500 + u32::try_from(st.boxes.len()).unwrap_or(0);
                st.boxes.push(Mailbox { name, attr: "", validity, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() });
                ok(self, "");
            }
        } else if upper.starts_with("RENAME ") {
            let mut names = rest.split('"').skip(1).step_by(2).map(str::to_string);
            let (from, to) = (names.next().unwrap_or_default(), names.next().unwrap_or_default());
            if st.mailbox(&from).is_none() || st.mailbox(&to).is_some() {
                self.say(&format!("{tag} NO cannot rename"));
            } else {
                for b in &mut st.boxes {
                    if b.name == from {
                        b.name = to.clone();
                    } else if let Some(rest) = b.name.strip_prefix(&format!("{from}/")) {
                        b.name = format!("{to}/{rest}");
                    }
                }
                ok(self, "");
            }
        } else if upper.starts_with("DELETE ") {
            let name = quoted_arg(rest);
            let before = st.boxes.len();
            st.boxes.retain(|b| b.name != name);
            if st.boxes.len() == before {
                self.say(&format!("{tag} NO [NONEXISTENT] no such mailbox"));
            } else {
                ok(self, "");
            }
        } else if upper.starts_with("SUBSCRIBE ") || upper.starts_with("UNSUBSCRIBE ") || upper == "UNSELECT" {
            if upper == "UNSELECT" {
                self.selected = None;
            }
            ok(self, "");
        } else if upper.starts_with("APPEND ") {
            let name = quoted_arg(rest);
            let flags = paren_arg(rest);
            st.modseq += 1;
            let modseq = st.modseq;
            match st.mailbox(&name) {
                Some(b) => {
                    let uid = b.uid_next;
                    b.uid_next += 1;
                    b.msgs.push(Msg { uid, flags, raw: literal.unwrap_or_default(), modseq });
                    let v = b.validity;
                    ok(self, &format!("[APPENDUID {v} {uid}] "));
                }
                None => self.say(&format!("{tag} NO [TRYCREATE] no such mailbox")),
            }
        } else if upper.starts_with("UID ") || upper == "EXPUNGE" || upper == "NOOP" || upper == "LOGOUT" {
            let sel = self.selected.clone().unwrap_or_default();
            let modseq_now = st.modseq;
            let mut lines: Vec<Vec<u8>> = Vec::new();
            let mut extra = String::new();
            let mut moved: Vec<Msg> = Vec::new();
            let mut dest = String::new();
            // Gmail mode: an EXPUNGE in a label folder (anything but Trash/Spam) drops the label
            // and archives the message into "[Gmail]/All Mail".
            let labels = st.caps.contains("X-GM-EXT-1");
            let mut copied_from = 0;
            let mut copyuid = String::new();
            {
                let Some(b) = st.boxes.iter_mut().find(|b| b.name == sel) else {
                    if upper == "NOOP" || upper == "LOGOUT" {
                        ok(self, "");
                    } else {
                        self.say(&format!("{tag} BAD no mailbox selected"));
                    }
                    return;
                };
                b.msgs.sort_by_key(|m| m.uid);
                let max = b.msgs.last().map_or(0, |m| m.uid);
                let words: Vec<&str> = rest.split_whitespace().collect();
                let verb = words.get(1).map(|s| s.to_ascii_uppercase()).unwrap_or_default();
                if upper == "NOOP" {
                    // What a real server says to a NOOP: the folder's size, now.
                    lines.push(format!("* {} EXISTS", b.msgs.len()).into_bytes());
                } else if upper == "LOGOUT" {
                } else if verb == "SEARCH" {
                    // The keys after `SEARCH`, a literal last (as MailCraft sends a UTF-8 word).
                    let after = rest.splitn(3, ' ').nth(2).unwrap_or("");
                    let mut keys = search_tokens(after);
                    if let Some(l) = &literal {
                        keys.push(String::from_utf8_lossy(l).into_owned());
                    }
                    let exists = u32::try_from(b.msgs.len()).unwrap_or(u32::MAX);
                    let hits: Vec<String> = b
                        .msgs
                        .iter()
                        .enumerate()
                        .filter(|(i, m)| search_matches(m, u32::try_from(*i + 1).unwrap_or(0), &keys, max, exists))
                        .map(|(_, m)| m.uid.to_string())
                        .collect();
                    lines.push(format!("* SEARCH {}\r\n", hits.join(" ")).trim_end().as_bytes().to_vec());
                } else if verb == "FETCH" {
                    let set = words.get(2).copied().unwrap_or("");
                    let since: u64 = rest.split("CHANGEDSINCE ").nth(1).and_then(|s| s.trim_end_matches(')').trim().parse().ok()).unwrap_or(0);
                    for (i, m) in b.msgs.iter().enumerate() {
                        if !in_set(m.uid, set, max) || m.modseq <= since {
                            continue;
                        }
                        let mut item = format!("* {} FETCH (UID {} FLAGS ({}) MODSEQ ({})", i + 1, m.uid, m.flags.join(" "), m.modseq).into_bytes();
                        if upper.contains("RFC822.SIZE") {
                            item.extend(format!(" RFC822.SIZE {}", m.raw.len()).as_bytes());
                        }
                        if upper.contains("INTERNALDATE") {
                            item.extend(b" INTERNALDATE \"14-Oct-2026 09:30:00 +0000\"");
                        }
                        if upper.contains("BODY.PEEK[HEADER]") {
                            let end = m.raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(m.raw.len(), |p| p + 4);
                            item.extend(format!(" BODY[HEADER] {{{end}}}\r\n").as_bytes());
                            item.extend(&m.raw[..end]);
                        }
                        if upper.contains("BODY.PEEK[]") {
                            item.extend(format!(" BODY[] {{{}}}\r\n", m.raw.len()).as_bytes());
                            item.extend(&m.raw);
                        }
                        item.push(b')');
                        lines.push(item);
                    }
                } else if verb == "STORE" {
                    let set = words.get(2).copied().unwrap_or("");
                    let add = words.get(3).is_some_and(|w| w.starts_with('+'));
                    let flags = paren_arg(rest);
                    for m in b.msgs.iter_mut().filter(|m| in_set(m.uid, set, max)) {
                        for f in &flags {
                            m.flags.retain(|x| !x.eq_ignore_ascii_case(f));
                            if add {
                                m.flags.push(f.clone());
                            }
                        }
                        m.modseq = modseq_now + 1;
                    }
                } else if verb == "MOVE" || verb == "COPY" {
                    let set = words.get(2).copied().unwrap_or("");
                    dest = quoted_arg(rest);
                    if verb == "MOVE" {
                        let (go, stay): (Vec<Msg>, Vec<Msg>) = std::mem::take(&mut b.msgs).into_iter().partition(|m| in_set(m.uid, set, max));
                        b.msgs = stay;
                        b.expunged.extend(go.iter().map(|m| (m.uid, modseq_now + 1)));
                        moved = go;
                    } else {
                        moved = b
                            .msgs
                            .iter()
                            .filter(|m| in_set(m.uid, set, max))
                            .map(|m| Msg { uid: m.uid, flags: m.flags.clone(), raw: m.raw.clone(), modseq: 0 })
                            .collect();
                    }
                    copied_from = b.validity;
                } else if verb == "EXPUNGE" || upper == "EXPUNGE" {
                    let set = words.get(2).copied().unwrap_or("1:*");
                    let gone: Vec<u32> =
                        b.msgs.iter().filter(|m| in_set(m.uid, set, max) && m.flags.iter().any(|f| f == "\\Deleted")).map(|m| m.uid).collect();
                    if labels && !matches!(b.attr, "\\Trash" | "\\Junk") {
                        moved = b
                            .msgs
                            .iter()
                            .filter(|m| gone.contains(&m.uid))
                            .map(|m| Msg { uid: m.uid, flags: Vec::new(), raw: m.raw.clone(), modseq: 0 })
                            .collect();
                        dest = "[Gmail]/All Mail".into();
                    }
                    b.msgs.retain(|m| !gone.contains(&m.uid));
                    b.expunged.extend(gone.iter().map(|u| (*u, modseq_now + 1)));
                } else {
                    extra = "BAD".into();
                }
            }
            if !moved.is_empty() || !dest.is_empty() {
                st.modseq += 1;
                let ms = st.modseq;
                if let Some(d) = st.mailbox(&dest) {
                    let (mut from, mut to) = (Vec::new(), Vec::new());
                    for mut m in moved {
                        from.push(m.uid.to_string());
                        m.uid = d.uid_next;
                        to.push(m.uid.to_string());
                        d.uid_next += 1;
                        m.modseq = ms;
                        d.msgs.push(m);
                    }
                    if copied_from != 0 && !from.is_empty() {
                        copyuid = format!("[COPYUID {} {} {}] ", d.validity, from.join(","), to.join(","));
                    }
                }
            } else if upper.contains("STORE") || upper.contains("EXPUNGE") {
                st.modseq += 1;
            }
            for l in lines {
                self.out.extend(l);
                self.out.extend(b"\r\n");
            }
            if extra == "BAD" {
                self.say(&format!("{tag} BAD unsupported"));
            } else {
                if upper == "LOGOUT" {
                    self.say("* BYE bye");
                }
                ok(self, &copyuid);
            }
        } else {
            self.say(&format!("{tag} BAD unsupported"));
        }
    }
}

fn quoted_arg(s: &str) -> String {
    s.split('"').nth(1).unwrap_or_default().to_string()
}

fn paren_arg(s: &str) -> Vec<String> {
    let Some(open) = s.find('(') else { return Vec::new() };
    let close = s[open..].find(')').map_or(s.len(), |c| open + c);
    s[open + 1..close].split_whitespace().map(str::to_string).collect()
}

/// A `SEARCH` command's keys and strings (quotes taken off).
fn search_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut escaped) = (false, false);
    for c in s.chars() {
        match c {
            _ if escaped => {
                cur.push(c);
                escaped = false;
            }
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Whether a message (`seq`, its message number) matches every key (RFC 3501 §6.4.4; strings as
/// substrings, any case; a sequence set is the message numbers').
fn search_matches(m: &Msg, seq: u32, keys: &[String], max: u32, exists: u32) -> bool {
    let raw = String::from_utf8_lossy(&m.raw).to_lowercase();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let header = |name: &str| head.lines().filter(|l| l.starts_with(&format!("{name}:"))).collect::<Vec<_>>().join("\n");
    let mut i = 0;
    let mut negate = false;
    while let Some(k) = keys.get(i) {
        let arg = keys.get(i + 1).map(|a| a.to_lowercase()).unwrap_or_default();
        let (ok, used) = match k.to_ascii_uppercase().as_str() {
            "ALL" => (true, 1),
            "NOT" => {
                negate = !negate;
                i += 1;
                continue;
            }
            "CHARSET" => {
                i += 2;
                continue;
            }
            "UNSEEN" => (!m.flags.iter().any(|f| f == "\\Seen"), 1),
            "SEEN" => (m.flags.iter().any(|f| f == "\\Seen"), 1),
            "FLAGGED" => (m.flags.iter().any(|f| f == "\\Flagged"), 1),
            "UNFLAGGED" => (!m.flags.iter().any(|f| f == "\\Flagged"), 1),
            "UID" => (in_set(m.uid, &arg, max), 2),
            "TEXT" => (raw.contains(&arg), 2),
            "BODY" => (body.contains(&arg), 2),
            "SUBJECT" => (header("subject").contains(&arg), 2),
            "FROM" => (header("from").contains(&arg), 2),
            "TO" => (header("to").contains(&arg), 2),
            "KEYWORD" => (m.flags.iter().any(|f| f.eq_ignore_ascii_case(&arg)), 2),
            set if set.starts_with(|c: char| c.is_ascii_digit() || c == '*') => (in_set(seq, set, exists), 1),
            _ => (false, 1),
        };
        if ok == negate {
            return false;
        }
        negate = false;
        i += used;
    }
    true
}

fn in_set(uid: u32, set: &str, max: u32) -> bool {
    set.split(',').any(|part| {
        let val = |x: &str| if x == "*" { max } else { x.parse().unwrap_or(0) };
        match part.split_once(':') {
            Some((a, b)) => {
                let (a, b) = (val(a), val(b));
                (a.min(b)..=a.max(b)).contains(&uid)
            }
            None => val(part) == uid,
        }
    })
}

impl Read for Fake {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let trickle = self.st.lock().map(|s| s.trickle).unwrap_or_default();
        if !trickle.is_zero() {
            std::thread::sleep(trickle);
        }
        if self.out.is_empty()
            && let Some(f) = &self.flood
        {
            self.out.extend(f.iter());
        }
        let n = buf.len().min(self.out.len()).min(11);
        for b in buf.iter_mut().take(n) {
            *b = self.out.pop_front().unwrap_or(0);
        }
        Ok(n)
    }
}

impl Write for Fake {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inbuf.extend_from_slice(buf);
        self.pump();
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serve the fake over TCP on localhost (one thread per connection), for end-to-end tests of
/// the worker pool. Returns the port.
pub fn serve_tcp(st: &Arc<Mutex<State>>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let st = st.clone();
    std::thread::spawn(move || {
        for sock in listener.incoming().flatten() {
            let st = st.clone();
            std::thread::spawn(move || {
                let mut fake = Fake::connect(&st);
                let mut sock = sock;
                let mut buf = [0u8; 8192];
                loop {
                    let out: Vec<u8> = fake.out.drain(..).collect();
                    if !out.is_empty() && sock.write_all(&out).is_err() {
                        return;
                    }
                    match sock.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            let _ = fake.write(&buf[..n]);
                        }
                    }
                }
            });
        }
    });
    port
}

/// A store in a temporary folder of its own, for other fakes' tests.
pub(crate) fn tests_store(name: &str) -> Store {
    store(name)
}

fn store(name: &str) -> Store {
    let dir = std::env::temp_dir().join(format!("mailcraft-fake-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Store::open(dir).unwrap()
}

/// Gap #7 through the worker, the same for JMAP, Graph and EWS accounts: a server whose Inbox
/// has `Message 1`…`Message 25` (bodies `tag1`…, the first five unread) and a window of ten. A
/// search of the server brings in what it finds past the window and keeps it across syncs while
/// the server has it, its flags followed; words, fields and the read state are asked; Show more
/// keeps more. `server(n, true)` marks `Message n` read on the server, `server(n, false)` deletes it.
pub(crate) fn search_the_server_past_the_window(name: &str, account: mailcraft_model::AccountConfig, secret: &str, server: &dyn Fn(u32, bool)) {
    use mailcraft_model::job::Secret;
    use mailcraft_model::{Event, Job, SyncStatus};
    let store = store(name);
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let until = |pool: &crate::worker::Pool, what: &str, f: &dyn Fn(&Event) -> bool| {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut seen = Vec::new();
        while std::time::Instant::now() < end {
            if let Some(e) = pool.wait(std::time::Duration::from_millis(100)) {
                let hit = f(&e);
                seen.push(e);
                if hit {
                    return seen;
                }
            }
        }
        panic!("timed out waiting for {what}; saw {seen:?}");
    };
    let online = |e: &Event| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. });
    let subjects = || {
        let mut v: Vec<String> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.subject.clone()).collect();
        v.sort();
        v
    };
    let has = |s: &str| subjects().iter().any(|x| x == s);
    let search = |pool: &mut crate::worker::Pool, q: &str| {
        pool.submit(Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: q.into(), search: 1 });
        // What's found comes as it's found, then the end.
        let seen = until(pool, "searched", &|e| matches!(e, Event::ServerSearched { done: true, .. }));
        match seen.last() {
            Some(Event::ServerSearched { error: None, .. }) => {
                seen.iter().map(|e| if let Event::ServerSearched { keys, .. } = e { keys.len() } else { 0 }).sum::<usize>()
            }
            other => panic!("{q}: {other:?}"),
        }
    };
    pool.submit(Job::SetSync { messages: 10, offline: 0 });
    pool.submit(Job::Start { account, password: Some(Secret(secret.into())) });
    until(&pool, "online", &online);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while subjects().len() < 10 {
        assert!(std::time::Instant::now() < end, "never synced: {:?}", subjects());
        let _ = pool.wait(std::time::Duration::from_millis(100));
    }
    assert!(has("Message 16") && !has("Message 15") && !has("Message 3"), "{:?}", subjects());

    // Words: found past the window, and kept by the next sync.
    assert_eq!(search(&mut pool, "tag3"), 1);
    assert!(has("Message 3"), "{:?}", subjects());
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "synced", &online);
    assert!(has("Message 3"), "kept past the window: {:?}", subjects());
    // A field and a word.
    assert_eq!(search(&mut pool, "subject:message tag7"), 1);
    assert!(has("Message 7"));

    // Show more: five more than kept (ten and the one found).
    pool.submit(Job::MoreMail { account: "a1".into(), folder: "INBOX".into(), more: 5 });
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !has("Message 10") {
        assert!(std::time::Instant::now() < end, "never showed more: {:?}", subjects());
        let _ = pool.wait(std::time::Duration::from_millis(100));
    }

    // The read state alone; finds add up, as IMAP keeps what it fetches (another search doesn't
    // take Message 3 and 7 away).
    assert_eq!(search(&mut pool, "is:unread"), 5);
    assert_eq!(search(&mut pool, "nothing-like-this"), 0);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "synced", &online);
    assert!(has("Message 1") && has("Message 5") && has("Message 3") && has("Message 7"), "{:?}", subjects());

    // On the server, Message 3 is deleted and Message 2 read: the next sync follows both.
    server(3, false);
    server(2, true);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    // (A push stream may say "online" before this sync ends: wait for what it does.)
    let seen = |s: &str| store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == s).map(|m| m.flags.seen);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while has("Message 3") || seen("Message 2") != Some(true) {
        assert!(std::time::Instant::now() < end, "never followed the server: {:?} {:?}", subjects(), seen("Message 2"));
        let _ = pool.wait(std::time::Duration::from_millis(100));
    }
    assert!(has("Message 7"), "{:?}", subjects());
    assert_eq!(seen("Message 1"), Some(false));
    pool.submit(Job::Remove { account: "a1".into() });
}

fn login(st: &Arc<Mutex<State>>) -> Client<Fake> {
    let mut c = Client::from_stream(Fake::connect(st)).unwrap();
    c.login("me@studio.example", "pw").unwrap();
    c
}

#[cfg(test)]
mod tests;
