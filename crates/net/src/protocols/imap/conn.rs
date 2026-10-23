//! Response framing and the command loop: a response is one line plus any `{n}` literals it
//! announces, taken off the buffer only once complete, so a read that times out (IDLE waits in
//! short slices) never loses half a response. Server input is hostile: a line, a literal and the
//! whole of one command's responses are capped, and a command has an overall deadline.

use super::*;

/// Longest line accepted outside literals.
const MAX_LINE: usize = 1 << 20;

/// What one command may cost: the bytes and number of its responses, and how long it may run.
/// Past any of them the command fails with a protocol error, which drops the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Total bytes of one command's responses. Our largest legitimate command is a prefetch
    /// chunk of message sources, which `sync` keeps well under this; a single message is at
    /// most [`MAX_LITERAL`].
    pub bytes: usize,
    /// Number of responses to one command (a big folder's `UID FETCH … FLAGS` gets one per
    /// message; chunks keep that in the thousands).
    pub responses: usize,
    /// Wall time for one command, so a server trickling a byte just inside the read timeout
    /// can't hold a command forever.
    pub time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { bytes: 4 * MAX_LITERAL, responses: 1_000_000, time: Duration::from_secs(15 * 60) }
    }
}

/// The error for a command whose responses ran past its byte or count budget.
fn over_budget() -> NetError {
    NetError::Protocol("one command's responses went over MailCraft's size budget".into())
}

/// Buffered protocol connection.
pub struct Conn<S> {
    s: S,
    pub(super) buf: Vec<u8>,
    pub(super) pos: usize,
    tag: u32,
    limits: Limits,
    /// Bytes and responses read since the current command began.
    used_bytes: usize,
    used_count: usize,
    /// When the current command must be done by (`None`: none running, or IDLE waiting).
    deadline: Option<Instant>,
}

impl<S: Read + Write> Conn<S> {
    pub fn new(s: S) -> Self {
        Conn { s, buf: Vec::with_capacity(16 << 10), pos: 0, tag: 0, limits: Limits::default(), used_bytes: 0, used_count: 0, deadline: None }
    }

    /// Replace the per-command budget (the hostile-server tests shrink it).
    pub fn set_limits(&mut self, budget: Limits) {
        self.limits = budget;
    }

    /// Give back the underlying stream (STARTTLS wraps it; tests read what was written).
    pub fn into_inner(self) -> S {
        self.s
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.s
    }

    pub(super) fn next_tag(&mut self) -> String {
        self.tag = self.tag.wrapping_add(1);
        format!("M{}", self.tag)
    }

    /// Start counting a new command against the limits.
    pub(super) fn begin(&mut self) {
        self.used_bytes = 0;
        self.used_count = 0;
        self.deadline = Instant::now().checked_add(self.limits.time);
    }

    /// Lift the deadline while IDLE waits (its own `max` bounds it); the counts restart.
    pub(super) fn waiting(&mut self) {
        self.used_bytes = 0;
        self.used_count = 0;
        self.deadline = None;
    }

    /// Bytes the current command may still receive.
    fn room(&self) -> usize {
        self.limits.bytes.saturating_sub(self.used_bytes)
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
                // Refuse before buffering it: the literal would take the command past its byte cap.
                Some(n) if i.saturating_add(n) > self.room() => return Err(over_budget()),
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
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(NetError::Protocol("the server took too long to answer one command".into()));
        }
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

    /// The next complete response's bytes, counted against the current command's limits.
    pub fn read_raw(&mut self) -> Result<Vec<u8>> {
        loop {
            if let Some(n) = self.complete_len()? {
                self.used_bytes = self.used_bytes.saturating_add(n);
                self.used_count = self.used_count.saturating_add(1);
                if self.used_bytes > self.limits.bytes || self.used_count > self.limits.responses {
                    return Err(over_budget());
                }
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
        let fixed = if raw.ends_with(b"\r\n") { raw } else { crlf_line_ends(&raw) };
        let fixed = space_after_code(fixed);
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
        self.begin();
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

    pub(super) fn wait_continue(&mut self, tag: &str, collected: &mut Vec<Response<'static>>) -> Result<()> {
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

    pub(super) fn finish(&mut self, tag: &str, mut collected: Vec<Response<'static>>) -> Result<Vec<Response<'static>>> {
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

/// A space between a status response's `[code]` and a non-ASCII character after it. imap-proto
/// 0.17 drops the byte after the `]` (meant to be the space) by slicing the text at 1, which
/// panics inside a multi-byte character: `* OK [ALERT]é` from a server would crash the client.
pub(super) fn space_after_code(mut raw: Vec<u8>) -> Vec<u8> {
    let line = raw.get(..raw.iter().position(|&b| b == b'\n').unwrap_or(raw.len())).unwrap_or_default();
    // A continuation's space is optional to imap-proto (some servers leave it out).
    let text = if let Some(rest) = line.strip_prefix(b"+") {
        rest.strip_prefix(b" ").unwrap_or(rest)
    } else {
        // `tag status text`: skip the tag, then the status word; the text is what follows.
        let after_tag = line.iter().position(|&b| b == b' ').and_then(|sp| line.get(sp + 1..)).unwrap_or_default();
        let (status, text) = match after_tag.iter().position(|&b| b == b' ') {
            Some(sp) => (after_tag.get(..sp).unwrap_or_default(), after_tag.get(sp + 1..).unwrap_or_default()),
            None => (after_tag, &[][..]),
        };
        let status = String::from_utf8_lossy(status).to_ascii_uppercase();
        if !["OK", "NO", "BAD", "PREAUTH", "BYE"].contains(&status.as_str()) {
            return raw;
        }
        text
    };
    if text.first() != Some(&b'[') {
        return raw;
    }
    let Some(close) = text.iter().position(|&b| b == b']') else { return raw };
    if text.get(close + 1).is_some_and(|b| !b.is_ascii()) {
        let at = line.len() - text.len() + close + 1;
        raw.insert(at, b' ');
    }
    raw
}

/// `raw` with each line feed that lacks its carriage return given one (so `\n` becomes `\r\n`).
fn crlf_line_ends(raw: &[u8]) -> Vec<u8> {
    let mut fixed = Vec::with_capacity(raw.len().saturating_add(raw.len() / 16));
    for (i, piece) in raw.split(|&b| b == b'\n').enumerate() {
        if i > 0 {
            if fixed.last() != Some(&b'\r') {
                fixed.push(b'\r');
            }
            fixed.push(b'\n');
        }
        fixed.extend_from_slice(piece);
    }
    fixed
}

/// A response `imap-proto` couldn't parse: harmless if untagged; for our tag, read the status
/// by hand so an odd completion line doesn't hang the command.
pub(super) fn check_unknown(tag: &str, raw: &[u8]) -> Result<()> {
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

/// The size a line announces for the literal after it: `{123}`, or the non-synchronising
/// `{123+}` (RFC 7888), as the line's last bytes. At most twelve digits, so it can't overflow.
pub(super) fn literal_len(line: &[u8]) -> Option<usize> {
    let inside = match line.split_last() {
        Some((&b'}', head)) => head,
        _ => return None,
    };
    let inside = match inside.split_last() {
        Some((&b'+', head)) => head,
        _ => inside,
    };
    let count = inside.iter().rev().take_while(|b| b.is_ascii_digit()).count();
    let brace = inside.len().checked_sub(count)?.checked_sub(1)?;
    if count == 0 || count > 12 || inside.get(brace) != Some(&b'{') {
        return None;
    }
    inside.get(brace + 1..)?.iter().try_fold(0usize, |n, &d| n.checked_mul(10)?.checked_add(usize::from(d - b'0')))
}
