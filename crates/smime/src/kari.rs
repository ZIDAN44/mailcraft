//! Encryption to and from EC keys: `KeyAgreeRecipientInfo` with ephemeral-static ECDH (RFC 5753,
//! RFC 5652 §6.2.2). The sender makes a key pair for the message, agrees a secret with the
//! recipient's P-256 or P-384 key, derives a key-encryption key from it (ANSI X9.63, over the
//! ECC-CMS-SharedInfo), and wraps the content key with it (AES key wrap, RFC 3394).
//!
//! Sent: SHA-384 derivation and AES-256 wrap, what NSS (Thunderbird) sends for a 256-bit content
//! key, with NULL wrap parameters, which Outlook needs. Read: SHA-1 (OpenSSL's default) to
//! SHA-512, standard and cofactor (the same on these curves), AES-128/192/256 wrap, the shared
//! info with the wrap parameters absent or NULL, as either side may have written it.

use cms::cert::IssuerAndSerialNumber;
use cms::content_info::CmsVersion;
use cms::enveloped_data::{
    KeyAgreeRecipientIdentifier, KeyAgreeRecipientInfo, OriginatorIdentifierOrKey, OriginatorPublicKey, RecipientEncryptedKey,
};
use der::asn1::{BitString, ObjectIdentifier as Oid, OctetString};
use der::{Any, Encode, Sequence};
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;

use crate::cert::Hash;
use crate::{Error, oids};

const SHA1_KDF: Oid = Oid::new_unwrap("1.3.133.16.840.63.0.2");
const SHA224_KDF: Oid = Oid::new_unwrap("1.3.132.1.11.0");
const SHA256_KDF: Oid = Oid::new_unwrap("1.3.132.1.11.1");
const SHA384_KDF: Oid = Oid::new_unwrap("1.3.132.1.11.2");
const SHA512_KDF: Oid = Oid::new_unwrap("1.3.132.1.11.3");
const COFACTOR_SHA1_KDF: Oid = Oid::new_unwrap("1.3.133.16.840.63.0.3");
const COFACTOR_SHA224_KDF: Oid = Oid::new_unwrap("1.3.132.1.14.0");
const COFACTOR_SHA256_KDF: Oid = Oid::new_unwrap("1.3.132.1.14.1");
const COFACTOR_SHA384_KDF: Oid = Oid::new_unwrap("1.3.132.1.14.2");
const COFACTOR_SHA512_KDF: Oid = Oid::new_unwrap("1.3.132.1.14.3");
const AES128_WRAP: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.5");
const AES192_WRAP: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.25");
const AES256_WRAP: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.45");

/// RFC 5753 §7.2.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct EccCmsSharedInfo {
    key_info: AlgorithmIdentifierOwned,
    #[asn1(context_specific = "0", tag_mode = "EXPLICIT", optional = "true")]
    entity_u_info: Option<OctetString>,
    #[asn1(context_specific = "2", tag_mode = "EXPLICIT")]
    supp_pub_info: OctetString,
}

fn kdf_hash(oid: &Oid) -> Option<Hash> {
    match *oid {
        SHA1_KDF | COFACTOR_SHA1_KDF => Some(Hash::Sha1),
        SHA256_KDF | COFACTOR_SHA256_KDF => Some(Hash::Sha256),
        SHA384_KDF | COFACTOR_SHA384_KDF => Some(Hash::Sha384),
        SHA512_KDF | COFACTOR_SHA512_KDF => Some(Hash::Sha512),
        // SHA-224 isn't among the digests MailCraft has: unsupported.
        SHA224_KDF | COFACTOR_SHA224_KDF => None,
        _ => None,
    }
}

fn wrap_len(oid: &Oid) -> Option<usize> {
    match *oid {
        AES128_WRAP => Some(16),
        AES192_WRAP => Some(24),
        AES256_WRAP => Some(32),
        _ => None,
    }
}

/// ANSI X9.63 key derivation: SHA(Z || counter || SharedInfo), counter from 1, as long as `len`.
fn x963(hash: Hash, z: &[u8], shared_info: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 64);
    let mut counter: u32 = 1;
    while out.len() < len {
        let input = [z, &counter.to_be_bytes(), shared_info].concat();
        out.extend(hash.digest(&input));
        counter = counter.wrapping_add(1);
    }
    out.truncate(len);
    out
}

/// The key-encryption key for a wrap algorithm (as `key_info`), the agreed secret and `ukm`.
fn kek(hash: Hash, z: &[u8], key_info: AlgorithmIdentifierOwned, ukm: Option<&OctetString>, len: usize) -> Result<Vec<u8>, Error> {
    let bits = u32::try_from(len * 8).map_err(|_| Error::Decrypt)?;
    let info = EccCmsSharedInfo {
        key_info,
        entity_u_info: ukm.cloned(),
        supp_pub_info: OctetString::new(bits.to_be_bytes().to_vec()).map_err(|_| Error::Decrypt)?,
    };
    let der = info.to_der().map_err(|_| Error::Decrypt)?;
    Ok(x963(hash, z, &der, len))
}

fn wrap(kek: &[u8], cek: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = vec![0u8; cek.len() + 8];
    let done = match kek.len() {
        16 => aes_kw::KekAes128::try_from(kek).ok().and_then(|k| k.wrap(cek, &mut out).ok()),
        24 => aes_kw::KekAes192::try_from(kek).ok().and_then(|k| k.wrap(cek, &mut out).ok()),
        _ => aes_kw::KekAes256::try_from(kek).ok().and_then(|k| k.wrap(cek, &mut out).ok()),
    };
    done.map(|_| out).ok_or(Error::Malformed("the key can't be wrapped"))
}

fn unwrap(kek: &[u8], wrapped: &[u8]) -> Option<Vec<u8>> {
    let mut out = vec![0u8; wrapped.len().checked_sub(8)?];
    let ok = match kek.len() {
        16 => aes_kw::KekAes128::try_from(kek).ok()?.unwrap(wrapped, &mut out).is_ok(),
        24 => aes_kw::KekAes192::try_from(kek).ok()?.unwrap(wrapped, &mut out).is_ok(),
        _ => aes_kw::KekAes256::try_from(kek).ok()?.unwrap(wrapped, &mut out).is_ok(),
    };
    ok.then_some(out)
}

/// The curve of an EC key's SubjectPublicKeyInfo.
fn curve(spki: &spki::SubjectPublicKeyInfoOwned) -> Option<Oid> {
    (spki.algorithm.oid == oids::EC_PUBLIC_KEY).then(|| spki.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<Oid>().ok())).flatten()
}

/// A recipient info for `cert` (an EC certificate) carrying `cek`.
pub(crate) fn recipient(cert: &Certificate, cek: &[u8], rng: &mut impl rand_core::CryptoRngCore) -> Result<KeyAgreeRecipientInfo, Error> {
    let spki = &cert.tbs_certificate.subject_public_key_info;
    let point = spki.subject_public_key.raw_bytes();
    let (z, ours) = match curve(spki) {
        Some(c) if c == oids::P256 => {
            let them = p256::PublicKey::from_sec1_bytes(point).map_err(|_| Error::Malformed("a recipient's key can't be read"))?;
            let eph = p256::ecdh::EphemeralSecret::random(rng);
            let ours = p256::EncodedPoint::from(eph.public_key()).as_bytes().to_vec();
            (eph.diffie_hellman(&them).raw_secret_bytes().to_vec(), ours)
        }
        Some(c) if c == oids::P384 => {
            let them = p384::PublicKey::from_sec1_bytes(point).map_err(|_| Error::Malformed("a recipient's key can't be read"))?;
            let eph = p384::ecdh::EphemeralSecret::random(rng);
            let ours = p384::EncodedPoint::from(eph.public_key()).as_bytes().to_vec();
            (eph.diffie_hellman(&them).raw_secret_bytes().to_vec(), ours)
        }
        _ => return Err(Error::Unsupported("encrypting to this kind of EC key".into())),
    };
    let wrap_alg = AlgorithmIdentifierOwned { oid: AES256_WRAP, parameters: Some(Any::null()) };
    let key = kek(Hash::Sha384, &z, wrap_alg.clone(), None, 32)?;
    let wrapped = wrap(&key, cek)?;
    let params = Any::encode_from(&wrap_alg).map_err(|_| Error::Malformed("can't be written"))?;
    Ok(KeyAgreeRecipientInfo {
        version: CmsVersion::V3,
        originator: OriginatorIdentifierOrKey::OriginatorKey(OriginatorPublicKey {
            algorithm: AlgorithmIdentifierOwned { oid: oids::EC_PUBLIC_KEY, parameters: None },
            public_key: BitString::from_bytes(&ours).map_err(|_| Error::Malformed("can't be written"))?,
        }),
        ukm: None,
        key_enc_alg: AlgorithmIdentifierOwned { oid: SHA384_KDF, parameters: Some(params) },
        recipient_enc_keys: vec![RecipientEncryptedKey {
            rid: KeyAgreeRecipientIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
                issuer: cert.tbs_certificate.issuer.clone(),
                serial_number: cert.tbs_certificate.serial_number.clone(),
            }),
            enc_key: OctetString::new(wrapped).map_err(|_| Error::Malformed("can't be written"))?,
        }],
    })
}

/// The content key from a recipient info addressed to `me`, with `key` (an EC key's). `None` when
/// it isn't addressed to `me`.
pub(crate) fn content_key(info: &KeyAgreeRecipientInfo, me: &Certificate, key: &mut dyn crate::PrivateKey) -> Option<Result<Vec<u8>, Error>> {
    let mine = info.recipient_enc_keys.iter().find(|r| match &r.rid {
        KeyAgreeRecipientIdentifier::IssuerAndSerialNumber(i) => {
            i.issuer == me.tbs_certificate.issuer && i.serial_number == me.tbs_certificate.serial_number
        }
        KeyAgreeRecipientIdentifier::RKeyId(k) => crate::sign::key_id(me).is_some_and(|id| id == k.subject_key_identifier.0.as_bytes()),
    })?;
    Some(open(info, mine, key))
}

fn open(info: &KeyAgreeRecipientInfo, mine: &RecipientEncryptedKey, key: &mut dyn crate::PrivateKey) -> Result<Vec<u8>, Error> {
    let OriginatorIdentifierOrKey::OriginatorKey(theirs) = &info.originator else {
        return Err(Error::Unsupported("mail encrypted with the sender's own static key".into()));
    };
    let hash = kdf_hash(&info.key_enc_alg.oid).ok_or_else(|| Error::Unsupported(format!("key derivation {}", info.key_enc_alg.oid)))?;
    let wrap_alg: AlgorithmIdentifierOwned = info.key_enc_alg.parameters.as_ref().and_then(|p| p.decode_as().ok()).ok_or(Error::Decrypt)?;
    let len = wrap_len(&wrap_alg.oid).ok_or_else(|| Error::Unsupported(format!("key wrap {}", wrap_alg.oid)))?;
    let point = theirs.public_key.raw_bytes();
    let z = key.ecdh(point).map_err(|_| Error::Decrypt)?;
    // The shared info names the wrap algorithm: with its parameters absent, as RFC 5753 says, or
    // NULL, as Outlook and NSS write them. AES key wrap's check tells which one the sender used.
    let wrapped = mine.enc_key.as_bytes();
    for params in [None, Some(Any::null())] {
        let key_info = AlgorithmIdentifierOwned { oid: wrap_alg.oid, parameters: params };
        let k = kek(hash, &z, key_info, info.ukm.as_ref(), len)?;
        if let Some(cek) = unwrap(&k, wrapped) {
            return Ok(cek);
        }
    }
    Err(Error::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ANSI X9.63 KDF, SHA-256: the NIST CAVS vector also used by RFC 8418's test.
    #[test]
    fn x963_matches_the_published_vector() {
        let hex = |s: &str| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect::<Vec<u8>>();
        let z = hex("96c05619d56c328ab95fe84b18264b08725b85e33fd34f08");
        let info = hex("");
        let out = x963(Hash::Sha256, &z, &info, 16);
        assert_eq!(out, hex("443024c3dae66b95e6f5670601558f71"));
    }

    #[test]
    fn wrapping_round_trips() {
        let kek = [7u8; 32];
        let cek = [9u8; 32];
        let w = wrap(&kek, &cek).unwrap();
        assert_eq!(unwrap(&kek, &w).unwrap(), cek);
        assert_eq!(unwrap(&[8u8; 32], &w), None);
    }
}
