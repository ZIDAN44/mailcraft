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
//!
//! Minisign's files are read here; the checking itself is `ring`'s Ed25519 and the `blake2`
//! crate's BLAKE2b, both already in MailCraft, so signing adds no crate of its own. A public key
//! is `Ed`, an 8-byte key id and the 32-byte Ed25519 key, in base64. A signature file is four
//! lines: an untrusted comment; `ED` (Ed25519 over the file's BLAKE2b-512 hash), the key id and
//! the 64-byte signature, in base64; `trusted comment: …`; and the 64-byte signature of the first
//! signature followed by that comment, in base64.

use blake2::{Blake2b512, Digest};
use ring::signature::{ED25519, UnparsedPublicKey};

use crate::{NetError, Result};

/// The public keys a release's checksums must be signed with: minisign's (the second line of its
/// `.pub` file, `RW…`). From the first version built with a key, every later release must be
/// signed or that version won't install it (versions before it hold a download to its SHA-256
/// alone). Two may be listed while one key takes over from another. How the key was made and how
/// releases are signed with it: `docs/updates.md`.
pub const KEYS: &[&str] = &[
    // The project's key (made 2026-11-27); its secret half is the release workflow's
    // `MAILCRAFT_UPDATE_KEY`.
    "RWQ8EKag3Naj3gC0lqpZDefK16t00l/3VO185Ey2T3i/gvy8TRqbJnye",
];

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
        .then(|| read_signature(sig))
        .flatten()
        .ok_or_else(|| NetError::Protocol(format!("{tag}'s checksums signature can't be read; it isn't installed")))?;
    let signed = keys.iter().filter_map(|k| read_key(k)).any(|key| signs(&key, &sig, sums));
    if !signed {
        return Err(NetError::Protocol(format!("{tag}'s checksums aren't signed with MailCraft's key; it isn't installed")));
    }
    // Checked only once the signature is: the comment is part of what's signed.
    if sig.comment != trusted_comment(tag) {
        return Err(NetError::Protocol(format!("{tag}'s checksums are signed for another release; it isn't installed")));
    }
    Ok(())
}

/// A minisign public key.
struct Key {
    id: [u8; 8],
    ed25519: [u8; 32],
}

/// A minisign signature file, read.
struct Sig {
    /// Over the file's BLAKE2b-512 hash (`ED`), not the file itself (the legacy `Ed`).
    prehashed: bool,
    key_id: [u8; 8],
    signature: [u8; 64],
    comment: String,
    /// Of `signature` followed by `comment`.
    global: [u8; 64],
}

/// The public key a `.pub` file's second line (`RW…`) holds; `None` when it isn't one.
fn read_key(text: &str) -> Option<Key> {
    let raw = mailcraft_mime::decode_base64(text.trim())?;
    if raw.len() != 42 || raw.get(..2)? != b"Ed" {
        return None;
    }
    Some(Key { id: raw.get(2..10)?.try_into().ok()?, ed25519: raw.get(10..42)?.try_into().ok()? })
}

/// A `.minisig` file's four lines (Windows' line ends taken too); `None` when it isn't one.
fn read_signature(text: &str) -> Option<Sig> {
    let mut lines = text.lines();
    lines.next()?.strip_prefix("untrusted comment:")?;
    let raw = mailcraft_mime::decode_base64(lines.next()?)?;
    let prehashed = match raw.get(..2)? {
        b"ED" => true,
        b"Ed" => false,
        _ => return None,
    };
    if raw.len() != 74 {
        return None;
    }
    let comment = lines.next()?.strip_prefix("trusted comment: ")?.to_string();
    let global = mailcraft_mime::decode_base64(lines.next()?)?;
    // Nothing after it but empty lines.
    if lines.any(|l| !l.trim().is_empty()) {
        return None;
    }
    Some(Sig {
        prehashed,
        key_id: raw.get(2..10)?.try_into().ok()?,
        signature: raw.get(10..74)?.try_into().ok()?,
        comment,
        global: global.as_slice().try_into().ok()?,
    })
}

/// Whether `sig` is `key`'s signature of `data` and of its trusted comment. Only a prehashed one
/// (`ED`): the legacy kind isn't taken.
fn signs(key: &Key, sig: &Sig, data: &[u8]) -> bool {
    if !sig.prehashed || sig.key_id != key.id {
        return false;
    }
    let ed25519 = UnparsedPublicKey::new(&ED25519, key.ed25519);
    if ed25519.verify(&Blake2b512::digest(data), &sig.signature).is_err() {
        return false;
    }
    let mut global = sig.signature.to_vec();
    global.extend_from_slice(sig.comment.as_bytes());
    ed25519.verify(&global, &sig.global).is_ok()
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

    /// What the release workflow runs, the minisign program (0.12, Ubuntu's), signs `SUMS` so it's
    /// taken; its legacy kind (`-l`), the same key, the same comment, isn't. Made from a third
    /// throwaway key, as `docs/updates.md` says to make the project's.
    #[test]
    fn the_minisign_programs_signatures_are_read() {
        const KEY_C: &str = "RWR9AA6l0wsvIX/2JGXGUxP+1skFenLKXfiPR5nCn/DlP4XQSC5SyqZ2";
        const BY_MINISIGN: &str = "untrusted comment: signature from minisign secret key\n\
                                   RUR9AA6l0wsvIakTlaCuOsUZoY4gxvm3qrP9RfUKTgBeN00UBezyUQf2KwZUNYwDzPguFafA698Jrxc3G6RcIszZJ2OFAeZ8AA0=\n\
                                   trusted comment: MailCraft v0.43.0-test.5\n\
                                   Gd3u0OrEZyTTR8aONTGHTFpczGPPxoDyLkyuL5eAVyDu1Am/SEyDebt3z3/rFtg3HFjJPvUWbfTYCCh5VrXTBg==\n";
        const LEGACY: &str = "untrusted comment: signature from minisign secret key\n\
                              RWR9AA6l0wsvISUa/HGCi2xltvlSIWigougALWNq1/e0P7fygWjTXIG+FgrteI/T34iGjmPg0ACrgXe0DZ+AtUlCN+6Vc981pQI=\n\
                              trusted comment: MailCraft v0.43.0-test.5\n\
                              zyy+AiJzrQ+0elVw0fzCPw9/MNCldBQlH1g1a5noJcPMOcvJbL4MT81+FHajhq6p7q8MgUnPFaPijLeJzLbxDA==\n";
        verify_sums(&[KEY_C], TAG, SUMS.as_bytes(), BY_MINISIGN).unwrap();
        verify_sums(&[KEY_A, KEY_C], TAG, SUMS.as_bytes(), BY_MINISIGN).unwrap();
        let legacy = verify_sums(&[KEY_C], TAG, SUMS.as_bytes(), LEGACY).unwrap_err().to_string();
        assert!(legacy.contains("aren't signed"), "{legacy}");
        assert!(verify_sums(&[KEY_A], TAG, SUMS.as_bytes(), BY_MINISIGN).is_err(), "another key's");
    }

    /// Every key MailCraft is built with can be read: one that can't would be passed over, and
    /// with it alone no release would install.
    #[test]
    fn every_key_built_in_is_one() {
        for key in KEYS {
            assert!(read_key(key).is_some(), "{key:?} isn't a minisign public key (the `RW…` line of its .pub file)");
        }
    }
}
