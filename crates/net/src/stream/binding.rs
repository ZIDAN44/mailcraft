//! TLS channel binding for EWS sign-in (issue #249): what on-premises Exchange's Extended
//! Protection checks (on by default since Exchange 2019 CU14 and in Exchange SE). An NTLM or
//! Kerberos sign-in made over HTTPS carries a hash of the server's certificate as the client saw
//! it, so a sign-in relayed through another TLS connection (a man in the middle) doesn't verify;
//! one without it is refused like a wrong password.
//!
//! The binding is RFC 5929's `tls-server-end-point`: the server's certificate hashed with the
//! hash of its own signature (SHA-256 for MD5 and SHA-1, and for any it doesn't name), as
//! `tls-server-end-point:` and the hash; NTLM sends the MD5 of the GSS-API channel bindings
//! made of it (RFC 2744 §3.11, MS-NLMP 2.2.2.1 `MsvAvChannelBindings`), and sspi takes it as
//! Windows' `SEC_CHANNEL_BINDINGS` for Kerberos.
//!
//! The certificate is the server's, so hostile: it is only read as far as its signature
//! algorithm, every length checked.

use md5::{Digest, Md5};
use ring::digest;

/// The binding's prefix (RFC 5929 §4.1).
const PREFIX: &[u8] = b"tls-server-end-point:";

/// The signature algorithms hashed other than with SHA-256 (RFC 5929 §4.1: the signature's own
/// hash), as their DER object identifiers.
const SHA384: [&[u8]; 2] = [
    // sha384WithRSAEncryption, 1.2.840.113549.1.1.12
    &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0c],
    // ecdsa-with-SHA384, 1.2.840.10045.4.3.3
    &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x03],
];
const SHA512: [&[u8]; 2] = [
    // sha512WithRSAEncryption, 1.2.840.113549.1.1.13
    &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0d],
    // ecdsa-with-SHA512, 1.2.840.10045.4.3.4
    &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x04],
];

/// The channel binding's application data for a server's certificate (`cert`, DER):
/// `tls-server-end-point:` and the certificate's hash.
pub fn end_point(cert: &[u8]) -> Vec<u8> {
    let algorithm = match signature_algorithm(cert) {
        Some(oid) if SHA384.contains(&oid) => &digest::SHA384,
        Some(oid) if SHA512.contains(&oid) => &digest::SHA512,
        _ => &digest::SHA256,
    };
    let mut data = PREFIX.to_vec();
    data.extend_from_slice(digest::digest(algorithm, cert).as_ref());
    data
}

/// The MD5 of the GSS-API channel bindings with no addresses and `data` as their application
/// data: NTLM's `MsvAvChannelBindings`.
pub fn ntlm_hash(data: &[u8]) -> [u8; 16] {
    let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    let mut md5 = Md5::new();
    // The initiator's and acceptor's address types and lengths: none.
    md5.update([0u8; 16]);
    md5.update(len.to_le_bytes());
    md5.update(data);
    md5.finalize().into()
}

/// Windows' `SEC_CHANNEL_BINDINGS` with `data` as the application data (no addresses), as sspi
/// takes channel bindings for Kerberos.
pub fn sec_channel_bindings(data: &[u8]) -> Vec<u8> {
    const HEADER: u32 = 32;
    let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(32 + data.len());
    // Initiator's type, length and offset; acceptor's: none.
    out.extend_from_slice(&[0u8; 24]);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&HEADER.to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// One DER element at the start of `der`: its tag, its contents and what follows it.
fn element(der: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = der.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        // The long form, four length bytes at most (a certificate is far smaller).
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 {
            return None;
        }
        let bytes = rest.get(..n)?;
        (bytes.iter().fold(0usize, |l, b| (l << 8) | usize::from(*b)), rest.get(n..)?)
    };
    Some((tag, rest.get(..len)?, rest.get(len..)?))
}

/// The object identifier of a certificate's signature algorithm: `Certificate ::= SEQUENCE {
/// tbsCertificate, signatureAlgorithm AlgorithmIdentifier, signature }` (RFC 5280 §4.1).
fn signature_algorithm(cert: &[u8]) -> Option<&[u8]> {
    const SEQUENCE: u8 = 0x30;
    const OID: u8 = 0x06;
    let (tag, body, _) = element(cert)?;
    if tag != SEQUENCE {
        return None;
    }
    let (_, _, after_tbs) = element(body)?;
    let (tag, algorithm, _) = element(after_tbs)?;
    if tag != SEQUENCE {
        return None;
    }
    let (tag, oid, _) = element(algorithm)?;
    (tag == OID).then_some(oid)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NTLM's hash of the test server's bindings, worked out apart from this code: Python's
    /// hashlib, over 16 zero bytes, the application data's length (little-endian) and the data.
    const EXPECTED_NTLM_HASH: &str = "26ce83bd0df8b2f9f7b052ea2c6bf117";

    /// A certificate's outline, enough for its signature algorithm to be read.
    fn outline(oid: &[u8]) -> Vec<u8> {
        let mut alg = vec![0x06, u8::try_from(oid.len()).unwrap_or(0)];
        alg.extend_from_slice(oid);
        let mut body = vec![0x30, 0x03, 0x02, 0x01, 0x01, 0x30, u8::try_from(alg.len()).unwrap_or(0)];
        body.extend_from_slice(&alg);
        body.extend_from_slice(&[0x03, 0x02, 0x00, 0x00]);
        let mut cert = vec![0x30, u8::try_from(body.len()).unwrap_or(0)];
        cert.extend_from_slice(&body);
        cert
    }

    /// The test server's certificate (ECDSA with SHA-256): its end point is its SHA-256, and
    /// NTLM's hash of the bindings is the one worked out apart from this code (Python's
    /// hashlib over the same bytes as RFC 2744 lays them out).
    #[test]
    fn a_certificate_hashed_as_its_signature_is() {
        let cert = crate::stream::trust::test_certs::server();
        let data = end_point(&cert);
        assert_eq!(data.get(..PREFIX.len()), Some(PREFIX));
        assert_eq!(data.get(PREFIX.len()..), Some(digest::digest(&digest::SHA256, &cert).as_ref()));
        let hex: String = ntlm_hash(&data).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, EXPECTED_NTLM_HASH);
        // Windows' form: the header, then the same application data.
        let sec = sec_channel_bindings(&data);
        assert_eq!(sec.len(), 32 + data.len());
        assert_eq!(sec.get(24..28), Some(&u32::try_from(data.len()).unwrap_or(0).to_le_bytes()[..]));
        assert_eq!(sec.get(28..32), Some(&32u32.to_le_bytes()[..]));
        assert_eq!(sec.get(32..), Some(&data[..]));
    }

    /// SHA-384 and SHA-512 signatures hash with theirs; SHA-256, SHA-1, MD5 and anything else
    /// with SHA-256.
    #[test]
    fn the_hash_follows_the_signature() {
        let len = |oid: &[u8]| end_point(&outline(oid)).len() - PREFIX.len();
        for oid in SHA384 {
            assert_eq!(len(oid), 48);
        }
        for oid in SHA512 {
            assert_eq!(len(oid), 64);
        }
        // sha1WithRSAEncryption, md5WithRSAEncryption, ecdsa-with-SHA256.
        for oid in [
            &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x05][..],
            &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x04],
            &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02],
        ] {
            assert_eq!(len(oid), 32);
        }
    }

    /// Bytes that aren't a certificate, cut short or claiming more than there is: SHA-256, never
    /// a panic.
    #[test]
    fn junk_is_hashed_with_sha_256() {
        let whole = outline(SHA384[0]);
        for cut in 0..whole.len() {
            let _ = end_point(&whole[..cut]);
        }
        for junk in [&[][..], &[0x30], &[0x30, 0x84, 0xff, 0xff, 0xff, 0xff], &[0x30, 0x85, 1, 1, 1, 1, 1], &[0x31, 0x00], &[0x30, 0x80]] {
            assert_eq!(end_point(junk).len() - PREFIX.len(), 32, "{junk:?}");
        }
    }
}
