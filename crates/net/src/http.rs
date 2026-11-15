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

mod body;
mod streamed;
pub use streamed::{exchange_into, get_into};

/// Longest header block accepted.
const MAX_HEAD: usize = 64 << 10;
pub(crate) const MAX_REDIRECTS: usize = 5;

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
        Ok(Url { https, host: host.to_ascii_lowercase(), port, path: spaces_escaped(path) })
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

/// A path with its spaces and controls percent-encoded rather than sent raw (each of their UTF-8
/// bytes: `U+0085` is `%C2%85`); the path itself, untouched, when it has none.
fn spaces_escaped(path: String) -> String {
    let raw = |c: char| c == ' ' || c.is_control();
    if !path.contains(raw) {
        return path;
    }
    let mut out = String::with_capacity(path.len() + 16);
    let mut utf8 = [0u8; 4];
    for c in path.chars() {
        if raw(c) {
            // Neither is unreserved: every byte comes out as `%XX`.
            out.push_str(&pct(c.encode_utf8(&mut utf8)));
        } else {
            out.push(c);
        }
    }
    out
}

/// Whether a [`Url`]'s host is this computer (`localhost`, `127.0.0.1`, `::1`: an IPv6 host is
/// kept without its brackets): the one place plain http is let through, to a test server.
pub fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
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
    send_to(req, false)
}

/// GET from the internet only ([`stream::tcp_public`]), every redirect included: what a message
/// asks for (its pictures) never reaches this computer or its network.
pub fn get_public(url: &str, max: usize) -> Result<Response> {
    send_to(&Request::get(url, max), true)
}

/// Send a request to the internet only, as [`get_public`]: an address a message or a certificate
/// names never reaches this computer or its network.
pub fn send_public(req: &Request) -> Result<Response> {
    send_to(req, true)
}

/// Whether hosts `a` and `b` are one site's: the same, one under the other, or two under one
/// name that isn't a registry's (`api.example.com` and `www.example.com`; not `a.co.uk` and
/// `b.co.uk`, whose shared name is taken for a registry's: two letters after three at most).
pub(crate) fn same_site(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim_end_matches('.').to_ascii_lowercase(), b.trim_end_matches('.').to_ascii_lowercase());
    let under = |host: &str, parent: &str| host.strip_suffix(parent).is_some_and(|h| h.ends_with('.'));
    // An address is itself and no other's neighbour.
    if a == b {
        return true;
    }
    if a.parse::<std::net::IpAddr>().is_ok() || b.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    // One under the other, when the upper one is someone's (not `uk`, `co.uk`, `github.io`).
    if (under(&a, &b) && !registry(&b)) || (under(&b, &a) && !registry(&a)) {
        return true;
    }
    let parent = |host: &str| host.split_once('.').map(|(_, p)| p.to_string());
    match (parent(&a), parent(&b)) {
        (Some(pa), Some(pb)) if pa == pb => !registry(&pa),
        _ => false,
    }
}

/// Whether `name` is one anyone can have a name under, as far as can be told without the list
/// of them: a top-level name, a country's second level (`co.uk`, `com.au`: two letters after
/// three at most), or one of the widely used ones that give out names to all comers.
pub(crate) fn registry(name: &str) -> bool {
    const SHARED: [&str; 16] = [
        "github.io",
        "gitlab.io",
        "pages.dev",
        "workers.dev",
        "herokuapp.com",
        "azurewebsites.net",
        "cloudfront.net",
        "amazonaws.com",
        "duckdns.org",
        "dyndns.org",
        "no-ip.org",
        "netlify.app",
        "vercel.app",
        "web.app",
        "firebaseapp.com",
        "blogspot.com",
    ];
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    match name.rsplit_once('.') {
        None => true,
        Some((label, tld)) => (tld.len() == 2 && label.len() <= 3 && !label.contains('.')) || SHARED.contains(&name.as_str()),
    }
}

/// `req` as it goes on to `next` after a redirect from `from`: without its sign-in when that's
/// another site (the password or token went along to wherever a redirect led).
pub(crate) fn redirected(req: &Request, from: &Url, next: &Url) -> Option<Request> {
    if same_site(&from.host, &next.host) || !req.headers.iter().any(|(k, _)| secret_header(k)) {
        return None;
    }
    let mut bare = req.clone();
    bare.headers.retain(|(k, _)| !secret_header(k));
    Some(bare)
}

fn secret_header(name: &str) -> bool {
    ["authorization", "cookie", "proxy-authorization"].iter().any(|h| name.eq_ignore_ascii_case(h))
}

/// Where a redirect sends a request, if `resp` is one.
fn redirect(resp: &Response) -> Option<&str> {
    resp.header("Location").filter(|_| matches!(resp.status, 301 | 302 | 303 | 307 | 308))
}

/// Where the redirect `resp` (an answer from `url`) leads: `None` when it isn't one. Never from
/// HTTPS down to plain HTTP.
pub(crate) fn next_hop(url: &Url, resp: &Response) -> Result<Option<Url>> {
    let Some(loc) = redirect(resp) else { return Ok(None) };
    let next = url.join(loc)?;
    if url.https && !next.https {
        return Err(NetError::Tls("redirected from https to http".into()));
    }
    Ok(Some(next))
}

/// `req` sent by `once` (a connection of its own each time), then, for a GET or HEAD, on to
/// where up to [`MAX_REDIRECTS`] redirects lead, without its sign-in once that's another site.
fn follow(req: &Request, mut once: impl FnMut(&Request, &Url) -> Result<Response>) -> Result<Response> {
    let mut url = Url::parse(&req.url)?;
    let mut bare: Option<Request> = None;
    let follows = req.method == "GET" || req.method == "HEAD";
    for _ in 0..=MAX_REDIRECTS {
        let now = bare.as_ref().unwrap_or(req);
        let resp = once(now, &url)?;
        let Some(next) = (if follows { next_hop(&url, &resp)? } else { None }) else { return Ok(resp) };
        if let Some(without) = redirected(now, &url, &next) {
            bare = Some(without);
        }
        url = next;
    }
    Err(NetError::Protocol("too many redirects".into()))
}

fn send_to(req: &Request, public_only: bool) -> Result<Response> {
    follow(req, |req, url| exchange(&mut connect(url, public_only)?, req, url))
}

pub fn get(url: &str, max: usize) -> Result<Response> {
    send(&Request::get(url, max))
}

/// A connection to `url`'s server: to the internet only when `public_only`
/// ([`stream::tcp_public`]), and through TLS for `https`.
fn connect(url: &Url, public_only: bool) -> Result<Stream> {
    let sock = if public_only { stream::tcp_public(&url.host, url.port)? } else { stream::tcp(&url.host, url.port)? };
    if url.https { stream::tls(sock, &url.host) } else { Ok(Stream::Plain(sock)) }
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
        Ok(Conn { s: connect(&url, false)?, url, closing: false })
    }

    /// Whether this connection reaches `url`'s server.
    pub fn reaches(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|u| self.reaches_url(&u))
    }

    fn reaches_url(&self, u: &Url) -> bool {
        u.https == self.url.https && u.host.eq_ignore_ascii_case(&self.url.host) && u.port == self.url.port
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
        if !self.reaches_url(&url) {
            return Err(NetError::Protocol("a request for another server".into()));
        }
        // Assume the worst until a full response has been read: after an error mid-exchange the
        // stream position is unknown.
        self.closing = true;
        write_request(&mut self.s, req, &url, true)?;
        let (resp, cut) = read_answer(&mut self.s, req.max, req.method == "HEAD")?;
        self.closing = cut
            || resp.header("Connection").is_some_and(|c| c.eq_ignore_ascii_case("close"))
            || (resp.header("Content-Length").is_none() && resp.header("Transfer-Encoding").is_none() && resp.status != 204 && resp.status != 304);
        Ok(resp)
    }
}

fn write_request<S: Write>(s: &mut S, req: &Request, url: &Url, keep: bool) -> Result<()> {
    let host = if url.port == default_port(url.https) { url.host.clone() } else { format!("{}:{}", url.host, url.port) };
    let mut head = format!("{} {} HTTP/1.1\r\nHost: {host}\r\nConnection: {}\r\n", req.method, url.path, if keep { "keep-alive" } else { "close" });
    // MailCraft's own name, unless the request gives another (an account's own User-Agent).
    if !req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("User-Agent")) {
        head.push_str(&format!("User-Agent: MailCraft/{}\r\n", env!("CARGO_PKG_VERSION")));
    }
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
    let rest: Vec<u8> = buf.get(head_end + 4..).map(<[u8]>::to_vec).unwrap_or_default();
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
    Ok((status, headers, rest))
}

/// The answer and its body (at most `max` bytes), read whole: as a download is
/// ([`body::copy_body`]), into memory.
fn read_response<S: Read>(s: &mut S, max: usize, head_only: bool) -> Result<Response> {
    read_answer(s, max, head_only).map(|(resp, _)| resp)
}

/// [`read_response`], and whether the connection can carry no other answer though this one's
/// body came whole ([`body::Ahead::cut`]).
fn read_answer<S: Read>(s: &mut S, max: usize, head_only: bool) -> Result<(Response, bool)> {
    let (status, headers, rest) = read_head(s)?;
    let mut resp = Response { status, headers, body: Vec::new() };
    if head_only || status == 204 || status == 304 || (100..200).contains(&status) {
        return Ok((resp, false));
    }
    let mut body = Vec::new();
    let mut ahead = body::Ahead::new(s, rest);
    body::copy_body(&mut ahead, &resp, u64::try_from(max).unwrap_or(u64::MAX), &mut body, "response")?;
    resp.body = body;
    Ok((resp, ahead.cut()))
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
    let mut s = connect(&u, false)?;
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
    Ok(EventStream { raw: rest, ..EventStream::new(s, chunked) })
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
                match self.raw.get(..2) {
                    None => return Ok(()),
                    Some(b"\r\n") => {}
                    Some(_) => return Err(NetError::Protocol("bad chunk end".into())),
                }
                self.raw.drain(..2);
                self.crlf = false;
            }
            let Some(end) = body::line_end(&self.raw)? else { return Ok(()) };
            let size = body::chunk_size(self.raw.get(..end).unwrap_or_default())?;
            self.raw.drain(..end + 2);
            if size == 0 {
                return Err(NetError::Closed("the event stream ended".into()));
            }
            let Some(size) = usize::try_from(size).ok().filter(|s| *s <= MAX_EVENT) else {
                return Err(NetError::Protocol("an event is too long".into()));
            };
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
pub use mailcraft_jmap::pct;

#[cfg(test)]
pub(crate) mod tests;
