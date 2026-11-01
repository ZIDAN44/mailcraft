//! A property's value as the rest of the import reads it.
//!
//! `outlook-pst`'s text values have no public constructors, so code that reads an item's
//! properties takes a closure giving a [`Val`] for a property id: the reader hands it the file's
//! values, and tests hand it their own.

use std::borrow::Cow;

use outlook_pst::ltp::prop_context::PropertyValue;

use super::mime;

/// A property's value, by what it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Val<'a> {
    Text(Cow<'a, str>),
    Bin(&'a [u8]),
    Int(i64),
    Bool(bool),
    /// A FILETIME: 100 ns since 1601.
    Time(i64),
    /// Several texts (a multi-valued string: categories).
    Texts(Vec<String>),
    Object,
    /// Present, of a type the import does not read (a 16- or 64-bit integer, a float, a GUID,
    /// an error code...): it still hides a fallback property.
    Other,
}

impl Val<'_> {
    /// The text, when it is text.
    pub(crate) fn into_text(self) -> Option<String> {
        match self {
            Val::Text(t) => Some(t.into_owned()),
            _ => None,
        }
    }

    /// A time as Unix seconds.
    pub(crate) fn time(&self) -> Option<i64> {
        match self {
            Val::Time(t) if *t > 0 => Some(t / 10_000_000 - 11_644_473_600),
            _ => None,
        }
    }
}

/// A value read from the file, its 8-bit text in `code_page`.
pub(super) fn val(v: &PropertyValue, code_page: u32) -> Val<'_> {
    match v {
        PropertyValue::Unicode(u) => Val::Text(Cow::Owned(String::from_utf16_lossy(u.buffer()).trim_end_matches('\0').to_string())),
        PropertyValue::String8(s) => Val::Text(Cow::Owned(decode(s.buffer(), code_page, false).trim_end_matches('\0').to_string())),
        PropertyValue::Binary(b) => Val::Bin(b.buffer()),
        PropertyValue::Integer32(n) => Val::Int(i64::from(*n)),
        PropertyValue::Boolean(b) => Val::Bool(*b),
        PropertyValue::Time(t) => Val::Time(*t),
        PropertyValue::MultipleUnicode(list) => {
            Val::Texts(list.iter().map(|u| String::from_utf16_lossy(u.buffer()).trim_end_matches('\0').to_string()).collect())
        }
        PropertyValue::MultipleString8(list) => {
            Val::Texts(list.iter().map(|s| decode(s.buffer(), code_page, false).trim_end_matches('\0').to_string()).collect())
        }
        PropertyValue::Object(_) => Val::Object,
        _ => Val::Other,
    }
}

pub(crate) fn text_of(v: &PropertyValue) -> Option<String> {
    val(v, 0).into_text()
}

/// The charset an HTML body declares counts only in its first this many bytes (where a `<meta>`
/// tag is).
const DECLARED_WITHIN: usize = 4096;
/// The longest `<meta>` tag read for its charset.
const MAX_META_TAG: usize = 1024;

/// A code page property (`PidTagMessageCodepage`, `PidTagInternetCodepage`); 0 when there is
/// none.
pub(super) fn code_page(v: Option<&PropertyValue>) -> u32 {
    match v {
        Some(PropertyValue::Integer32(n)) => u32::try_from(*n).unwrap_or(0),
        _ => 0,
    }
}

/// The provider of a one-off entry id: an address written into the item itself.
const ONE_OFF: [u8; 16] = [0x81, 0x2B, 0x1F, 0xA4, 0xBE, 0xA3, 0x10, 0x19, 0x9D, 0x6E, 0x00, 0xDD, 0x01, 0x0F, 0x54, 0x02];
/// Longest one-off entry id read.
const MAX_ONE_OFF: usize = 1024;

/// The display name and email address in a one-off entry id (MS-OXCDATA 2.2.5.1): four flag
/// bytes, the one-off provider, a version and flags (0x8000: its strings are UTF-16), then the
/// display name, the address type and the address, each ending in a NUL. `None` unless it is one
/// and the address has an `@` (an Exchange address is a directory path).
pub(super) fn one_off_address(b: &[u8]) -> Option<(String, String)> {
    if b.len() > MAX_ONE_OFF || b.get(4..20)? != ONE_OFF {
        return None;
    }
    let unicode = u16::from_le_bytes([*b.get(22)?, *b.get(23)?]) & 0x8000 != 0;
    let mut rest = b.get(24..)?;
    let mut next = || -> String {
        if unicode {
            let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|u| *u != 0).collect();
            rest = rest.get((units.len() + 1) * 2..).unwrap_or_default();
            String::from_utf16_lossy(&units)
        } else {
            let end = rest.iter().position(|c| *c == 0).unwrap_or(rest.len());
            let s = decode(rest.get(..end).unwrap_or_default(), 0, false);
            rest = rest.get(end + 1..).unwrap_or_default();
            s
        }
    };
    let name = next();
    let _kind = next();
    let address = next().trim().to_string();
    is_address(&address).then(|| (clean_name(&name), address))
}

/// Whether text from the file is an address a header line can carry: with an `@` (an Exchange
/// address is a directory path), no control characters (line breaks), at most
/// [`super::MAX_ADDRESS`] bytes. Anything else is kept as the file has it (a quoted local part's
/// spaces, say).
pub(super) fn is_address(s: &str) -> bool {
    s.len() <= super::MAX_ADDRESS && s.contains('@') && !s.chars().any(char::is_control)
}

/// A name from the file as a header line can carry it: control characters and line breaks as
/// spaces (runs of spaces one), at most [`super::MAX_NAME`] characters.
pub(super) fn clean_name(s: &str) -> String {
    let name: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(super::MAX_NAME).collect();
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Bytes from the file as text in `code_page`: an 8-bit string in the message's code page, or
/// the HTML body (`html`) in its internet code page.
///
/// An HTML body's byte-order mark says what its bytes are (a string's bytes are its page's, even
/// when they start like one). An HTML body in UTF-16 is read as UTF-16; a string labelled UTF-16
/// or UTF-7 is read as with no page. With no page, US-ASCII, UTF-8 or one this doesn't know, the
/// bytes are UTF-8 when they're valid, else Windows-1252. Otherwise valid UTF-8 that isn't plain
/// ASCII is UTF-8 (Outlook labels some UTF-8 mail with the sender's code page) unless an HTML
/// body declares another 8-bit charset in a `<meta>` tag near its top; anything else is in the
/// page.
pub(super) fn decode(bytes: &[u8], code_page: u32, html: bool) -> String {
    read(bytes, code_page, html).0
}

/// [`decode`], and whether text was lost to a page MailCraft can't read (ISO-2022-KR: only its
/// ASCII is kept, so a shift or a byte beyond ASCII is lost).
pub(super) fn read(bytes: &[u8], code_page: u32, html: bool) -> (String, bool) {
    // An 8-bit string is never UTF-16 or UTF-7, whatever its message's page: it reads as with
    // no page.
    let code_page = if html || mime::byte_code_page(code_page) { code_page } else { 0 };
    if html {
        if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
            return (String::from_utf8_lossy(rest).into_owned(), false);
        }
        if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
            return (mime::decode_code_page(rest, 1200), false);
        }
        if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
            return (mime::decode_code_page(rest, 1201), false);
        }
    }
    if code_page == 65001 {
        return (String::from_utf8_lossy(bytes).into_owned(), false);
    }
    // UTF-16's ASCII is bytes with NULs, and much of the rest happens to be valid UTF-8.
    if mime::utf16_code_page(code_page) {
        let text = mime::decode_code_page(bytes, code_page);
        return (text.strip_prefix('\u{FEFF}').map(str::to_string).unwrap_or(text), false);
    }
    let utf8 = std::str::from_utf8(bytes).ok();
    if matches!(code_page, 0 | 20127) || !mime::known_code_page(code_page) && !mime::unsupported_code_page(code_page) {
        return match utf8 {
            Some(s) => (s.to_string(), false),
            None => (mime::decode_code_page(bytes, 1252), false),
        };
    }
    if let Some(s) = utf8
        && !s.is_ascii()
        && (!html || !declares_other_charset(bytes))
    {
        return (s.to_string(), false);
    }
    let lost = mime::unsupported_code_page(code_page) && bytes.iter().any(|c| !c.is_ascii() || *c == 0x0E);
    (mime::decode_code_page(bytes, code_page), lost)
}

/// Whether an HTML body declares, in a `<meta>` tag that starts in its first 4 KB, a charset
/// that says what bytes beyond ASCII are and isn't UTF-8 (`charset=…`, quoted or not, any case).
/// `charset=` elsewhere (a comment, quoted text, a link) isn't a declaration, and neither is one
/// that can't be 8-bit text's: US-ASCII, or the "unicode" (UTF-16) Word writes in its HTML.
fn declares_other_charset(html: &[u8]) -> bool {
    let head = html.get(..html.len().min(DECLARED_WITHIN.saturating_add(4))).unwrap_or_default();
    let mut from = 0;
    while let Some(at) = head.get(from..).and_then(|h| h.windows(5).position(|w| w.eq_ignore_ascii_case(b"<meta"))) {
        let start = from.saturating_add(at);
        if start >= DECLARED_WITHIN {
            break;
        }
        from = start.saturating_add(5);
        // The whole tag, read past 4 KB if it crosses it; one that doesn't end soon doesn't count.
        let rest = html.get(start..).unwrap_or_default();
        let rest = rest.get(..rest.len().min(MAX_META_TAG)).unwrap_or_default();
        let Some(end) = rest.iter().position(|c| *c == b'>') else { continue };
        if let Some(charset) = rest.get(..end).and_then(charset_in) {
            let c = charset.to_ascii_lowercase();
            let says_nothing =
                c.starts_with(b"utf") || c.starts_with(b"unicode") || c.starts_with(b"ucs") || matches!(c.as_slice(), b"us-ascii" | b"ascii");
            if !says_nothing {
                return true;
            }
        }
    }
    false
}

/// The value of `charset=` in a tag.
fn charset_in(tag: &[u8]) -> Option<&[u8]> {
    let mut from = 0;
    while let Some(at) = tag.get(from..)?.windows(7).position(|w| w.eq_ignore_ascii_case(b"charset")) {
        let mut i = from.saturating_add(at).saturating_add(7);
        from = i;
        let skip = |i: &mut usize, set: &[u8]| {
            while tag.get(*i).is_some_and(|c| set.contains(c)) {
                *i += 1;
            }
        };
        skip(&mut i, b" \t\r\n");
        if tag.get(i) != Some(&b'=') {
            continue;
        }
        i += 1;
        skip(&mut i, b" \t\r\n\"'");
        let start = i;
        while tag.get(i).is_some_and(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(c)) {
            i += 1;
        }
        if i > start {
            return tag.get(start..i);
        }
    }
    None
}
