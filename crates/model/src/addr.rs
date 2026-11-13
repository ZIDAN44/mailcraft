//! Email addresses: what the To/Cc boxes accept, and how names are shown.

use std::fmt;

use serde::{Deserialize, Serialize};

pub mod identity;

/// A display name and an address (`Zoe Park <zoe@example.org>`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EmailAddress {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
}

impl EmailAddress {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        EmailAddress { name: name.into(), email: email.into() }
    }

    /// The heap its name and address take ([`crate::summary::heap_room`]).
    pub fn heap_bytes(&self) -> usize {
        crate::summary::string_room(&self.name).saturating_add(crate::summary::string_room(&self.email))
    }

    /// The name if there is one, else the address.
    pub fn display(&self) -> &str {
        if self.name.trim().is_empty() { &self.email } else { self.name.trim() }
    }

    /// Up to two initials for the avatar (`Zoe Park` → `ZP`, `ops@x.org` → `O`).
    pub fn initials(&self) -> String {
        let src = self.display();
        let words: Vec<&str> = src.split(|c: char| c.is_whitespace() || c == '.' || c == '_' || c == '@').filter(|w| !w.is_empty()).collect();
        let pick = |w: &str| w.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
        match (self.name.trim().is_empty(), words.as_slice()) {
            (_, []) => "?".to_string(),
            (true, [first, ..]) => pick(first),
            (false, [only]) => pick(only),
            (false, [first, .., last]) => format!("{}{}", pick(first), pick(last)),
        }
    }

    /// A rough check: one `@`, something on both sides, a dot-free local part allowed, no spaces.
    pub fn is_valid(&self) -> bool {
        is_valid_email(&self.email)
    }
}

impl fmt::Display for EmailAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.name.trim().is_empty() {
            write!(f, "{}", self.email)
        } else if self.name.contains([',', ';', '"', '<', '>', '@']) {
            write!(f, "\"{}\" <{}>", self.name.replace('"', "'"), self.email)
        } else {
            write!(f, "{} <{}>", self.name.trim(), self.email)
        }
    }
}

pub fn is_valid_email(s: &str) -> bool {
    let s = s.trim();
    let Some((local, domain)) = s.rsplit_once('@') else { return false };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !s.chars().any(|c| c.is_whitespace() || c == '<' || c == '>' || c == ',' || c == ';')
        && !domain.contains('@')
}

/// Parse what was typed in a To/Cc/Bcc box: addresses separated by `,` or `;`, each either bare
/// (`zoe@example.org`) or with a name (`Zoe Park <zoe@example.org>`, `"Park, Zoe" <zoe@…>`).
/// Returns the invalid entries as the error.
pub fn parse_list(input: &str) -> Result<Vec<EmailAddress>, String> {
    let mut out = Vec::new();
    let mut bad = Vec::new();
    for part in split_outside_quotes(input) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match parse_one(part) {
            Some(a) => out.push(a),
            None => bad.push(part.to_string()),
        }
    }
    if bad.is_empty() { Ok(out) } else { Err(bad.join(", ")) }
}

/// The addresses of [`parse_list`]'s reading that are whole: for a draft, which may hold one
/// still being typed.
pub fn parse_valid(input: &str) -> Vec<EmailAddress> {
    split_outside_quotes(input).into_iter().filter_map(|p| parse_one(p.trim())).collect()
}

fn parse_one(s: &str) -> Option<EmailAddress> {
    if let (Some(lt), Some(gt)) = (s.rfind('<'), s.rfind('>')) {
        if gt < lt {
            return None;
        }
        // Nothing after it: with a quote left open (`Jo 6" Doe <jo@x.example>; bo@x.example`)
        // the next address is in the same piece, and was left out of the message unsaid.
        // Nor with a quote left open before it, which holds an address inside the name
        // (`Jo 6" Doe <jo@x.example>; Bo <bo@x.example>` was Bo alone, named all of that).
        // What ends an entry (`;`, `,`), and a quote around all of it, may follow.
        let after = s.get(gt + 1..).unwrap_or_default().trim_matches(|c: char| c.is_whitespace() || matches!(c, ';' | ',' | '"'));
        let open_quote = s.get(..lt).is_some_and(|name| name.matches('"').count() % 2 == 1) && !s.trim_start().starts_with('"');
        let inside = s.get(..lt).is_some_and(|name| name.contains('<') || name.contains('>'));
        if !after.is_empty() || (open_quote && inside) {
            return None;
        }
        let email = s.get(lt + 1..gt)?.trim().to_string();
        let name = s.get(..lt)?.trim().trim_matches('"').trim().to_string();
        let a = EmailAddress { name, email };
        return a.is_valid().then_some(a);
    }
    let a = EmailAddress { name: String::new(), email: s.trim().to_string() };
    a.is_valid().then_some(a)
}

fn split_outside_quotes(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut angle = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            // An address ends at its `>`: `Ann <a@b.org> Bob <c@d.org>` (two lines pasted) is two.
            '>' if !quoted && angle => {
                angle = false;
                parts.push(s.get(start..=i).unwrap_or_default());
                start = i + 1;
            }
            ',' | ';' if !quoted && !angle => {
                parts.push(s.get(start..i).unwrap_or_default());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(s.get(start..).unwrap_or_default());
    parts
}

/// Format a list for a header-like line (`Zoe Park <zoe@…>; ops@…`).
pub fn format_list(list: &[EmailAddress]) -> String {
    list.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
}

/// Recipients to put in a field being written: the list with a separator after it, so an
/// address typed next starts an entry of its own (empty for no one).
pub fn format_field(list: &[EmailAddress]) -> String {
    if list.is_empty() { String::new() } else { format!("{}; ", format_list(list)) }
}

/// Short form for the reading pane (`Zoe Park; ops@example.org`).
pub fn display_list(list: &[EmailAddress]) -> String {
    list.iter().map(EmailAddress::display).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mixed_lists() {
        let v = parse_list("Zoe Park <zoe@example.org>; ops@example.org, \"Park, Ann\" <ann@example.org>").unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], EmailAddress::new("Zoe Park", "zoe@example.org"));
        assert_eq!(v[1].email, "ops@example.org");
        assert_eq!(v[2].name, "Park, Ann");
        assert_eq!(parse_list("  ; ,").unwrap(), vec![]);
        assert_eq!(parse_list("good@x.org, not an address").unwrap_err(), "not an address");
        assert!(parse_list("a@b.c <").is_err());
        // Found in review: a quote left open kept the next address in the same piece, where
        // it was dropped without a word, and the message went without that person.
        let err = parse_list("Jo 6\" Doe <jo@x.org>; bob@y.org").unwrap_err();
        assert!(err.contains("bob@y.org"), "{err}");
        assert!(parse_list("Jo <jo@x.org> and more").is_err());
        assert_eq!(parse_list("Jo <jo@x.org> ; bob@y.org").unwrap().len(), 2);
        assert!(parse_valid("Jo 6\" Doe <jo@x.org>; bob@y.org").is_empty());
        // Found in a second review: nor when the next one has its own brackets (it was sent
        // to Bob alone, named all the rest); and what ends an entry may follow it.
        assert!(parse_list("Jo 6\" Doe <jo@x.org>; Bob <bob@y.org>").is_err());
        assert_eq!(parse_list("Jo 6\" Doe <jo@x.org>; ").unwrap(), [EmailAddress::new("Jo 6\" Doe", "jo@x.org")]);
        assert_eq!(parse_list("\"Zoe <zoe@x.org>\"").unwrap()[0].email, "zoe@x.org");
    }

    #[test]
    fn display_and_initials() {
        let a = EmailAddress::new("Zoe Park", "zoe@example.org");
        assert_eq!(a.to_string(), "Zoe Park <zoe@example.org>");
        assert_eq!(a.initials(), "ZP");
        assert_eq!(EmailAddress::new("", "ops@example.org").initials(), "O");
        assert_eq!(EmailAddress::new("Park, Ann", "a@x.y").to_string(), "\"Park, Ann\" <a@x.y>");
        assert_eq!(EmailAddress::new("", "").initials(), "?");
        assert_eq!(EmailAddress::new("Ünal", "u@x.y").initials(), "Ü");
    }

    #[test]
    fn validity() {
        assert!(is_valid_email("a@b"));
        assert!(!is_valid_email("a@"));
        assert!(!is_valid_email("@b"));
        assert!(!is_valid_email("a b@c"));
        assert!(!is_valid_email("a@.c"));
    }

    /// Two addresses with nothing but a space or a line end between them (pasted from a list)
    /// are two: the first was taken for the second's name and never got the message.
    #[test]
    fn addresses_run_together_are_each_read() {
        let got = parse_list("Ann <a@b.org>\nBob <c@d.org> e@f.org; \"Park, Zoe\" <z@x.org>").unwrap();
        let got: Vec<(&str, &str)> = got.iter().map(|a| (a.name.as_str(), a.email.as_str())).collect();
        assert_eq!(got, [("Ann", "a@b.org"), ("Bob", "c@d.org"), ("", "e@f.org"), ("Park, Zoe", "z@x.org")]);
        assert_eq!(parse_valid("\"Park, Zoe\" <z@x.org>, half@").len(), 1);
    }
}
