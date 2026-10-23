//! `mailto:` links (RFC 6068): who a new message is to, its subject and its first words.
//!
//! A link clicked in a browser or another app starts MailCraft with it (MailCraft as the system's
//! mail app). Only `to`, `cc`, `bcc`, `subject` and `body` are taken; every other field is left
//! out, and above all `attach`/`attachment`: a web page must never be able to attach the user's
//! files to a message. `from`, `reply-to` and `in-reply-to` are left out too, so a link can't
//! choose the sender or where replies go. Everything here comes from somewhere else, so it's
//! capped, and the address and subject fields lose their line breaks and control characters.

/// Longest link read; the rest is left out (browsers cut links far shorter).
pub const MAX_LINK: usize = 1 << 20;

/// The parts of a new message a `mailto:` link supplies. Addresses keep the link's spelling and
/// are separated by a comma and a space.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct Mailto {
    /// The path's addresses, then those of every `to=` field.
    pub to: String,
    /// Every `cc=` field's addresses.
    pub cc: String,
    /// Every `bcc=` field's addresses.
    pub bcc: String,
    /// The first `subject=` field, on a single line.
    pub subject: String,
    /// Every `body=` field, one after another, separated by a line break.
    pub body: String,
}

/// True when `s`, leading blanks ignored, starts with the `mailto:` scheme (any letter case).
pub fn is_mailto(s: &str) -> bool {
    s.trim_start().get(..7).is_some_and(|p| p.eq_ignore_ascii_case("mailto:"))
}

/// Read a `mailto:` link, or `None` when `link` isn't one.
pub fn parse(link: &str) -> Option<Mailto> {
    let link = link.trim();
    if !is_mailto(link) {
        return None;
    }
    let mut cut = link.len().min(MAX_LINK);
    while !link.is_char_boundary(cut) {
        cut -= 1;
    }
    // Everything after the scheme, up to any `#fragment` (which never reaches the message).
    let after_scheme = link.get(7..cut).unwrap_or_default();
    let without_fragment = match after_scheme.find('#') {
        Some(hash) => after_scheme.get(..hash).unwrap_or_default(),
        None => after_scheme,
    };
    let (addresses, fields) = match without_fragment.find('?') {
        Some(q) => (without_fragment.get(..q).unwrap_or_default(), without_fragment.get(q + 1..).unwrap_or_default()),
        None => (without_fragment, ""),
    };
    let mut out = Mailto::default();
    let mut subject_seen = false;
    let mut bodies = 0usize;
    add_addresses(&mut out.to, &percent_decode(addresses));
    for field in fields.split('&') {
        if field.is_empty() {
            continue;
        }
        let (key, raw) = match field.find('=') {
            Some(eq) => (field.get(..eq).unwrap_or_default(), field.get(eq + 1..).unwrap_or_default()),
            None => (field, ""),
        };
        let key = percent_decode(key).to_ascii_lowercase();
        let value = percent_decode(raw);
        if key == "to" {
            add_addresses(&mut out.to, &value);
        } else if key == "cc" {
            add_addresses(&mut out.cc, &value);
        } else if key == "bcc" {
            add_addresses(&mut out.bcc, &value);
        } else if key == "subject" && !subject_seen {
            subject_seen = true;
            out.subject = flatten(&value);
        } else if key == "body" {
            if bodies > 0 {
                out.body.push('\n');
            }
            out.body.push_str(&body_text(&value));
            bodies += 1;
        }
        // Anything else (attach, from, reply-to, arbitrary headers) is ignored on purpose.
    }
    Some(out)
}

/// Undo `%XX` escapes (RFC 3986), reading the bytes as UTF-8; links to `file:` paths use it too.
/// An escape that isn't `%` plus two hex digits is copied through untouched, invalid UTF-8 turns
/// into U+FFFD, and `+` means a plus sign, not a space (RFC 6068 §5).
pub fn percent_decode(s: &str) -> String {
    let src = s.as_bytes();
    // The value of the hex digit at `at`, if that byte is one.
    let nibble = |at: usize| -> Option<u8> {
        let h = *src.get(at)?;
        match h {
            b'0'..=b'9' => Some(h - b'0'),
            _ if h.is_ascii_hexdigit() => Some((h | 0x20) - b'a' + 10),
            _ => None,
        }
    };
    let mut bytes = Vec::<u8>::with_capacity(s.len());
    let mut at = 0usize;
    while at < src.len() {
        let byte = src.get(at).copied().unwrap_or_default();
        if byte == b'%'
            && let (Some(high), Some(low)) = (nibble(at + 1), nibble(at + 2))
        {
            bytes.push((high << 4) | low);
            at += 3;
        } else {
            bytes.push(byte);
            at += 1;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Append each comma-separated address in `value` (flattened, blanks dropped) to `field`.
fn add_addresses(field: &mut String, value: &str) {
    for address in value.split(',').map(flatten) {
        if address.is_empty() {
            continue;
        }
        if !field.is_empty() {
            field.push_str(", ");
        }
        field.push_str(&address);
    }
}

/// Collapse `s` to one trimmed line: any control character counts as a blank, and runs of blanks
/// shrink to one space.
fn flatten(s: &str) -> String {
    let mut line = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        let blank = ch.is_whitespace() || ch.is_control();
        if blank {
            pending_space = !line.is_empty();
            continue;
        }
        if std::mem::take(&mut pending_space) {
            line.push(' ');
        }
        line.push(ch);
    }
    line
}

/// A `body=` value as message text: CRLF and lone CR both become `\n` (the RFC encodes breaks as
/// `%0D%0A`), tabs survive, every other control character is dropped.
fn body_text(s: &str) -> String {
    let mut text = String::with_capacity(s.len());
    let mut after_cr = false;
    for ch in s.chars() {
        let follows_cr = std::mem::replace(&mut after_cr, ch == '\r');
        if ch == '\r' || (ch == '\n' && !follows_cr) {
            text.push('\n');
        } else if ch == '\t' || !ch.is_control() {
            text.push(ch);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(link: &str) -> Mailto {
        parse(link).unwrap_or_else(|| Mailto { subject: "NOT A LINK".into(), ..Mailto::default() })
    }

    /// RFC 6068's own examples (§6.1, §6.2).
    #[test]
    fn the_rfcs_examples() {
        assert_eq!(m("mailto:chris@example.com").to, "chris@example.com");
        let x = m("mailto:infobot@example.com?subject=current-issue");
        assert_eq!((x.to.as_str(), x.subject.as_str()), ("infobot@example.com", "current-issue"));
        assert_eq!(m("mailto:infobot@example.com?body=send%20current-issue").body, "send current-issue");
        assert_eq!(m("mailto:infobot@example.com?body=send%20current-issue%0D%0Asend%20index").body, "send current-issue\nsend index");
        let x = m("mailto:?to=joe@example.com&cc=bob@example.com&body=hello");
        assert_eq!((x.to.as_str(), x.cc.as_str(), x.body.as_str()), ("joe@example.com", "bob@example.com", "hello"));
        assert_eq!(m("mailto:joe@example.com?cc=bob@example.com&body=hello").cc, "bob@example.com");
        assert_eq!(m("mailto:gorby%25kremvax@example.com").to, "gorby%kremvax@example.com");
        assert_eq!(m("mailto:Mike%26family@example.org").to, "Mike&family@example.org");
        assert_eq!(m("mailto:%22not%40me%22@example.org").to, "\"not@me\"@example.org");
        // UTF-8, percent-encoded (§6.2).
        assert_eq!(m("mailto:user@example.org?subject=caf%C3%A9").subject, "café");
        assert_eq!(m("mailto:?to=a@x.example&to=b@x.example").to, "a@x.example, b@x.example");
        assert_eq!(m("mailto:a@x.example,b@x.example?bcc=c@x.example").bcc, "c@x.example");
    }

    /// The scheme in any case; field names in any case; `+` stays; a fragment is left out.
    #[test]
    fn case_plus_and_fragments() {
        let x = m("  MAILTO:A@X.example?SUBJECT=1+1%3D2&Body=hi#frag");
        assert_eq!((x.to.as_str(), x.subject.as_str(), x.body.as_str()), ("A@X.example", "1+1=2", "hi"));
        assert!(is_mailto("MailTo:x") && !is_mailto("https://x.example") && !is_mailto("mail") && !is_mailto(""));
        assert_eq!(parse("https://x.example/?to=a@b"), None);
        assert_eq!(parse("C:\\mail\\x.eml"), None);
    }

    /// A page can't attach files, pick the sender or add headers through a line break.
    #[test]
    fn hostile_links_fill_in_nothing_else() {
        let x = m(
            "mailto:a@x.example?attach=C:%5CUsers%5Czoe%5Csecret.txt&attachment=/etc/passwd&from=boss@x.example&reply-to=evil@x.example&in-reply-to=%3Cx%3E",
        );
        assert_eq!(x, Mailto { to: "a@x.example".into(), ..Mailto::default() });
        let x = m("mailto:a@x.example%0D%0ABcc:%20all@x.example?subject=Hi%0D%0ABcc:%20b@x.example&cc=c@x.example%0Ad");
        assert!(!x.to.contains(['\r', '\n']) && !x.subject.contains(['\r', '\n']) && !x.cc.contains('\n'), "{x:?}");
        assert_eq!(x.subject, "Hi Bcc: b@x.example");
        assert_eq!(m("mailto:a@b?body=x%00y%1B%09z").body, "xy\tz");
    }

    /// Broken escapes, bytes that aren't UTF-8, odd separators and huge links don't stop it.
    #[test]
    fn junk_is_read_without_trouble() {
        for odd in ["mailto:", "mailto:%", "mailto:%G1%1", "mailto:?", "mailto:?&&=&==", "mailto:%FF%FE?subject=%C3", "mailto:é?ü=ö", "mailto:,,,"]
        {
            assert!(parse(odd).is_some(), "{odd}");
        }
        assert_eq!(m("mailto:%FFa@b").to, "\u{FFFD}a@b");
        assert_eq!(m("mailto:,, a@b ,").to, "a@b");
        let huge = format!("mailto:a@b?body={}", "é".repeat(MAX_LINK));
        assert!(m(&huge).body.len() < MAX_LINK);
        let subject = format!("mailto:a@b?subject={}", "%".repeat(10_000));
        assert_eq!(m(&subject).subject.len(), 10_000);
    }
}
