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
    text: Vec<u8>,
}

impl SseReader {
    /// Add bytes; the events now complete. `Err` when an event grows too long.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Sse>, String> {
        self.text.extend(bytes.iter().copied().filter(|b| *b != b'\r'));
        let mut out = Vec::new();
        while let Some(end) = self.text.windows(2).position(|w| w == b"\n\n") {
            let block = String::from_utf8_lossy(self.text.get(..end).unwrap_or_default()).to_string();
            self.text.drain(..end + 2);
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
            if !data.is_empty() || !event.is_empty() {
                out.push(Sse { event: if event.is_empty() { "message".into() } else { event }, data: data.join("\n") });
            }
        }
        if self.text.len() > MAX_EVENT {
            self.text.clear();
            return Err("an event is too long".into());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_come_whole_however_the_bytes_are_cut() {
        let stream =
            b": keep-alive\r\n\r\nevent: state\r\ndata: {\"changed\":\r\ndata: {}}\r\n\r\nevent: ping\ndata: {\"interval\":30}\n\ndata: plain\n\n";
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
                ],
                "cut every {cut}"
            );
        }
        let mut r = SseReader::default();
        assert!(r.push(&vec![b'x'; MAX_EVENT + 1]).is_err());
        assert_eq!(r.push(b"event: state\n\n").unwrap().len(), 1, "it goes on after");
    }
}
