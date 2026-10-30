//! Alt+numpad character codes, as Windows types them (issue #23: É is Alt+0201 on a French
//! keyboard that has no key for it): which character a code names, in the system's code pages.

/// IBM PC (OEM) code page 437 code points for 0x80..=0xFF (the Unicode Consortium's mapping
/// file), the US English OEM page, which mail's character sets don't include.
const CP437: [u16; 128] = [
    0x00C7, 0x00FC, 0x00E9, 0x00E2, 0x00E4, 0x00E0, 0x00E5, 0x00E7, 0x00EA, 0x00EB, 0x00E8, 0x00EF, 0x00EE, 0x00EC, 0x00C4, 0x00C5, 0x00C9, 0x00E6,
    0x00C6, 0x00F4, 0x00F6, 0x00F2, 0x00FB, 0x00F9, 0x00FF, 0x00D6, 0x00DC, 0x00A2, 0x00A3, 0x00A5, 0x20A7, 0x0192, 0x00E1, 0x00ED, 0x00F3, 0x00FA,
    0x00F1, 0x00D1, 0x00AA, 0x00BA, 0x00BF, 0x2310, 0x00AC, 0x00BD, 0x00BC, 0x00A1, 0x00AB, 0x00BB, 0x2591, 0x2592, 0x2593, 0x2502, 0x2524, 0x2561,
    0x2562, 0x2556, 0x2555, 0x2563, 0x2551, 0x2557, 0x255D, 0x255C, 0x255B, 0x2510, 0x2514, 0x2534, 0x252C, 0x251C, 0x2500, 0x253C, 0x255E, 0x255F,
    0x255A, 0x2554, 0x2569, 0x2566, 0x2560, 0x2550, 0x256C, 0x2567, 0x2568, 0x2564, 0x2565, 0x2559, 0x2558, 0x2552, 0x2553, 0x256B, 0x256A, 0x2518,
    0x250C, 0x2588, 0x2584, 0x258C, 0x2590, 0x2580, 0x03B1, 0x00DF, 0x0393, 0x03C0, 0x03A3, 0x03C3, 0x00B5, 0x03C4, 0x03A6, 0x0398, 0x03A9, 0x03B4,
    0x221E, 0x03C6, 0x03B5, 0x2229, 0x2261, 0x00B1, 0x2265, 0x2264, 0x2320, 0x2321, 0x00F7, 0x2248, 0x00B0, 0x2219, 0x00B7, 0x221A, 0x207F, 0x00B2,
    0x25A0, 0x00A0,
];

/// The glyphs the OEM code pages show for 0x01..=0x1F (smileys, card suits, arrows), which
/// Windows types for those codes rather than control characters.
const OEM_GLYPHS: [u16; 31] = [
    0x263A, 0x263B, 0x2665, 0x2666, 0x2663, 0x2660, 0x2022, 0x25D8, 0x25CB, 0x25D9, 0x2642, 0x2640, 0x266A, 0x266B, 0x263C, 0x25BA, 0x25C4, 0x2195,
    0x203C, 0x00B6, 0x00A7, 0x25AC, 0x21A8, 0x2191, 0x2193, 0x2192, 0x2190, 0x221F, 0x2194, 0x25B2, 0x25BC,
];

/// One byte in a code page that mail's character sets name (`windows-1252`, `cp850`…).
fn page_byte(page: &str, b: u8) -> Option<char> {
    let decode = mail_parser::decoders::charsets::map::charset_decoder(page.as_bytes())?;
    decode(&[b]).chars().next()
}

/// The character an Alt+numpad code names, as Windows types it (digits as typed):
/// - with a leading 0, up to 255: the ANSI code page's (`ansi`, 1250–1258): `0201` → É, `0128`
///   → € in Windows-1252;
/// - without, 1–255: the OEM code page's (`oem`: 437, 850 or 866; any other reads as 437):
///   `130` → é, `144` → É; 1–31 and 127 are its glyphs (`1` → ☺);
/// - above 255: the Unicode character with that value (`8364` → €).
///
/// `None` for no digits, 0, a control character, or past Unicode.
pub fn alt_code_char(code: &str, ansi: u16, oem: u16) -> Option<char> {
    if code.is_empty() || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = code.trim_start_matches('0');
    // More than seven digits is past U+10FFFF (1114111).
    if digits.len() > 7 {
        return None;
    }
    let value: u32 = if digits.is_empty() { 0 } else { digits.parse().ok()? };
    let c = match u8::try_from(value) {
        Ok(0) => return None,
        Ok(b) if code.starts_with('0') && b < 0x80 => char::from(b),
        Ok(b) if code.starts_with('0') => page_byte(&format!("windows-{ansi}"), b).or_else(|| page_byte("windows-1252", b))?,
        Ok(b @ 0x01..=0x1F) => OEM_GLYPHS.get(usize::from(b - 1)).and_then(|u| char::from_u32(u32::from(*u)))?,
        Ok(0x7F) => '\u{2302}',
        Ok(b) if b < 0x80 => char::from(b),
        Ok(b) => match oem {
            850 | 866 => page_byte(&format!("cp{oem}"), b)?,
            _ => CP437.get(usize::from(b - 0x80)).and_then(|u| char::from_u32(u32::from(*u)))?,
        },
        Err(_) => char::from_u32(value)?,
    };
    Some(c).filter(|c| !c.is_control() && *c != '\u{FFFD}')
}

/// The (ANSI, OEM) code pages Windows has for a locale (a BCP 47 tag like `fr-FR`), among those
/// [`alt_code_char`] reads: Windows-1252 and OEM 850 for Western European languages, 437 for US
/// English and what isn't known.
pub fn alt_code_pages(locale: Option<&str>) -> (u16, u16) {
    let Some(tag) = locale else { return (1252, 437) };
    let parts: Vec<&str> = tag.split(['-', '_', '.', '@']).collect();
    let lang = parts.first().copied().unwrap_or_default().to_ascii_lowercase();
    let region = parts.get(1..).unwrap_or_default().iter().find(|p| p.len() == 2).map(|r| r.to_ascii_uppercase());
    const WESTERN: [&str; 18] = ["ca", "da", "de", "es", "eu", "fi", "fo", "fr", "ga", "gl", "is", "it", "nb", "nl", "nn", "no", "pt", "sv"];
    match lang.as_str() {
        "en" if region.as_deref().is_none_or(|r| r == "US") => (1252, 437),
        "en" => (1252, 850),
        l if WESTERN.contains(&l) => (1252, 850),
        "pl" | "cs" | "sk" | "hu" | "sl" | "hr" | "ro" | "sq" | "bs" => (1250, 437),
        "ru" | "uk" | "be" | "bg" => (1251, 866),
        "el" => (1253, 437),
        "tr" | "az" => (1254, 437),
        "he" => (1255, 437),
        "ar" | "fa" | "ur" => (1256, 437),
        "et" | "lv" | "lt" => (1257, 437),
        "vi" => (1258, 437),
        _ => (1252, 437),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_name_the_characters_windows_types() {
        // French (issue #23): the ANSI page with a leading 0, the OEM page without.
        let (ansi, oem) = alt_code_pages(Some("fr-FR"));
        assert_eq!((ansi, oem), (1252, 850));
        let typed: String =
            ["0201", "0200", "0199", "0233", "0128", "130", "144", "135"].iter().filter_map(|c| alt_code_char(c, ansi, oem)).collect();
        assert_eq!(typed, "ÉÈÇé€éÉç");
        // US English's OEM page 437, its glyphs, Unicode above 255, another ANSI page.
        assert_eq!(alt_code_char("233", 1252, 437), Some('Θ'));
        assert_eq!(alt_code_char("233", 1252, 850), Some('Ú'));
        assert_eq!(alt_code_char("1", 1252, 437), Some('☺'));
        assert_eq!(alt_code_char("065", 1252, 437), Some('A'));
        assert_eq!(alt_code_char("8364", 1252, 437), Some('€'));
        assert_eq!(alt_code_char("0185", 1250, 437), Some('ą'));
        assert_eq!(alt_code_char("0192", 1251, 866), Some('А'));
        assert_eq!(alt_code_char("128", 1251, 866), Some('А'));
        // Nothing for no code, 0, a control character, too many digits, or past Unicode.
        for bad in ["", "0", "000", "x1", "0009", "12345678", "1114112"] {
            assert_eq!(alt_code_char(bad, 1252, 437), None, "{bad}");
        }
        assert_eq!(alt_code_pages(None), (1252, 437));
        assert_eq!(alt_code_pages(Some("en-GB")), (1252, 850));
    }
}
