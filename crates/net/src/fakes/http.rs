//! What the HTTP fakes (DAV, EWS, Graph, JMAP) share: a request read off a connection, and an
//! answer written to it. Each fake adds its own headers and decides whether a connection stays
//! open.

use std::io::{BufRead, Write};

/// A request: its first line's method and target, its headers, its body.
#[derive(Default)]
pub(crate) struct Request {
    pub method: String,
    /// As the first line gives it: the path and its query.
    pub target: String,
    /// Each header's name and value, both trimmed, in the order they came.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// The first header of this name, its case aside.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Every header of this name, its case aside.
    pub fn headers_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.headers.iter().filter(move |(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// The next request on a connection: its first line, its headers up to the blank line, and as
/// much body as `Content-Length` says; `None` once the connection is closed before one came.
pub(crate) fn read_request(reader: &mut impl BufRead) -> Option<Request> {
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return None;
    }
    let mut words = first.split_whitespace();
    let mut r =
        Request { method: words.next().unwrap_or_default().to_string(), target: words.next().unwrap_or("/").to_string(), ..Request::default() };
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            r.headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let len = r.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0usize);
    let mut body = vec![0u8; len];
    let _ = reader.read_exact(&mut body);
    r.body = body;
    Some(r)
}

/// An answer: its status line (`HTTP/1.1` and `status`, a code and its text), its length, the
/// headers given, and its body.
pub(crate) fn respond(s: &mut impl Write, status: &str, headers: &[(&str, String)], body: &[u8]) {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_read_with_its_headers_in_any_case_and_its_body() {
        let wire = b"PUT /dav/x.ics HTTP/1.1\r\nif-match: \"e1\"\r\nPrefer: a\r\nprefer: b\r\nCONTENT-LENGTH: 4\r\n\r\nbodyMORE";
        let r = read_request(&mut &wire[..]).unwrap();
        assert_eq!((r.method.as_str(), r.target.as_str()), ("PUT", "/dav/x.ics"));
        assert_eq!(r.header("If-Match"), Some("\"e1\""));
        assert_eq!(r.headers_named("PREFER").collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(r.body, b"body");
        assert!(read_request(&mut &b""[..]).is_none(), "a closed connection has no request");
    }

    #[test]
    fn an_answer_says_its_length() {
        let mut out = Vec::new();
        respond(&mut out, "207 Multi-Status", &[("ETag", "\"e2\"".into())], b"<x/>");
        assert_eq!(out, b"HTTP/1.1 207 Multi-Status\r\nContent-Length: 4\r\nETag: \"e2\"\r\n\r\n<x/>");
    }
}
