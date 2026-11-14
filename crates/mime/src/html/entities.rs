//! Character references (`&amp;`, `&#38;`, `&#x26;`) made characters, one way for every pass over
//! a message's HTML that reads them: each had its own, with its own rules, so the same address
//! read differently in different passes (a picture's `&#38;` was read in its text, not in the
//! list of pictures to load, and the picture never came). XML read anywhere (an Office file here,
//! a server's answer in the network layer) takes its references the same way, as XML reads them
//! ([`Refs::Xml`]).

use std::borrow::Cow;

/// The longest name a reference is looked for with (`#1114111`, `#x10FFFF`, `thinsp`): its `;` is
/// looked for in the bytes after the `&` up to this many, not to the end of the text.
const NAME_MAX: usize = 11;

/// How [`decode_refs`] reads what isn't one of the references it makes a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refs {
    /// As a browser reads an attribute's value: `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` and
    /// numbered ones (`&#38;`, `&#x26;`; one that is no character `U+FFFD`). Other names stay as
    /// written.
    Value,
    /// For the words in a text: the five, and numbered ones that are a letter, digit or mark of
    /// punctuation; any other (`&nbsp;`, `&shy;`, `&zwj;`, a preview line's padding) a space, so
    /// that padding between words parts them.
    Words,
    /// As XML reads a text or a value (an Exchange answer, a mail provider's settings, an Office
    /// file): the five, and numbered ones that are a character. Any other stays as written.
    Xml,
}

/// XML's character references in `s` made characters ([`Refs::Xml`]): an attribute's value, or a
/// text an XML reader gives with its references in it.
pub fn decode_xml_refs(s: &str) -> Cow<'_, str> {
    decode_refs(s, Refs::Xml)
}

/// The character XML's reference `name` (between its `&` and `;`: `amp`, `#38`, `#x26`) stands
/// for; `None` for one that isn't XML's or names no character.
pub fn xml_ref_char(name: &str) -> Option<char> {
    character(name, Refs::Xml)
}

/// `s` with its character references made characters, as `refs` says. `s` itself when it has no
/// `&`. Never panics; each `;` is looked for in the few bytes a name takes.
pub(crate) fn decode_refs(s: &str, refs: Refs) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or(""));
        let after = rest.get(at + 1..).unwrap_or("");
        // Its `;`, looked for in the bytes (`;` is ASCII: where it is, a character ends).
        let len = after.as_bytes().iter().take(NAME_MAX + 1).position(|b| *b == b';');
        match len.and_then(|len| Some((character(after.get(..len)?, refs)?, len))) {
            Some((c, len)) => {
                out.push(c);
                rest = after.get(len + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// `s` written into HTML, as text or an attribute's value: its `&`, `<`, `>` and `"` as
/// references. The one copy for every place that writes HTML from text.
pub(crate) fn escaped(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The character the reference named `name` (between its `&` and `;`) stands for, as `refs`
/// reads it; `None` to leave it as written.
fn character(name: &str, refs: Refs) -> Option<char> {
    let known = match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    };
    if known.is_some() {
        return known;
    }
    if let Some(n) = name.strip_prefix('#') {
        let code = match n.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16),
            None => n.parse::<u32>(),
        }
        .ok();
        return match refs {
            Refs::Value => code.map(|c| char::from_u32(c).filter(|c| *c != '\0').unwrap_or('\u{fffd}')),
            Refs::Words => Some(code.and_then(char::from_u32).filter(|c| c.is_alphanumeric() || c.is_ascii_punctuation()).unwrap_or(' ')),
            Refs::Xml => code.and_then(char::from_u32),
        };
    }
    (refs == Refs::Words && !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric())).then_some(' ')
}

#[cfg(test)]
mod tests;
