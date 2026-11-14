//! `http(s)://` addresses: parsed, resolved against a base, and percent-encoding.

use std::fmt::Write as _;

use crate::{Error, Result};

/// A parsed `http(s)://host[:port]/path?query` URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub https: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
}

pub fn default_port(https: bool) -> u16 {
    if https { 443 } else { 80 }
}

fn strip_prefix_ci<'a>(s: &'a str, p: &str) -> Option<&'a str> {
    s.get(..p.len()).filter(|h| h.eq_ignore_ascii_case(p)).and_then(|_| s.get(p.len()..))
}

/// A path with its spaces and control characters percent-encoded rather than sent raw (each
/// byte of a character's UTF-8: U+0085 is `%C2%85`).
fn encode_controls(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if c == ' ' || c.is_control() {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                let _ = write!(out, "%{b:02X}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl Url {
    pub fn parse(s: &str) -> Result<Url> {
        let s = s.trim();
        let bad = || Error::Protocol(format!("not a web address: {}", s.chars().take(80).collect::<String>()));
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
        Ok(Url { https, host: host.to_ascii_lowercase(), port, path: encode_controls(&path) })
    }

    /// A `Location` (or any reference) resolved against this URL.
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

    /// The whole address again.
    pub fn full(&self) -> String {
        format!("{}{}", self.origin(), self.path)
    }
}

/// Percent-encode everything but RFC 3986's unreserved characters.
pub use mailcraft_model::mailto::percent_encode as pct;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_encodings() {
        let u = Url::parse("https://Api.Example.com/jmap/session?x=1#frag").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("api.example.com", 443, "/jmap/session?x=1"));
        assert_eq!(u.join("/api/").unwrap().full(), "https://api.example.com/api/");
        assert_eq!(u.join("upload/").unwrap().full(), "https://api.example.com/jmap/upload/");
        assert_eq!(u.join("http://other:8080/x").unwrap().full(), "http://other:8080/x");
        for bad in ["ftp://x", "https://", "https://user@host/", "https://h:0/", "https://[::1/"] {
            assert!(Url::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(Url::parse("http://[::1]:8080").unwrap().origin(), "http://[::1]:8080");
        assert_eq!(pct("a b/é"), "a%20b%2F%C3%A9");
    }

    #[test]
    fn a_paths_controls_go_as_their_utf8() {
        let u = Url::parse("https://h.example/a b\u{1}\u{7f}\u{85}\u{9f}é?q=x\ty").unwrap();
        assert_eq!(u.path, "/a%20b%01%7F%C2%85%C2%9Fé?q=x%09y");
    }
}
