use std::collections::VecDeque;

use super::*;

/// The connection's bytes, one piece per read.
struct Pieces(VecDeque<Vec<u8>>);

impl Read for Pieces {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(mut piece) = self.0.pop_front() else { return Ok(0) };
        let n = piece.len().min(buf.len());
        buf[..n].copy_from_slice(&piece[..n]);
        if n < piece.len() {
            piece.drain(..n);
            self.0.push_front(piece);
        }
        Ok(n)
    }
}

/// Found in the code audit: every read that ended inside a size line kept all that had been
/// read before it, so tiny chunks held the whole answer's framing besides the body.
#[test]
fn what_was_read_is_dropped_as_it_goes() {
    // Every read ends inside a size line: the line's start is all there is to keep.
    let mut pieces = VecDeque::from([b"1".to_vec()]);
    pieces.extend(std::iter::repeat_n(b"\r\nx\r\n1".to_vec(), 10_000));
    pieces.push_back(b"\r\nx\r\n0\r\n\r\n".to_vec());
    let mut s = Pieces(pieces);
    let mut body = Ahead::new(&mut s, Vec::new());
    let mut out = Vec::new();
    assert_eq!(copy_chunks(&mut body, 1 << 20, &mut out, "response").unwrap(), 10_001);
    assert!(body.buf.capacity() < 1024, "{} bytes held", body.buf.capacity());
}

/// `bytes` decoded as a chunked body capped at `max`, read `step` bytes at a time; with what's
/// left unread after it.
fn decode(bytes: &[u8], step: usize, max: u64) -> (Result<u64>, Vec<u8>, Vec<u8>) {
    let mut s = Pieces(bytes.chunks(step.max(1)).map(<[u8]>::to_vec).collect());
    let mut out = Vec::new();
    let mut body = Ahead::new(&mut s, Vec::new());
    let r = copy_chunks(&mut body, max, &mut out, "response");
    let mut left = body.buf.get(body.pos..).unwrap_or_default().to_vec();
    s.0.iter().for_each(|p| left.extend(p));
    (r, out, left)
}

/// The three chunk decoders became one (from the code audit), taking the strictest of what
/// each checked: the size line is hex digits only, at most 1 KB with its extensions; the CRLF
/// after each chunk is checked; the trailer is read to its end.
#[test]
fn chunk_framing_is_checked() {
    for (line, size) in [("1a", 26), ("1A;ext=v", 26), ("5 ;x", 5), ("5\t", 5), ("0000000000000000005", 5), ("ffffffffffffffff", u64::MAX)] {
        assert_eq!(chunk_size(line.as_bytes()).ok(), Some(size), "{line}");
    }
    for line in ["", ";x", "+5", " 5", "-1", "5x", "1_0", "0x5", "10000000000000000", "é"] {
        assert!(chunk_size(line.as_bytes()).is_err(), "{line}");
    }
    let long = |n: usize| [vec![b'0'; n], b"5\r\n".to_vec()].concat();
    assert_eq!(line_end(&long(1023)).unwrap(), Some(1024));
    assert!(line_end(&long(1024)).is_err(), "a line past 1 KB, its end there or not");
    assert!(line_end(&vec![b'0'; 2000]).is_err());
    assert_eq!(line_end(b"5").unwrap(), None);
    // The trailer is read, and nothing after it.
    for step in [1, 2, 5, 100] {
        let (r, out, left) = decode(b"3\r\nabc\r\n0\r\nX-Sum: 1\r\nX-Other: 2\r\n\r\nNEXT", step, 100);
        assert_eq!((r.unwrap(), out.as_slice(), left.as_slice()), (3, &b"abc"[..], &b"NEXT"[..]), "step {step}");
    }
    // The connection ending in the trailer has lost nothing; before it, the body is cut short.
    assert_eq!(decode(b"3\r\nabc\r\n0\r\n", 4, 100).0.unwrap(), 3);
    assert!(matches!(decode(b"3\r\nabc\r\n", 4, 100).0, Err(NetError::Closed(_))));
    assert!(matches!(decode(b"3\r\nab", 4, 100).0, Err(NetError::Closed(_))));
    // A chunk not followed by its CRLF, too many trailer fields.
    assert!(matches!(decode(b"3\r\nabcXY0\r\n\r\n", 4, 100).0, Err(NetError::Protocol(m)) if m.contains("chunk end")));
    let fields = "X: 1\r\n".repeat(65);
    assert!(decode(format!("1\r\na\r\n0\r\n{fields}\r\n").as_bytes(), 7, 100).0.is_err());
}

/// The connection's bytes, then a failure at every read after them.
struct ThenFails(Pieces, std::io::ErrorKind);

impl Read for ThenFails {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0.0.is_empty() { Err(self.1.into()) } else { self.0.read(buf) }
    }
}

/// A body is whole at its last chunk: a server that then stalls, or closes without saying so
/// (TLS with no `close_notify`), or resets, before the trailer's empty line lost the body that
/// had come. It is kept, and the connection is one to send nothing more on. Before the last
/// chunk the same failures still fail.
#[test]
fn a_failure_in_the_trailer_loses_no_body() {
    use std::io::ErrorKind::{ConnectionReset, TimedOut, UnexpectedEof};
    let read = |bytes: &[u8], fails: std::io::ErrorKind| {
        let mut s = ThenFails(Pieces(bytes.chunks(4).map(<[u8]>::to_vec).collect()), fails);
        let mut out = Vec::new();
        let mut body = Ahead::new(&mut s, Vec::new());
        let r = copy_chunks(&mut body, 100, &mut out, "response");
        (r, out, body.cut())
    };
    for fails in [TimedOut, UnexpectedEof, ConnectionReset] {
        for whole in [&b"3\r\nabc\r\n0\r\n"[..], b"3\r\nabc\r\n0\r\nX-Sum: 1\r\n", b"3\r\nabc\r\n0\r\nX-Su"] {
            let (r, out, cut) = read(whole, fails);
            assert_eq!((r.ok(), out.as_slice(), cut), (Some(3), &b"abc"[..], true), "{fails:?} after {whole:?}");
        }
        for part in [&b"3\r\nabc\r\n"[..], b"3\r\nab", b"3\r\nabc\r\n0\r"] {
            let (r, _, cut) = read(part, fails);
            assert!(matches!(r, Err(NetError::Io(_))) && !cut, "{fails:?} after {part:?}");
        }
        // Read to its end, the connection is where the next answer starts.
        let (r, _, cut) = read(b"3\r\nabc\r\n0\r\nX-Sum: 1\r\n\r\n", fails);
        assert_eq!((r.ok(), cut), (Some(3), false));
    }
    // The connection ending in the trailer is as much a cut; a trailer past the limits is refused.
    let mut s = Pieces([b"3\r\nabc\r\n0\r\n".to_vec()].into());
    let mut body = Ahead::new(&mut s, Vec::new());
    assert_eq!(copy_chunks(&mut body, 100, &mut Vec::new(), "response").ok(), Some(3));
    assert!(body.cut());
    let long = format!("1\r\na\r\n0\r\nX: {}\r\n\r\n", "x".repeat(2000));
    assert!(matches!(read(long.as_bytes(), TimedOut).0, Err(NetError::Protocol(_))));
}

/// `body` framed in chunks of the sizes `sizes` gives in turn, with an extension on some.
fn frame(body: &[u8], sizes: &[usize]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    for (i, size) in sizes.iter().cycle().enumerate() {
        if at >= body.len() {
            break;
        }
        let end = body.len().min(at + size.max(&1));
        out.extend(format!("{:x}{}\r\n", end - at, if i % 3 == 1 { ";ext=1" } else { "" }).as_bytes());
        out.extend(&body[at..end]);
        out.extend(b"\r\n");
        at = end;
    }
    out.extend(b"0\r\n\r\n");
    out
}

proptest::proptest! {
    /// Any body, in chunks of any sizes, read any number of bytes at a time, comes back whole;
    /// with the cap a byte under its size, it's refused, and no more than the cap is written.
    #[test]
    fn chunks_come_back_whole(
        body in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2000),
        sizes in proptest::collection::vec(1usize..300, 1..8),
        step in 1usize..40,
    ) {
        let framed = frame(&body, &sizes);
        let (r, out, left) = decode(&framed, step, body.len() as u64);
        proptest::prop_assert_eq!(r.ok(), Some(body.len() as u64));
        proptest::prop_assert_eq!(&out, &body);
        proptest::prop_assert!(left.is_empty());
        if let Some(cap) = (body.len() as u64).checked_sub(1) {
            let (r, out, _) = decode(&framed, step, cap);
            proptest::prop_assert!(r.is_err());
            proptest::prop_assert!(out.len() as u64 <= cap);
        }
    }

    /// Anything at all, or a framed body with one byte changed: no panic, and never more than
    /// the cap written.
    #[test]
    fn junk_stays_within_the_cap(
        junk in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..600),
        flip in proptest::prelude::any::<(usize, u8)>(),
        step in 1usize..40,
        max in 0u64..300,
    ) {
        let mut framed = frame(&junk, &[7, 50]);
        let at = flip.0 % framed.len();
        framed[at] = flip.1;
        for bytes in [&junk, &framed] {
            let (r, out, _) = decode(bytes, step, max);
            proptest::prop_assert!(out.len() as u64 <= max);
            if let Ok(n) = r {
                proptest::prop_assert_eq!(n, out.len() as u64);
            }
        }
    }
}
