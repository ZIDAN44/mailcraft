//! A small HTTP/1.1 client (RFC 9112) for the few things mail needs from the web: pictures in a
//! message once the user asks for them, account autoconfiguration, OAuth token requests and JMAP.
//!
//! One request per connection (`Connection: close`), or several on a [`Conn`] kept open (NTLM
//! signs in a connection, not a request); TLS through the same rustls setup as IMAP,
//! `Content-Length` and `chunked` bodies, a handful of redirects. Responses are hostile input:
//! header and body sizes are capped and every number is checked.

use std::io::{Read, Write};

use crate::stream::{self, Stream};
use crate::{NetError, Result};

/// Longest header block accepted.
const MAX_HEAD: usize = 64 << 10;
const MAX_REDIRECTS: usize = 5;

/// A parsed `http(s)://host[:port]/path?query` URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub https: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
}

impl Url {
    pub fn parse(s: &str) -> Result<Url> {
        let s = s.trim();
        let bad = || NetError::Protocol(format!("not a web address: {}", s.chars().take(80).collect::<String>()));
        let (https, rest) = if let Some(r) = strip_prefix_ci(s, "https://") {
            (true, r)
        } else if let Some(r) = strip_prefix_ci(s, "http://") {
            (false, r)
        } else {
            return Err(bad());
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if authority.is_empty() || authority.contains('@') || authority.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(bad());
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (h, after) = v6.split_once(']').ok_or_else(bad)?;
            let port = match after.strip_prefix(':') {
                Some(p) => p.parse().map_err(|_| bad())?,
                None if after.is_empty() => default_port(https),
                None => return Err(bad()),
            };
            (h.to_string(), port)
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), p.parse().map_err(|_| bad())?),
                None => (authority.to_string(), default_port(https)),
            }
        };
        if host.is_empty() || port == 0 {
            return Err(bad());
        }
        let tail = tail.split('#').next().unwrap_or("");
        let path = if tail.is_empty() {
            "/".to_string()
        } else if tail.starts_with('?') {
            format!("/{tail}")
        } else {
            tail.to_string()
        };
        // Spaces and controls are percent-encoded rather than sent raw.
        let path: String = path
            .chars()
            .flat_map(|c| if c == ' ' || c.is_control() { format!("%{:02X}", u32::from(c) & 0xff).chars().collect::<Vec<_>>() } else { vec![c] })
            .collect();
        Ok(Url { https, host: host.to_ascii_lowercase(), port, path })
    }

    /// `Location` resolved against this URL.
    pub fn join(&self, location: &str) -> Result<Url> {
        let l = location.trim();
        if strip_prefix_ci(l, "http://").is_some() || strip_prefix_ci(l, "https://").is_some() {
            return Url::parse(l);
        }
        let origin = self.origin();
        if let Some(rest) = l.strip_prefix("//") {
            return Url::parse(&format!("{}://{rest}", if self.https { "https" } else { "http" }));
        }
        if l.starts_with('/') {
            return Url::parse(&format!("{origin}{l}"));
        }
        let dir = self.path.split('?').next().unwrap_or("/").rsplit_once('/').map_or("", |(d, _)| d);
        Url::parse(&format!("{origin}{dir}/{l}"))
    }

    pub fn origin(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if self.port == default_port(self.https) { format!("{scheme}://{host}") } else { format!("{scheme}://{host}:{}", self.port) }
    }
}

fn default_port(https: bool) -> u16 {
    if https { 443 } else { 80 }
}

fn strip_prefix_ci<'a>(s: &'a str, p: &str) -> Option<&'a str> {
    s.get(..p.len()).filter(|h| h.eq_ignore_ascii_case(p)).and_then(|_| s.get(p.len()..))
}

/// A response.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// A request.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Largest body accepted.
    pub max: usize,
}

impl Request {
    pub fn get(url: &str, max: usize) -> Request {
        Request { method: "GET".into(), url: url.into(), headers: Vec::new(), body: Vec::new(), max }
    }

    pub fn post(url: &str, content_type: &str, body: Vec<u8>, max: usize) -> Request {
        Request { method: "POST".into(), url: url.into(), headers: vec![("Content-Type".into(), content_type.into())], body, max }
    }

    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// Send a request; GET and HEAD follow redirects.
pub fn send(req: &Request) -> Result<Response> {
    let mut url = Url::parse(&req.url)?;
    for _ in 0..=MAX_REDIRECTS {
        let resp = once(req, &url)?;
        let follow = matches!(resp.status, 301 | 302 | 303 | 307 | 308) && (req.method == "GET" || req.method == "HEAD");
        match resp.header("Location").filter(|_| follow) {
            Some(loc) => {
                let next = url.join(loc)?;
                // Never from HTTPS down to plain HTTP.
                if url.https && !next.https {
                    return Err(NetError::Tls("redirected from https to http".into()));
                }
                url = next;
            }
            None => return Ok(resp),
        }
    }
    Err(NetError::Protocol("too many redirects".into()))
}

pub fn get(url: &str, max: usize) -> Result<Response> {
    send(&Request::get(url, max))
}

fn once(req: &Request, url: &Url) -> Result<Response> {
    let sock = stream::tcp(&url.host, url.port)?;
    let mut s = if url.https { stream::tls(sock, &url.host)? } else { Stream::Plain(sock) };
    exchange(&mut s, req, url)
}

/// Write the request and read the response on an open connection.
pub fn exchange<S: Read + Write>(s: &mut S, req: &Request, url: &Url) -> Result<Response> {
    write_request(s, req, url, false)?;
    read_response(s, req.max, req.method == "HEAD")
}

/// A connection kept open for several requests to one server (no redirects followed).
pub struct Conn {
    s: Stream,
    url: Url,
    /// The server said it closes the connection after the last answer.
    closing: bool,
}

impl Conn {
    pub fn open(url: &str) -> Result<Conn> {
        let url = Url::parse(url)?;
        let sock = stream::tcp(&url.host, url.port)?;
        let s = if url.https { stream::tls(sock, &url.host)? } else { Stream::Plain(sock) };
        Ok(Conn { s, url, closing: false })
    }

    /// Whether this connection reaches `url`'s server.
    pub fn reaches(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|u| u.https == self.url.https && u.host.eq_ignore_ascii_case(&self.url.host) && u.port == self.url.port)
    }

    /// Whether another request can go on it.
    pub fn usable(&self) -> bool {
        !self.closing
    }

    /// Send a request to this server and read its answer.
    pub fn send(&mut self, req: &Request) -> Result<Response> {
        if self.closing {
            return Err(NetError::Closed("the server closed the connection".into()));
        }
        let url = Url::parse(&req.url)?;
        if !self.reaches(&req.url) {
            return Err(NetError::Protocol("a request for another server".into()));
        }
        // Assume the worst until a full response has been read: after an error mid-exchange the
        // stream position is unknown.
        self.closing = true;
        write_request(&mut self.s, req, &url, true)?;
        let resp = read_response(&mut self.s, req.max, req.method == "HEAD")?;
        self.closing = resp.header("Connection").is_some_and(|c| c.eq_ignore_ascii_case("close"))
            || (resp.header("Content-Length").is_none() && resp.header("Transfer-Encoding").is_none() && resp.status != 204 && resp.status != 304);
        Ok(resp)
    }
}

fn write_request<S: Write>(s: &mut S, req: &Request, url: &Url, keep: bool) -> Result<()> {
    let host = if url.port == default_port(url.https) { url.host.clone() } else { format!("{}:{}", url.host, url.port) };
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: MailCraft/{}\r\nConnection: {}\r\n",
        req.method,
        url.path,
        env!("CARGO_PKG_VERSION"),
        if keep { "keep-alive" } else { "close" }
    );
    if !req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("Accept")) {
        head.push_str("Accept: */*\r\n");
    }
    for (k, v) in &req.headers {
        if k.contains(['\r', '\n', ':']) || v.contains(['\r', '\n']) {
            return Err(NetError::Protocol("bad request header".into()));
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if !req.body.is_empty() || req.method == "POST" {
        head.push_str(&format!("Content-Length: {}\r\n", req.body.len()));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())?;
    s.write_all(&req.body)?;
    s.flush()?;
    Ok(())
}

/// The status, headers and whatever of the body came with them.
#[allow(clippy::type_complexity)]
fn read_head<S: Read>(s: &mut S) -> Result<(u16, Vec<(String, String)>, Vec<u8>)> {
    let mut buf: Vec<u8> = Vec::with_capacity(8 << 10);
    let mut tmp = [0u8; 16 << 10];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        if buf.len() > MAX_HEAD {
            return Err(NetError::Protocol("response headers too long".into()));
        }
        let n = s.read(&mut tmp)?;
        if n == 0 {
            return Err(NetError::Closed("the server closed the connection before answering".into()));
        }
        buf.extend_from_slice(tmp.get(..n).unwrap_or_default());
    };
    let head = String::from_utf8_lossy(buf.get(..head_end).unwrap_or_default()).to_string();
    let mut rest: Vec<u8> = buf.get(head_end + 4..).map(<[u8]>::to_vec).unwrap_or_default();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or("");
    let status: u16 = parts
        .next()
        .and_then(|c| c.parse().ok())
        .filter(|c| (100..600).contains(c))
        .ok_or_else(|| NetError::Protocol(format!("bad status line: {}", status_line.chars().take(60).collect::<String>())))?;
    if !version.starts_with("HTTP/1.") {
        return Err(NetError::Protocol(format!("not HTTP/1.x: {version}")));
    }
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).take(200).collect();
    Ok((status, headers, std::mem::take(&mut rest)))
}

fn read_response<S: Read>(s: &mut S, max: usize, head_only: bool) -> Result<Response> {
    let mut tmp = [0u8; 16 << 10];
    let (status, headers, mut rest) = read_head(s)?;
    let resp = |body| Response { status, headers: headers.clone(), body };
    let get = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.to_ascii_lowercase());
    if head_only || status == 204 || status == 304 || (100..200).contains(&status) {
        return Ok(resp(Vec::new()));
    }
    let too_big = || NetError::Protocol(format!("the response is larger than {} KB", max / 1024));
    let mut fill = |rest: &mut Vec<u8>, want: usize| -> Result<bool> {
        while rest.len() < want {
            let n = s.read(&mut tmp)?;
            if n == 0 {
                return Ok(false);
            }
            rest.extend_from_slice(tmp.get(..n).unwrap_or_default());
            if rest.len() > max.saturating_add(MAX_HEAD) {
                return Err(too_big());
            }
        }
        Ok(true)
    };
    if get("transfer-encoding").is_some_and(|t| t.contains("chunked")) {
        let mut body = Vec::new();
        let mut pos = 0usize;
        loop {
            // A size line.
            let line_end = loop {
                if let Some(p) = rest.get(pos..).and_then(|r| r.windows(2).position(|w| w == b"\r\n")) {
                    break pos + p;
                }
                if rest.len() - pos > 1024 {
                    return Err(NetError::Protocol("bad chunk size line".into()));
                }
                let want = rest.len() + 1;
                if !fill(&mut rest, want)? {
                    return Err(NetError::Closed("the response ended early".into()));
                }
            };
            let size_text = String::from_utf8_lossy(rest.get(pos..line_end).unwrap_or_default()).to_string();
            let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| NetError::Protocol("bad chunk size".into()))?;
            pos = line_end + 2;
            if size == 0 {
                return Ok(resp(body));
            }
            if body.len().saturating_add(size) > max {
                return Err(too_big());
            }
            let want = pos.saturating_add(size).saturating_add(2);
            if !fill(&mut rest, want)? {
                return Err(NetError::Closed("the response ended early".into()));
            }
            body.extend_from_slice(rest.get(pos..pos + size).unwrap_or_default());
            pos += size + 2;
        }
    }
    if let Some(len) = get("content-length") {
        let len: usize = len.trim().parse().map_err(|_| NetError::Protocol("bad Content-Length".into()))?;
        if len > max {
            return Err(too_big());
        }
        if !fill(&mut rest, len)? {
            return Err(NetError::Closed("the response ended early".into()));
        }
        rest.truncate(len);
        return Ok(resp(rest));
    }
    // No length: the body runs to the end of the connection.
    fill(&mut rest, usize::MAX)?;
    if rest.len() > max {
        return Err(too_big());
    }
    Ok(resp(rest))
}

/// Longest event accepted from an event stream.
const MAX_EVENT: usize = 1 << 20;

/// One server-sent event (the type, `message` when the server gave none, and its data).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sse {
    pub event: String,
    pub data: String,
}

/// A response that stays open and carries server-sent events (`text/event-stream`, as JMAP
/// push uses), read a slice at a time so the caller can do other work in between.
pub struct EventStream<S> {
    s: S,
    /// Bytes as they came off the connection.
    raw: Vec<u8>,
    /// The event text decoded so far (chunk framing removed, `\r` dropped).
    text: Vec<u8>,
    chunked: bool,
    /// Bytes left in the current chunk, and whether its closing CRLF is still to come.
    left: usize,
    crlf: bool,
}

/// Open an event stream: a GET that must answer 200 with `text/event-stream`.
pub fn open_events(url: &str, headers: &[(&str, &str)]) -> Result<EventStream<Stream>> {
    let u = Url::parse(url)?;
    let sock = stream::tcp(&u.host, u.port)?;
    let mut s = if u.https { stream::tls(sock, &u.host)? } else { Stream::Plain(sock) };
    let mut req = Request::get(url, MAX_EVENT).header("Accept", "text/event-stream").header("Cache-Control", "no-cache");
    for (k, v) in headers {
        req = req.header(k, v);
    }
    write_request(&mut s, &req, &u, false)?;
    let (status, hs, rest) = read_head(&mut s)?;
    let get = |n: &str| hs.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.to_ascii_lowercase());
    match status {
        200 => {}
        401 | 403 => return Err(NetError::Auth("the server refused the user name or password".into())),
        s => return Err(NetError::Protocol(format!("the event stream answered HTTP {s}"))),
    }
    if !get("content-type").is_some_and(|t| t.contains("text/event-stream")) {
        return Err(NetError::Protocol("not an event stream".into()));
    }
    let chunked = get("transfer-encoding").is_some_and(|t| t.contains("chunked"));
    Ok(EventStream { s, raw: rest, text: Vec::new(), chunked, left: 0, crlf: false })
}

impl<S: Read + stream::Timeouts> EventStream<S> {
    pub fn new(s: S, chunked: bool) -> EventStream<S> {
        EventStream { s, raw: Vec::new(), text: Vec::new(), chunked, left: 0, crlf: false }
    }

    /// The next event, waiting at most `wait` (`None`: nothing yet).
    pub fn next(&mut self, wait: std::time::Duration) -> Result<Option<Sse>> {
        let mut tmp = [0u8; 8 << 10];
        loop {
            self.decode()?;
            if let Some(e) = self.take() {
                return Ok(Some(e));
            }
            if self.text.len() > MAX_EVENT || self.raw.len() > MAX_EVENT {
                return Err(NetError::Protocol("an event is too long".into()));
            }
            self.s.set_read_timeout(Some(wait.max(std::time::Duration::from_millis(1))))?;
            match self.s.read(&mut tmp) {
                Ok(0) => return Err(NetError::Closed("the event stream ended".into())),
                Ok(n) => self.raw.extend_from_slice(tmp.get(..n).unwrap_or_default()),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(None),
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Move what has arrived from `raw` to `text`, removing chunk framing.
    fn decode(&mut self) -> Result<()> {
        let take = |out: &mut Vec<u8>, bytes: &[u8]| out.extend(bytes.iter().copied().filter(|b| *b != b'\r'));
        if !self.chunked {
            let raw = std::mem::take(&mut self.raw);
            take(&mut self.text, &raw);
            return Ok(());
        }
        loop {
            if self.left > 0 {
                let n = self.left.min(self.raw.len());
                if n == 0 {
                    return Ok(());
                }
                let part: Vec<u8> = self.raw.drain(..n).collect();
                take(&mut self.text, &part);
                self.left -= n;
                self.crlf = self.left == 0;
                continue;
            }
            if self.crlf {
                if self.raw.len() < 2 {
                    return Ok(());
                }
                self.raw.drain(..2);
                self.crlf = false;
            }
            let Some(end) = self.raw.windows(2).position(|w| w == b"\r\n") else {
                if self.raw.len() > 1024 {
                    return Err(NetError::Protocol("bad chunk size line".into()));
                }
                return Ok(());
            };
            let line = String::from_utf8_lossy(self.raw.get(..end).unwrap_or_default()).to_string();
            let size =
                usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16).map_err(|_| NetError::Protocol("bad chunk size".into()))?;
            self.raw.drain(..end + 2);
            if size == 0 {
                return Err(NetError::Closed("the event stream ended".into()));
            }
            if size > MAX_EVENT {
                return Err(NetError::Protocol("an event is too long".into()));
            }
            self.left = size;
        }
    }

    /// One complete event from `text` (events end with a blank line; `:` lines are comments).
    fn take(&mut self) -> Option<Sse> {
        loop {
            let end = self.text.windows(2).position(|w| w == b"\n\n")?;
            let block = String::from_utf8_lossy(self.text.get(..end).unwrap_or_default()).to_string();
            self.text.drain(..end + 2);
            let mut event = String::new();
            let mut data: Vec<&str> = Vec::new();
            for line in block.lines() {
                let (field, value) = line.split_once(':').map_or((line, ""), |(f, v)| (f, v.strip_prefix(' ').unwrap_or(v)));
                match field {
                    "event" => event = value.to_string(),
                    "data" => data.push(value),
                    _ => {}
                }
            }
            if !data.is_empty() || !event.is_empty() {
                return Some(Sse { event: if event.is_empty() { "message".into() } else { event }, data: data.join("\n") });
            }
        }
    }
}

/// `application/x-www-form-urlencoded` (for OAuth token requests).
pub fn form(pairs: &[(&str, &str)]) -> Vec<u8> {
    pairs.iter().map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect::<Vec<_>>().join("&").into_bytes()
}

/// Percent-encode everything but the unreserved characters (RFC 3986 §2.3).
pub fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    use super::*;

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
}
