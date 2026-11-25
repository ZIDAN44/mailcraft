//! Another program's automatic replies, read from its Sieve script (issue #243): a webmail's
//! filters (SOGo's `sogo` script, for one) can hold a `vacation` action (RFC 5230) that runs in
//! place of MailCraft's script, or with it when MailCraft's includes theirs. The server sends one
//! reply at a time: a script that would send two stops (Pigeonhole: "duplicate vacation action
//! not allowed"), and the mail is only kept, none of the rules run.
//!
//! Only what's needed to show it: the subject and text of the first `vacation` command, wherever
//! it sits. The script is the server's, so hostile: read token by token in one pass, never
//! recursing, keeping a capped amount of text.

/// The most of a reply's text or subject kept, in bytes.
const MAX_KEPT: usize = 64 << 10;

/// The first automatic reply in `script`: its subject (empty when it has none) and its text.
pub fn vacation_in(script: &str) -> Option<(String, String)> {
    let mut t = Tokens { s: script, at: 0 };
    // At the start of a command: `vacation` there is the action, elsewhere a word of another.
    let mut start = true;
    while let Some(tok) = t.next() {
        match tok {
            Tok::Word(w) if start && w.eq_ignore_ascii_case("vacation") => return arguments(&mut t),
            Tok::Punct(';' | '{' | '}') => start = true,
            _ => start = false,
        }
    }
    None
}

/// `vacation`'s arguments (RFC 5230 §4): its tags, then the reason.
fn arguments(t: &mut Tokens<'_>) -> Option<(String, String)> {
    let mut subject = String::new();
    loop {
        match t.next()? {
            Tok::Tag(tag) => match tag.as_str() {
                "subject" => match t.next()? {
                    Tok::Str(s) => subject = s,
                    _ => return None,
                },
                // A value each, not shown.
                "days" | "seconds" | "from" | "handle" => {
                    t.next()?;
                }
                "addresses" => skip_list(t)?,
                // `:mime`, and tags of extensions: no value.
                _ => {}
            },
            Tok::Str(reason) => return Some((subject, reason)),
            _ => return None,
        }
    }
}

/// A string, or a list of them in brackets.
fn skip_list(t: &mut Tokens<'_>) -> Option<()> {
    match t.next()? {
        Tok::Str(_) => Some(()),
        Tok::Punct('[') => loop {
            if let Tok::Punct(']') = t.next()? {
                return Some(());
            }
        },
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Tok {
    /// An identifier: a command, a test, a word.
    Word(String),
    /// `:name`, lowercased.
    Tag(String),
    /// A quoted string or a `text:` one, as it reads.
    Str(String),
    Number,
    Punct(char),
}

struct Tokens<'a> {
    s: &'a str,
    /// Always at a character's start: moved by whole characters or to just past an ASCII one.
    at: usize,
}

impl Tokens<'_> {
    fn rest(&self) -> &str {
        self.s.get(self.at..).unwrap_or("")
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at = self.at.saturating_add(c.len_utf8());
        Some(c)
    }

    /// Past the end of this line.
    fn skip_line(&mut self) {
        self.at = match self.rest().find('\n') {
            Some(i) => self.at.saturating_add(i + 1),
            None => self.s.len(),
        };
    }

    fn skip_space(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('#') => self.skip_line(),
                Some('/') if self.rest().starts_with("/*") => {
                    self.at = match self.rest().find("*/") {
                        Some(i) => self.at.saturating_add(i + 2),
                        None => self.s.len(),
                    };
                }
                _ => return,
            }
        }
    }

    fn word(&mut self) -> String {
        let mut w = String::new();
        while let Some(c) = self.peek().filter(|c| c.is_ascii_alphanumeric() || *c == '_') {
            keep(&mut w, c);
            self.bump();
        }
        w
    }

    /// A quoted string's text, after its opening quote: `\` keeps the next character as it is.
    fn quoted(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.bump() {
            match c {
                '"' => break,
                '\\' => {
                    if let Some(n) = self.bump() {
                        keep(&mut out, n);
                    }
                }
                c => keep(&mut out, c),
            }
        }
        out
    }

    /// A `text:` string, after `text:`: the lines up to one that's a lone dot, a dot that starts
    /// one doubled (RFC 5228 §2.4.2).
    fn multiline(&mut self) -> String {
        // The rest of the `text:` line: spaces, perhaps a comment.
        self.skip_line();
        let mut out = String::new();
        while self.at < self.s.len() {
            let end = self.rest().find('\n').map_or(self.s.len(), |i| self.at.saturating_add(i));
            let line = self.s.get(self.at..end).unwrap_or("").trim_end_matches('\r');
            self.at = end.saturating_add(1).min(self.s.len());
            if line == "." {
                break;
            }
            let line = if line.starts_with("..") { line.get(1..).unwrap_or("") } else { line };
            for c in line.chars() {
                keep(&mut out, c);
            }
            keep(&mut out, '\n');
        }
        if out.ends_with('\n') {
            out.pop();
        }
        out
    }

    fn next(&mut self) -> Option<Tok> {
        self.skip_space();
        let c = self.peek()?;
        Some(match c {
            '"' => {
                self.bump();
                Tok::Str(self.quoted())
            }
            ':' => {
                self.bump();
                Tok::Tag(self.word().to_ascii_lowercase())
            }
            c if c.is_ascii_digit() => {
                self.word();
                Tok::Number
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let w = self.word();
                if w.eq_ignore_ascii_case("text") && self.peek() == Some(':') {
                    self.bump();
                    Tok::Str(self.multiline())
                } else {
                    Tok::Word(w)
                }
            }
            c => {
                self.bump();
                Tok::Punct(c)
            }
        })
    }
}

/// `c` added, while what's kept is short enough.
fn keep(out: &mut String, c: char) {
    if out.len() < MAX_KEPT {
        out.push(c);
    }
}

#[cfg(test)]
mod tests;
