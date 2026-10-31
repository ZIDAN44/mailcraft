//! Text in a Windows code page (the numbers MAPI keeps in `PidTagInternetCodepage`,
//! `PidTagMessageCodepage` and RTF's `\ansicpg`), read with mail-parser's decoders.
//!
//! Bytes are hostile: every decoder takes any input, and pages this doesn't know read as
//! Windows-1252, the page Outlook falls back to.

/// How a code page is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Utf8,
    Utf16Le,
    /// UTF-16 big-endian: its ASCII letters are ASCII bytes with NULs, so no ASCII shortcut.
    Utf16Be,
    /// UTF-7 (65000), read here.
    Utf7,
    /// Windows-1252, with the five bytes it leaves undefined kept as the C1 controls they number.
    Windows1252,
    /// US-ASCII: other bytes are replaced.
    Ascii,
    /// A decoder by its charset name; `seven_bit` for the stateful encodings that are all ASCII
    /// bytes (ISO-2022-JP), which the ASCII shortcut would leave undecoded.
    Named {
        name: &'static [u8],
        seven_bit: bool,
    },
    /// ISO-2022-KR (50225): mail-parser has no decoder for it (its "replacement" one would turn
    /// the text into U+FFFD), so its ASCII is kept and the rest replaced.
    Unsupported,
    /// A number this doesn't know: Windows-1252.
    Unknown,
}

fn page(cp: u32) -> Page {
    let named = |name| Page::Named { name, seven_bit: false };
    match cp {
        65001 => Page::Utf8,
        1200 => Page::Utf16Le,
        1201 => Page::Utf16Be,
        65000 => Page::Utf7,
        1252 => Page::Windows1252,
        20127 => Page::Ascii,
        50225 => Page::Unsupported,
        50220..=50222 => Page::Named { name: b"iso-2022-jp", seven_bit: true },
        850 => named(b"ibm850"),
        866 => named(b"ibm866"),
        874 => named(b"windows-874"),
        932 => named(b"shift_jis"),
        936 => named(b"gbk"),
        54936 => named(b"gb18030"),
        949 => named(b"euc-kr"),
        950 => named(b"big5"),
        1250 => named(b"windows-1250"),
        1251 => named(b"windows-1251"),
        1253 => named(b"windows-1253"),
        1254 => named(b"windows-1254"),
        1255 => named(b"windows-1255"),
        1256 => named(b"windows-1256"),
        1257 => named(b"windows-1257"),
        1258 => named(b"windows-1258"),
        10000 => named(b"macintosh"),
        10007 => named(b"x-mac-cyrillic"),
        20866 => named(b"koi8-r"),
        21866 => named(b"koi8-u"),
        20932 | 51932 => named(b"euc-jp"),
        51949 => named(b"euc-kr"),
        28591 => named(b"iso-8859-1"),
        28592 => named(b"iso-8859-2"),
        28593 => named(b"iso-8859-3"),
        28594 => named(b"iso-8859-4"),
        28595 => named(b"iso-8859-5"),
        28596 => named(b"iso-8859-6"),
        28597 => named(b"iso-8859-7"),
        28598 => named(b"iso-8859-8"),
        28599 => named(b"iso-8859-9"),
        28603 => named(b"iso-8859-13"),
        28605 => named(b"iso-8859-15"),
        28606 => named(b"iso-8859-16"),
        _ => Page::Unknown,
    }
}

/// Whether `cp` is a code page this reads as itself (an unknown one reads as Windows-1252).
pub fn known_code_page(cp: u32) -> bool {
    !matches!(page(cp), Page::Unknown | Page::Unsupported)
}

/// Whether `cp` is a code page this knows but can't read (ISO-2022-KR): only its ASCII is kept.
pub fn unsupported_code_page(cp: u32) -> bool {
    page(cp) == Page::Unsupported
}

/// Whether `cp` is UTF-16 (1200, 1201): its bytes are never another page's relabelled.
pub fn utf16_code_page(cp: u32) -> bool {
    matches!(page(cp), Page::Utf16Le | Page::Utf16Be)
}

/// Bytes in a Windows code page as text (Windows-1252 for pages this doesn't know).
pub fn decode_code_page(b: &[u8], cp: u32) -> String {
    let name = match page(cp) {
        Page::Utf8 => return String::from_utf8_lossy(b).into_owned(),
        Page::Utf16Le => return utf16(b),
        Page::Utf16Be => return utf16_with(b, u16::from_be_bytes),
        Page::Utf7 => return utf7(b),
        Page::Windows1252 | Page::Unknown => return windows_1252(b),
        Page::Ascii | Page::Unsupported => return b.iter().map(|c| if c.is_ascii() { char::from(*c) } else { '\u{FFFD}' }).collect(),
        // Escapes and shifts are ASCII bytes: the decoder reads them.
        Page::Named { name, seven_bit: true } => return decode_with(b, name),
        Page::Named { name, seven_bit: false } => name,
    };
    // Plain ASCII reads the same in all the others.
    if b.is_ascii() {
        return b.iter().map(|c| char::from(*c)).collect();
    }
    decode_with(b, name)
}

fn decode_with(b: &[u8], name: &[u8]) -> String {
    match mail_parser::decoders::charsets::map::charset_decoder(name) {
        Some(f) => f(b),
        None => String::from_utf8_lossy(b).into_owned(),
    }
}

/// Windows-1252 as text. 0x81, 0x8D, 0x8F, 0x90 and 0x9D have no character: they read as the C1
/// controls with their numbers (as Windows does), so text that has them comes through unchanged.
fn windows_1252(b: &[u8]) -> String {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—',
        '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    b.iter()
        .map(|&c| match c {
            0x80..=0x9F => HIGH.get(usize::from(c - 0x80)).copied().unwrap_or('\u{FFFD}'),
            _ => char::from(c),
        })
        .collect()
}

/// UTF-7 (RFC 2152) as text: `+-` is a plus, `+` starts a base64 run of UTF-16, and any other
/// character or the end of the text ends one (mail-parser's decoder drops a run left open at the
/// end, and keeps `+-` as it is).
fn utf7(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
    let mut run: Option<Vec<u16>> = None;
    let (mut bits, mut held) = (0u32, 0u32);
    let flush = |out: &mut String, units: Vec<u16>| out.extend(char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')));
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        i += 1;
        if let Some(units) = run.as_mut() {
            if let Some(v) = base64_value(c) {
                bits = ((bits << 6) | u32::from(v)) & 0x003F_FFFF;
                held += 6;
                if held >= 16 {
                    held -= 16;
                    units.push(u16::try_from((bits >> held) & 0xFFFF).unwrap_or(0xFFFD));
                }
                continue;
            }
            if let Some(units) = run.take() {
                flush(&mut out, units);
            }
            (bits, held) = (0, 0);
            if c == b'-' {
                continue;
            }
        }
        match c {
            b'+' if b.get(i) == Some(&b'-') => {
                out.push('+');
                i += 1;
            }
            b'+' => run = Some(Vec::new()),
            _ => out.push(if c.is_ascii() { char::from(c) } else { '\u{FFFD}' }),
        }
    }
    if let Some(units) = run {
        flush(&mut out, units);
    }
    out
}

/// A base64 character's value.
fn base64_value(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// UTF-16 (little-endian) as text, without its NULs.
pub(crate) fn utf16(b: &[u8]) -> String {
    utf16_with(b, u16::from_le_bytes)
}

/// UTF-16 as text, its units read by `unit`, without its NULs.
fn utf16_with(b: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| unit(*c)).collect();
    char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).filter(|c| *c != '\0').collect()
}
