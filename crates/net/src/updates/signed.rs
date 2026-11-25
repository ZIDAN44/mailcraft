//! Signed updates: a release's `SHA256SUMS.txt` signed with minisign (`SHA256SUMS.txt.minisig`)
//! by the project's own key, and checked against the public keys built in here before a checksum
//! in it is believed. The signature covers every file the release lists, by its checksum.
//!
//! A checksum alone only proves a download is the file the release lists: whoever can change the
//! release's files (a leaked token that may upload, a changed copy on the way) changes both. The
//! private key is only in the release workflow's secret, so a release's files changed after it's
//! published, or a build published some other way, don't check out. (Someone who can run that
//! workflow with changed code can still sign; that's for the repository's own protections.)
//!
//! The signature's trusted comment (signed with it) names the release, `MailCraft <tag>`, so an
//! older release's signed checksums can't be passed off as a newer one's.

use minisign_verify::{PublicKey, Signature};

use crate::{NetError, Result};

/// The public keys a release's checksums must be signed with: minisign's (the second line of its
/// `.pub` file, `RW…`). None yet: until a version is built with one, releases aren't signed and a
/// download is held to its SHA-256 alone. From the first version built with a key, every later
/// release must be signed or that version won't install it. Two may be listed while one key takes
/// over from another. How to make the key and sign with it: `docs/updates.md`.
pub const KEYS: &[&str] = &[];

/// The largest signature file read (minisign's are about 300 bytes).
pub(super) const MAX_SIG: usize = 4 << 10;

/// What the release workflow signs `tag`'s checksums with as the trusted comment.
pub fn trusted_comment(tag: &str) -> String {
    format!("MailCraft {tag}")
}

/// Whether `sig` (a `.minisig` file's text) is a signature of `sums` by one of `keys`, made for
/// the release `tag`. A key that can't be read is passed over; a signature by none of them, of
/// other contents, or naming another release isn't taken; nor is one of minisign's legacy kind
/// (Ed25519 over the file itself, `-l`): only what minisign 0.10 on and rsign2 write, Ed25519
/// over the file's BLAKE2b hash.
pub fn verify_sums(keys: &[&str], tag: &str, sums: &[u8], sig: &str) -> Result<()> {
    let sig = (sig.len() <= MAX_SIG)
        .then(|| Signature::decode(sig).ok())
        .flatten()
        .ok_or_else(|| NetError::Protocol(format!("{tag}'s checksums signature can't be read; it isn't installed")))?;
    let signed = keys.iter().filter_map(|k| PublicKey::from_base64(k.trim()).ok()).any(|key| key.verify(sums, &sig, false).is_ok());
    if !signed {
        return Err(NetError::Protocol(format!("{tag}'s checksums aren't signed with MailCraft's key; it isn't installed")));
    }
    // Checked only once the signature is: the comment is part of what's signed.
    if sig.trusted_comment() != trusted_comment(tag) {
        return Err(NetError::Protocol(format!("{tag}'s checksums are signed for another release; it isn't installed")));
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    // Made for these tests with rsign2 (minisign's format), from two throwaway keys: A signs
    // `SUMS` for `v0.43.0-test.5` (`SIG`) and for `v0.42.0-test.4` (`SIG_OLD_TAG`); B signs it
    // for `v0.43.0-test.5` (`SIG_B`).
    pub(crate) const KEY_A: &str = "RWQliLDa3ZytQjuBRmVLoTUtf0v4mJRwUxCKRqfeCbYmCCPEOrmGKJ6t";
    pub(crate) const KEY_B: &str = "RWSSCCb8JVKmZIheBqHpdcQA4okHXmCUeeq1Ky/uReG+oxGyacrJAWJP";
    pub(crate) const SUMS: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08  mailcraft-0.43.0-windows-x64.msi\n\
                                   e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  mailcraft-0.43.0-windows-x64-portable.zip\n";
    pub(crate) const SIG: &str = "untrusted comment: signature from rsign secret key\n\
                                  RUQliLDa3ZytQi4OfRK4YD2u+jDCIoSgKJDJBiRIBVF2iIkVypJndmLW0ShVXtlIZ5AtP39jia4Wz2Jw8NqUbMDCMRR9MyBuGgk=\n\
                                  trusted comment: MailCraft v0.43.0-test.5\n\
                                  +xidXS0iUyyNdGcjoSrXJcvu03lM/CfIdsn/Z+JeeLt/evaHYmS28VJTiqtbOIPTjsP1uOIE9Xd9hdyvWqlIBw==\n";
    const SIG_B: &str = "untrusted comment: signature from rsign secret key\n\
                         RUSSCCb8JVKmZNXyRThUF38cFzSvDeqSoSDv1rXvAvU3NJ3QC6RRzexSH9yhAkn2qcwDvriVTWDKlhgNVfIFNfAnPBBE5mnp5ww=\n\
                         trusted comment: MailCraft v0.43.0-test.5\n\
                         ANopBj/8FV4tC2HVzU8Fb36kOpWv8dNBv9oC3IpUGnNIrJb9LzaLaXihgsnv13X/fw11BkhgqYqHdPQ3/nT4Bw==\n";
    const SIG_OLD_TAG: &str = "untrusted comment: signature from rsign secret key\n\
                               RUQliLDa3ZytQk80ECF8D4n0d8uxv6B6pnD71NStwOk+0xVTw8F+dj7B+su7jFxz256r05ID3lAk0N7p2o3luw6AE8c8xJ37hw4=\n\
                               trusted comment: MailCraft v0.42.0-test.4\n\
                               pd4c2zfTIIBlppO39bXWZZhFBaob6vJUw46+px1/Lm/09t5qEbyqnWFPttjZ4E4L1v0DOtzzt7K5KjkSpwoNBw==\n";
    const TAG: &str = "v0.43.0-test.5";

    /// The project's key's signature of the release's own checksums is taken, by either key
    /// while two are listed, and with Windows' line ends.
    #[test]
    fn checksums_signed_by_the_projects_key_are_taken() {
        verify_sums(&[KEY_A], TAG, SUMS.as_bytes(), SIG).unwrap();
        verify_sums(&[KEY_B, KEY_A], TAG, SUMS.as_bytes(), SIG).unwrap();
        verify_sums(&["not a key", KEY_A], TAG, SUMS.as_bytes(), SIG).unwrap();
        verify_sums(&[KEY_A], TAG, SUMS.as_bytes(), &SIG.replace('\n', "\r\n")).unwrap();
    }

    /// Anything else isn't: another key's signature, checksums changed by one byte, an older
    /// release's signed checksums under a newer tag, a signature that isn't one, and no key.
    #[test]
    fn anything_else_is_refused() {
        let refused = |keys: &[&str], tag: &str, sums: &str, sig: &str| verify_sums(keys, tag, sums.as_bytes(), sig).unwrap_err().to_string();
        assert!(refused(&[KEY_A], TAG, SUMS, SIG_B).contains("aren't signed"));
        let changed = SUMS.replacen("9f86", "9f87", 1);
        assert!(refused(&[KEY_A], TAG, &changed, SIG).contains("aren't signed"));
        assert!(refused(&[KEY_A], TAG, &format!("{SUMS}\n"), SIG).contains("aren't signed"));
        assert!(refused(&[KEY_A], TAG, SUMS, SIG_OLD_TAG).contains("another release"));
        assert!(refused(&[KEY_A], "v0.43.0-test.6", SUMS, SIG).contains("another release"));
        // The trusted comment can't be swapped: it's signed with the rest.
        let retagged = SIG.replace("MailCraft v0.43.0-test.5", "MailCraft v0.43.0-test.6");
        assert!(refused(&[KEY_A], "v0.43.0-test.6", SUMS, &retagged).contains("aren't signed"));
        for junk in ["", "untrusted comment: x\n", "\n\n\n\n", "RWQ\nRWQ\nRWQ\nRWQ", &"A".repeat(MAX_SIG + 1), &SIG.replace("RUQ", "RXQ")] {
            assert!(refused(&[KEY_A], TAG, SUMS, junk).contains("can't be read"), "{junk:.40}");
        }
        // Marked as minisign's legacy kind ("Ed" in place of "ED"): not taken.
        assert!(refused(&[KEY_A], TAG, SUMS, &SIG.replace("RUQ", "RWQ")).contains("aren't signed"));
        assert!(refused(&[], TAG, SUMS, SIG).contains("aren't signed"));
    }

    /// The trusted comment the release workflow writes is the one taken.
    #[test]
    fn the_trusted_comment_names_the_release() {
        assert_eq!(trusted_comment(TAG), "MailCraft v0.43.0-test.5");
    }
}
