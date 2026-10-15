//! `http(s)://` addresses: parsed, resolved against a base, and percent-encoding.

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
        // Spaces and controls are percent-encoded rather than sent raw.
        let path: String = path
            .chars()
            .flat_map(|c| if c == ' ' || c.is_control() { format!("%{:02X}", u32::from(c) & 0xff).chars().collect::<Vec<_>>() } else { vec![c] })
            .collect();
        Ok(Url { https, host: host.to_ascii_lowercase(), port, path })
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

/// Base64 (RFC 4648), padded.
pub fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let idx = usize::try_from((n >> (18 - 6 * i)) & 63).unwrap_or(0);
                out.push(char::from(A.get(idx).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

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
        assert_eq!(base64(b"me:pw"), "bWU6cHc=");
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"ab"), "YWI=");
    }
}
