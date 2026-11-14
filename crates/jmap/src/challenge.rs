//! `WWW-Authenticate` (RFC 9110 §11.6.1): how a server asks to be signed in to, read so that a
//! server that takes only OAuth bearer tokens (RFC 6750 §3) is told from one that takes
//! passwords. The header comes from the network: only its first [`MAX_HEADER`] bytes are read,
//! at most [`MAX_CHALLENGES`] challenges of [`MAX_PARAMS`] parameters each are kept, and
//! reading it never fails. Whatever can't be read ends the list, and what came before stays.

/// The most of a header that's read, in bytes.
pub const MAX_HEADER: usize = 8 << 10;
/// The most challenges kept.
pub const MAX_CHALLENGES: usize = 16;
/// The most parameters kept for one challenge.
pub const MAX_PARAMS: usize = 32;

/// One way to sign in that a server offers. `scheme` is lowercased (`basic`, `bearer`…). The
/// parameters have lowercased names and unquoted values; some schemes send one `token68`
/// instead (`Negotiate YII…`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Challenge {
    pub scheme: String,
    pub token68: Option<String>,
    pub params: Vec<(String, String)>,
}

impl Challenge {
    /// A parameter's value, by its name in any case.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// The challenges in a response's headers: every `WWW-Authenticate`, read as one list.
pub fn in_headers(headers: &[(String, String)]) -> Vec<Challenge> {
    let all: Vec<&str> = headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case("www-authenticate")).map(|(_, v)| v.as_str()).collect();
    parse(&all.join(", "))
}

/// The challenge of `scheme` (lowercase) among `challenges`, when the server offers it.
pub fn offered<'c>(challenges: &'c [Challenge], scheme: &str) -> Option<&'c Challenge> {
    challenges.iter().find(|c| c.scheme == scheme)
}

/// The challenges in a `WWW-Authenticate` value. Several headers are one list (RFC 9110 §5.3):
/// join them with commas first.
pub fn parse(header: &str) -> Vec<Challenge> {
    let mut c = Cursor { s: within(header, MAX_HEADER), i: 0 };
    let mut out = Vec::new();
    while out.len() < MAX_CHALLENGES {
        // Empty list elements are allowed (RFC 9110 §5.6.1).
        c.skip(b" \t,");
        let Some(scheme) = c.token() else { break };
        let mut ch = Challenge { scheme: scheme.to_ascii_lowercase(), ..Challenge::default() };
        let more = if c.skip(b" \t") > 0 { c.rest_of(&mut ch) } else { c.at_end_or_comma() };
        out.push(ch);
        if !more {
            break;
        }
    }
    out
}

/// The longest start of `s` within `max` bytes.
fn within(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or_default()
}

/// `tchar` (RFC 9110 §5.6.2).
fn tchar(c: u8) -> bool {
    c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&c)
}

/// A `token68` character (RFC 9110 §11.2), its `=` padding aside.
fn token68_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || b"-._~+/".contains(&c)
}

/// A control character, which a quoted string can't hold (a tab aside).
fn ctl(c: u8) -> bool {
    (c < 0x20 && c != b'\t') || c == 0x7f
}

struct Cursor<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Cursor<'a> {
    fn at(&self, i: usize) -> Option<u8> {
        self.s.as_bytes().get(i).copied()
    }

    fn peek(&self) -> Option<u8> {
        self.at(self.i)
    }

    /// Past any of `set`; how many.
    fn skip(&mut self, set: &[u8]) -> usize {
        let start = self.i;
        while self.peek().is_some_and(|c| set.contains(&c)) {
            self.i += 1;
        }
        self.i - start
    }

    fn token(&mut self) -> Option<&'a str> {
        let start = self.i;
        while self.peek().is_some_and(tchar) {
            self.i += 1;
        }
        if self.i == start { None } else { self.s.get(start..self.i) }
    }

    /// The end, or a comma before what comes next (spaces aside): whether the list goes on.
    fn at_end_or_comma(&mut self) -> bool {
        self.skip(b" \t");
        matches!(self.peek(), None | Some(b','))
    }

    /// A token68, when that's all the challenge has: nothing but spaces after it, then a comma
    /// or the end. (`realm="x"` and `realm=x` are parameters.)
    fn token68(&mut self) -> Option<&'a str> {
        let start = self.i;
        let mut j = start;
        while self.at(j).is_some_and(token68_char) {
            j += 1;
        }
        if j == start {
            return None;
        }
        while self.at(j) == Some(b'=') {
            j += 1;
        }
        let mut k = j;
        while matches!(self.at(k), Some(b' ' | b'\t')) {
            k += 1;
        }
        if !matches!(self.at(k), None | Some(b',')) {
            return None;
        }
        self.i = j;
        self.s.get(start..j)
    }

    /// A quoted string (RFC 9110 §5.6.4), the cursor on its opening quote; `None` when it
    /// doesn't end or holds a control character.
    fn quoted(&mut self) -> Option<String> {
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            match c {
                b'"' => return Some(String::from_utf8_lossy(&out).into_owned()),
                b'\\' => {
                    let e = self.peek().filter(|e| !ctl(*e))?;
                    self.i += 1;
                    out.push(e);
                }
                c if ctl(c) => return None,
                c => out.push(c),
            }
        }
    }

    /// What follows a challenge's scheme and its space: a token68, or parameters until the
    /// next challenge. Whether the list goes on.
    fn rest_of(&mut self, ch: &mut Challenge) -> bool {
        if let Some(t) = self.token68() {
            ch.token68 = Some(t.to_string());
            return self.at_end_or_comma();
        }
        loop {
            let start = self.i;
            let Some(name) = self.token() else { return self.at_end_or_comma() };
            self.skip(b" \t");
            if self.peek() != Some(b'=') {
                // No `=`: not a parameter but the next challenge's scheme.
                self.i = start;
                return true;
            }
            self.i += 1;
            self.skip(b" \t");
            let value = if self.peek() == Some(b'"') { self.quoted() } else { self.token().map(str::to_string) };
            let Some(value) = value else { return false };
            if ch.params.len() < MAX_PARAMS {
                ch.params.push((name.to_ascii_lowercase(), value));
            }
            self.skip(b" \t");
            match self.peek() {
                None => return true,
                Some(b',') => {
                    self.skip(b" \t,");
                }
                Some(_) => return false,
            }
        }
    }
}

#[cfg(test)]
mod tests;
