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

/// A code page property (`PidTagMessageCodepage`, `PidTagInternetCodepage`); 0 when there is
/// none.
pub(super) fn code_page(v: Option<&PropertyValue>) -> u32 {
    match v {
        Some(PropertyValue::Integer32(n)) => u32::try_from(*n).unwrap_or(0),
        _ => 0,
    }
}

/// Bytes from the file as text in `code_page`: an 8-bit string in the message's code page, or
/// the HTML body (`html`) in its internet code page.
///
/// An HTML body's byte-order mark says what its bytes are (a string's bytes are its page's, even
/// when they start like one). UTF-16 is read as UTF-16. With no page, US-ASCII, UTF-8 or one this
/// doesn't know, the bytes are UTF-8 when they're valid, else Windows-1252. Otherwise valid UTF-8
/// that isn't plain ASCII is UTF-8 (Outlook labels some UTF-8 mail with the sender's code page)
/// unless an HTML body declares another charset near its top; anything else is in the page.
pub(super) fn decode(bytes: &[u8], code_page: u32, html: bool) -> String {
    read(bytes, code_page, html).0
}

/// [`decode`], and whether text was lost to a page MailCraft can't read (ISO-2022-KR: only its
/// ASCII is kept, so a shift or a byte beyond ASCII is lost).
pub(super) fn read(bytes: &[u8], code_page: u32, html: bool) -> (String, bool) {
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
        && (!html || declared_charset(bytes).is_none_or(|c| c.eq_ignore_ascii_case(b"utf-8") || c.eq_ignore_ascii_case(b"utf8")))
    {
        return (s.to_string(), false);
    }
    let lost = mime::unsupported_code_page(code_page) && bytes.iter().any(|c| !c.is_ascii() || *c == 0x0E);
    (mime::decode_code_page(bytes, code_page), lost)
}

/// The charset an HTML body declares near its top (`charset=…`, quoted or not, any case).
fn declared_charset(html: &[u8]) -> Option<&[u8]> {
    let head = html.get(..html.len().min(DECLARED_WITHIN))?;
    let mut from = 0;
    while let Some(at) = head.get(from..)?.windows(7).position(|w| w.eq_ignore_ascii_case(b"charset")) {
        let mut i = from.saturating_add(at).saturating_add(7);
        from = i;
        let skip = |i: &mut usize, set: &[u8]| {
            while head.get(*i).is_some_and(|c| set.contains(c)) {
                *i += 1;
            }
        };
        skip(&mut i, b" \t\r\n");
        if head.get(i) != Some(&b'=') {
            continue;
        }
        i += 1;
        skip(&mut i, b" \t\r\n\"'");
        let start = i;
        while head.get(i).is_some_and(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(c)) {
            i += 1;
        }
        if i > start {
            return head.get(start..i);
        }
    }
    None
}
