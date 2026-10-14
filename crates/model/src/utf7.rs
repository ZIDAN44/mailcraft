//! IMAP's modified UTF-7 for mailbox names (RFC 3501 §5.1.3).
//!
//! Printable ASCII stands for itself except `&`, written `&-`. Other characters are UTF-16BE,
//! base64-encoded with `,` instead of `/`, between `&` and `-`.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

fn b64_value(c: u8) -> Option<u32> {
    ALPHABET.iter().position(|&a| a == c).and_then(|p| u32::try_from(p).ok())
}

/// Decode a mailbox name. Malformed sections are kept as written rather than dropped, so a
/// broken name still shows something recognisable.
pub fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(rest.get(..amp).unwrap_or_default());
        let after = rest.get(amp + 1..).unwrap_or_default();
        let Some(dash) = after.find('-') else {
            out.push_str(rest.get(amp..).unwrap_or_default());
            return out;
        };
        let chunk = after.get(..dash).unwrap_or_default();
        if chunk.is_empty() {
            out.push('&');
        } else if let Some(text) = decode_chunk(chunk) {
            out.push_str(&text);
        } else {
            out.push('&');
            out.push_str(chunk);
            out.push('-');
        }
        rest = after.get(dash + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

fn decode_chunk(chunk: &str) -> Option<String> {
    let mut bits: u32 = 0;
    let mut nbits = 0u32;
    let mut units = Vec::new();
    for c in chunk.bytes() {
        bits = (bits << 6) | b64_value(c)?;
        nbits += 6;
        if nbits >= 16 {
            nbits -= 16;
            units.push(((bits >> nbits) & 0xFFFF) as u16);
        }
        bits &= (1 << nbits) - 1;
    }
    String::from_utf16(&units).ok()
}

/// Encode a mailbox name for the wire.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if pending.is_empty() {
            return;
        }
        out.push('&');
        let mut bits: u32 = 0;
        let mut nbits = 0u32;
        for u in pending.drain(..) {
            bits = (bits << 16) | u32::from(u);
            nbits += 16;
            while nbits >= 6 {
                nbits -= 6;
                let i = ((bits >> nbits) & 0x3F) as usize;
                out.push(char::from(ALPHABET.get(i).copied().unwrap_or(b'A')));
            }
            bits &= (1 << nbits) - 1;
        }
        if nbits > 0 {
            let i = ((bits << (6 - nbits)) & 0x3F) as usize;
            out.push(char::from(ALPHABET.get(i).copied().unwrap_or(b'A')));
        }
        out.push('-');
    };
    for c in s.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut pending, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_examples() {
        // RFC 3501 §5.1.3: "~peter/mail/台北/日本語"
        assert_eq!(decode("~peter/mail/&U,BTFw-/&ZeVnLIqe-"), "~peter/mail/台北/日本語");
        assert_eq!(encode("~peter/mail/台北/日本語"), "~peter/mail/&U,BTFw-/&ZeVnLIqe-");
        assert_eq!(decode("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(encode("Tom & Jerry"), "Tom &- Jerry");
    }

    #[test]
    fn round_trips() {
        for s in ["Café", "Entwürfe", "Отправленные", "😀 fun", "plain", "a&b", ""] {
            assert_eq!(decode(&encode(s)), s, "{s}");
        }
    }

    #[test]
    fn malformed_input_is_kept() {
        assert_eq!(decode("bad &!!- name"), "bad &!!- name");
        assert_eq!(decode("open &AOk"), "open &AOk");
    }
}
