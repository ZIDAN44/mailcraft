//! How strings are written: as UTF-16 in a Unicode file, 8-bit in an ANSI one (MS-PST 2.5).

use crate::ltp::{Bytes, PropertyType, PropertyValue, Utf16};
use crate::ndb::Format;

/// The characters Windows-1252 keeps at 0x80 to 0x9F; 0 where it has none.
const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0, 0, 0x2018, 0x2019, 0x201C,
    0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

/// `c` in Windows-1252.
pub(crate) fn windows_1252(c: char) -> Option<u8> {
    let u = u32::from(c);
    match u {
        0..=0x7F | 0xA0..=0xFF => u8::try_from(u).ok(),
        _ => {
            let high = u16::try_from(u).ok()?;
            let at = CP1252_HIGH.iter().position(|h| *h == high && *h != 0)?;
            u8::try_from(0x80 + at).ok()
        }
    }
}

/// An ANSI message's strings are in one of these (`PidTagMessageCodepage`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CodePage {
    Windows1252,
    Utf8,
}

impl CodePage {
    fn id(self) -> i32 {
        match self {
            Self::Windows1252 => 1252,
            Self::Utf8 => 65001,
        }
    }

    fn encode(self, s: &str) -> Vec<u8> {
        match self {
            Self::Windows1252 => s.chars().map(|c| windows_1252(c).unwrap_or(b'?')).collect(),
            Self::Utf8 => s.as_bytes().to_vec(),
        }
    }
}

/// How strings are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Text {
    Unicode,
    Ansi(CodePage),
}

impl Text {
    /// A message's: in an ANSI file, Windows-1252 when every string is in it, else UTF-8.
    pub(crate) fn for_message<'a>(format: Format, mut strings: impl Iterator<Item = &'a str>) -> Text {
        match format {
            Format::Unicode => Text::Unicode,
            Format::Ansi if strings.all(|s| s.chars().all(|c| windows_1252(c).is_some())) => Text::Ansi(CodePage::Windows1252),
            Format::Ansi => Text::Ansi(CodePage::Utf8),
        }
    }

    /// A store's or folder's: no property gives a code page for them.
    pub(crate) fn plain(format: Format) -> Text {
        match format {
            Format::Unicode => Text::Unicode,
            Format::Ansi => Text::Ansi(CodePage::Windows1252),
        }
    }

    /// The code page property an ANSI message gets.
    pub(crate) fn code_page(self) -> Option<i32> {
        match self {
            Text::Unicode => None,
            Text::Ansi(p) => Some(p.id()),
        }
    }

    /// The type a string is written with.
    pub(crate) fn string_type(self) -> u16 {
        match self {
            Text::Unicode => PropertyType::UNICODE,
            Text::Ansi(_) => PropertyType::STRING8,
        }
    }

    /// `s` as a value of [`Self::string_type`].
    pub(crate) fn value(self, s: &str) -> PropertyValue {
        match self {
            Text::Unicode => PropertyValue::Unicode(Utf16(s.encode_utf16().collect())),
            Text::Ansi(p) => PropertyValue::String8(Bytes(p.encode(s))),
        }
    }

    /// A value given in Unicode, as this text writes it.
    pub(crate) fn narrow(self, value: PropertyValue) -> PropertyValue {
        let Text::Ansi(page) = self else { return value };
        let plain = |u: &Utf16| {
            let mut s = String::from_utf16_lossy(u.buffer());
            if s.ends_with('\0') {
                s.pop();
            }
            s
        };
        match value {
            PropertyValue::Unicode(u) => PropertyValue::String8(Bytes(page.encode(&plain(&u)))),
            PropertyValue::MultipleUnicode(l) => PropertyValue::MultipleString8(
                l.iter()
                    .map(|u| {
                        let mut b = page.encode(&plain(u));
                        b.push(0);
                        Bytes(b)
                    })
                    .collect(),
            ),
            other => other,
        }
    }

    /// A string's bytes as a cell of a column of type `kind` keeps them.
    pub(crate) fn cell(self, kind: u16, s: &str) -> Option<Vec<u8>> {
        match (kind, self) {
            (PropertyType::UNICODE, _) => Some(s.encode_utf16().flat_map(u16::to_le_bytes).collect()),
            (PropertyType::STRING8, Text::Ansi(p)) => Some(p.encode(s)),
            (PropertyType::STRING8, Text::Unicode) => Some(s.as_bytes().to_vec()),
            _ => None,
        }
    }
}

/// Whether a folder's name read from an 8-bit value is the name asked for: its bytes, or that
/// name written the way a folder's is, ASCII letters in either case.
pub(crate) fn same_ansi_name(stored: &[u8], wanted: &str) -> bool {
    let stored = stored.strip_suffix(&[0]).unwrap_or(stored);
    let written = CodePage::Windows1252.encode(wanted);
    stored.eq_ignore_ascii_case(&written) || stored.eq_ignore_ascii_case(wanted.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_1252_covers_its_own_high_characters() {
        assert_eq!(windows_1252('€'), Some(0x80));
        assert_eq!(windows_1252('–'), Some(0x96));
        assert_eq!(windows_1252('é'), Some(0xE9));
        assert_eq!(windows_1252('Ж'), None);
    }

    #[test]
    fn a_message_outside_1252_is_written_in_utf8() {
        assert_eq!(Text::for_message(Format::Ansi, ["café"].into_iter()), Text::Ansi(CodePage::Windows1252));
        assert_eq!(Text::for_message(Format::Ansi, ["café", "Жук"].into_iter()), Text::Ansi(CodePage::Utf8));
        assert_eq!(Text::for_message(Format::Unicode, ["Жук"].into_iter()), Text::Unicode);
    }

    #[test]
    fn folder_names_match_however_they_were_written() {
        assert!(same_ansi_name(b"Archive\0", "archive"));
        assert!(same_ansi_name(b"??????", "Письма"));
        assert!(!same_ansi_name(b"Archive", "Sent"));
        assert!(same_ansi_name(&[0x96], "–"));
    }
}
