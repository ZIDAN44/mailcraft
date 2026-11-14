//! An answer's body as it comes off the connection, into any writer (a `Vec` for an answer read
//! whole, a file for a download). Hostile input: the body is capped on its own bytes (not on
//! the chunk framing around them), a line of the framing is at most 1 KB, and what has been
//! read is dropped as it goes, so besides the body no more than a line and one read are held.
//! The event stream (`EventStream`) reads its chunks with the same size-line helpers.

use super::*;

/// What came with the head, then the connection.
pub(super) struct Ahead<'a, S> {
    s: &'a mut S,
    buf: Vec<u8>,
    pos: usize,
    /// A chunked body came whole but its trailer didn't ([`copy_chunks`]): where the next
    /// answer would start isn't known.
    cut: bool,
}

impl<'a, S: Read> Ahead<'a, S> {
    /// Reading on from `s`, after `buf` (what came with the head).
    pub(super) fn new(s: &'a mut S, buf: Vec<u8>) -> Ahead<'a, S> {
        Ahead { s, buf, pos: 0, cut: false }
    }

    /// Whether the body read was whole though the connection failed or ended before the
    /// answer did: no other request can go on it.
    pub(super) fn cut(&self) -> bool {
        self.cut
    }

    /// More bytes from the connection; false at its end.
    fn more(&mut self) -> Result<bool> {
        // What has been read goes first: at most the start of a line is left to keep.
        self.buf.drain(..self.pos.min(self.buf.len()));
        self.pos = 0;
        let mut tmp = [0u8; 16 << 10];
        let n = self.s.read(&mut tmp)?;
        self.buf.extend_from_slice(tmp.get(..n).unwrap_or_default());
        Ok(n > 0)
    }

    /// Up to `n` bytes (fewer when the connection ends first) into `out`: how many.
    fn copy<W: Write>(&mut self, n: u64, out: &mut W) -> Result<u64> {
        let mut done = 0u64;
        while done < n {
            if self.pos >= self.buf.len() && !self.more()? {
                break;
            }
            let have = self.buf.len().saturating_sub(self.pos);
            let take = usize::try_from(n - done).map_or(have, |left| left.min(have));
            out.write_all(self.buf.get(self.pos..self.pos + take).unwrap_or_default())?;
            self.pos += take;
            done += take as u64;
        }
        Ok(done)
    }

    /// A line ended by CRLF (at most 1 KB), without its end; `None` when the connection ends
    /// first.
    fn line(&mut self) -> Result<Option<&[u8]>> {
        let end = loop {
            if let Some(p) = line_end(self.buf.get(self.pos..).unwrap_or_default())? {
                break p;
            }
            if !self.more()? {
                return Ok(None);
            }
        };
        let start = self.pos;
        self.pos += end + 2;
        Ok(Some(self.buf.get(start..start + end).unwrap_or_default()))
    }
}

/// `max` bytes, in words: "640 KB", "96 MB".
pub(super) fn size_text(max: u64) -> String {
    if max >= 1 << 20 { format!("{} MB", max >> 20) } else { format!("{} KB", max >> 10) }
}

/// The body of `head`'s answer, at most `max` bytes, into `out`: by its length, its chunks, or
/// to the end of the connection. `what` names the body in errors ("response", "download").
pub(super) fn copy_body<S: Read, W: Write>(body: &mut Ahead<'_, S>, head: &Response, max: u64, out: &mut W, what: &str) -> Result<u64> {
    let too_big = || NetError::Protocol(format!("the {what} is larger than {}", size_text(max)));
    let ended = || NetError::Closed(format!("the {what} ended early"));
    if head.header("Transfer-Encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
        return copy_chunks(body, max, out, what);
    }
    if let Some(len) = head.header("Content-Length") {
        let len: u64 = len.trim().parse().map_err(|_| NetError::Protocol("bad Content-Length".into()))?;
        if len > max {
            return Err(too_big());
        }
        return if body.copy(len, out)? == len { Ok(len) } else { Err(ended()) };
    }
    // No length: the body runs to the end of the connection.
    let done = body.copy(max.saturating_add(1), out)?;
    if done > max { Err(too_big()) } else { Ok(done) }
}

/// The body of a chunked answer (RFC 9112 §7.1), at most `max` bytes, into `out`: how many.
/// The cap is on the body's bytes, not the framing around them; every chunk is followed by its
/// CRLF. The trailer after the last chunk is read to its end, so a connection kept open is
/// where the next answer starts. The body is whole at its last chunk: a connection that ends,
/// stalls or fails in the trailer has lost nothing of it, and is marked as one to send nothing
/// more on ([`Ahead::cut`]). A trailer that breaks the framing's own limits is still refused.
pub(super) fn copy_chunks<S: Read, W: Write>(body: &mut Ahead<'_, S>, max: u64, out: &mut W, what: &str) -> Result<u64> {
    let ended = || NetError::Closed(format!("the {what} ended early"));
    let mut done = 0u64;
    loop {
        let size = chunk_size(body.line()?.ok_or_else(ended)?)?;
        if size == 0 {
            break;
        }
        if done.saturating_add(size) > max {
            return Err(NetError::Protocol(format!("the {what} is larger than {}", size_text(max))));
        }
        if body.copy(size, out)? < size {
            return Err(ended());
        }
        done += size;
        if !body.line()?.ok_or_else(ended)?.is_empty() {
            return Err(NetError::Protocol("bad chunk end".into()));
        }
    }
    for _ in 0..MAX_TRAILERS {
        match body.line().map(|field| field.map(<[u8]>::is_empty)) {
            Ok(Some(false)) => {}
            Ok(Some(true)) => return Ok(done),
            Err(e @ NetError::Protocol(_)) => return Err(e),
            Ok(None) | Err(_) => {
                body.cut = true;
                return Ok(done);
            }
        }
    }
    Err(NetError::Protocol("too many trailer fields".into()))
}

/// Most fields in a chunked answer's trailer.
const MAX_TRAILERS: usize = 64;

/// Longest line in a chunked body's framing: a size line with its extensions, a trailer field.
const MAX_LINE: usize = 1024;

/// Where the CRLF ending the line `bytes` start with is: `None` while it may still come, an
/// error once the line is longer than [`MAX_LINE`].
pub(super) fn line_end(bytes: &[u8]) -> Result<Option<usize>> {
    let near = bytes.get(..MAX_LINE + 2).unwrap_or(bytes);
    match near.windows(2).position(|w| w == b"\r\n") {
        Some(p) => Ok(Some(p)),
        None if near.len() < MAX_LINE + 2 => Ok(None),
        None => Err(NetError::Protocol("bad chunk size line".into())),
    }
}

/// The size a chunk's size line (without its CRLF) gives: hex digits, then any `;` extensions
/// (RFC 9112 §7.1). Nothing else is taken: no sign, no space before the digits.
pub(super) fn chunk_size(line: &[u8]) -> Result<u64> {
    let digits = line.split(|b| *b == b';').next().unwrap_or_default();
    let end = digits.iter().rposition(|b| !matches!(b, b' ' | b'\t')).map_or(0, |p| p + 1);
    let digits = digits.get(..end).unwrap_or_default();
    let size = digits.iter().try_fold(0u64, |n, b| n.checked_mul(16)?.checked_add(u64::from(char::from(*b).to_digit(16)?)));
    size.filter(|_| !digits.is_empty()).ok_or_else(|| NetError::Protocol("bad chunk size".into()))
}

#[cfg(test)]
mod tests;
