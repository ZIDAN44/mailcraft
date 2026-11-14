use std::io::{BufRead, BufReader};
use std::net::TcpListener;

use super::*;

/// Found in review: a redirect to another site took the request's sign-in along (the
/// password of a JMAP account, a Microsoft token). It stays with the site it was for.
#[test]
fn a_sign_in_doesnt_follow_a_redirect_to_another_site() {
    for (a, b, same) in [
        ("example.com", "EXAMPLE.com.", true),
        ("example.com", "api.example.com", true),
        ("api.example.com", "www.example.com", true),
        ("a.b.example.com", "c.example.com", false),
        ("example.com", "example.net", false),
        ("example.com", "evilexample.com", false),
        ("a.co.uk", "b.co.uk", false),
        ("mail.a.co.uk", "dav.a.co.uk", true),
        ("127.0.0.1", "127.0.0.1", true),
        ("127.0.0.1", "127.0.0.2", false),
        ("localhost", "example.com", false),
        ("example.com", "com", false),
        ("a.github.io", "b.github.io", false),
        ("x.a.github.io", "a.github.io", true),
        ("mail.example.co.uk", "co.uk", false),
    ] {
        assert_eq!(same_site(a, b), same, "{a} / {b}");
    }
    let req =
        Request::get("https://example.com/.well-known/jmap", 1000).header("Authorization", "Basic c2VjcmV0").header("Accept", "application/json");
    let at = |u: &str| Url::parse(u).unwrap();
    // Another site: the rest of the request goes on, without it.
    let bare = redirected(&req, &at("https://example.com/a"), &at("https://elsewhere.example/b")).unwrap();
    assert_eq!(bare.headers, [("Accept".to_string(), "application/json".to_string())]);
    // The same site, or nothing secret in it: as it is.
    assert!(redirected(&req, &at("https://example.com/a"), &at("https://api.example.com/b")).is_none());
    assert!(redirected(&bare, &at("https://example.com/a"), &at("https://elsewhere.example/b")).is_none());
}

/// Serve `responses` (one per connection, in order) on localhost; returns the base URL and
/// the requests received.
pub(crate) fn serve(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let h = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for r in responses {
            let (mut s, _) = l.accept().unwrap();
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut req = String::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                req.push_str(&line);
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            req.push_str(&String::from_utf8_lossy(&body));
            seen.push(req);
            s.write_all(&r).unwrap();
        }
        seen
    });
    (base, h)
}

#[test]
fn urls() {
    let u = Url::parse("HTTPS://Mail.Example.com/a/b?x=1#frag").unwrap();
    assert_eq!((u.https, u.host.as_str(), u.port, u.path.as_str()), (true, "mail.example.com", 443, "/a/b?x=1"));
    assert_eq!(Url::parse("http://h:8080").unwrap().path, "/");
    assert_eq!(Url::parse("http://[::1]:99/x").unwrap().host, "::1");
    assert_eq!(Url::parse("http://h/a b").unwrap().path, "/a%20b");
    for bad in ["ftp://h/", "http://", "http://user@h/", "http://h:0/", "http://h:x/", "javascript:alert(1)", "http://h h/"] {
        assert!(Url::parse(bad).is_err(), "{bad}");
    }
    let u = Url::parse("https://h.example/a/b/c?q").unwrap();
    assert_eq!(u.join("/x").unwrap().path, "/x");
    assert_eq!(u.join("d").unwrap().path, "/a/b/d");
    assert_eq!(u.join("//other.example/p").unwrap().host, "other.example");
    assert!(!u.join("http://x.example/").unwrap().https);
    assert_eq!(pct("a b&c=é"), "a%20b%26c%3D%C3%A9");
    assert_eq!(form(&[("a", "1 2"), ("b", "/")]), b"a=1%202&b=%2F".to_vec());
}

#[test]
fn lengths_chunks_redirects_and_limits() {
    let (base, h) = serve(vec![
        b"HTTP/1.1 302 Found\r\nLocation: /two\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: text/plain\r\n\r\nhello".to_vec(),
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 999999\r\n\r\nxx".to_vec(),
        b"HTTP/1.0 200 OK\r\n\r\nuntil the end".to_vec(),
        b"HTTP/1.1 201 Created\r\nContent-Length: 2\r\n\r\nok".to_vec(),
        b"garbage\r\n\r\n".to_vec(),
    ]);
    let r = get(&format!("{base}/one"), 1000).unwrap();
    assert_eq!((r.status, r.body.as_slice(), r.header("content-type")), (200, &b"hello"[..], Some("text/plain")));
    assert_eq!(get(&base, 1000).unwrap().body, b"Wikipedia");
    assert!(matches!(get(&base, 1000), Err(NetError::Protocol(m)) if m.contains("larger")));
    assert_eq!(get(&base, 1000).unwrap().body, b"until the end");
    let r = send(
        &Request::post(&format!("{base}/token"), "application/x-www-form-urlencoded", form(&[("code", "x y")]), 100)
            .header("Authorization", "Bearer t"),
    )
    .unwrap();
    assert!(r.ok() && r.body == b"ok");
    assert!(get(&base, 1000).is_err());
    let seen = h.join().unwrap();
    assert!(seen[0].starts_with("GET /one HTTP/1.1\r\nHost: 127.0.0.1:"), "{}", seen[0]);
    assert!(seen[1].starts_with("GET /two "), "redirect followed: {}", seen[1]);
    assert!(seen[5].starts_with("POST /token ") && seen[5].contains("Authorization: Bearer t") && seen[5].ends_with("code=x%20y"), "{}", seen[5]);
    // Header injection is refused before anything is sent.
    assert!(send(&Request::get(&base, 10).header("X", "a\r\nEvil: 1")).is_err());
}

/// Hands out bytes a few at a time, then "times out".
struct Drip(Vec<u8>, usize);

impl Read for Drip {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "nothing yet"));
        }
        let n = self.1.min(self.0.len()).min(buf.len());
        buf[..n].copy_from_slice(&self.0[..n]);
        self.0.drain(..n);
        Ok(n)
    }
}

impl crate::stream::Timeouts for Drip {
    fn set_read_timeout(&mut self, _: Option<std::time::Duration>) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn event_streams() {
    let body = ": hello\n\nevent: state\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\ndata: plain\n\n";
    // Chunked, split at awkward places.
    let mut chunked = Vec::new();
    for part in [&body[..5], &body[5..23], &body[23..]] {
        chunked.extend(format!("{:x};ext=1\r\n{part}\r\n", part.len()).as_bytes());
    }
    for step in [1, 3, 64] {
        let mut es = EventStream::new(Drip(chunked.clone(), step), true);
        let mut got = Vec::new();
        for _ in 0..200 {
            if let Some(e) = es.next(std::time::Duration::from_millis(1)).unwrap() {
                got.push(e);
            }
        }
        assert_eq!(
            got,
            vec![Sse { event: "state".into(), data: "{\"a\":\n1}".into() }, Sse { event: "message".into(), data: "plain".into() }],
            "step {step}"
        );
    }
    let mut es = EventStream::new(Drip(body.as_bytes().to_vec(), 7), false);
    let first = (0..50).find_map(|_| es.next(std::time::Duration::from_millis(1)).unwrap());
    assert_eq!(first.map(|e| e.event), Some("state".into()), "plain (not chunked) too");
    // The end of the stream, and hostile input.
    assert!(matches!(EventStream::new(Drip(b"0\r\n\r\n".to_vec(), 9), true).next(std::time::Duration::ZERO), Err(NetError::Closed(_))));
    assert!(EventStream::new(Drip(b"zz\r\n".to_vec(), 9), true).next(std::time::Duration::ZERO).is_err());
    let endless = vec![b'x'; MAX_EVENT + 10];
    let mut es = EventStream::new(Drip(endless, 1 << 16), false);
    assert!((0..100).any(|_| es.next(std::time::Duration::ZERO).is_err()), "an event without an end is refused");
}
