//! Email addresses: what the To/Cc boxes accept, and how names are shown.

use std::fmt;

use serde::{Deserialize, Serialize};

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

fn parse_one(s: &str) -> Option<EmailAddress> {
    if let (Some(lt), Some(gt)) = (s.rfind('<'), s.rfind('>')) {
        if gt < lt {
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
            '>' if !quoted => angle = false,
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
}
