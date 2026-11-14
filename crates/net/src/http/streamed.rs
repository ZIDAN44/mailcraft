//! A GET whose body goes to a writer as it comes, for a download too big to hold in memory (an
//! update's installer): the answer's head first, so the caller can choose where the body goes
//! (a partial file continued, or started again), then its bytes. Hostile input as every answer
//! is: the head and the body are capped, and chunk sizes checked.

use super::body::{Ahead, copy_body};
use super::*;

/// Send `req` (a GET, following redirects as [`send`] does) and hand the final answer's status
/// and headers to `open`, which gives the writer for its body, or `None` to leave the body
/// unread. At most `req.max` bytes of body are accepted. The answer comes back without its body;
/// a body cut short is an error, after what came of it was written.
pub fn get_into<W: Write>(req: &Request, open: impl FnOnce(&Response) -> Result<Option<W>>) -> Result<Response> {
    let mut open = Some(open);
    follow(req, |req, url| exchange_into(&mut connect(url, false)?, req, url, |head| open.take().map_or(Ok(None), |open| open(head))))
}

/// Write the request on an open connection and read the answer's head; its body goes to the
/// writer `open` gives, as [`get_into`] says. A redirect's body is left unread (`open` isn't
/// asked): the caller follows it on a connection of its own.
pub fn exchange_into<S: Read + Write, W: Write>(
    s: &mut S,
    req: &Request,
    url: &Url,
    open: impl FnOnce(&Response) -> Result<Option<W>>,
) -> Result<Response> {
    write_request(s, req, url, false)?;
    let (status, headers, rest) = read_head(s)?;
    let head = Response { status, headers, body: Vec::new() };
    if redirect(&head).is_some() {
        return Ok(head);
    }
    if let Some(mut out) = open(&head)? {
        copy_body(&mut Ahead::new(s, rest), &head, u64::try_from(req.max).unwrap_or(u64::MAX), &mut out, "download")?;
        out.flush()?;
    }
    Ok(head)
}

#[cfg(test)]
mod tests {
    use super::super::tests::serve;
    use super::*;

    fn fetch(url: &str, max: usize) -> (Result<Response>, Vec<u8>) {
        let mut got = Vec::new();
        let r = get_into(&Request::get(url, max), |_| Ok(Some(&mut got)));
        (r, got)
    }

    /// Lengths, chunks split anywhere, to the end, redirects, and every way a body goes wrong.
    #[test]
    fn bodies_are_written_as_they_come() {
        let (base, h) = serve(vec![
            b"HTTP/1.1 302 Found\r\nLocation: /file\r\nContent-Length: 3\r\n\r\nnop".to_vec(),
            b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 2-6/7\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\n\r\n".to_vec(),
            b"HTTP/1.0 200 OK\r\n\r\nuntil the end".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 99999\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWikiXX0\r\n\r\n".to_vec(),
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found".to_vec(),
        ]);
        let (r, got) = fetch(&format!("{base}/start"), 1000);
        let r = r.unwrap();
        assert_eq!((r.status, r.header("content-range"), got.as_slice()), (206, Some("bytes 2-6/7"), &b"hello"[..]));
        assert_eq!(fetch(&base, 1000).1, b"Wikipedia");
        assert_eq!(fetch(&base, 1000).1, b"until the end");
        let (r, got) = fetch(&base, 1000);
        assert!(matches!(r, Err(NetError::Closed(_))), "{r:?}");
        assert_eq!(got, b"short", "what came is kept, to carry on from");
        assert!(matches!(fetch(&base, 1000).0, Err(NetError::Protocol(m)) if m.contains("larger")));
        assert!(fetch(&base, 1000).0.is_err());
        assert!(fetch(&base, 1000).0.is_err(), "a chunk without its end");
        // The caller can look at the answer and leave its body.
        let r = get_into::<Vec<u8>>(&Request::get(&base, 1000), |head| Ok((head.status == 200).then(Vec::new))).unwrap();
        assert_eq!(r.status, 404);
        let seen = h.join().unwrap();
        assert!(seen[1].starts_with("GET /file "), "redirect followed: {}", seen[1]);
    }
}
