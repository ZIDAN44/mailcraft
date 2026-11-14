//! Base64 (RFC 4648 §4): the standard alphabet, padded with `=`, on one line. SASL, HTTP Basic,
//! S/MIME and Autocrypt bodies, vCard and iCalendar attachments all use it; whoever needs lines
//! (76 in MIME, RFC 2045 §6.8) breaks the text this gives.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `data` in base64, padded, with no line breaks.
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3).saturating_mul(4));
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        // A chunk of k bytes gives k + 1 characters; `=` fills the group of four.
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648 §10's test vectors, and every byte value.
    #[test]
    fn rfc_vectors() {
        for (plain, coded) in
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")]
        {
            assert_eq!(encode(plain.as_bytes()), coded, "{plain}");
        }
        assert_eq!(encode(b"\0user\0pass"), "AHVzZXIAcGFzcw==");
        assert_eq!(encode(&[0xFB, 0xFF, 0xBF]), "+/+/");
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(encode(&all).len(), 344);
    }
}
