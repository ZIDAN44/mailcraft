//! The shared base64 writes what each copy it replaced wrote: the copies as they were (in the
//! IMAP client, the JMAP client, S/MIME and Autocrypt), kept here as the reference, for any
//! bytes (every padding), and in the lines S/MIME (76) and Autocrypt (72) break it into.

use mailcraft_model::base64;
use proptest::prelude::*;

/// `mailcraft_net::imap::base64`, as it was.
fn imap(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(T.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `mailcraft_jmap::base64`, as it was.
fn jmap(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let idx = usize::try_from((n >> (18 - 6 * i)) & 63).unwrap_or(0);
                out.push(char::from(A.get(idx).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `mailcraft_mime::smime::base64`, as it was.
fn smime(data: &[u8]) -> Vec<u8> {
    let mut text = Vec::with_capacity(data.len() * 4 / 3 + 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            let c = if i <= chunk.len() { B64.get(((n >> (18 - 6 * i)) & 63) as usize).copied().unwrap_or(b'A') } else { b'=' };
            text.push(c);
        }
    }
    text
}

/// The Autocrypt header's copy (`mailcraft_pgp::autocrypt`), as it was.
fn autocrypt(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 4 / 3 + 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            let c = if i <= chunk.len() { B64.get(((n >> (18 - 6 * i)) & 63) as usize).copied().unwrap_or(b'A') } else { b'=' };
            out.push(char::from(c));
        }
    }
    out
}

/// S/MIME's lines: 76 characters, each ended by CR LF.
fn smime_lines(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + text.len() / 38);
    for line in text.chunks(76) {
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// The Autocrypt header's `keydata`: folded every 72 characters.
fn autocrypt_folded(text: &str) -> String {
    let mut out = String::new();
    for line in text.as_bytes().chunks(72) {
        out.push_str("\r\n ");
        out.push_str(&String::from_utf8_lossy(line));
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Any bytes: lengths 0 to 600 take every padding (none, `=`, `==`) and several lines.
    #[test]
    fn the_shared_base64_writes_what_every_copy_wrote(data in prop::collection::vec(any::<u8>(), 0..600)) {
        let shared = base64::encode(&data);
        prop_assert_eq!(&shared, &imap(&data));
        prop_assert_eq!(&shared, &jmap(&data));
        let smime_text = smime(&data);
        prop_assert_eq!(shared.as_bytes(), smime_text.as_slice());
        prop_assert_eq!(&shared, &autocrypt(&data));
        prop_assert_eq!(smime_lines(shared.as_bytes()), smime_lines(&smime(&data)));
        prop_assert_eq!(autocrypt_folded(&shared), autocrypt_folded(&autocrypt(&data)));
    }
}

/// Each padding and the line edges, by name: 56, 57 and 58 bytes are one 76-character line, a
/// full one, and one character into the next.
#[test]
fn padding_and_line_edges() {
    for len in [0usize, 1, 2, 3, 4, 5, 53, 54, 55, 56, 57, 58, 59, 113, 114, 115, 1000] {
        let data: Vec<u8> = (0..len).map(|i| (i * 37 % 256) as u8).collect();
        let shared = base64::encode(&data);
        assert_eq!(shared, imap(&data), "{len}");
        assert_eq!(shared, jmap(&data), "{len}");
        assert_eq!(shared.as_bytes(), smime(&data), "{len}");
        assert_eq!(shared, autocrypt(&data), "{len}");
        assert_eq!(shared.len() % 4, 0, "{len}");
    }
}
