//! HTTP Digest sign-in (RFC 7616), for calendar servers that take no other: Baïkal's default
//! (sabre/dav), and DAViCal or Apache set up so. MD5 and SHA-256 (and their `-sess` forms), with
//! `qop=auth`; a nonce the server says is stale is answered again with the new one.

use md5::{Digest, Md5};

/// The longest `WWW-Authenticate` value read, and the most parameters kept from it: a server's
/// headers are hostile like the rest of what it sends.
const MAX_HEADER: usize = 4096;
const MAX_PARAMS: usize = 32;

/// A server's Digest challenge, and how many requests have answered its nonce (`nc`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub sha256: bool,
    /// `-sess`: the first hash includes the nonces.
    pub sess: bool,
    /// The server offers `qop=auth`.
    pub qop: bool,
    /// The nonce was refused only for being old: the same password, with the new nonce.
    pub stale: bool,
    pub count: u32,
}

/// The challenges in one `WWW-Authenticate` value: each scheme with its parameters (names lower
/// case, quoted values unquoted). `Basic realm="a", Digest realm="b", nonce="c"` is two.
pub(crate) fn challenges(value: &str) -> Vec<(String, Vec<(String, String)>)> {
    let s: Vec<char> = value.chars().take(MAX_HEADER).collect();
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut i = 0;
    while i < s.len() {
        while s.get(i).is_some_and(|c| *c == ',' || c.is_whitespace()) {
            i += 1;
        }
        let start = i;
        while s.get(i).is_some_and(|c| !matches!(c, ',' | '=') && !c.is_whitespace()) {
            i += 1;
        }
        let token: String = s.get(start..i).unwrap_or_default().iter().collect();
        if token.is_empty() {
            // A stray `=`: past it.
            i += 1;
            continue;
        }
        let mut j = i;
        while s.get(j).is_some_and(|c| c.is_whitespace()) {
            j += 1;
        }
        if s.get(j) != Some(&'=') {
            out.push((token.to_ascii_lowercase(), Vec::new()));
            continue;
        }
        i = j + 1;
        while s.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        let mut value = String::new();
        if s.get(i) == Some(&'"') {
            i += 1;
            while let Some(&c) = s.get(i) {
                i += 1;
                match c {
                    '"' => break,
                    '\\' => {
                        if let Some(&n) = s.get(i) {
                            value.push(n);
                            i += 1;
                        }
                    }
                    c => value.push(c),
                }
            }
        } else {
            while let Some(&c) = s.get(i).filter(|c| **c != ',') {
                value.push(c);
                i += 1;
            }
            value = value.trim().to_string();
        }
        if let Some((_, params)) = out.last_mut()
            && params.len() < MAX_PARAMS
        {
            params.push((token.to_ascii_lowercase(), value));
        }
    }
    out
}

/// The strongest Digest challenge in a response's `WWW-Authenticate` headers (SHA-256 before
/// MD5); `None` when there's none it can answer (no nonce, `auth-int` only, another algorithm).
pub(crate) fn challenge(headers: &[(String, String)]) -> Option<Challenge> {
    let mut found: Vec<Challenge> = Vec::new();
    for (_, v) in headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("WWW-Authenticate")) {
        for (scheme, params) in challenges(v) {
            if scheme != "digest" {
                continue;
            }
            let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
            let (sha256, sess) = match get("algorithm").map(|a| a.to_ascii_uppercase()).as_deref() {
                None | Some("MD5") => (false, false),
                Some("MD5-SESS") => (false, true),
                Some("SHA-256") => (true, false),
                Some("SHA-256-SESS") => (true, true),
                Some(_) => continue,
            };
            let qops = get("qop");
            let qop = qops.as_deref().is_some_and(|q| q.split(',').any(|x| x.trim().eq_ignore_ascii_case("auth")));
            // Only `auth-int` (a hash of every body): not answered.
            if qops.is_some() && !qop {
                continue;
            }
            let Some(nonce) = get("nonce").filter(|n| !n.is_empty()) else { continue };
            let stale = get("stale").is_some_and(|s| s.eq_ignore_ascii_case("true"));
            found.push(Challenge { realm: get("realm").unwrap_or_default(), nonce, opaque: get("opaque"), sha256, sess, qop, stale, count: 0 });
        }
    }
    found.iter().find(|c| c.sha256).or(found.first()).cloned()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn hash(sha256: bool, s: &str) -> String {
    if sha256 { hex(ring::digest::digest(&ring::digest::SHA256, s.as_bytes()).as_ref()) } else { hex(&Md5::digest(s.as_bytes())) }
}

/// A quoted-string's inside: `"` and `\` escaped.
fn quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

impl Challenge {
    /// The `Authorization` value for a request (`uri`: its path and query), counting it.
    pub(crate) fn authorization(&mut self, user: &str, password: &str, method: &str, uri: &str) -> String {
        let mut bytes = [0u8; 16];
        let cnonce = match ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes) {
            Ok(()) => hex(&bytes),
            // No randomness: still one a server hasn't seen with this nonce.
            Err(_) => hash(true, &format!("{}:{}:{}", self.nonce, self.count, uri)).chars().take(32).collect(),
        };
        self.answer(user, password, method, uri, &cnonce)
    }

    fn answer(&mut self, user: &str, password: &str, method: &str, uri: &str, cnonce: &str) -> String {
        self.count = self.count.saturating_add(1);
        let nc = format!("{:08x}", self.count);
        let h = |s: &str| hash(self.sha256, s);
        let mut ha1 = h(&format!("{user}:{}:{password}", self.realm));
        if self.sess {
            ha1 = h(&format!("{ha1}:{}:{cnonce}", self.nonce));
        }
        let ha2 = h(&format!("{method}:{uri}"));
        let response =
            if self.qop { h(&format!("{ha1}:{}:{nc}:{cnonce}:auth:{ha2}", self.nonce)) } else { h(&format!("{ha1}:{}:{ha2}", self.nonce)) };
        let algorithm = match (self.sha256, self.sess) {
            (false, false) => "MD5",
            (false, true) => "MD5-sess",
            (true, false) => "SHA-256",
            (true, true) => "SHA-256-sess",
        };
        let mut out = format!(
            "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", algorithm={algorithm}, response=\"{response}\"",
            quoted(user),
            quoted(&self.realm),
            quoted(&self.nonce),
            quoted(uri)
        );
        if self.qop {
            out.push_str(&format!(", qop=auth, nc={nc}, cnonce=\"{cnonce}\""));
        }
        if let Some(o) = &self.opaque {
            out.push_str(&format!(", opaque=\"{}\"", quoted(o)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(v: &str) -> Vec<(String, String)> {
        vec![("WWW-Authenticate".into(), v.into())]
    }

    /// RFC 7616 3.9.1's example, both algorithms.
    #[test]
    fn the_rfcs_example_is_answered() {
        let rfc = r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=SHA-256, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS", Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=MD5, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#;
        let mut c = challenge(&headers(rfc)).unwrap();
        assert!(c.sha256 && c.qop && !c.sess, "SHA-256 is taken first");
        let cnonce = "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ";
        let a = c.answer("Mufasa", "Circle of Life", "GET", "/dir/index.html", cnonce);
        assert!(a.contains("response=\"753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1\""), "{a}");
        assert!(a.contains("nc=00000001") && a.contains("algorithm=SHA-256") && a.contains("opaque=\"FQhe"), "{a}");
        let mut md5 = challenge(&headers(rfc.split(", Digest").nth(1).map(|r| format!("Digest{r}")).unwrap().as_str())).unwrap();
        assert!(!md5.sha256);
        let a = md5.answer("Mufasa", "Circle of Life", "GET", "/dir/index.html", cnonce);
        assert!(a.contains("response=\"8ca523f5e9506fed4657c9700eebdbec\""), "{a}");
        // The next request counts on.
        assert!(md5.answer("Mufasa", "Circle of Life", "GET", "/", cnonce).contains("nc=00000002"));
    }

    /// Baïkal (sabre/dav) sends Basic beside it, and a server may send the older form with no qop.
    #[test]
    fn its_challenge_is_found_among_others() {
        let both = headers(r#"Basic realm="BaikalDAV", Digest realm="BaikalDAV",qop="auth",nonce="6bd3e",opaque="9ab""#);
        let c = challenge(&both).unwrap();
        assert_eq!((c.realm.as_str(), c.nonce.as_str(), c.opaque.as_deref(), c.qop), ("BaikalDAV", "6bd3e", Some("9ab"), true));
        let old = challenge(&headers(r#"Digest realm="r", nonce="n""#)).unwrap();
        assert!(!old.qop);
        assert!(!old.clone().answer("u", "p", "PROPFIND", "/", "c").contains("qop"), "no qop, none said");
        assert!(challenge(&headers(r#"Digest realm="r", nonce="n", stale=TRUE"#)).unwrap().stale);
        assert!(challenge(&headers("Basic realm=\"x\"")).is_none());
        assert!(challenge(&headers(r#"Digest realm="r", nonce="n", qop="auth-int""#)).is_none(), "auth-int only isn't answered");
        assert!(challenge(&headers(r#"Digest realm="r", nonce="n", algorithm=SHA-512-256"#)).is_none());
        assert!(challenge(&headers(r#"Digest realm="r""#)).is_none(), "no nonce");
    }

    /// Whatever a server sends: no panic, and a user name with a quote stays one value.
    #[test]
    fn hostile_headers_are_survived() {
        for v in ["", "=", "Digest", "Digest ,,,", "Digest realm=\"", "Digest realm=\"\\", "Digest =x, nonce", "Digest a=\"b\\\"c\", nonce=n"] {
            let _ = challenge(&headers(v));
        }
        let long = format!("Digest nonce=\"n\", {}", "x=y, ".repeat(10_000));
        assert!(challenges(&long).first().is_some_and(|(_, p)| p.len() <= MAX_PARAMS));
        let mut c = challenge(&headers(r#"Digest realm="r", nonce="n", qop=auth"#)).unwrap();
        assert!(c.answer("a\"b\\c", "p", "GET", "/", "c").contains(r#"username="a\"b\\c""#));
    }
}
