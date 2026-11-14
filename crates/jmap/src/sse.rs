//! Server-sent events (`text/event-stream`, the format of JMAP push, RFC 8620 §7.3), parsed from
//! the bytes of a response as they arrive: the browser reads its push stream through `fetch`,
//! which hands over the body in pieces of any size.

/// The longest event kept (an event this long without its end is refused).
pub const MAX_EVENT: usize = 1 << 20;

/// One event: its type (`message` when it says none) and its data lines joined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sse {
    pub event: String,
    pub data: String,
}

/// Bytes received and not yet a whole event.
#[derive(Default)]
pub struct SseReader {
    /// What's left of the stream, its line ends all made `\n`.
    text: Vec<u8>,
    /// How much of `text` was looked through for an event's end already.
    scanned: usize,
    /// The last byte was a `\r`: a `\n` right after it ends the same line.
    after_cr: bool,
}

impl SseReader {
    /// Add bytes; the events now complete. `Err` when an event grows too long.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Sse>, String> {
        // A line ends with CR LF, LF or a CR alone (the format's `end-of-line`).
        self.text.reserve(bytes.len());
        for &b in bytes {
            if !(self.after_cr && b == b'\n') {
                self.text.push(if b == b'\r' { b'\n' } else { b });
            }
            self.after_cr = b == b'\r';
        }
        // Events end with an empty line: look for it only in what's new (and the byte before),
        // and drop the events read all at once.
        let mut out = Vec::new();
        let mut start = 0;
        let mut from = self.scanned.saturating_sub(1);
        while let Some(at) = self.text.get(from..).and_then(|t| t.windows(2).position(|w| w == b"\n\n")) {
            let end = from + at;
            out.extend(event(self.text.get(start..end).unwrap_or_default()));
            start = end + 2;
            from = start;
        }
        self.text.drain(..start.min(self.text.len()));
        self.scanned = self.text.len();
        if self.text.len() > MAX_EVENT {
            self.text.clear();
            self.scanned = 0;
            return Err("an event is too long".into());
        }
        Ok(out)
    }
}

/// The event a block of lines makes, if it has a type or data.
fn event(block: &[u8]) -> Option<Sse> {
    let block = String::from_utf8_lossy(block);
    let mut event = String::new();
    let mut data: Vec<&str> = Vec::new();
    for line in block.lines() {
        // `:` starts a comment (keep-alives).
        let (field, value) = line.split_once(':').map_or((line, ""), |(f, v)| (f, v.strip_prefix(' ').unwrap_or(v)));
        match field {
            "event" => event = value.to_string(),
            "data" => data.push(value),
            _ => {}
        }
    }
    (!data.is_empty() || !event.is_empty()).then(|| Sse { event: if event.is_empty() { "message".into() } else { event }, data: data.join("\n") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_come_whole_however_the_bytes_are_cut() {
        // Lines end with CR LF, LF, or a CR alone (the last event's).
        let stream = b": keep-alive\r\n\r\nevent: state\r\ndata: {\"changed\":\r\ndata: {}}\r\n\r\nevent: ping\ndata: {\"interval\":30}\n\ndata: plain\n\ndata: a\rdata: b\r\r";
        for cut in 1..stream.len() {
            let mut r = SseReader::default();
            let mut got = Vec::new();
            for piece in stream.chunks(cut) {
                got.extend(r.push(piece).unwrap());
            }
            assert_eq!(
                got,
                [
                    Sse { event: "state".into(), data: "{\"changed\":\n{}}".into() },
                    Sse { event: "ping".into(), data: "{\"interval\":30}".into() },
                    Sse { event: "message".into(), data: "plain".into() },
                    Sse { event: "message".into(), data: "a\nb".into() },
                ],
                "cut every {cut}"
            );
        }
        let mut r = SseReader::default();
        assert!(r.push(&vec![b'x'; MAX_EVENT + 1]).is_err());
        assert_eq!(r.push(b"event: state\n\n").unwrap().len(), 1, "it goes on after");
    }

    #[test]
    #[ignore = "a measurement: cargo test --release -p mailcraft-jmap sse_cost -- --ignored --nocapture"]
    fn sse_cost() {
        let burst: Vec<u8> = (0..100_000).flat_map(|i| format!("event: state\r\ndata: {{\"n\":{i}}}\r\n\r\n").into_bytes()).collect();
        let t = std::time::Instant::now();
        let got = SseReader::default().push(&burst).unwrap().len();
        println!("MEASURE 100000 events in one piece ({} KB): {} ms", burst.len() / 1024, t.elapsed().as_millis());
        assert_eq!(got, 100_000);
        let big: Vec<u8> = [b"data: ".as_slice(), &vec![b'x'; 512 << 10], b"\n\n"].concat();
        let t = std::time::Instant::now();
        let mut r = SseReader::default();
        let got: usize = big.chunks(64).map(|p| r.push(p).unwrap().len()).sum();
        println!("MEASURE one 512 KB event in 64-byte pieces: {} ms", t.elapsed().as_millis());
        assert_eq!(got, 1);
    }
}
