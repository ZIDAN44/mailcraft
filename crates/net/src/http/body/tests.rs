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
