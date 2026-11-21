//! An in-memory IMAP server for tests: enough of RFC 3501 + UIDPLUS, MOVE and CONDSTORE to run
//! the real client and sync code against, with state the test can change between syncs.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

use mailcraft_store::Store;

use crate::imap::Client;
use crate::sync::{SyncOptions, sync_folder};
use search::{in_set, search_matches, search_tokens};
use serving::login;
pub use serving::serve_tcp;
use serving::serve_tcp_on;
pub(crate) use serving::{search_the_server_past_the_window, store};

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

impl Mailbox {
    /// An empty folder: its name, its SPECIAL-USE attribute (or none), its UIDVALIDITY.
    pub fn new(name: impl Into<String>, attr: &'static str, validity: u32) -> Mailbox {
        Mailbox { name: name.into(), attr, validity, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() }
    }
}

pub struct State {
    pub boxes: Vec<Mailbox>,
    pub modseq: u64,
    pub caps: &'static str,
    /// `NOTIFY SET` is refused (BAD) though `caps` says the server has it.
    pub refuse_notify: bool,
    /// Every command received (tags stripped).
    pub log: Vec<String>,
    /// Every command received, with how long its connection had been quiet when it came: since
    /// the answer to the one before, or since the connection was made. Measured here, as the
    /// client's waits are, and kept apart from `log` (which tests clear).
    pub quiet: Vec<(String, std::time::Duration)>,
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
    /// The connection the next command comes on is dropped without an answer, as a server drops
    /// one that's been quiet.
    pub hang_up: bool,
    /// How long each read of the connection waits (a server trickling its answer).
    pub trickle: std::time::Duration,
    /// Mail delivered to the Inbox (this subject) when a command containing this text first
    /// comes: mail arriving at a set point of a sync.
    pub deliver_on: Option<(String, String)>,
    /// Folders can only be made inside this one (`INBOX/`), as cPanel's and Courier's servers
    /// keep every folder in the Inbox.
    pub create_under: Option<String>,
    /// The `PERMANENTFLAGS` every folder reports, when set (`\Seen $label2`: no `\*`, so no
    /// new keywords, as on a shared folder without the right to write); a STORE of another
    /// keyword is taken and forgotten, as Dovecot does.
    pub permanent_flags: Option<&'static str>,
    /// Commands containing this text take this long (a big folder on a slow server): the
    /// connection that sent it waits, others don't.
    pub stall_on: Option<(String, std::time::Duration)>,
    /// Commands that have begun to take that long: on their way, not answered yet.
    pub stalled: usize,
    /// The first command containing a text, held unanswered until the test lets it go
    /// ([`Hold::on`]).
    pub hold_on: Option<Hold>,
    /// A header line the server adds to every message it's given (`APPEND`), so it keeps each at
    /// another size than sent, as a server that stamps what it stores does.
    pub stamp_appends: Option<&'static str>,
    /// An `APPEND` with this text in it is refused (a keyword, a date the server won't keep).
    pub refuse_appends: Option<&'static str>,
}

impl State {
    pub fn new() -> Arc<Mutex<State>> {
        Arc::new(Mutex::new(State {
            boxes: vec![
                Mailbox::new("INBOX", "", 100),
                Mailbox::new("Sent", "\\Sent", 200),
                Mailbox::new("Archive", "\\Archive", 300),
                Mailbox::new("Projects/Caf&AOk-", "", 400),
            ],
            modseq: 10,
            caps: "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA",
            refuse_notify: false,
            log: Vec::new(),
            quiet: Vec::new(),
            not_connected: Vec::new(),
            refuse_passwords: None,
            search_delay: std::time::Duration::ZERO,
            flood: None,
            hang_up: false,
            trickle: std::time::Duration::ZERO,
            deliver_on: None,
            create_under: None,
            permanent_flags: None,
            stall_on: None,
            stalled: 0,
            hold_on: None,
            stamp_appends: None,
            refuse_appends: None,
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

/// A draft's copy as MailCraft wrote it: without the mark it files each copy with, first in its
/// header (after a line the server stamped it with, if any).
pub fn as_written(raw: &[u8]) -> Vec<u8> {
    let mark = format!("{}: ", mailcraft_mime::COPY_HEADER).into_bytes();
    let Some(at) = raw.windows(mark.len()).position(|w| w == mark.as_slice()) else { return raw.to_vec() };
    let end = raw[at..].windows(2).position(|w| w == b"\r\n").map_or(raw.len(), |e| at + e + 2);
    [&raw[..at], &raw[end..]].concat()
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
    /// NOTIFY is on (`notifying.rs`): each folder's next UID when last told of.
    notifying: Option<std::collections::HashMap<String, u32>>,
    /// The next command on this connection is dropped unanswered (see [`LetGo`]).
    hang_up_next: bool,
    /// When the last command was answered, or the connection made (see `State::quiet`).
    answered: std::time::Instant,
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
            notifying: None,
            hang_up_next: false,
            answered: std::time::Instant::now(),
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
                self.answered = std::time::Instant::now();
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
            self.answered = std::time::Instant::now();
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
        let quiet = self.answered.elapsed();
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
        let stall = self.st.lock().ok().and_then(|s| s.stall_on.clone()).filter(|(text, _)| line.contains(text.as_str()));
        if let Some((_, how_long)) = stall {
            if let Ok(mut s) = self.st.lock() {
                s.stalled += 1;
            }
            std::thread::sleep(how_long);
        }
        self.held(line);
        let st = self.st.clone();
        let mut st = st.lock().unwrap();
        st.log.push(rest.to_string());
        st.quiet.push((rest.to_string(), quiet));
        let upper = rest.to_ascii_uppercase();
        if let Some((on, subject)) = st.deliver_on.clone()
            && rest.contains(on.as_str())
        {
            st.deliver_on = None;
            st.add("INBOX", &subject, &[]);
        }
        if upper.starts_with("NOTIFY SET") {
            self.notify_set(&st, tag);
            return;
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
            self.ok(tag, "");
        } else if upper.starts_with("LOGIN ") {
            self.login(&st, tag, rest);
        } else if upper.starts_with("GETQUOTAROOT") {
            let used: usize = st.boxes.iter().flat_map(|b| b.msgs.iter()).map(|m| m.raw.len()).sum::<usize>() / 1024 + 1;
            self.say("* QUOTAROOT INBOX \"\"");
            self.say(&format!("* QUOTA \"\" (STORAGE {used} 1024)"));
            self.ok(tag, "");
        } else if upper.starts_with("ENABLE") {
            if upper.contains("QRESYNC") && st.caps.contains("QRESYNC") {
                self.say("* ENABLED QRESYNC");
            } else {
                self.say("* ENABLED CONDSTORE");
            }
            self.ok(tag, "");
        } else if upper.starts_with("LIST") {
            self.list(&st, tag);
        } else if upper.starts_with("SELECT ") || upper.starts_with("EXAMINE ") {
            self.select(&mut st, tag, rest, &upper);
        } else if upper.starts_with("CREATE ") {
            self.create(&mut st, tag, rest);
        } else if upper.starts_with("RENAME ") {
            self.rename(&mut st, tag, rest);
        } else if upper.starts_with("DELETE ") {
            self.delete(&mut st, tag, rest);
        } else if upper.starts_with("SUBSCRIBE ") || upper.starts_with("UNSUBSCRIBE ") || upper == "UNSELECT" {
            if upper == "UNSELECT" {
                self.selected = None;
            }
            self.ok(tag, "");
        } else if upper.starts_with("APPEND ") && st.refuse_appends.is_some_and(|text| rest.contains(text)) {
            self.say(&format!("{tag} NO won't keep it"));
        } else if upper.starts_with("APPEND ") {
            self.append(&mut st, tag, rest, literal);
        } else if upper.starts_with("UID ") || upper == "EXPUNGE" || upper == "NOOP" || upper == "LOGOUT" {
            self.on_selected(&mut st, tag, rest, &upper, literal);
        } else {
            self.say(&format!("{tag} BAD unsupported"));
        }
    }

    /// The tagged `OK` that ends a command, `extra` (a response code) before its text.
    fn ok(&mut self, tag: &str, extra: &str) {
        self.say(&format!("{tag} OK {extra}done"));
    }

    fn login(&mut self, st: &State, tag: &str, rest: &str) {
        if let Some(why) = &st.refuse_passwords {
            self.say(&format!("{tag} NO {why}"));
        } else if rest.ends_with("\"pw\"") {
            self.ok(tag, &format!("[CAPABILITY {}] ", st.caps));
        } else {
            self.say(&format!("{tag} NO [AUTHENTICATIONFAILED] wrong password"));
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

/// The most a read hands the client: short and odd-sized, so the client's reading of lines and
/// literals across many partial reads is exercised, as a slow link splits them.
const SHORT_READ: usize = 11;

impl Read for Fake {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let trickle = self.st.lock().map(|s| s.trickle).unwrap_or_default();
        if !trickle.is_zero() {
            std::thread::sleep(trickle);
        }
        if self.out.is_empty() {
            self.tell_notified();
        }
        if self.out.is_empty()
            && let Some(f) = &self.flood
        {
            self.out.extend(f.iter());
        }
        let n = buf.len().min(self.out.len()).min(SHORT_READ);
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

mod holding;
mod mailboxes;
mod messages;
mod notifying;
mod search;
mod serving;
mod structure;
pub use holding::{Hold, LetGo};
#[cfg(test)]
mod tests;
