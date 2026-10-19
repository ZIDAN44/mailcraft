//! A small IMAP4rev1 client (RFC 3501).
//!
//! Framing is done here: a response is one line plus any `{n}` literals it announces, and it is
//! only taken off the buffer once complete, so a read that times out (IDLE waits in short slices)
//! never loses half a response. Parsing is `imap-proto`'s. Server input is hostile: line and
//! literal sizes are capped and nothing indexes unchecked.

use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use imap_proto::{AttributeValue, Capability, MailboxDatum, NameAttribute, Response, ResponseCode, Status, StatusAttribute};
use mailcraft_model::{Folder, FolderRole, ServerConfig};

use crate::stream::{self, IO_TIMEOUT, Stream, Timeouts};
use crate::{NetError, Result};

/// Largest literal accepted (a message source).
pub const MAX_LITERAL: usize = 64 << 20;
/// Longest line accepted outside literals.
const MAX_LINE: usize = 1 << 20;

/// A piece of a command: plain text, or a literal sent after the server's `+` go-ahead.
pub enum Part<'a> {
    Text(&'a str),
    Literal(&'a [u8]),
}

/// An IMAP `astring` argument: quoted when printable ASCII, else a literal (8-bit allowed).
pub fn quoted(s: &str) -> Option<String> {
    if s.chars().all(|c| (' '..='~').contains(&c)) { Some(format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))) } else { None }
}

/// Mailbox names are already 7-bit (modified UTF-7); quote them, or fall back to a literal.
fn mailbox_arg(name: &str) -> String {
    quoted(name).unwrap_or_else(|| format!("\"{}\"", name.chars().filter(|c| (' '..='~').contains(c) && *c != '"' && *c != '\\').collect::<String>()))
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

/// Split UIDs into chunks whose sets stay short (servers limit command length).
pub fn uid_chunks(uids: &[u32], per: usize) -> Vec<Vec<u32>> {
    uids.chunks(per.max(1)).map(<[u32]>::to_vec).collect()
}

/// Parse an INTERNALDATE (`14-Oct-2026 09:30:00 +0200`) to Unix seconds.
pub fn parse_internal_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(' ')?;
    let mut d = date.trim().split('-');
    let day: u32 = d.next()?.trim().parse().ok()?;
    let mon = d.next()?;
    let year: i64 = d.next()?.parse().ok()?;
    let month =
        ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|m| m.eq_ignore_ascii_case(mon))? as u32
            + 1;
    let mut parts = rest.split_whitespace();
    let time = parts.next()?;
    let zone = parts.next().unwrap_or("+0000");
    let mut t = time.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let m: i64 = t.next()?.parse().ok()?;
    let sec: i64 = t.next().unwrap_or("0").parse().ok()?;
    let sign = if zone.starts_with('-') { -1 } else { 1 };
    let z = zone.trim_start_matches(['+', '-']);
    let zh: i64 = z.get(..2)?.parse().ok()?;
    let zm: i64 = z.get(2..4)?.parse().ok()?;
    if !(1..=31).contains(&day) || h > 23 || m > 59 || sec > 60 {
        return None;
    }
    let days = mailcraft_model::dates::days_from_civil(year, month, day);
    Some(days * 86_400 + h * 3600 + m * 60 + sec - sign * (zh * 3600 + zm * 60))
}

fn auth_error(e: NetError) -> NetError {
    match e {
        NetError::No(m) | NetError::Bad(m) => NetError::Auth(if m.is_empty() { "the sign-in was refused".into() } else { m }),
        other => other,
    }
}

fn info(o: &imap_proto::Outcome<'_>) -> String {
    o.information.as_deref().unwrap_or("").to_string()
}

/// The destination UIDs of a `COPYUID` code among a command's responses (at most 100,000).
fn copied_uids(rs: &[Response<'_>]) -> Vec<u32> {
    let mut out = Vec::new();
    for r in rs {
        let outcome = match r {
            Response::Data { status: Status::Ok, outcome } | Response::Done { status: Status::Ok, outcome, .. } => outcome,
            _ => continue,
        };
        if let Some(ResponseCode::CopyUid(_, _, to)) = &outcome.code {
            for m in to {
                match m {
                    imap_proto::rfc4315::UidSetMember::Uid(u) => out.push(*u),
                    imap_proto::rfc4315::UidSetMember::UidRange(r) => out.extend(r.clone().take(100_000usize.saturating_sub(out.len()))),
                }
                if out.len() >= 100_000 {
                    return out;
                }
            }
        }
    }
    out
}

/// One message from a `UID FETCH`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fetched {
    pub uid: u32,
    pub flags: Option<Vec<String>>,
    pub size: u32,
    pub internal_date: Option<i64>,
    pub header: Option<Vec<u8>>,
    pub body: Option<Vec<u8>>,
    pub modseq: u64,
}

/// What `SELECT` reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectInfo {
    pub exists: u32,
    pub validity: u32,
    pub uid_next: u32,
    pub highest_modseq: u64,
    pub read_only: bool,
}

/// What a QRESYNC `SELECT` reports: the usual data, the UID ranges expunged since the cached
/// state, and the messages whose flags changed.
pub type Resync = (SelectInfo, Vec<std::ops::RangeInclusive<u32>>, Vec<Fetched>);

/// Buffered protocol connection.
pub struct Conn<S> {
    s: S,
    buf: Vec<u8>,
    pos: usize,
    tag: u32,
}

impl<S: Read + Write> Conn<S> {
    pub fn new(s: S) -> Self {
        Conn { s, buf: Vec::with_capacity(16 << 10), pos: 0, tag: 0 }
    }

    pub fn into_inner(self) -> S {
        self.s
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.s
    }

    fn next_tag(&mut self) -> String {
        self.tag = self.tag.wrapping_add(1);
        format!("M{}", self.tag)
    }

    /// The length of the complete response at the front of the buffer, if there is one.
    fn complete_len(&self) -> Result<Option<usize>> {
        let data = self.buf.get(self.pos..).unwrap_or_default();
        let mut i = 0usize;
        loop {
            let rest = data.get(i..).unwrap_or_default();
            let Some(nl) = rest.iter().position(|&b| b == b'\n') else {
                if rest.len() > MAX_LINE {
                    return Err(NetError::Protocol("response line too long".into()));
                }
                return Ok(None);
            };
            if nl > MAX_LINE {
                return Err(NetError::Protocol("response line too long".into()));
            }
            let line = rest.get(..nl).unwrap_or_default();
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            i += nl + 1;
            match literal_len(line) {
                Some(n) if n > MAX_LITERAL => return Err(NetError::Protocol(format!("literal of {n} bytes is too large"))),
                Some(n) => {
                    if data.len() < i + n {
                        return Ok(None);
                    }
                    i += n;
                }
                None => return Ok(Some(i)),
            }
        }
    }

    fn fill(&mut self) -> Result<()> {
        if self.pos > 0 && (self.pos == self.buf.len() || self.pos > (1 << 20)) {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        let mut tmp = [0u8; 32 << 10];
        let n = self.s.read(&mut tmp)?;
        if n == 0 {
            return Err(NetError::Closed("connection closed".into()));
        }
        self.buf.extend_from_slice(tmp.get(..n).unwrap_or_default());
        Ok(())
    }

    /// The next complete response's bytes.
    pub fn read_raw(&mut self) -> Result<Vec<u8>> {
        loop {
            if let Some(n) = self.complete_len()? {
                let out = self.buf.get(self.pos..self.pos + n).map(<[u8]>::to_vec).unwrap_or_default();
                self.pos += n;
                return Ok(out);
            }
            self.fill()?;
        }
    }

    /// The next response, parsed. `Err(raw)` when `imap-proto` doesn't understand it.
    pub fn read(&mut self) -> Result<std::result::Result<Response<'static>, Vec<u8>>> {
        let raw = self.read_raw()?;
        // imap-proto wants CRLF line ends; tolerate servers that send bare LF.
        let fixed = if raw.ends_with(b"\r\n") { raw } else { normalize_eol(&raw) };
        Ok(match Response::parse(&fixed) {
            Ok((_, r)) => Ok(r.into_owned()),
            Err(_) => Err(fixed),
        })
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.s.write_all(bytes)?;
        self.s.flush()?;
        Ok(())
    }

    /// Run a command; returns the untagged responses followed by the tagged completion.
    pub fn run(&mut self, parts: &[Part<'_>]) -> Result<Vec<Response<'static>>> {
        let tag = self.next_tag();
        let mut collected = Vec::new();
        let mut line: Vec<u8> = format!("{tag} ").into_bytes();
        for p in parts {
            match p {
                Part::Text(t) => line.extend_from_slice(t.as_bytes()),
                Part::Literal(bytes) => {
                    line.extend_from_slice(format!("{{{}}}\r\n", bytes.len()).as_bytes());
                    self.send(&line)?;
                    line.clear();
                    self.wait_continue(&tag, &mut collected)?;
                    line.extend_from_slice(bytes);
                }
            }
        }
        line.extend_from_slice(b"\r\n");
        self.send(&line)?;
        self.finish(&tag, collected)
    }

    fn wait_continue(&mut self, tag: &str, collected: &mut Vec<Response<'static>>) -> Result<()> {
        loop {
            match self.read()? {
                Ok(Response::Continue(_)) => return Ok(()),
                Ok(Response::Done { tag: t, status, outcome }) if t.0 == tag => {
                    return Err(match status {
                        Status::Bad => NetError::Bad(info(&outcome)),
                        _ => NetError::No(info(&outcome)),
                    });
                }
                Ok(Response::Data { status: Status::Bye, outcome }) => return Err(NetError::Closed(info(&outcome))),
                Ok(other) => collected.push(other),
                Err(raw) => check_unknown(tag, &raw)?,
            }
        }
    }

    fn finish(&mut self, tag: &str, mut collected: Vec<Response<'static>>) -> Result<Vec<Response<'static>>> {
        loop {
            match self.read()? {
                Ok(Response::Done { tag: t, status, outcome }) if t.0 == tag => {
                    return match status {
                        Status::Ok => {
                            collected.push(Response::Done { tag: t, status, outcome });
                            Ok(collected)
                        }
                        Status::Bad => Err(NetError::Bad(info(&outcome))),
                        _ => Err(NetError::No(info(&outcome))),
                    };
                }
                Ok(Response::Data { status: Status::Bye, outcome }) => return Err(NetError::Closed(info(&outcome))),
                Ok(other) => collected.push(other),
                Err(raw) => check_unknown(tag, &raw)?,
            }
        }
    }
}

fn normalize_eol(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 8);
    let mut prev = 0u8;
    for &b in raw {
        if b == b'\n' && prev != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        prev = b;
    }
    out
}

/// A response `imap-proto` couldn't parse: harmless if untagged; for our tag, read the status
/// by hand so an odd completion line doesn't hang the command.
fn check_unknown(tag: &str, raw: &[u8]) -> Result<()> {
    let text = String::from_utf8_lossy(raw);
    let mut words = text.split_whitespace();
    if words.next() == Some(tag) {
        let status = words.next().unwrap_or("").to_ascii_uppercase();
        let rest: Vec<&str> = words.collect();
        return match status.as_str() {
            "OK" => Err(NetError::Protocol(format!("unparsed completion: {}", text.trim()))),
            "BAD" => Err(NetError::Bad(rest.join(" "))),
            _ => Err(NetError::No(rest.join(" "))),
        };
    }
    log::debug!("ignoring unparsed response: {}", text.trim());
    Ok(())
}

/// `{123}` or `{123+}` at the end of a line.
fn literal_len(line: &[u8]) -> Option<usize> {
    let body = line.strip_suffix(b"}")?;
    let open = body.iter().rposition(|&b| b == b'{')?;
    let digits = body.get(open + 1..)?;
    let digits = digits.strip_suffix(b"+").unwrap_or(digits);
    if digits.is_empty() || digits.len() > 12 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

/// An authenticated or not-yet-authenticated IMAP session.
pub struct Client<S> {
    conn: Conn<S>,
    caps: HashSet<String>,
    selected: Option<String>,
    /// QRESYNC is enabled on this connection (RFC 7162 §3.2.3).
    qresync: bool,
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
    /// Connect and read the greeting, upgrading with STARTTLS when configured.
    pub fn connect(cfg: &ServerConfig) -> Result<Client<Stream>> {
        let sock = stream::tcp(&cfg.host, cfg.port)?;
        match cfg.security {
            mailcraft_model::Security::Tls => Client::from_stream(stream::tls(sock, &cfg.host)?),
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
                if c.conn.pos != c.conn.buf.len() {
                    // Plain-text data after STARTTLS would be an injection attempt (RFC 3207 §4.2 analogue).
                    return Err(NetError::Tls("unexpected data before TLS negotiation".into()));
                }
                let Stream::Plain(sock) = c.conn.into_inner() else {
                    return Err(NetError::Tls("connection already encrypted".into()));
                };
                let mut c = Client { conn: Conn::new(stream::tls(sock, &cfg.host)?), caps: HashSet::new(), selected: None, qresync: false };
                // Capabilities learned before TLS must be discarded (RFC 3501 §6.2.1).
                c.capability()?;
                Ok(c)
            }
        }
    }
}

impl<S: Read + Write> Client<S> {
    /// Wrap a connected stream and read the server greeting.
    pub fn from_stream(s: S) -> Result<Client<S>> {
        let mut c = Client { conn: Conn::new(s), caps: HashSet::new(), selected: None, qresync: false };
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
        let rs = self.conn.run(&[Part::Text("CAPABILITY")])?;
        for r in rs {
            if let Response::Capabilities(list) = r {
                self.caps = caps_from(&list);
            }
        }
        Ok(())
    }

    /// Sign in: `AUTHENTICATE PLAIN` with SASL-IR when offered, else `LOGIN`.
    pub fn login(&mut self, user: &str, pass: &str) -> Result<()> {
        let as_auth = |e: NetError| match e {
            NetError::No(m) | NetError::Bad(m) => {
                NetError::Auth(if m.is_empty() { "the server rejected the user name or password".into() } else { m })
            }
            other => other,
        };
        let res = if self.has("AUTH=PLAIN") && self.has("SASL-IR") {
            let token = base64(format!("\0{user}\0{pass}").as_bytes());
            self.conn.run(&[Part::Text("AUTHENTICATE PLAIN "), Part::Text(&token)])
        } else if self.has("LOGINDISABLED") {
            return Err(NetError::Auth("the server does not allow sign-in on this connection (use TLS)".into()));
        } else {
            let (u, p) = (quoted(user), quoted(pass));
            let mut parts = vec![Part::Text("LOGIN ")];
            match &u {
                Some(q) => parts.push(Part::Text(q)),
                None => parts.push(Part::Literal(user.as_bytes())),
            }
            parts.push(Part::Text(" "));
            match &p {
                Some(q) => parts.push(Part::Text(q)),
                None => parts.push(Part::Literal(pass.as_bytes())),
            }
            self.conn.run(&parts)
        };
        let rs = res.map_err(as_auth)?;
        self.after_login(&rs)
    }

    /// Sign in with an OAuth access token (SASL XOAUTH2, as Gmail and Microsoft 365 take it).
    pub fn login_xoauth2(&mut self, user: &str, token: &str) -> Result<()> {
        let ir = crate::oauth::xoauth2(user, token);
        let tag = self.conn.next_tag();
        let mut collected = Vec::new();
        if self.has("SASL-IR") {
            self.conn.send(format!("{tag} AUTHENTICATE XOAUTH2 {ir}\r\n").as_bytes())?;
        } else {
            self.conn.send(format!("{tag} AUTHENTICATE XOAUTH2\r\n").as_bytes())?;
            self.conn.wait_continue(&tag, &mut collected).map_err(auth_error)?;
            self.conn.send(format!("{ir}\r\n").as_bytes())?;
        }
        // A refused token comes back as a challenge carrying the error: answer it empty, then
        // the server says NO.
        loop {
            match self.conn.read()? {
                Ok(Response::Continue(_)) => self.conn.send(b"\r\n")?,
                Ok(Response::Done { tag: t, status, outcome }) if t.0 == tag => {
                    if status != Status::Ok {
                        return Err(NetError::Auth(if info(&outcome).is_empty() { "the sign-in token was refused".into() } else { info(&outcome) }));
                    }
                    collected.push(Response::Done { tag: t, status, outcome });
                    break;
                }
                Ok(Response::Data { status: Status::Bye, outcome }) => return Err(NetError::Closed(info(&outcome))),
                Ok(other) => collected.push(other),
                Err(raw) => check_unknown(&tag, &raw).map_err(auth_error)?,
            }
        }
        self.after_login(&collected)
    }

    /// After signing in: the new capabilities, and the extensions sync uses.
    fn after_login(&mut self, rs: &[Response<'static>]) -> Result<()> {
        // Capabilities often change after login; take them from the completion or ask.
        let mut got = false;
        for r in rs {
            if let Response::Done { outcome, .. } = r
                && let Some(ResponseCode::Capabilities(list)) = &outcome.code
            {
                self.caps = caps_from(list);
                got = true;
            }
        }
        if !got {
            self.capability()?;
        }
        if self.has("ENABLE") && self.has("QRESYNC") {
            // QRESYNC implies CONDSTORE. Only an ENABLED reply naming it turns it on.
            if let Ok(rs) = self.conn.run(&[Part::Text("ENABLE QRESYNC")]) {
                self.qresync = rs.iter().any(|r| {
                    matches!(r, Response::Capabilities(list) if list.iter().any(|c| matches!(c, Capability::Atom(a) if a.eq_ignore_ascii_case("QRESYNC"))))
                });
            }
        } else if self.has("ENABLE") && self.has("CONDSTORE") {
            let _ = self.conn.run(&[Part::Text("ENABLE CONDSTORE")]);
        }
        Ok(())
    }

    /// Whether QRESYNC is on for this connection.
    pub fn qresync(&self) -> bool {
        self.qresync
    }

    /// All folders, with special-use roles.
    pub fn list(&mut self, account: &str) -> Result<Vec<Folder>> {
        let cmd = if self.has("SPECIAL-USE") && self.has("LIST-EXTENDED") { "LIST \"\" \"*\" RETURN (SPECIAL-USE)" } else { "LIST \"\" \"*\"" };
        let rs = self.conn.run(&[Part::Text(cmd)])?;
        let mut out: Vec<Folder> = Vec::new();
        for r in rs {
            let Response::MailboxData(MailboxDatum::List(l)) = r else { continue };
            let delimiter = l.delimiter.as_deref().and_then(|d| d.chars().next());
            let path = l.name.to_string();
            let mut role = None;
            let mut selectable = true;
            for a in &l.name_attributes {
                match a {
                    NameAttribute::NoSelect => selectable = false,
                    NameAttribute::Extension(e) if e.eq_ignore_ascii_case("\\NonExistent") => selectable = false,
                    NameAttribute::Sent => role = Some(FolderRole::Sent),
                    NameAttribute::Drafts => role = Some(FolderRole::Drafts),
                    NameAttribute::Trash => role = Some(FolderRole::Trash),
                    NameAttribute::Junk => role = Some(FolderRole::Junk),
                    NameAttribute::Archive => role = Some(FolderRole::Archive),
                    NameAttribute::All => role = role.or(Some(FolderRole::Archive)),
                    NameAttribute::Extension(e) => role = role.or(FolderRole::from_attribute(e)),
                    _ => {}
                }
            }
            let role =
                if path.eq_ignore_ascii_case("INBOX") { FolderRole::Inbox } else { role.unwrap_or_else(|| FolderRole::from_name(&path, delimiter)) };
            if out.iter().any(|f| f.path == path) {
                continue;
            }
            out.push(Folder { account: account.to_string(), path, delimiter, role, selectable, unread: 0, total: 0 });
        }
        // One folder per special role: the first wins, later ones become ordinary folders.
        let mut seen = HashSet::new();
        for f in &mut out {
            if f.role != FolderRole::Other && !seen.insert(f.role) {
                f.role = FolderRole::Other;
            }
        }
        Ok(out)
    }

    /// `SELECT` (or `EXAMINE` when `read_only`), asking for CONDSTORE data when supported.
    pub fn select(&mut self, folder: &str, read_only: bool) -> Result<SelectInfo> {
        let verb = if read_only { "EXAMINE " } else { "SELECT " };
        let arg = mailbox_arg(folder);
        let condstore = if self.has("CONDSTORE") || self.qresync { " (CONDSTORE)" } else { "" };
        let cmd = format!("{verb}{arg}{condstore}");
        let rs = self.conn.run(&[Part::Text(&cmd)])?;
        let (info, _, _) = self.selected_from(folder, rs);
        Ok(info)
    }

    /// `SELECT … (QRESYNC (validity modseq known-uids))` (RFC 7162 §3.2.5): besides the usual
    /// data, the server reports what was expunged (`VANISHED (EARLIER)`, as UID ranges) and the
    /// flags that changed since `modseq`, in one round trip. Only for a cache whose UIDVALIDITY
    /// is `validity`: when the server's differs it sends neither, and the caller starts over.
    pub fn select_qresync(&mut self, folder: &str, validity: u32, modseq: u64, known: &str) -> Result<Resync> {
        let known = if known.is_empty() { String::new() } else { format!(" {known}") };
        let cmd = format!("SELECT {} (QRESYNC ({validity} {modseq}{known}))", mailbox_arg(folder));
        let rs = self.conn.run(&[Part::Text(&cmd)])?;
        Ok(self.selected_from(folder, rs))
    }

    fn selected_from(&mut self, folder: &str, rs: Vec<Response<'static>>) -> Resync {
        let mut info = SelectInfo::default();
        let mut vanished = Vec::new();
        let mut changed = Vec::new();
        for r in rs {
            match r {
                Response::Vanished { uids, .. } => vanished.extend(uids),
                Response::Fetch(_, attrs) => {
                    let f = fetched(attrs);
                    if f.uid != 0 {
                        changed.push(f);
                    }
                }
                Response::MailboxData(MailboxDatum::Exists(n)) => info.exists = n,
                Response::Data { outcome, .. } | Response::Done { outcome, .. } => match outcome.code {
                    Some(ResponseCode::UidValidity(v)) => info.validity = v,
                    Some(ResponseCode::UidNext(v)) => info.uid_next = v,
                    Some(ResponseCode::HighestModSeq(v)) => info.highest_modseq = v,
                    Some(ResponseCode::ReadOnly) => info.read_only = true,
                    _ => {}
                },
                _ => {}
            }
        }
        self.selected = Some(folder.to_string());
        (info, vanished, changed)
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// `STATUS` counts for a folder that isn't selected: (messages, unseen).
    pub fn status(&mut self, folder: &str) -> Result<(u32, u32)> {
        let cmd = format!("STATUS {} (MESSAGES UNSEEN)", mailbox_arg(folder));
        let rs = self.conn.run(&[Part::Text(&cmd)])?;
        let (mut total, mut unseen) = (0, 0);
        for r in rs {
            if let Response::MailboxData(MailboxDatum::Status { status, .. }) = r {
                for s in status {
                    match s {
                        StatusAttribute::Messages(n) => total = n,
                        StatusAttribute::Unseen(n) => unseen = n,
                        _ => {}
                    }
                }
            }
        }
        Ok((total, unseen))
    }

    pub fn uid_search(&mut self, criteria: &str) -> Result<Vec<u32>> {
        let cmd = format!("UID SEARCH {criteria}");
        let rs = self.conn.run(&[Part::Text(&cmd)])?;
        let mut out = Vec::new();
        for r in rs {
            if let Response::MailboxData(MailboxDatum::Search(v)) = r {
                out.extend(v);
            }
        }
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// `UID FETCH set (items)` with an optional modifier (`(CHANGEDSINCE n)`).
    pub fn uid_fetch(&mut self, set: &str, items: &str, modifier: &str) -> Result<Vec<Fetched>> {
        if set.is_empty() {
            return Ok(Vec::new());
        }
        let cmd = if modifier.is_empty() { format!("UID FETCH {set} ({items})") } else { format!("UID FETCH {set} ({items}) {modifier}") };
        let rs = self.conn.run(&[Part::Text(&cmd)])?;
        let mut out: Vec<Fetched> = Vec::new();
        for r in rs {
            let Response::Fetch(_, attrs) = r else { continue };
            let f = fetched(attrs);
            // Unsolicited FETCHes without a UID (flag pushes for other messages) are skipped.
            if f.uid == 0 {
                continue;
            }
            if let Some(existing) = out.iter_mut().find(|e| e.uid == f.uid) {
                existing.flags = f.flags.or(existing.flags.take());
                existing.header = f.header.or(existing.header.take());
                existing.body = f.body.or(existing.body.take());
                existing.size = existing.size.max(f.size);
                existing.modseq = existing.modseq.max(f.modseq);
                existing.internal_date = existing.internal_date.or(f.internal_date);
            } else {
                out.push(f);
            }
        }
        Ok(out)
    }

    /// Add (`add`) or remove flags, silently.
    pub fn uid_store(&mut self, set: &str, add: bool, flags: &str) -> Result<()> {
        if set.is_empty() {
            return Ok(());
        }
        let cmd = format!("UID STORE {set} {}FLAGS.SILENT ({flags})", if add { "+" } else { "-" });
        self.conn.run(&[Part::Text(&cmd)]).map(|_| ())
    }

    /// Move messages: `UID MOVE` (RFC 6851) or COPY + `\Deleted` + expunge.
    pub fn uid_move(&mut self, set: &str, dest: &str) -> Result<()> {
        self.uid_move_to(set, dest).map(drop)
    }

    /// Move messages and say their UIDs in `dest`, when the server does (UIDPLUS `COPYUID`, sent
    /// with MOVE's untagged OK or COPY's tagged one); empty when it doesn't.
    pub fn uid_move_to(&mut self, set: &str, dest: &str) -> Result<Vec<u32>> {
        if set.is_empty() {
            return Ok(Vec::new());
        }
        let dest = mailbox_arg(dest);
        if self.has("MOVE") {
            let cmd = format!("UID MOVE {set} {dest}");
            return self.conn.run(&[Part::Text(&cmd)]).map(|rs| copied_uids(&rs));
        }
        let cmd = format!("UID COPY {set} {dest}");
        let uids = copied_uids(&self.conn.run(&[Part::Text(&cmd)])?);
        self.uid_store(set, true, "\\Deleted")?;
        self.expunge(set)?;
        Ok(uids)
    }

    /// Expunge exactly `set` (UIDPLUS), or everything marked `\Deleted` without it.
    pub fn expunge(&mut self, set: &str) -> Result<()> {
        let cmd = if self.has("UIDPLUS") { format!("UID EXPUNGE {set}") } else { "EXPUNGE".to_string() };
        self.conn.run(&[Part::Text(&cmd)]).map(|_| ())
    }

    /// Space used on the server and its limit, in bytes (`GETQUOTAROOT`, RFC 9208); `None` when the
    /// server has no QUOTA extension or no storage limit for the Inbox.
    pub fn quota(&mut self) -> Result<Option<(u64, u64)>> {
        if !self.has("QUOTA") && !self.caps.iter().any(|c| c.starts_with("QUOTA=")) {
            return Ok(None);
        }
        let rs = self.conn.run(&[Part::Text("GETQUOTAROOT INBOX")])?;
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

    /// Delete every message in a folder for good.
    pub fn empty(&mut self, folder: &str) -> Result<()> {
        let info = self.select(folder, false)?;
        if info.exists == 0 {
            return Ok(());
        }
        self.uid_store("1:*", true, "\\Deleted")?;
        self.conn.run(&[Part::Text("EXPUNGE")]).map(|_| ())
    }

    /// Store a message in a folder; returns its UID when the server says (UIDPLUS `APPENDUID`).
    pub fn append(&mut self, folder: &str, flags: &str, raw: &[u8]) -> Result<Option<u32>> {
        let head = format!("APPEND {} ({flags}) ", mailbox_arg(folder));
        let rs = self.conn.run(&[Part::Text(&head), Part::Literal(raw)])?;
        for r in rs {
            if let Response::Done { outcome, .. } = r
                && let Some(ResponseCode::AppendUid(_, uids)) = outcome.code
            {
                return Ok(uids.first().map(|m| match m {
                    imap_proto::rfc4315::UidSetMember::Uid(u) => *u,
                    imap_proto::rfc4315::UidSetMember::UidRange(r) => *r.start(),
                }));
            }
        }
        Ok(None)
    }

    /// Create a folder and subscribe to it (some clients only show subscribed folders). A folder
    /// that already exists counts as created.
    pub fn create(&mut self, folder: &str) -> Result<()> {
        let cmd = format!("CREATE {}", mailbox_arg(folder));
        match self.conn.run(&[Part::Text(&cmd)]) {
            Ok(_) => {}
            Err(NetError::No(m)) if m.to_ascii_uppercase().contains("ALREADYEXISTS") || m.to_ascii_lowercase().contains("exists") => {}
            Err(e) => return Err(e),
        }
        let _ = self.conn.run(&[Part::Text(&format!("SUBSCRIBE {}", mailbox_arg(folder)))]);
        Ok(())
    }

    /// Rename a folder (folders inside it move with it, RFC 3501 §6.3.5).
    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        if self.selected.as_deref().is_some_and(|s| s.starts_with(from)) {
            // Renaming the selected folder is allowed, but leaves the session in an odd state on
            // some servers: deselect first.
            self.unselect();
        }
        let cmd = format!("RENAME {} {}", mailbox_arg(from), mailbox_arg(to));
        self.conn.run(&[Part::Text(&cmd)])?;
        let _ = self.conn.run(&[Part::Text(&format!("UNSUBSCRIBE {}", mailbox_arg(from)))]);
        let _ = self.conn.run(&[Part::Text(&format!("SUBSCRIBE {}", mailbox_arg(to)))]);
        Ok(())
    }

    /// Delete a folder. One that is already gone counts as deleted.
    pub fn delete(&mut self, folder: &str) -> Result<()> {
        if self.selected.as_deref().is_some_and(|s| s.starts_with(folder)) {
            self.unselect();
        }
        let _ = self.conn.run(&[Part::Text(&format!("UNSUBSCRIBE {}", mailbox_arg(folder)))]);
        match self.conn.run(&[Part::Text(&format!("DELETE {}", mailbox_arg(folder)))]) {
            Ok(_) => Ok(()),
            Err(NetError::No(m)) if m.to_ascii_uppercase().contains("NONEXISTENT") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Leave the selected folder (`UNSELECT` when offered; `CLOSE` would expunge).
    fn unselect(&mut self) {
        if self.has("UNSELECT") {
            let _ = self.conn.run(&[Part::Text("UNSELECT")]);
        } else if self.selected.is_some() {
            // A failed EXAMINE leaves the selected state without expunging (RFC 3501 §6.3.1).
            let _ = self.conn.run(&[Part::Text("EXAMINE \"MailCraft no such folder\"")]);
        }
        self.selected = None;
    }

    pub fn noop(&mut self) -> Result<Vec<Response<'static>>> {
        self.conn.run(&[Part::Text("NOOP")])
    }

    pub fn logout(&mut self) {
        let _ = self.conn.run(&[Part::Text("LOGOUT")]);
    }
}

impl<S: Read + Write + Timeouts> Client<S> {
    /// IDLE (RFC 2177) on the selected folder for up to `max`, checking `stop` about twice a
    /// second. Returns whether the server reported a change (new, expunged or changed mail).
    pub fn idle(&mut self, max: Duration, mut stop: impl FnMut() -> bool) -> Result<bool> {
        let tag = self.conn.next_tag();
        self.conn.send(format!("{tag} IDLE\r\n").as_bytes())?;
        let mut ignored = Vec::new();
        self.conn.wait_continue(&tag, &mut ignored)?;
        let mut changed = ignored.iter().any(is_change);
        let start = Instant::now();
        self.conn.get_mut().set_read_timeout(Some(Duration::from_millis(500)))?;
        let mut err = None;
        while !changed && start.elapsed() < max && !stop() {
            match self.conn.read() {
                Ok(Ok(r)) => {
                    if let Response::Data { status: Status::Bye, outcome } = &r {
                        err = Some(NetError::Closed(info(outcome)));
                        break;
                    }
                    changed |= is_change(&r);
                }
                Ok(Err(_)) => {}
                Err(NetError::Io(e)) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }
        self.conn.get_mut().set_read_timeout(Some(IO_TIMEOUT))?;
        if let Some(e) = err {
            return Err(e);
        }
        self.conn.send(b"DONE\r\n")?;
        let rs = self.conn.finish(&tag, Vec::new())?;
        Ok(changed || rs.iter().any(is_change))
    }
}

/// One FETCH response's attributes.
fn fetched(attrs: Vec<AttributeValue<'_>>) -> Fetched {
    let mut f = Fetched::default();
    for a in attrs {
        match a {
            AttributeValue::Uid(u) => f.uid = u,
            AttributeValue::Flags(fl) => f.flags = Some(fl.iter().map(|x| x.to_string()).collect()),
            AttributeValue::Rfc822Size(n) => f.size = n,
            AttributeValue::InternalDate(d) => f.internal_date = parse_internal_date(&d),
            AttributeValue::ModSeq(m) => f.modseq = m,
            AttributeValue::BodySection { section: None, data, .. } | AttributeValue::Rfc822(data) => f.body = data.map(|d| d.into_owned()),
            AttributeValue::BodySection { section: Some(_), data, .. } | AttributeValue::Rfc822Header(data) => {
                f.header = data.map(|d| d.into_owned())
            }
            _ => {}
        }
    }
    f
}

fn is_change(r: &Response<'_>) -> bool {
    matches!(
        r,
        Response::MailboxData(MailboxDatum::Exists(_) | MailboxDatum::Recent(_))
            | Response::Expunge(_)
            | Response::Vanished { .. }
            | Response::Fetch(..)
    )
}

/// Standard base64 (for SASL).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(T.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canned server: reads are served from `input`; writes are recorded.
    pub(crate) struct Script {
        input: io::Cursor<Vec<u8>>,
        pub written: Vec<u8>,
    }

    impl Script {
        fn new(s: &[u8]) -> Self {
            Script { input: io::Cursor::new(s.to_vec()), written: Vec::new() }
        }
    }

    impl Read for Script {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            // Deliver a few bytes at a time to exercise framing across reads.
            let n = buf.len().min(7);
            self.input.read(&mut buf[..n])
        }
    }

    impl Write for Script {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn sets_and_dates() {
        assert_eq!(uid_set(&[5, 1, 2, 3, 9, 10, 3, 0]), "1:3,5,9:10");
        assert_eq!(uid_set(&[]), "");
        assert_eq!(uid_set(&[u32::MAX, u32::MAX - 1]), format!("{}:{}", u32::MAX - 1, u32::MAX));
        assert_eq!(parse_internal_date("14-Oct-2026 09:30:00 +0200"), Some(1_791_963_000));
        assert_eq!(parse_internal_date(" 4-Jan-1970 00:00:00 -0100"), Some(3 * 86_400 + 3600));
        assert_eq!(parse_internal_date("junk"), None);
        assert_eq!(parse_internal_date("99-Oct-2026 09:30:00 +0200"), None);
        assert_eq!(base64(b"\0user\0pass"), "AHVzZXIAcGFzcw==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(quoted("a\"b\\c").as_deref(), Some("\"a\\\"b\\\\c\""));
        assert_eq!(quoted("pässword"), None);
    }

    #[test]
    fn literals() {
        assert_eq!(literal_len(b"* 1 FETCH (BODY[] {12}"), Some(12));
        assert_eq!(literal_len(b"A1 APPEND x {3+}"), Some(3));
        assert_eq!(literal_len(b"* OK {abc}"), None);
        assert_eq!(literal_len(b"* OK done"), None);
        assert_eq!(literal_len(b"{99999999999999999}"), None);
    }

    #[test]
    fn greeting_login_fetch_with_literals() {
        let server = b"* OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS] ready\r\n\
M1 OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS CONDSTORE] logged in\r\n\
* 1 FETCH (UID 7 FLAGS (\\Seen) RFC822.SIZE 12 BODY[HEADER] {12}\r\nSubject: x\r\n)\r\n\
* 2 FETCH (FLAGS (\\Flagged))\r\n\
M2 OK done\r\n";
        let mut c = Client::from_stream(Script::new(server)).unwrap();
        assert!(c.has("move") && !c.has("CONDSTORE"));
        c.login("zoe@x", "pässword").unwrap_err(); // literal path needs a '+' the script lacks
    }

    #[test]
    fn login_quoted_then_fetch() {
        let server = b"* OK hi\r\n\
M1 OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS] logged in\r\n\
* 1 FETCH (UID 7 FLAGS (\\Seen) RFC822.SIZE 12 BODY[HEADER] {12}\r\nSubject: x\r\n)\r\n\
* 2 FETCH (FLAGS (\\Flagged))\r\n\
M2 OK done\r\n";
        let mut c = Client::from_stream(Script::new(server)).unwrap();
        c.login("zoe@x", "pw").unwrap();
        assert!(c.has("UIDPLUS"));
        let f = c.uid_fetch("7", "UID FLAGS RFC822.SIZE BODY.PEEK[HEADER]", "").unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].uid, 7);
        assert_eq!(f[0].header.as_deref(), Some(&b"Subject: x\r\n"[..]));
        assert_eq!(f[0].flags.as_deref(), Some(&["\\Seen".to_string()][..]));
        let written = String::from_utf8(c.conn.into_inner().written).unwrap();
        assert!(written.starts_with("M1 LOGIN \"zoe@x\" \"pw\"\r\nM2 UID FETCH 7 ("), "{written}");
    }

    #[test]
    fn errors_and_hostile_input() {
        let mut c = Client::from_stream(Script::new(b"* OK hi\r\nM1 NO [AUTHENTICATIONFAILED] bad password\r\n")).unwrap();
        assert!(matches!(c.login("u", "p"), Err(NetError::Auth(m)) if m.contains("bad password")));
        assert!(matches!(Client::from_stream(Script::new(b"* BYE go away\r\n")), Err(NetError::Closed(_))));
        assert!(Client::from_stream(Script::new(b"")).is_err());
        assert!(Client::from_stream(Script::new(b"garbage\r\n")).is_err());
        let mut c = Client::from_stream(Script::new(b"* OK hi\r\n* 1 FETCH (BODY[] {999999999999}\r\n")).unwrap();
        assert!(matches!(c.uid_fetch("1", "BODY.PEEK[]", ""), Err(NetError::Protocol(_))));
        let mut c = Client::from_stream(Script::new(b"* OK hi\r\n* WEIRD stuff\r\nM1 OK fine\r\n")).unwrap();
        assert!(c.noop().is_ok());
        // Bare LF line ends are tolerated.
        let mut c = Client::from_stream(Script::new(b"* OK hi\n* SEARCH 3 1 2\nM1 OK done\n")).unwrap();
        assert_eq!(c.uid_search("ALL").unwrap(), vec![1, 2, 3]);
    }
}
