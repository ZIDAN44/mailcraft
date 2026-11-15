use std::io::{BufRead, BufReader, ErrorKind};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use super::*;
use crate::fakes::measure::peak_working_set;

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

/// How long a test's server waits for its next connection, and for each read or write on one,
/// before it gives up: a client that never comes fails the test that waits on the server, where
/// it hung the whole run (a test waited for a token request that wasn't coming, for an hour).
pub(crate) const TEST_SERVER_WAIT: Duration = Duration::from_secs(60);

/// The next connection to `l` within `wait`, its reads and writes timing out after as long;
/// `None` when none came in that time.
pub(crate) fn accept_within(l: &TcpListener, wait: Duration) -> Option<TcpStream> {
    l.set_nonblocking(true).ok()?;
    let end = Instant::now() + wait;
    loop {
        match l.accept() {
            Ok((s, _)) => {
                // Taken from a listener that doesn't block, it wouldn't either (on Windows).
                s.set_nonblocking(false).ok()?;
                s.set_read_timeout(Some(wait)).ok()?;
                s.set_write_timeout(Some(wait)).ok()?;
                return Some(s);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock && Instant::now() < end => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => return None,
        }
    }
}

/// Serve `responses` (one per connection, in order) on localhost; returns the base URL and
/// the requests received. A request that doesn't come within [`TEST_SERVER_WAIT`] fails the
/// server's thread, and so whoever joins it, saying how many came.
pub(crate) fn serve(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    serve_within(responses, TEST_SERVER_WAIT)
}

/// [`serve`], waiting `wait` for each request.
fn serve_within(responses: Vec<Vec<u8>>, wait: Duration) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let h = std::thread::spawn(move || {
        let mut seen = Vec::new();
        let all = responses.len();
        for r in responses {
            let Some(mut s) = accept_within(&l, wait) else {
                panic!("the test server was sent {} of its {all} requests: no other came in {wait:?}", seen.len());
            };
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

/// Test hang: a test joined the token server once asked to sign in again, and the token request
/// the server still waited for never came; the server waited for it, and the test with it, for an
/// hour. A request that doesn't come in time fails the server's thread, saying how many did.
#[test]
fn a_test_server_waits_for_its_requests_only_so_long() {
    let ok = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec();
    let (base, h) = serve_within(vec![ok.clone(), ok], Duration::from_millis(300));
    assert_eq!(get(&base, 100).unwrap().body, b"ok");
    let began = Instant::now();
    let why = h.join().expect_err("the second request never came");
    assert!(began.elapsed() < Duration::from_secs(10), "{:?}", began.elapsed());
    let why = why.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(why.contains("1 of its 2 requests"), "{why}");
    // A client that connects and says nothing is let go as soon.
    let (base, h) = serve_within(vec![b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec()], Duration::from_millis(300));
    let quiet = TcpStream::connect(base.trim_start_matches("http://")).unwrap();
    let seen = h.join();
    drop(quiet);
    assert!(seen.is_ok_and(|s| s == [""]), "an empty request served, not waited on");
}

/// The path encoder before it wrote each character's UTF-8 bytes, as it was.
fn old_spaces_escaped(path: &str) -> String {
    path.chars()
        .flat_map(|c| if c == ' ' || c.is_control() { format!("%{:02X}", u32::from(c) & 0xff).chars().collect::<Vec<_>>() } else { vec![c] })
        .collect()
}

/// A control past ASCII is written as its UTF-8 bytes, not one byte of its number; everything
/// else in a path is as it was.
#[test]
fn a_paths_controls_are_written_as_their_utf8_bytes() {
    assert_eq!(Url::parse("http://h/a\u{85}b\u{9f}").unwrap().path, "/a%C2%85b%C2%9F");
    assert_eq!(old_spaces_escaped("/a\u{85}b"), "/a%85b", "one byte of the number, no character");
    for path in ["/", "/dav/cal/1.ics", "/a b/c\td", "/é €/x\r\ny", "/?q=a b&r=\u{7f}", "/\u{0}\u{1f}", "/日本 語"] {
        assert_eq!(Url::parse(&format!("https://h{path}")).unwrap().path, old_spaces_escaped(path), "{path:?}");
    }
}

/// Run by hand (release, under the machine's lock): Url::parse on DAV-like addresses.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_url_parse() {
    let urls = ["https://dav.example.com/dav/calendars/riley/work/0123456789abcdef-0123-4567.ics", "https://h.example/a b/c d e/f g.ics"];
    for url in urls {
        let n = 300_000;
        let start = Instant::now();
        let mut len = 0;
        for _ in 0..n {
            len += Url::parse(std::hint::black_box(url)).map(|u| u.path.len()).unwrap_or(0);
        }
        let took = start.elapsed();
        println!("MEASURE url_parse {url:?}: {:.0} ns each ({len})", took.as_nanos() as f64 / f64::from(n));
    }
}

#[test]
fn urls() {
    let u = Url::parse("HTTPS://Mail.Example.com/a/b?x=1#frag").unwrap();
    assert_eq!((u.https, u.host.as_str(), u.port, u.path.as_str()), (true, "mail.example.com", 443, "/a/b?x=1"));
    assert_eq!(Url::parse("http://h:8080").unwrap().path, "/");
    assert_eq!(Url::parse("http://[::1]:99/x").unwrap().host, "::1");
    for (url, here) in [("http://[::1]:99/", true), ("http://LOCALHOST/", true), ("http://127.0.0.1:8/", true), ("http://127.0.0.2/", false)] {
        assert_eq!(is_loopback(&Url::parse(url).unwrap().host), here, "{url}");
    }
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

/// A chunked answer whose connection ends before its trailer does is whole, but nothing more
/// goes on that connection: the next request would be read from the middle of nothing.
#[test]
fn a_connection_cut_in_the_trailer_isnt_used_again() {
    let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n";
    for (end, usable) in [("0\r\n", false), ("0\r\nX-Sum: 1\r\n", false), ("0\r\n\r\n", true)] {
        let (base, h) = serve(vec![format!("{head}{end}").into_bytes()]);
        let mut conn = Conn::open(&base).unwrap();
        assert_eq!(conn.send(&Request::get(&base, 100)).unwrap().body, b"abc", "{end:?}");
        assert_eq!(conn.usable(), usable, "{end:?}");
        h.join().unwrap();
    }
}

/// A chunked answer carrying `n` bytes of `x`, in chunks of `size`.
fn chunked(n: usize, size: usize) -> Vec<u8> {
    let mut v = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    let mut left = n;
    while left > 0 {
        let c = left.min(size);
        v.extend(format!("{c:x}\r\n").as_bytes());
        v.extend(std::iter::repeat_n(b'x', c));
        v.extend(b"\r\n");
        left -= c;
    }
    v.extend(b"0\r\n\r\n");
    v
}

/// Found in the code audit: a chunked body was capped on the bytes as they came, framing
/// included, so a body under the cap sent in small chunks was refused as larger than it.
#[test]
fn a_chunked_body_is_capped_by_its_size_not_its_framing() {
    let max = 100_000;
    let (base, h) = serve(vec![chunked(max, 1), chunked(max + 1, 1)]);
    let r = get(&base, max).unwrap();
    assert!(r.body.len() == max && r.body.iter().all(|b| *b == b'x'), "{} bytes", r.body.len());
    assert!(matches!(get(&base, max), Err(NetError::Protocol(m)) if m.contains("larger")), "one byte past the cap");
    // The server's write of the refused answer may fail: the client hung up.
    let _ = h.join();
}

/// A chunked answer of `n` bytes in chunks of `size`, made as it is read (so the answer itself
/// takes no memory), sent in reads of 16 KB.
struct Chunks {
    left: usize,
    size: usize,
    out: Vec<u8>,
    pos: usize,
    ended: bool,
}

impl Read for Chunks {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.out.len() {
            self.out.clear();
            self.pos = 0;
            while self.out.len() < 16 << 10 && !self.ended {
                let c = self.left.min(self.size);
                self.out.extend(format!("{c:x}\r\n").as_bytes());
                self.out.extend(std::iter::repeat_n(b'x', c));
                self.out.extend(if c == 0 { &b""[..] } else { b"\r\n" });
                self.ended = c == 0;
                self.left -= c;
            }
            if self.ended && self.out.ends_with(b"0\r\n") {
                self.out.extend(b"\r\n");
            }
        }
        let n = buf.len().min(self.out.len() - self.pos);
        buf[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Write for Chunks {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Time and peak memory of reading a chunked answer of `n` bytes in chunks of `size`.
fn measure_chunked(n: usize, size: usize) {
    let url = Url::parse("http://127.0.0.1/big").unwrap();
    let req = Request::get("http://127.0.0.1/big", 1 << 30);
    let mut head = std::io::Cursor::new(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec());
    let mut body = Chunks { left: n, size, out: Vec::new(), pos: 0, ended: false };
    let mut s = ReadThen(&mut head, &mut body);
    let before = peak_working_set();
    let t = std::time::Instant::now();
    let r = exchange(&mut s, &req, &url).unwrap();
    let took = t.elapsed();
    assert_eq!(r.body.len(), n);
    let grew = before.zip(peak_working_set()).map(|(b, a)| a.saturating_sub(b) >> 20);
    println!("MEASURE chunked {} MB in {size}-byte chunks: {took:?}, peak working set grew {grew:?} MB", n >> 20);
}

/// One reader, then another; writes go nowhere.
struct ReadThen<'a, A, B>(&'a mut A, &'a mut B);

impl<A: Read, B: Read> Read for ReadThen<'_, A, B> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.0.read(buf)? {
            0 => self.1.read(buf),
            n => Ok(n),
        }
    }
}

impl<A, B> Write for ReadThen<'_, A, B> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Run alone (`--exact`): the peak is the process's.
#[test]
#[ignore = "a measurement: run alone, in release"]
fn measure_chunked_big_chunks() {
    measure_chunked(64 << 20, 16 << 10);
}

/// Run alone (`--exact`): the peak is the process's.
#[test]
#[ignore = "a measurement: run alone, in release"]
fn measure_chunked_one_byte_chunks() {
    measure_chunked(8 << 20, 1);
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
    // The chunk decoders became one (from the code audit): the stream checks the CRLF after a
    // chunk, and takes a size line as answers do.
    let mut es = EventStream::new(Drip(b"9\r\ndata: a\n\nXY5\r\n".to_vec(), 4), true);
    assert!((0..20).any(|_| matches!(es.next(std::time::Duration::ZERO), Err(NetError::Protocol(m)) if m.contains("chunk end"))));
    for bad in [&b"+7\r\n"[..], b" 7\r\n", &[b'0'; 1100]] {
        assert!(EventStream::new(Drip(bad.to_vec(), 2000), true).next(std::time::Duration::ZERO).is_err());
    }
}

/// The redirect loops became one (from the code audit): up to five redirects are followed,
/// never from https down to http, and only for a GET or a HEAD.
#[test]
fn redirects() {
    let at = |u: &str| Url::parse(u).unwrap();
    let answer = |status: u16, location: Option<&str>| Response {
        status,
        headers: location.map(|l| ("Location".to_string(), l.to_string())).into_iter().collect(),
        body: Vec::new(),
    };
    let from = at("https://example.com/a/b");
    assert_eq!(next_hop(&from, &answer(302, Some("c"))).unwrap(), Some(at("https://example.com/a/c")));
    assert_eq!(next_hop(&from, &answer(308, Some("https://other.example/"))).unwrap(), Some(at("https://other.example/")));
    assert_eq!(next_hop(&from, &answer(302, None)).unwrap(), None);
    assert_eq!(next_hop(&from, &answer(200, Some("/x"))).unwrap(), None);
    assert!(matches!(next_hop(&from, &answer(301, Some("http://example.com/"))), Err(NetError::Tls(_))));
    let hop = b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\n\r\n".to_vec();
    let done = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec();
    let (base, h) = serve([vec![hop.clone(); 5], vec![done.clone()]].concat());
    assert_eq!(get(&base, 100).unwrap().body, b"ok", "five redirects");
    h.join().unwrap();
    let (base, h) = serve(vec![hop.clone(); 6]);
    assert!(matches!(get(&base, 100), Err(NetError::Protocol(m)) if m.contains("too many redirects")));
    h.join().unwrap();
    let (base, h) = serve(vec![hop.clone(); 6]);
    let mut got = Vec::new();
    assert!(get_into(&Request::get(&base, 100), |_| Ok(Some(&mut got))).is_err(), "downloads too");
    h.join().unwrap();
    // A POST's redirect is its answer.
    let (base, h) = serve(vec![hop]);
    assert_eq!(send(&Request::post(&base, "text/plain", b"x".to_vec(), 100)).unwrap().status, 302);
    h.join().unwrap();
}
