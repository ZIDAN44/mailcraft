//! A small IMAP4rev1 client (RFC 3501).
//!
//! Framing is done here: a response is one line plus any `{n}` literals it announces, and it is
//! only taken off the buffer once complete, so a read that times out (IDLE waits in short slices)
//! never loses half a response. Parsing is `imap-proto`'s. Server input is hostile: line and
//! literal sizes are capped and nothing indexes unchecked.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use imap_proto::{AttributeValue, Capability, MailboxDatum, NameAttribute, Response, ResponseCode, Status, StatusAttribute};
use mailcraft_model::{Folder, FolderRole, ServerConfig};

use crate::stream::{self, IO_TIMEOUT, Stream, Timeouts};
use crate::{NetError, Result};

mod conn;
mod folders;
mod idle;
mod messages;
mod quotes;
mod sign_in;
mod text_first;
use conn::check_unknown;
#[cfg(test)]
use conn::literal_len;
pub use conn::{Conn, Limits};
pub use folders::{Resync, SelectInfo};
pub use messages::Fetched;
use messages::fetched;
pub use text_first::TEXT_FIRST_OVER;

/// Largest literal accepted (a message source).
pub const MAX_LITERAL: usize = 64 << 20;
/// Most folders kept as said to have changed between looks (NOTIFY): past that, the rest wait
/// for the next full sync.
const MAX_NOTIFIED: usize = 200;

/// A piece of a command: plain text, or a literal sent after the server's `+` go-ahead.
pub enum Part<'a> {
    Text(&'a str),
    Literal(&'a [u8]),
}

/// An IMAP `astring` argument: quoted when printable ASCII, else a literal (8-bit allowed).
pub fn quoted(s: &str) -> Option<String> {
    s.chars().all(|c| (' '..='~').contains(&c)).then(|| super::quote(s))
}

/// Mailbox names are 7-bit (modified UTF-7) and go quoted. One a server listed in UTF-8 goes
/// quoted too, whole: its letters outside ASCII were left out, which named another mailbox
/// (`Büro` became `Bro`). Control characters never go: a line end would start a command.
fn mailbox_arg(name: &str) -> String {
    quoted(name).unwrap_or_else(|| super::quote(&name.chars().filter(|c| !c.is_control()).collect::<String>()))
}

/// Compress UIDs into an IMAP sequence set (`1:4,7,9:12`).
pub fn uid_set(uids: &[u32]) -> String {
    let mut v: Vec<u32> = uids.iter().copied().filter(|u| *u > 0).collect();
    v.sort_unstable();
    v.dedup();
    let mut out = String::new();
    let mut i = 0;
    while let Some(&start) = v.get(i) {
        let mut end = start;
        while v.get(i + 1).is_some_and(|&n| Some(n) == end.checked_add(1)) {
            i += 1;
            end = v.get(i).copied().unwrap_or(end);
        }
        if !out.is_empty() {
            out.push(',');
        }
        if start == end {
            out.push_str(&start.to_string());
        } else {
            out.push_str(&format!("{start}:{end}"));
        }
        i += 1;
    }
    out
}

/// Unix seconds as an INTERNALDATE in UTC (`14-Oct-2026 07:30:00 +0000`); none for a time
/// before 1970 or past the year 9999, which no server takes.
pub fn internal_date(t: i64) -> Option<String> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    if !(1..253_402_300_800).contains(&t) {
        return None;
    }
    let (y, m, d) = mailcraft_model::dates::civil_from_days(t.div_euclid(86_400));
    let secs = t.rem_euclid(86_400);
    let month = MONTHS.get(usize::try_from(m.checked_sub(1)?).ok()?)?;
    Some(format!("{d:02}-{month}-{y:04} {:02}:{:02}:{:02} +0000", secs / 3600, secs / 60 % 60, secs % 60))
}

/// Parse an INTERNALDATE (`14-Oct-2026 09:30:00 +0200`) to Unix seconds. The server's numbers
/// are digits only, the year from 1 to 9999 (RFC 3501 gives it four digits).
pub fn parse_internal_date(s: &str) -> Option<i64> {
    // Digits only: `parse` alone takes a sign, and an unbounded year overflowed the day count.
    let num = |x: &str| -> Option<i64> { (!x.is_empty() && x.len() <= 4 && x.bytes().all(|b| b.is_ascii_digit())).then(|| x.parse().ok())? };
    let s = s.trim();
    let (date, rest) = s.split_once(' ')?;
    let mut d = date.trim().split('-');
    let day = num(d.next()?.trim())?;
    let mon = d.next()?;
    let year = num(d.next()?)?;
    let month =
        ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|m| m.eq_ignore_ascii_case(mon))? as u32
            + 1;
    let mut parts = rest.split_whitespace();
    let time = parts.next()?;
    let zone = parts.next().unwrap_or("+0000");
    let mut t = time.split(':');
    let h = num(t.next()?)?;
    let m = num(t.next()?)?;
    let sec = num(t.next().unwrap_or("0"))?;
    let sign = if zone.starts_with('-') { -1 } else { 1 };
    let z = zone.trim_start_matches(['+', '-']);
    let zh = num(z.get(..2)?)?;
    let zm = num(z.get(2..4)?)?;
    if !(1..=31).contains(&day) || year < 1 || h > 23 || m > 59 || sec > 60 {
        return None;
    }
    let day = u32::try_from(day).ok()?;
    let days = mailcraft_model::dates::days_from_civil(year, month, day);
    Some(days * 86_400 + h * 3600 + m * 60 + sec - sign * (zh * 3600 + zm * 60))
}

fn info(o: &imap_proto::Outcome<'_>) -> String {
    o.information.as_deref().unwrap_or("").to_string()
}

/// A `NO`: one saying the folder isn't there and may be made (`[TRYCREATE]`) told apart.
fn refused(o: &imap_proto::Outcome<'_>) -> NetError {
    if matches!(o.code, Some(ResponseCode::TryCreate)) { NetError::NoFolder(info(o)) } else { NetError::No(info(o)) }
}

/// An authenticated or not-yet-authenticated IMAP session.
pub struct Client<S> {
    conn: Conn<S>,
    caps: HashSet<String>,
    selected: Option<String>,
    /// QRESYNC is enabled on this connection (RFC 7162 §3.2.3).
    qresync: bool,
    /// What the last `SELECT` of each folder reported on this connection.
    seen: std::collections::HashMap<String, SelectInfo>,
    /// The selected folder's message count, as the server last said.
    exists: u32,
    /// The server said the selected folder grew (an untagged `EXISTS` with a new count) during a
    /// command since it was selected: new mail no command has looked at yet.
    news: bool,
    /// The server tells of changes in every folder of the user's (NOTIFY, RFC 5465), not only the
    /// selected one: asked once on this connection (issue #232).
    notifying: bool,
    /// The folders it said changed since asked ([`Client::take_notified`]).
    notified: Vec<String>,
}

fn caps_from(list: &[Capability<'_>]) -> HashSet<String> {
    list.iter()
        .map(|c| match c {
            Capability::Imap4rev1 => "IMAP4REV1".to_string(),
            Capability::Auth(a) => format!("AUTH={}", a.to_ascii_uppercase()),
            Capability::Atom(a) => a.to_ascii_uppercase(),
        })
        .collect()
}

impl Client<Stream> {
    /// [`Self::connect_trusting`] trusting no certificate by hand (the tests' servers).
    #[cfg(test)]
    pub fn connect(cfg: &ServerConfig) -> Result<Client<Stream>> {
        Self::connect_trusting(cfg, stream::Trusted::default())
    }

    /// Connect and read the greeting, upgrading with STARTTLS when configured: `trusted` is what
    /// the account trusts besides the usual authorities ([`stream::tls_trusting`]).
    pub fn connect_trusting(cfg: &ServerConfig, trusted: stream::Trusted<'_>) -> Result<Client<Stream>> {
        let sock = stream::tcp(&cfg.host, cfg.port)?;
        match cfg.security {
            mailcraft_model::Security::Tls => Client::from_stream(stream::tls_trusting(sock, &cfg.host, trusted)?),
            mailcraft_model::Security::None => Client::from_stream(Stream::Plain(sock)),
            mailcraft_model::Security::StartTls => {
                let mut c = Client::from_stream(Stream::Plain(sock))?;
                if !c.has("STARTTLS") {
                    c.capability()?;
                }
                if !c.has("STARTTLS") {
                    return Err(NetError::Tls("the server does not offer STARTTLS".into()));
                }
                c.conn.run(&[Part::Text("STARTTLS")])?;
                let unread = c.conn.unread();
                let mut c = Client::new(stream::starttls(c.conn.into_inner(), unread, &cfg.host, trusted)?);
                // Capabilities learned before TLS must be discarded (RFC 3501 §6.2.1).
                c.capability()?;
                Ok(c)
            }
        }
    }
}

impl<S: Read + Write> Client<S> {
    /// A session on `s` that knows nothing of the server yet.
    fn new(s: S) -> Client<S> {
        Client {
            conn: Conn::new(s),
            caps: HashSet::new(),
            selected: None,
            qresync: false,
            seen: Default::default(),
            exists: 0,
            news: false,
            notifying: false,
            notified: Vec::new(),
        }
    }

    /// Wrap a connected stream and read the server greeting.
    pub fn from_stream(s: S) -> Result<Client<S>> {
        let mut c = Client::new(s);
        match c.conn.read()? {
            Ok(Response::Data { status: Status::Ok | Status::PreAuth, outcome }) => {
                if let Some(ResponseCode::Capabilities(list)) = &outcome.code {
                    c.caps = caps_from(list);
                }
                Ok(c)
            }
            Ok(Response::Data { status: Status::Bye, outcome }) => Err(NetError::Closed(info(&outcome))),
            Ok(other) => Err(NetError::Protocol(format!("unexpected greeting: {other:?}"))),
            Err(raw) => Err(NetError::Protocol(format!("unexpected greeting: {}", String::from_utf8_lossy(&raw).trim()))),
        }
    }

    pub fn conn_mut(&mut self) -> &mut Conn<S> {
        &mut self.conn
    }

    pub fn has(&self, cap: &str) -> bool {
        self.caps.contains(&cap.to_ascii_uppercase())
    }

    pub fn capability(&mut self) -> Result<()> {
        let rs = self.cmd(&[Part::Text("CAPABILITY")])?;
        for r in rs {
            if let Response::Capabilities(list) = r {
                self.caps = caps_from(&list);
            }
        }
        Ok(())
    }

    /// Whether QRESYNC is on for this connection.
    pub fn qresync(&self) -> bool {
        self.qresync
    }

    /// Run a command, noting what it says about the selected folder's size (RFC 3501 §7.3.1:
    /// any command's answer may carry `EXISTS` for mail that just arrived).
    fn cmd(&mut self, parts: &[Part<'_>]) -> Result<Vec<Response<'static>>> {
        let rs = self.conn.run(parts)?;
        for r in &rs {
            self.note_notified(r);
        }
        if self.selected.is_some() {
            for r in &rs {
                match r {
                    Response::MailboxData(MailboxDatum::Exists(n)) => {
                        self.news |= *n > self.exists;
                        self.exists = *n;
                    }
                    Response::Expunge(_) => self.exists = self.exists.saturating_sub(1),
                    // QRESYNC's word for it (RFC 7162 §3.2.10), a range a time: uncounted, the
                    // next message's `EXISTS` was no larger and wasn't taken for new mail.
                    Response::Vanished { earlier: false, uids } => {
                        let gone: u32 = uids.iter().map(|r| r.end().saturating_sub(*r.start()).saturating_add(1)).fold(0, u32::saturating_add);
                        self.exists = self.exists.saturating_sub(gone);
                    }
                    _ => {}
                }
            }
        }
        Ok(rs)
    }

    /// A folder the server says changed (an untagged `STATUS` once NOTIFY is on), noted.
    pub(super) fn note_notified(&mut self, r: &Response<'_>) {
        if !self.notifying {
            return;
        }
        if let Response::MailboxData(MailboxDatum::Status { mailbox, .. }) = r
            && self.notified.len() < MAX_NOTIFIED
            && !self.notified.iter().any(|f| f == mailbox.as_ref())
        {
            self.notified.push(mailbox.to_string());
        }
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Space used on the server and its limit, in bytes (`GETQUOTAROOT`, RFC 9208); `None` when the
    /// server has no QUOTA extension or no storage limit for the Inbox.
    pub fn quota(&mut self) -> Result<Option<(u64, u64)>> {
        if !self.has("QUOTA") && !self.caps.iter().any(|c| c.starts_with("QUOTA=")) {
            return Ok(None);
        }
        let rs = self.cmd(&[Part::Text("GETQUOTAROOT INBOX")])?;
        for r in rs {
            if let Response::Quota(q) = r
                && let Some(res) = q.resources.iter().find(|r| r.name == imap_proto::rfc2087::QuotaResourceName::Storage)
            {
                // Units of 1024 octets.
                return Ok(Some((res.usage.saturating_mul(1024), res.limit.saturating_mul(1024))));
            }
        }
        Ok(None)
    }

    pub fn noop(&mut self) -> Result<Vec<Response<'static>>> {
        self.cmd(&[Part::Text("NOOP")])
    }

    pub fn logout(&mut self) {
        let _ = self.cmd(&[Part::Text("LOGOUT")]);
    }
}

#[cfg(test)]
mod tests;
