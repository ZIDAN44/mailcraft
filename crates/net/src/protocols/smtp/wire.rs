//! SMTP on the wire (RFC 5321): a server's replies read, one line or several, commands written,
//! and the message itself sent with its lines escaped. Replies are hostile input: a line, the
//! lines of a reply and the extensions an `EHLO` names are capped, and words that aren't UTF-8
//! are read with what isn't replaced (a server set up in Latin-1, as one in German may greet:
//! "Grüß Gott"; discussion #309).

use std::io::{BufRead, BufReader, Read, Write};
use std::time::Duration;

use crate::stream::Timeouts;
use crate::{NetError, Result};

/// The longest reply line read: RFC 5321 §4.5.3.1.5 allows 512 octets, and servers write longer.
const MAX_LINE: usize = 16 << 10;
/// The most lines one reply may have: with [`MAX_LINE`], a server holds at most 16 MB of ours.
const MAX_LINES: usize = 1000;
/// The most sign-in methods an `EHLO` may name that are kept.
const MAX_SIGN_INS: usize = 32;

/// A server's reply: its code, and each line's words after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reply {
    pub(crate) code: u16,
    pub(crate) lines: Vec<String>,
}

impl Reply {
    /// The reply as the server put it: its code and words, its lines joined by spaces.
    pub(crate) fn said(&self) -> String {
        let words: Vec<&str> = self.lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
        if words.is_empty() { self.code.to_string() } else { format!("{} {}", self.code, words.join(" ")) }
    }

    /// The first word of its first line: a `334`'s challenge.
    pub(crate) fn first_word(&self) -> Option<&str> {
        self.lines.first()?.split_whitespace().next()
    }

    /// Whether it's a success (2xx).
    pub(crate) fn done(&self) -> bool {
        (200..300).contains(&self.code)
    }
}

/// One reply line: its code, whether more lines follow (`250-`), and its words.
pub(crate) fn parse_line(line: &str) -> Option<(u16, bool, &str)> {
    let digits = line.get(..3)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) || !digits.as_bytes().first().is_some_and(|d| (b'2'..=b'5').contains(d)) {
        return None;
    }
    let code: u16 = digits.parse().ok()?;
    match line.as_bytes().get(3) {
        None => Some((code, false, "")),
        Some(b' ') => Some((code, false, line.get(4..)?)),
        Some(b'-') => Some((code, true, line.get(4..)?)),
        Some(_) => None,
    }
}

/// What an `EHLO` named: each extension's keyword in capitals, with its parameters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Offers {
    extensions: Vec<(String, String)>,
}

impl Offers {
    /// From the `EHLO` reply: its first line greets, each after it names an extension.
    pub(crate) fn of(r: &Reply) -> Offers {
        let extensions = r
            .lines
            .iter()
            .skip(1)
            .filter_map(|l| {
                let l = l.trim();
                // `AUTH=PLAIN LOGIN`, as some older servers write it, is `AUTH` too.
                let (keyword, params) = match l.split_once([' ', '=']) {
                    Some((k, p)) => (k, p),
                    None => (l, ""),
                };
                (!keyword.is_empty()).then(|| (keyword.to_ascii_uppercase(), params.trim().to_string()))
            })
            .collect();
        Offers { extensions }
    }

    /// Whether the server named `keyword` (`STARTTLS`, `8BITMIME`, `SMTPUTF8`).
    pub(crate) fn has(&self, keyword: &str) -> bool {
        self.extensions.iter().any(|(k, _)| k.eq_ignore_ascii_case(keyword))
    }

    /// The sign-in methods it named, in capitals, each once, in its order.
    pub(crate) fn sign_ins(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (_, params) in self.extensions.iter().filter(|(k, _)| k == "AUTH") {
            for m in params.split_whitespace() {
                let m = m.to_ascii_uppercase();
                if out.len() < MAX_SIGN_INS && !out.contains(&m) {
                    out.push(m);
                }
            }
        }
        out
    }
}

/// A connection to an SMTP server, over a stream plain or secured.
pub(crate) struct Conn<S: Read + Write> {
    r: BufReader<S>,
}

impl<S: Read + Write + Timeouts> Conn<S> {
    pub(crate) fn new(stream: S) -> Conn<S> {
        Conn { r: BufReader::new(stream) }
    }

    /// The next reply.
    pub(crate) fn reply(&mut self) -> Result<Reply> {
        read_reply(&mut self.r)
    }

    /// `line` sent (its line end added), and the server's reply to it. A line with a line end
    /// of its own would be two commands: never sent.
    pub(crate) fn command(&mut self, line: &str) -> Result<Reply> {
        if line.contains(['\r', '\n']) {
            return Err(NetError::Protocol("a command for the outgoing server held a line break".into()));
        }
        let w = self.r.get_mut();
        w.write_all(line.as_bytes())?;
        w.write_all(b"\r\n")?;
        w.flush()?;
        self.reply()
    }

    /// The message sent as `DATA`'s text (RFC 5321 §4.5.2), ended by a line of a lone dot, and
    /// the server's reply to it.
    pub(crate) fn message(&mut self, raw: &[u8]) -> Result<Reply> {
        let w = self.r.get_mut();
        w.write_all(&data_lines(raw))?;
        w.flush()?;
        self.reply()
    }

    /// How long a reply may take from here.
    pub(crate) fn set_timeout(&mut self, t: Duration) -> Result<()> {
        Ok(self.r.get_mut().set_read_timeout(Some(t))?)
    }

    /// The stream, for STARTTLS to secure, once the server has agreed: anything it sent past
    /// its go-ahead would be plain text taken as if it came secured (RFC 3207 §4.2), so the
    /// connection is dropped then.
    pub(crate) fn into_stream(self) -> Result<S> {
        if !self.r.buffer().is_empty() {
            return Err(NetError::Tls("unexpected data before TLS negotiation".into()));
        }
        Ok(self.r.into_inner())
    }

    /// `QUIT`, its reply not waited on beyond the usual (the message is already settled).
    pub(crate) fn quit(&mut self) {
        let _ = self.command("QUIT");
    }
}

/// One reply read from `r`: a line, or several (`250-…` until `250 …`), all with one code.
pub(crate) fn read_reply(r: &mut impl BufRead) -> Result<Reply> {
    let mut lines = Vec::new();
    let mut code = None;
    loop {
        let line = read_line(r)?;
        let Some((c, more, words)) = parse_line(&line) else {
            return Err(NetError::Protocol(format!("the outgoing server's answer isn't SMTP: {}", shortened(&line))));
        };
        if code.is_some_and(|first| first != c) {
            return Err(NetError::Protocol(format!("the outgoing server's answer changed its code mid-way: {}", shortened(&line))));
        }
        code = Some(c);
        if lines.len() >= MAX_LINES {
            return Err(NetError::Protocol("the outgoing server's answer is too long".into()));
        }
        lines.push(words.to_string());
        if !more {
            break;
        }
    }
    let code = code.unwrap_or(0);
    Ok(Reply { code, lines })
}

/// One line, its line end taken off, read as UTF-8 with what isn't replaced.
fn read_line(r: &mut impl BufRead) -> Result<String> {
    let mut buf = Vec::new();
    let limit = u64::try_from(MAX_LINE).unwrap_or(u64::MAX).saturating_add(1);
    r.take(limit).read_until(b'\n', &mut buf)?;
    if buf.is_empty() {
        return Err(NetError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the outgoing server ended the connection")));
    }
    if buf.last() != Some(&b'\n') {
        if buf.len() > MAX_LINE {
            return Err(NetError::Protocol("the outgoing server's answer has a line too long".into()));
        }
        return Err(NetError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the outgoing server ended the connection mid-answer")));
    }
    buf.pop();
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// The start of a line, for an error to quote.
fn shortened(line: &str) -> String {
    line.chars().take(120).collect()
}

/// `raw` as `DATA` sends it: each line ended CR LF (a lone CR or LF is a line end too: SMTP
/// takes no other, and servers turn away mail with bare ones), a line starting with a dot given
/// another (RFC 5321 §4.5.2, so it isn't taken for the end), and the end's line of a lone dot.
pub(crate) fn data_lines(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len().saturating_add(raw.len() / 32).saturating_add(5));
    let mut line_start = true;
    let mut i = 0;
    while let Some(&b) = raw.get(i) {
        match b {
            b'\r' | b'\n' => {
                out.extend_from_slice(b"\r\n");
                // CR LF is one line end.
                if b == b'\r' && raw.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                line_start = true;
            }
            _ => {
                if line_start && b == b'.' {
                    out.push(b'.');
                }
                out.push(b);
                line_start = false;
            }
        }
        i += 1;
    }
    if !line_start {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

/// What `data_lines` wrote, read back as a server reads it: the lone dot's line ends it, and a
/// dot that starts a line is taken off.
#[cfg(test)]
pub(crate) fn read_data(sent: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for line in sent.split_inclusive(|&b| b == b'\n') {
        if line == b".\r\n" {
            return Some(out);
        }
        out.extend_from_slice(line.strip_prefix(b".").unwrap_or(line));
    }
    None
}
