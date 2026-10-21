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
}

/// The only access token the fake accepts.
pub const GOOD_TOKEN: &str = "good-token";

impl Fake {
    pub fn connect(st: &Arc<Mutex<State>>) -> Fake {
        let mut f = Fake { st: st.clone(), selected: None, inbuf: Vec::new(), out: VecDeque::new(), literal: None, sasl: None, refused: false };
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
        let st = self.st.clone();
        let mut st = st.lock().unwrap();
        st.log.push(rest.to_string());
        let upper = rest.to_ascii_uppercase();
        let ok = |s: &mut Fake, extra: &str| s.say(&format!("{tag} OK {extra}done"));
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
                if upper == "NOOP" || upper == "LOGOUT" {
                } else if verb == "SEARCH" {
                    // The keys after `SEARCH`, a literal last (as MailCraft sends a UTF-8 word).
                    let after = rest.splitn(3, ' ').nth(2).unwrap_or("");
                    let mut keys = search_tokens(after);
                    if let Some(l) = &literal {
                        keys.push(String::from_utf8_lossy(l).into_owned());
                    }
                    let hits: Vec<String> = b.msgs.iter().filter(|m| search_matches(m, &keys, max)).map(|m| m.uid.to_string()).collect();
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

/// Whether a message matches every key (RFC 3501 §6.4.4; strings as substrings, any case).
fn search_matches(m: &Msg, keys: &[String], max: u32) -> bool {
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
        pool.submit(Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: q.into() });
        let seen = until(pool, "searched", &|e| matches!(e, Event::ServerSearched { .. }));
        match seen.last() {
            Some(Event::ServerSearched { keys, error: None, .. }) => keys.len(),
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
mod tests {
    use std::time::{Duration, Instant};

    use mailcraft_model::job::Secret;
    use mailcraft_model::{AccountConfig, Event, Flag, FolderRole, Job, MessageKey, Op, Security, ServerConfig, SyncStatus};

    use super::*;
    use crate::worker::apply_remote;

    #[test]
    fn wrong_password_is_an_auth_error() {
        let st = State::new();
        let mut c = Client::from_stream(Fake::connect(&st)).unwrap();
        assert!(matches!(c.login("me", "nope"), Err(crate::NetError::Auth(_))));
    }

    #[test]
    fn lists_folders_with_roles() {
        let st = State::new();
        let mut c = login(&st);
        let f = c.list("a1").unwrap();
        let roles: Vec<(String, FolderRole)> = f.iter().map(|f| (f.display_name(), f.role)).collect();
        assert_eq!(
            roles,
            vec![
                ("Inbox".to_string(), FolderRole::Inbox),
                ("Sent Items".to_string(), FolderRole::Sent),
                ("Archive".to_string(), FolderRole::Archive),
                ("Café".to_string(), FolderRole::Other)
            ]
        );
    }

    #[test]
    fn full_sync_cycle() {
        let st = State::new();
        {
            let mut s = st.lock().unwrap();
            s.add("INBOX", "first", &["\\Seen"]);
            s.add("INBOX", "second", &[]);
            s.add("INBOX", "third", &[]);
        }
        let store = store("cycle");
        let opts = SyncOptions::default();
        let mut c = login(&st);

        // First sync: everything arrives, sources prefetched, previews filled.
        let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
        assert_eq!((r.new, r.removed, r.total, r.unread), (3, 0, 3, 2));
        let cache = store.load_folder("a1", "INBOX").unwrap();
        assert_eq!(cache.validity, 100);
        assert_eq!(cache.uid_next, 4);
        let subjects: Vec<&str> = cache.messages.iter().map(|m| m.subject.as_str()).collect();
        assert_eq!(subjects, vec!["third", "second", "first"]);
        assert_eq!(cache.messages[0].preview, "Body of third.");
        assert!(store.has_body(&cache.messages[0].key));

        // Nothing changed: CONDSTORE means no flag fetch at all.
        st.lock().unwrap().log.clear();
        let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
        assert!(!r.touched());
        assert!(!st.lock().unwrap().log.iter().any(|l| l.contains("FLAGS")), "{:?}", st.lock().unwrap().log);

        // New mail, a flag change and a deletion on the server.
        {
            let mut s = st.lock().unwrap();
            s.add("INBOX", "fourth", &[]);
            s.modseq += 1;
            let ms = s.modseq;
            let b = s.mailbox("INBOX").unwrap();
            let m = b.msgs.iter_mut().find(|m| m.uid == 2).unwrap();
            m.flags.push("\\Flagged".into());
            m.modseq = ms;
            b.msgs.retain(|m| m.uid != 1);
        }
        let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
        assert_eq!((r.new, r.changed, r.removed), (1, 1, 1));
        let cache = store.load_folder("a1", "INBOX").unwrap();
        assert_eq!(cache.messages.len(), 3);
        assert!(cache.find(2).unwrap().flags.flagged);
        assert!(cache.find(1).is_none());

        // UIDVALIDITY changes: the cache is rebuilt from scratch.
        st.lock().unwrap().mailbox("INBOX").unwrap().validity = 101;
        let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
        assert!(r.reset);
        let cache = store.load_folder("a1", "INBOX").unwrap();
        assert_eq!(cache.validity, 101);
        assert!(cache.messages.iter().all(|m| m.key.validity == 101));
        assert_eq!(cache.messages.len(), 3);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn qresync_learns_expunges_and_flags_from_select() {
        let st = State::new();
        {
            let mut s = st.lock().unwrap();
            s.caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE QRESYNC ENABLE";
            for i in 0..5 {
                s.add("INBOX", &format!("m{i}"), &[]);
            }
        }
        let store = store("qresync");
        let opts = SyncOptions::default();
        let mut c = login(&st);
        assert!(c.qresync());
        sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
        {
            let mut s = st.lock().unwrap();
            s.expunge("INBOX", 2);
            s.expunge("INBOX", 4);
            s.set_flags("INBOX", 3, &["\\Seen", "\\Flagged"]);
            s.add("INBOX", "new", &[]);
            s.log.clear();
        }
        let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
        assert_eq!((r.new, r.changed, r.removed), (1, 1, 2));
        let cache = store.load_folder("a1", "INBOX").unwrap();
        let uids: Vec<u32> = cache.messages.iter().map(|m| m.key.uid).collect();
        assert_eq!(uids, vec![6, 5, 3, 1]);
        assert!(cache.find(3).unwrap().flags.flagged);
        let log = st.lock().unwrap().log.clone();
        assert!(log.iter().any(|l| l.contains("(QRESYNC (100 ")), "{log:?}");
        // No flag fetch and no UID search over the cached range: the SELECT said it all.
        assert!(!log.iter().any(|l| l.contains("UID SEARCH UID 1:5") || l.contains("CHANGEDSINCE")), "{log:?}");

        // A new UIDVALIDITY still starts over.
        st.lock().unwrap().mailbox("INBOX").unwrap().validity = 101;
        let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
        assert!(r.reset);
        assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 4);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn without_condstore_flags_are_compared() {
        let st = State::new();
        {
            let mut s = st.lock().unwrap();
            s.caps = "IMAP4rev1";
            s.add("INBOX", "a", &[]);
        }
        let store = store("nocs");
        let mut c = login(&st);
        sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
        st.lock().unwrap().mailbox("INBOX").unwrap().msgs[0].flags.push("\\Seen".into());
        let r = sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
        assert_eq!(r.changed, 1);
        assert!(store.load_folder("a1", "INBOX").unwrap().messages[0].flags.seen);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn first_sync_takes_a_window() {
        let st = State::new();
        for i in 0..8 {
            st.lock().unwrap().add("INBOX", &format!("m{i}"), &[]);
        }
        let store = store("window");
        let mut c = login(&st);
        let opts = SyncOptions { window: 3, prefetch: 1, ..Default::default() };
        let r = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
        assert_eq!(r.new, 3);
        let cache = store.load_folder("a1", "INBOX").unwrap();
        let uids: Vec<u32> = cache.messages.iter().map(|m| m.key.uid).collect();
        assert_eq!(uids, vec![8, 7, 6]);
        assert!(store.has_body(&cache.messages[0].key));
        assert!(!store.has_body(&cache.messages[1].key));
        // Later syncs only add what's new, not the older backlog.
        st.lock().unwrap().add("INBOX", "m8", &[]);
        assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap().new, 1);
        // More kept since (Options): the older mail comes in once, up to the new number.
        let more = SyncOptions { window: 6, ..opts };
        assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &more).unwrap().new, 2);
        let uids: Vec<u32> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.key.uid).collect();
        assert_eq!(uids, vec![9, 8, 7, 6, 5, 4]);
        assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &more).unwrap().new, 0, "once");
        let all = SyncOptions { window: usize::MAX, ..opts };
        assert_eq!(sync_folder(&mut c, &store, "a1", "INBOX", &all).unwrap().new, 3, "all of it");
        let _ = std::fs::remove_dir_all(store.root());
    }

    /// Permanent delete on a Gmail-style server: moved into Trash and expunged there. Other
    /// servers get a plain EXPUNGE.
    #[test]
    fn gmail_permanent_delete_moves_to_trash_then_expunges() {
        let st = State::new();
        {
            let mut s = st.lock().unwrap();
            s.caps = "IMAP4rev1 UIDPLUS MOVE X-GM-EXT-1";
            for (name, attr, validity) in [("[Gmail]/All Mail", "\\All", 500), ("[Gmail]/Trash", "\\Trash", 600)] {
                s.boxes.push(Mailbox { name: name.into(), attr, validity, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() });
            }
            for subject in ["a", "b", "c", "d"] {
                s.add("INBOX", subject, &[]);
            }
            s.add("[Gmail]/Trash", "binned", &[]);
        }
        let mut c = login(&st);
        let key = |folder: &str, validity, uid| MessageKey { account: "a1".into(), folder: folder.into(), validity, uid };
        let count = |name: &str| st.lock().unwrap().mailbox(name).map_or(0, |b| b.msgs.len());
        // No Trash path given: plain EXPUNGE, which Gmail turns into an archive.
        apply_remote(&mut c, &Op::Expunge { keys: vec![key("INBOX", 100, 1)] }).unwrap();
        assert_eq!((count("INBOX"), count("[Gmail]/All Mail")), (3, 1));
        // With the Trash path: moved, then expunged; nothing left behind.
        let trash = Some("[Gmail]/Trash");
        crate::worker::apply_remote_with(&mut c, &Op::Expunge { keys: vec![key("INBOX", 100, 2), key("INBOX", 100, 3)] }, trash).unwrap();
        assert_eq!((count("INBOX"), count("[Gmail]/All Mail"), count("[Gmail]/Trash")), (1, 1, 1));
        // Keys already in Trash: expunged in place.
        crate::worker::apply_remote_with(&mut c, &Op::Expunge { keys: vec![key("[Gmail]/Trash", 600, 1)] }, trash).unwrap();
        assert_eq!((count("[Gmail]/All Mail"), count("[Gmail]/Trash")), (1, 0));
        // Non-Gmail server: the Trash path is ignored.
        st.lock().unwrap().caps = "IMAP4rev1 UIDPLUS MOVE";
        let mut c = login(&st);
        crate::worker::apply_remote_with(&mut c, &Op::Expunge { keys: vec![key("INBOX", 100, 4)] }, trash).unwrap();
        assert_eq!((count("INBOX"), count("[Gmail]/All Mail"), count("[Gmail]/Trash")), (0, 1, 0));
    }

    /// A move says where its messages went (COPYUID), with MOVE and without.
    #[test]
    fn a_move_says_the_new_uids() {
        let st = State::new();
        for s in ["a", "b", "c"] {
            st.lock().unwrap().add("INBOX", s, &[]);
        }
        let mut c = login(&st);
        c.select("INBOX", false).unwrap();
        assert_eq!(c.uid_move_to("1,3", "Archive").unwrap(), vec![1, 2]);
        st.lock().unwrap().caps = "IMAP4rev1 UIDPLUS";
        let mut c = login(&st);
        c.select("INBOX", false).unwrap();
        assert_eq!(c.uid_move_to("2", "Archive").unwrap(), vec![3]);
        assert_eq!(st.lock().unwrap().mailbox("INBOX").map(|b| b.msgs.len()), Some(0));
    }

    #[test]
    fn remote_changes_flags_moves_deletes_and_appends() {
        let st = State::new();
        for s in ["a", "b", "c"] {
            st.lock().unwrap().add("INBOX", s, &[]);
        }
        let mut c = login(&st);
        let key = |uid| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid };
        apply_remote(&mut c, &Op::SetFlag { keys: vec![key(1), key(2)], flag: Flag::Seen, on: true }).unwrap();
        apply_remote(&mut c, &Op::Move { keys: vec![key(2)], to: "Archive".into() }).unwrap();
        apply_remote(&mut c, &Op::Expunge { keys: vec![key(3)] }).unwrap();
        // A stale key (old UIDVALIDITY) is skipped, not applied to whatever has that UID now.
        let mut stale = key(1);
        stale.validity = 99;
        apply_remote(&mut c, &Op::Expunge { keys: vec![stale] }).unwrap();
        let uid = c.append("Sent", "\\Seen", b"Subject: sent\r\n\r\nx\r\n").unwrap();
        assert_eq!(uid, Some(1));
        let mut s = st.lock().unwrap();
        let inbox: Vec<(u32, Vec<String>)> = s.mailbox("INBOX").unwrap().msgs.iter().map(|m| (m.uid, m.flags.clone())).collect();
        assert_eq!(inbox, vec![(1, vec!["\\Seen".to_string()])]);
        let arch = s.mailbox("Archive").unwrap();
        assert_eq!(arch.msgs.len(), 1);
        assert!(arch.msgs[0].flags.contains(&"\\Seen".to_string()));
        assert_eq!(s.mailbox("Sent").unwrap().msgs[0].raw, b"Subject: sent\r\n\r\nx\r\n");
    }

    /// For looking at the app against a mail server that acts as Outlook.com does, on
    /// 127.0.0.1:1143 without TLS (user name anything, password `pw`, OAuth token `good-token`):
    /// `MAILCRAFT_FAKE_REFUSE` refuses every password with its text ("Basic authentication is
    /// disabled"); `MAILCRAFT_FAKE_CLOSED` (an address) takes that address's token but won't
    /// open its mailbox. Run: `cargo test -p mailcraft-net serve_imap_for_the_app -- --ignored`.
    #[test]
    #[ignore]
    fn serve_imap_for_the_app() {
        let st = State::new();
        {
            let mut s = st.lock().unwrap();
            s.refuse_passwords = std::env::var("MAILCRAFT_FAKE_REFUSE").ok().filter(|v| !v.is_empty());
            s.not_connected.extend(std::env::var("MAILCRAFT_FAKE_CLOSED").ok().filter(|v| !v.is_empty()));
            s.add("INBOX", "Welcome", &[]);
            // `MAILCRAFT_FAKE_COUNT`: that many more in the Inbox, oldest first ("Message 1" is
            // the oldest; each body says `needle-<n>`), for more mail than is kept (issue #4).
            let count: u32 = std::env::var("MAILCRAFT_FAKE_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
            for n in 1..=count.min(20_000) {
                let day = 1 + n / 400;
                s.add_raw(
                    "INBOX",
                    format!(
                        "From: Ana <ana@studio.example>\r\nTo: me@studio.example\r\nSubject: Message {n}\r\nDate: Mon, {day:02} Jun 2025 10:00:00 +0000\r\nMessage-ID: <m{n}@studio.example>\r\n\r\nBody of message {n}: needle-{n}.\r\n"
                    )
                    .into_bytes(),
                );
            }
            // `MAILCRAFT_FAKE_EML`: messages to put in the Inbox (paths, separated by `;`).
            for path in std::env::var("MAILCRAFT_FAKE_EML").unwrap_or_default().split(';').filter(|p| !p.is_empty()) {
                if let Ok(raw) = std::fs::read(path) {
                    s.add_raw("INBOX", raw);
                }
            }
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:1143").unwrap();
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
    }

    #[test]
    fn xoauth2_sign_in() {
        for caps in ["IMAP4rev1 AUTH=XOAUTH2 SASL-IR", "IMAP4rev1 AUTH=XOAUTH2"] {
            let st = State::new();
            st.lock().unwrap().caps = caps;
            let mut c = Client::from_stream(Fake::connect(&st)).unwrap();
            c.capability().unwrap();
            c.login_xoauth2("me@studio.example", GOOD_TOKEN).unwrap();
            assert!(c.list("a1").is_ok(), "signed in with {caps}");
            let mut c = Client::from_stream(Fake::connect(&st)).unwrap();
            c.capability().unwrap();
            assert!(matches!(c.login_xoauth2("me@studio.example", "expired"), Err(crate::NetError::Auth(_))), "{caps}");
        }
    }

    #[test]
    fn keywords_round_trip_with_the_server() {
        let st = State::new();
        st.lock().unwrap().add("INBOX", "tagged", &[]);
        let store = store("keywords");
        let mut c = login(&st);
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid: 1 };
        apply_remote(&mut c, &Op::SetKeyword { keys: vec![key.clone()], keyword: "$label2".into(), on: true }).unwrap();
        apply_remote(&mut c, &Op::SetKeyword { keys: vec![key.clone()], keyword: "$Pinned".into(), on: true }).unwrap();
        // Not an atom: never sent.
        apply_remote(&mut c, &Op::SetKeyword { keys: vec![key], keyword: "two words".into(), on: true }).unwrap();
        assert!(!st.lock().unwrap().log.iter().any(|l| l.contains("two words")));
        sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
        let m = store.load_folder("a1", "INBOX").unwrap().messages[0].clone();
        assert!(m.flags.has("$label2") && m.flags.pinned(), "{:?}", m.flags);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn remote_folder_changes() {
        let st = State::new();
        st.lock().unwrap().add("Archive", "kept", &[]);
        let mut c = login(&st);
        apply_remote(&mut c, &Op::CreateFolder { path: "Clients".into() }).unwrap();
        // Creating an existing folder is not an error.
        apply_remote(&mut c, &Op::CreateFolder { path: "Clients".into() }).unwrap();
        c.select("Archive", false).unwrap();
        apply_remote(&mut c, &Op::RenameFolder { from: "Archive".into(), to: "Clients/Old".into() }).unwrap();
        assert!(c.selected().is_none(), "renaming the selected folder deselects it first");
        apply_remote(&mut c, &Op::DeleteFolder { path: "Projects/Caf&AOk-".into() }).unwrap();
        // Deleting a folder that's already gone is fine; a refused rename is an error.
        apply_remote(&mut c, &Op::DeleteFolder { path: "Nowhere".into() }).unwrap();
        assert!(matches!(apply_remote(&mut c, &Op::RenameFolder { from: "Nope".into(), to: "X".into() }), Err(crate::NetError::No(_))));
        let names: Vec<String> = c.list("a1").unwrap().into_iter().map(|f| f.path).collect();
        assert_eq!(names, vec!["INBOX", "Sent", "Clients/Old", "Clients"]);
        let s = st.lock().unwrap();
        assert_eq!(s.boxes.iter().find(|b| b.name == "Clients/Old").unwrap().msgs.len(), 1, "the mail moved with the folder");
        assert!(s.log.iter().any(|l| l.starts_with("SUBSCRIBE \"Clients\"")));
    }

    fn until(pool: &crate::worker::Pool, what: &str, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
        let end = Instant::now() + Duration::from_secs(20);
        let mut seen = Vec::new();
        while Instant::now() < end {
            if let Some(e) = pool.wait(Duration::from_millis(100)) {
                let hit = f(&e);
                seen.push(e);
                if hit {
                    return seen;
                }
            }
        }
        panic!("timed out waiting for {what}; saw {seen:?}");
    }

    #[test]
    fn mailbox_settings_over_imap_and_managesieve() {
        let st = State::new();
        st.lock().unwrap().mailbox("Archive").unwrap().msgs.push(Msg {
            uid: 1,
            flags: Vec::new(),
            raw: b"Subject: old

x"
            .to_vec(),
            modseq: 1,
        });
        st.lock().unwrap().mailbox("Archive").unwrap().uid_next = 2;
        let imap_port = serve_tcp(&st);
        let (sieve_port, sieve) = crate::sieve::tests::serve(crate::sieve::tests::Fake {
            scripts: Vec::new(),
            active: None,
            sieve: "fileinto vacation date relational include",
            log: Vec::new(),
        });
        let store = store("mailbox");
        let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
        pool.sieve_port = sieve_port;
        let account = AccountConfig {
            id: "a1".into(),
            email: "me@studio.example".into(),
            username: "me@studio.example".into(),
            imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
            smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
            ..Default::default()
        };
        pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
        let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        assert!(seen.iter().any(|e| matches!(e, Event::Quota { limit: 1_048_576, used, .. } if *used >= 1024)), "GETQUOTAROOT, in bytes");

        pool.submit(Job::GetVacation { account: "a1".into() });
        until(&pool, "replies read", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
        let away = mailcraft_model::job::Vacation { enabled: true, subject: "Away".into(), text: "Back soon".into(), ..Default::default() };
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
        until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
        assert_eq!(sieve.lock().unwrap().active.as_deref(), Some(crate::sieve::SCRIPT));

        // Emptying a folder removes what isn't downloaded too.
        pool.submit(Job::Apply { account: "a1".into(), op: Op::EmptyFolder { path: "Archive".into() } });
        let end = Instant::now() + Duration::from_secs(10);
        while !st.lock().unwrap().mailbox("Archive").unwrap().msgs.is_empty() {
            assert!(Instant::now() < end, "never emptied");
            std::thread::sleep(Duration::from_millis(50));
        }
        pool.submit(Job::Remove { account: "a1".into() });
        // An offline account answers at once.
        pool.submit(Job::GetVacation { account: "a1".into() });
        until(&pool, "offline answer", |e| matches!(e, Event::Vacation { vacation: None, error: Some(_), .. }));
        let _ = std::fs::remove_dir_all(store.root());
    }

    /// Send / Receive › Work Offline: connections close; changes, sent mail and drafts wait on
    /// this computer; turned off, the account connects and the draft reaches the server.
    #[test]
    fn working_offline_keeps_drafts_and_mail_until_back() {
        let st = State::new();
        let imap_port = serve_tcp(&st);
        let store = store("offline");
        let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig {
            id: "a1".into(),
            email: "me@studio.example".into(),
            username: "me@studio.example".into(),
            imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
            smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
            ..Default::default()
        };
        // Working offline before it starts: it never connects.
        pool.submit(Job::WorkOffline { on: true });
        pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
        until(&pool, "offline", |e| matches!(e, Event::Status { status: SyncStatus::Offline, .. }));
        let draft = b"From: me@studio.example
Subject: Kept

written offline
"
        .to_vec();
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: draft, folder: "INBOX".into(), replaces: None });
        until(&pool, "draft kept", |e| matches!(e, Event::Notice { message, .. } if message.contains("kept on this computer")));
        pool.submit(Job::Send {
            account: "a1".into(),
            id: 7,
            raw: b"Subject: x

y"
            .to_vec(),
            from: "me@studio.example".into(),
            recipients: vec!["a@b.example".into()],
            sent_folder: None,
        });
        until(&pool, "queued", |e| matches!(e, Event::Queued { id: 7, .. }));
        pool.submit(Job::SyncAll { account: "a1".into() });
        assert!(st.lock().unwrap().mailbox("INBOX").unwrap().msgs.is_empty(), "nothing reached the server");
        // Back online: the kept draft goes up first.
        pool.submit(Job::WorkOffline { on: false });
        let end = Instant::now() + Duration::from_secs(20);
        while st.lock().unwrap().mailbox("INBOX").unwrap().msgs.is_empty() {
            assert!(Instant::now() < end, "the draft never reached the server");
            std::thread::sleep(Duration::from_millis(50));
        }
        let saved = st.lock().unwrap().mailbox("INBOX").unwrap().msgs[0].raw.clone();
        assert!(String::from_utf8_lossy(&saved).contains("written offline"));
        assert!(store.outbox("a1").iter().any(|i| i.id == 7), "sent mail waits in the Outbox");
        pool.submit(Job::Remove { account: "a1".into() });
        let _ = std::fs::remove_dir_all(store.root());
    }

    /// Issue #4: the search box's words as IMAP SEARCH keys; what IMAP can't ask is left out.
    #[test]
    fn a_query_becomes_imap_search_keys() {
        use crate::sync::{SearchArg, imap_criteria};
        let a = |s: &str| SearchArg::Atom(s.into());
        let st = |s: &str| SearchArg::Str(s.into());
        assert_eq!(imap_criteria("quarterly"), Some(vec![a("TEXT"), st("quarterly")]));
        assert_eq!(
            imap_criteria("from:ana subject:\"the plan\" -draft is:unread"),
            Some(vec![a("FROM"), st("ana"), a("SUBJECT"), st("the plan"), a("NOT"), a("TEXT"), st("draft"), a("UNSEEN")])
        );
        assert_eq!(imap_criteria("is:flagged -is:unread"), Some(vec![a("FLAGGED"), a("SEEN")]));
        assert_eq!(imap_criteria("has:attachment"), None, "nothing IMAP can ask");
        assert_eq!(imap_criteria(""), None);
        assert_eq!(imap_criteria("Grüße"), Some(vec![a("TEXT"), st("grüße")]));
        // Sizes; a list as nested ORs (left to the list when one name is several words).
        assert_eq!(imap_criteria("larger:1KB -smaller:10"), Some(vec![a("LARGER"), a("1024"), a("NOT"), a("SMALLER"), a("10")]));
        assert_eq!(
            imap_criteria("from:ana,jun,\"zoe park\""),
            Some(vec![a("OR"), a("FROM"), st("ana"), a("OR"), a("FROM"), st("jun"), a("FROM"), st("zoe park")])
        );
        assert_eq!(imap_criteria("-to:ana,jun"), Some(vec![a("NOT"), a("OR"), a("TO"), st("ana"), a("TO"), st("jun")]));
        assert_eq!(
            imap_criteria("from:ana,jun lee"),
            Some(vec![a("TEXT"), st("lee")]).map(|v| [vec![a("OR"), a("FROM"), st("ana"), a("FROM"), st("jun")], v].concat())
        );
    }

    /// Gap #7: the query as Exchange's KQL/AQS words with the read, flag and attachment
    /// conditions apart, and as a JMAP filter. What a user types can't break out of a word: quotes
    /// and backslashes are dropped, a word spelled like an operator is quoted.
    #[test]
    fn a_query_becomes_exchange_words_and_a_jmap_filter() {
        use crate::sync::{WordQuery, word_query};
        use serde_json::json;
        let w = |text: &str| WordQuery { text: text.into(), ..Default::default() };
        assert_eq!(word_query("quarterly"), Some(w("quarterly")));
        assert_eq!(
            word_query("from:ana subject:\"the plan\" -draft is:unread"),
            Some(WordQuery { text: "from:ana subject:\"the plan\" NOT draft".into(), unread: Some(true), ..Default::default() })
        );
        assert_eq!(
            word_query("is:flagged -is:unread has:attachment"),
            Some(WordQuery { unread: Some(false), flagged: Some(true), attachment: Some(true), ..Default::default() })
        );
        assert_eq!(word_query(""), None);
        assert_eq!(word_query("is:important"), None, "nothing Exchange's searches ask");
        assert_eq!(word_query("OR NOT"), Some(w("or not")), "words, lowercased by the parser: not operators");
        assert_eq!(word_query("a(b) ana@x.example").map(|q| q.text), Some("a b ana x.example".into()), "the parser's own words");
        for hostile in ["subject:\"a\\\" OR from:x\" a(b)", "\"\"", "\\\"", "from:\"", "x\" OR \"y", "\u{0}\"\r\n", "-\"-\""] {
            let Some(q) = word_query(hostile) else { continue };
            assert_eq!(q.text.matches('"').count() % 2, 0, "{hostile:?} → {:?}", q.text);
            assert!(!q.text.contains('\\') && !q.text.contains("\"\"") && !q.text.chars().any(char::is_control), "{hostile:?} → {:?}", q.text);
        }
        let long = "x".repeat(5000);
        assert!(word_query(&long).unwrap().text.len() <= 202);
        let read = WordQuery { unread: Some(true), ..Default::default() };
        assert!(read.admits(&mailcraft_model::Flags::default(), false));
        assert!(!read.admits(&mailcraft_model::Flags { seen: true, ..Default::default() }, false));

        assert_eq!(
            mailcraft_jmap::search_filter("from:ana -draft is:unread is:flagged"),
            Some(json!({"operator": "AND", "conditions": [
                {"from": "ana"},
                {"operator": "NOT", "conditions": [{"text": "draft"}]},
                {"notKeyword": "$seen"},
                {"hasKeyword": "$flagged"},
            ]}))
        );
        assert_eq!(
            mailcraft_jmap::search_filter("-is:unread has:attachment"),
            Some(json!({"operator": "AND", "conditions": [{"hasKeyword": "$seen"}, {"hasAttachment": true}]}))
        );
        assert_eq!(mailcraft_jmap::search_filter("is:important"), None);
        assert_eq!(mailcraft_jmap::search_filter(""), None);
        // Sizes (minSize is at least, so one more byte), and a list as OR.
        assert_eq!(
            mailcraft_jmap::search_filter("larger:1KB smaller:2KB from:ana,\"zoe park\""),
            Some(json!({"operator": "AND", "conditions": [
                {"minSize": 1025},
                {"maxSize": 2048},
                {"operator": "OR", "conditions": [{"from": "ana"}, {"from": "zoe park"}]},
            ]}))
        );
    }

    /// Issue #4: mail past what's kept is found on the server. A folder keeps its newest 10 of
    /// 30; searching the server finds an old one by a word of its body (and one by a word in
    /// UTF-8, sent as a literal), brings it into the list, and then shows more fills in the
    /// newest of the rest without skipping what's around the found one.
    #[test]
    fn mail_past_whats_kept_is_found_and_shown_more() {
        let st = State::new();
        {
            let mut s = st.lock().unwrap();
            for n in 1..=30 {
                let body = if n == 3 { "Grüße und needle-three".to_string() } else { format!("body {n}") };
                s.add_raw("INBOX", format!("Subject: Message {n}\r\nMessage-ID: <m{n}@x>\r\n\r\n{body}\r\n").into_bytes());
            }
        }
        let store = store("past-kept");
        let mut c = login(&st);
        sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 10, prefetch: 0, ..Default::default() }).unwrap();
        assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 10);

        let criteria = crate::sync::imap_criteria("needle-three").unwrap();
        let (keys, new) = crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &criteria, 500).unwrap();
        assert_eq!((keys.iter().map(|k| k.uid).collect::<Vec<_>>(), new), (vec![3], 1));
        let kept = store.load_folder("a1", "INBOX").unwrap();
        assert!(kept.messages.iter().any(|m| m.subject == "Message 3") && kept.messages.len() == 11);
        // Found again: nothing new to bring in.
        assert_eq!(crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &criteria, 500).unwrap(), (keys.clone(), 0));
        // A word in UTF-8: sent as a literal with CHARSET UTF-8.
        let (keys, _) = crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &crate::sync::imap_criteria("Grüße").unwrap(), 500).unwrap();
        assert_eq!(keys.iter().map(|k| k.uid).collect::<Vec<_>>(), [3]);
        assert!(st.lock().unwrap().log.iter().any(|l| l.ends_with("UID SEARCH CHARSET UTF-8 TEXT ")), "a literal follows");
        // Nothing found: nothing changes.
        assert_eq!(
            crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &crate::sync::imap_criteria("absent").unwrap(), 500).unwrap(),
            (vec![], 0)
        );

        // Show more: 10 more, the newest of what isn't kept (20 down to 11), the found one kept.
        sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 20, prefetch: 0, ..Default::default() }).unwrap();
        let mut uids: Vec<u32> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.key.uid).collect();
        uids.sort_unstable();
        assert_eq!(uids, [3].into_iter().chain(12..=30).collect::<Vec<u32>>(), "the newest of the rest, up to 20 in all");
        // The usual window again: nothing is dropped.
        sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 10, prefetch: 0, ..Default::default() }).unwrap();
        assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 20);
        let _ = std::fs::remove_dir_all(store.root());
    }

    /// Issue #4 through the worker: Show more keeps the folder's larger window across syncs
    /// (windows.json), and a server search answers with what it found, in the list.
    #[test]
    fn show_more_and_search_the_server_through_the_worker() {
        let st = State::new();
        {
            let mut s = st.lock().unwrap();
            for n in 1..=25 {
                s.add_raw("INBOX", format!("Subject: Message {n}\r\nMessage-ID: <w{n}@x>\r\n\r\nbody needle-{n}\r\n").into_bytes());
            }
        }
        let port = serve_tcp(&st);
        let store = store("worker-more");
        let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
        pool.submit(Job::SetSync { messages: 10, offline: 0 });
        let account = AccountConfig {
            id: "a1".into(),
            email: "me@studio.example".into(),
            username: "me@studio.example".into(),
            imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
            smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
            ..Default::default()
        };
        pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
        until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 10);

        pool.submit(Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: "needle-2".into() });
        let seen = until(&pool, "searched", |e| matches!(e, Event::ServerSearched { .. }));
        let found = seen.iter().find_map(|e| match e {
            Event::ServerSearched { keys, error: None, query, .. } if query == "needle-2" => Some(keys.iter().map(|k| k.uid).collect::<Vec<_>>()),
            _ => None,
        });
        // "needle-2" is the words "needle" and "2", each found anywhere as IMAP's TEXT finds them
        // (so "needle-12" too).
        assert_eq!(found, Some(vec![2, 12, 20, 21, 22, 23, 24, 25]), "{seen:?}");
        assert!(seen.iter().any(|e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX")));
        assert!(store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.key.uid == 2));

        pool.submit(Job::MoreMail { account: "a1".into(), folder: "INBOX".into(), more: 5 });
        let end = Instant::now() + Duration::from_secs(20);
        while store.load_folder("a1", "INBOX").unwrap().messages.len() < 17 {
            assert!(Instant::now() < end, "never showed more");
            let _ = pool.wait(Duration::from_millis(100));
        }
        let windows: std::collections::HashMap<String, usize> = store.load_account_doc("a1", "windows.json").unwrap();
        assert_eq!(windows.get("INBOX"), Some(&17), "12 kept (10, and 2 found), 5 more");
        // A later sync keeps them.
        pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
        until(&pool, "synced", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 17);
        pool.submit(Job::Remove { account: "a1".into() });
        // Offline: answered at once.
        pool.submit(Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: "x".into() });
        until(&pool, "offline answer", |e| matches!(e, Event::ServerSearched { error: Some(_), .. }));
        let _ = std::fs::remove_dir_all(store.root());
    }

    /// Issue #4: an account kept online only syncs its folders and opens its mail as any other,
    /// but none of it reaches the disk: no data file, nothing downloaded ahead of reading, and
    /// once MailCraft closes, nothing of its mail is anywhere under the data folders. The other
    /// account beside it keeps its mail as before (and shows the search finds what's kept).
    #[test]
    fn an_online_only_account_syncs_and_reads_with_nothing_on_disk() {
        fn on_disk(dir: &std::path::Path, needle: &[u8]) -> bool {
            let Ok(rd) = std::fs::read_dir(dir) else { return false };
            rd.flatten().any(|e| {
                let p = e.path();
                if p.is_dir() { on_disk(&p, needle) } else { std::fs::read(&p).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle)) }
            })
        }
        let mail = |subject: &str, body: &str| {
            format!("From: Ana <ana@studio.example>\r\nTo: me@studio.example\r\nSubject: {subject}\r\nMessage-ID: <{subject}@studio.example>\r\n\r\n{body}\r\n").into_bytes()
        };
        let (online, kept) = (State::new(), State::new());
        for i in 1..=3 {
            online.lock().unwrap().add_raw("INBOX", mail(&format!("ONLINE-SUBJECT-{i}"), &format!("ONLINE-BODY-{i}")));
            kept.lock().unwrap().add_raw("INBOX", mail(&format!("KEPT-SUBJECT-{i}"), &format!("KEPT-BODY-{i}")));
        }
        let t = std::env::temp_dir().join(format!("mailcraft-fake-online-only-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let store = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
        let account = |id: &str, port: u16, online_only: bool| AccountConfig {
            id: id.into(),
            email: format!("{id}@studio.example"),
            username: "me@studio.example".into(),
            imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
            smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
            online_only,
            ..Default::default()
        };
        let (a1, a2) = (account("a1", serve_tcp(&online), true), account("a2", serve_tcp(&kept), false));
        store.save_accounts(&[a1.clone(), a2.clone()]).unwrap();
        // As the engine does when it opens.
        store.set_online_only("a1", true).unwrap();
        let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
        pool.submit(Job::Start { account: a1, password: Some(Secret("pw".into())) });
        pool.submit(Job::Start { account: a2, password: Some(Secret("pw".into())) });
        let end = Instant::now() + Duration::from_secs(20);
        let synced = |a: &str| store.load_folder(a, "INBOX").is_ok_and(|f| f.messages.len() == 3);
        while !(synced("a1") && synced("a2") && store.load_folder("a2", "INBOX").unwrap().messages.iter().all(|m| store.has_body(&m.key))) {
            assert!(Instant::now() < end, "never synced");
            let _ = pool.wait(Duration::from_millis(100));
        }
        let inbox = store.load_folder("a1", "INBOX").unwrap();
        assert!(inbox.messages.iter().any(|m| m.subject == "ONLINE-SUBJECT-2"));
        assert!(inbox.messages.iter().all(|m| !store.has_body(&m.key)), "nothing downloaded ahead of reading");
        // Opening one downloads it, into memory.
        let key = inbox.messages[0].key.clone();
        pool.submit(Job::FetchBody { key: key.clone() });
        let seen = until(&pool, "the opened message", |e| matches!(e, Event::Body { key: k, .. } if *k == key));
        assert!(seen.iter().any(|e| matches!(e, Event::Body { raw, .. } if String::from_utf8_lossy(raw).contains("ONLINE-BODY"))));
        assert!(store.has_body(&key));
        assert_eq!(store.data_file("a1"), None, "no data file");

        // MailCraft closes (its threads let the data files go).
        let kept_file = store.data_file("a2").unwrap();
        drop(pool);
        drop(store);
        let end = Instant::now() + Duration::from_secs(20);
        while std::fs::read(&kept_file).is_err() {
            assert!(Instant::now() < end, "the data file stayed open");
            std::thread::sleep(Duration::from_millis(50));
        }
        for i in 1..=3 {
            assert!(!on_disk(&t, format!("ONLINE-SUBJECT-{i}").as_bytes()) && !on_disk(&t, format!("ONLINE-BODY-{i}").as_bytes()), "{i}");
            assert!(on_disk(&t, format!("KEPT-SUBJECT-{i}").as_bytes()) && on_disk(&t, format!("KEPT-BODY-{i}").as_bytes()), "{i}");
        }
        let files: Vec<String> =
            std::fs::read_dir(t.join("MailCraft Files")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(files, ["a2@studio.example.mailcraft"], "only the other account has a data file");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn outbox_holds_mail_until_smtp_is_back() {
        let st = State::new();
        let imap_port = serve_tcp(&st);
        // An SMTP port with nothing listening yet: sending fails at once (connection refused).
        let smtp_port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let store = store("outbox");
        let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig {
            id: "a1".into(),
            email: "me@studio.example".into(),
            username: "me@studio.example".into(),
            imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
            smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
            ..Default::default()
        };
        // Not started yet: a send goes straight to the Outbox.
        let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: queued\r\nMessage-ID: <q@x>\r\n\r\nhi\r\n".to_vec();
        let send = |id: u64| Job::Send {
            account: "a1".into(),
            id,
            raw: raw.clone(),
            from: "me@studio.example".into(),
            recipients: vec!["zoe@studio.example".into()],
            sent_folder: Some("Sent".into()),
        };
        pool.submit(send(1));
        until(&pool, "queued offline", |e| matches!(e, Event::Queued { id: 1, .. }));
        assert_eq!(store.outbox("a1").len(), 1);

        // Online, but the SMTP server is down: the next message joins the queue.
        pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
        until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        pool.submit(send(2));
        until(&pool, "queued on failure", |e| matches!(e, Event::Queued { id: 2, .. }));
        assert_eq!(store.outbox("a1").len(), 2);
        assert!(store.outbox("a1")[1].error.as_deref().is_some_and(|e| e.contains("network")), "{:?}", store.outbox("a1"));

        // A message held back for tomorrow (Delay Delivery) isn't sent with them.
        let later = mailcraft_store::OutboxItem {
            id: 3,
            from: "me@studio.example".into(),
            recipients: vec!["zoe@studio.example".into()],
            not_before: crate::worker::now() + 86_400,
            ..Default::default()
        };
        store.outbox_add("a1", &later, &raw).unwrap();

        // The server comes back: Send now delivers both, oldest first, and files copies.
        let (_, log1) = crate::smtp::tests::serve_once_on(std::net::TcpListener::bind(("127.0.0.1", smtp_port)).unwrap());
        pool.submit(Job::FlushOutbox { account: "a1".into() });
        until(&pool, "first sent", |e| matches!(e, Event::Sent { id: 1, .. }));
        assert!(log1.join().unwrap().contains("Subject: queued"));
        // serve_once takes one message; the second waits for the next server.
        let (_, log2) = crate::smtp::tests::serve_once_on(std::net::TcpListener::bind(("127.0.0.1", smtp_port)).unwrap());
        pool.submit(Job::FlushOutbox { account: "a1".into() });
        until(&pool, "second sent", |e| matches!(e, Event::Sent { id: 2, .. }));
        assert!(log2.join().unwrap().contains("RCPT TO:<zoe@studio.example>"));
        assert_eq!(store.outbox("a1").iter().map(|i| i.id).collect::<Vec<_>>(), vec![3], "the held-back one waits");
        let end = Instant::now() + Duration::from_secs(10);
        while st.lock().unwrap().mailbox("Sent").unwrap().msgs.len() < 2 {
            assert!(Instant::now() < end, "copies never reached Sent");
            std::thread::sleep(Duration::from_millis(50));
        }
        pool.submit(Job::Remove { account: "a1".into() });
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn worker_pool_end_to_end_over_tcp() {
        let st = State::new();
        {
            let mut s = st.lock().unwrap();
            s.add("INBOX", "hello", &[]);
            s.add("INBOX", "second", &["\\Seen"]);
        }
        let imap_port = serve_tcp(&st);
        let (smtp_port, smtp_log) = crate::smtp::tests::serve_once();
        let store = store("e2e");
        let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig {
            id: "a1".into(),
            name: "Me".into(),
            email: "me@studio.example".into(),
            username: "me@studio.example".into(),
            imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
            smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
            ..Default::default()
        };

        // Wrong password first: the worker reports it and stops retrying.
        pool.submit(Job::Start { account: account.clone(), password: Some(Secret("nope".into())) });
        until(&pool, "password prompt", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));

        // Right password: folders, the Inbox, then online.
        pool.submit(Job::Start { account: account.clone(), password: Some(Secret("pw".into())) });
        let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        assert!(seen.iter().any(|e| matches!(e, Event::Folders { folders, .. } if folders.len() == 4)));
        assert!(seen.iter().any(|e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX")));
        let inbox = store.load_folder("a1", "INBOX").unwrap();
        assert_eq!(inbox.messages.len(), 2);

        // Open a message (already prefetched: answered from the cache).
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid: 1 };
        pool.submit(Job::FetchBody { key: key.clone() });
        until(&pool, "body", |e| matches!(e, Event::Body { key: k, raw } if *k == key && raw.starts_with(b"From:")));

        // Mark read: cache at once, server right after.
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
        until(&pool, "flag change", |e| matches!(e, Event::FolderChanged { .. }));
        assert!(store.load_folder("a1", "INBOX").unwrap().find(1).unwrap().flags.seen);
        let end = Instant::now() + Duration::from_secs(10);
        while !st.lock().unwrap().mailbox("INBOX").unwrap().msgs[0].flags.iter().any(|f| f == "\\Seen") {
            assert!(Instant::now() < end, "the server never got the flag");
            std::thread::sleep(Duration::from_millis(50));
        }

        // Send: SMTP submission, then a copy filed in Sent.
        let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: e2e\r\nMessage-ID: <e2e@x>\r\n\r\nhi\r\n".to_vec();
        pool.submit(Job::Send {
            account: "a1".into(),
            id: 7,
            raw,
            from: "me@studio.example".into(),
            recipients: vec!["zoe@studio.example".into()],
            sent_folder: Some("Sent".into()),
        });
        until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
        until(&pool, "sent copy", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Sent"));
        assert_eq!(st.lock().unwrap().mailbox("Sent").unwrap().msgs.len(), 1);
        assert!(smtp_log.join().unwrap().contains("Subject: e2e"));
        assert_eq!(store.load_folder("a1", "Sent").unwrap().messages[0].subject, "e2e");

        pool.submit(Job::Remove { account: "a1".into() });
        let _ = std::fs::remove_dir_all(store.root());
    }

    /// Issue #3: an outgoing server whose user name and password aren't the incoming server's.
    /// Without its password kept, sending says so (and nothing goes as the IMAP user); once kept,
    /// SMTP signs in with its own user name and password while IMAP keeps its own.
    #[test]
    fn the_outgoing_server_signs_in_with_its_own_user_name_and_password() {
        let st = State::new();
        let imap_port = serve_tcp(&st);
        let (smtp_port, smtp_log) = crate::smtp::tests::serve_once();
        let store = store("smtp-own-sign-in");
        let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig {
            id: "a1".into(),
            email: "me@studio.example".into(),
            username: "me@studio.example".into(),
            imap: ServerConfig { host: "127.0.0.1".into(), port: imap_port, security: Security::None },
            smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
            smtp_username: Some("relay-user".into()),
            ..Default::default()
        };
        pool.submit(Job::Start { account: account.clone(), password: Some(Secret("pw".into())) });
        until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        let send = |id: u64| Job::Send {
            account: "a1".into(),
            id,
            raw: b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: relay\r\n\r\nhi\r\n".to_vec(),
            from: "me@studio.example".into(),
            recipients: vec!["zoe@studio.example".into()],
            sent_folder: None,
        };
        pool.submit(send(1));
        until(&pool, "no password", |e| match e {
            Event::SendFailed { id: 1, error, .. } | Event::Queued { id: 1, error, .. } => error.contains("outgoing server's password"),
            _ => false,
        });
        pool.submit(Job::SmtpPassword { account: account.clone(), password: Secret("smtp-secret".into()) });
        pool.submit(send(2));
        until(&pool, "sent", |e| matches!(e, Event::Sent { id: 2, .. }));
        let log = smtp_log.join().unwrap();
        let own = crate::imap::base64(b"\0relay-user\0smtp-secret");
        assert!(log.contains(&own), "SMTP signed in as relay-user: {log}");
        assert!(!log.contains(&crate::imap::base64(b"\0me@studio.example\0pw")), "not as the IMAP user");
        pool.submit(Job::Remove { account: "a1".into() });
        let _ = std::fs::remove_dir_all(store.root());
    }
}
