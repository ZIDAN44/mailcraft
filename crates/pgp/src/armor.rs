//! Inline PGP: an armored block in a message's text (RFC 9580 §6.2, §7).

use crate::keys::armored;

/// A PGP block in a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inline<'a> {
    /// `-----BEGIN PGP MESSAGE-----`: encrypted, or signed with what it holds inside.
    Message(&'a [u8]),
    /// `-----BEGIN PGP SIGNED MESSAGE-----`: a text signed as it reads, its signature after it.
    Signed(&'a [u8]),
}

/// The first PGP message or cleartext-signed text in `text`, its armor lines included. A block
/// counts only at the start of a line and with its end line.
pub fn inline(text: &[u8]) -> Option<Inline<'_>> {
    armored(text, |k| k == "MESSAGE" || k == "SIGNED MESSAGE")
        .into_iter()
        .next()
        .map(|b| if b.starts_with(b"-----BEGIN PGP SIGNED MESSAGE-----") { Inline::Signed(b) } else { Inline::Message(b) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_are_found_in_text() {
        let t = b"Hello\r\n-----BEGIN PGP MESSAGE-----\r\n\r\nhQEM\r\n-----END PGP MESSAGE-----\r\nBye\r\n";
        assert!(matches!(inline(t), Some(Inline::Message(b)) if b.ends_with(b"END PGP MESSAGE-----")));
        let s = b"-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\nhi\n-----BEGIN PGP SIGNATURE-----\n\niQ\n-----END PGP SIGNATURE-----\n";
        assert!(matches!(inline(s), Some(Inline::Signed(b)) if b.ends_with(b"END PGP SIGNATURE-----")));
        // Quoted (not at the start of a line), unended, or a key: none.
        assert_eq!(inline(b"> -----BEGIN PGP MESSAGE-----\n> x\n> -----END PGP MESSAGE-----\n"), None);
        assert_eq!(inline(b"-----BEGIN PGP MESSAGE-----\nhQEM\n"), None);
        assert_eq!(inline(b"-----BEGIN PGP PUBLIC KEY BLOCK-----\n-----END PGP PUBLIC KEY BLOCK-----\n"), None);
    }
}
