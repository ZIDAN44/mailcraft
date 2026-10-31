//! A property's value as the rest of the import reads it.
//!
//! `outlook-pst`'s text values have no public constructors, so code that reads an item's
//! properties takes a closure giving a [`Val`] for a property id: the reader hands it the file's
//! values, and tests hand it their own.

use std::borrow::Cow;

use outlook_pst::ltp::prop_context::PropertyValue;

/// A property's value, by what it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Val<'a> {
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
        PropertyValue::String8(s) => Val::Text(Cow::Owned(decode(s.buffer(), code_page).trim_end_matches('\0').to_string())),
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

/// Bytes in a Windows code page: UTF-8 when they are (or say so), else Windows-1252.
pub(super) fn decode(bytes: &[u8], code_page: u32) -> String {
    if code_page == 65001 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—',
        '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or('\u{FFFD}'),
            _ => char::from(b),
        })
        .collect()
}
