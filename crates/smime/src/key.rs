//! A private key as signing and decrypting use it: three operations, so a key MailCraft can't
//! read (on a smart card, in the system's keychain) does them where it is. [`Software`] is a key
//! held here, as a `.p12` file brings it (PKCS #8).

use der::Decode as _;
use pkcs8::DecodePrivateKey as _;

use crate::cert::Hash;
use crate::{Error, oids};

/// What a private key does for S/MIME. The kind of key (RSA or EC) is its certificate's.
pub trait PrivateKey {
    /// A signature over a SHA-256 `digest`: RSA PKCS #1 v1.5 (the DigestInfo is the key's to
    /// add), or ECDSA as DER (`SEQUENCE { r, s }`).
    fn sign_sha256(&mut self, digest: &[u8]) -> Result<Vec<u8>, Error>;
    /// An RSA key's `wrapped` content key, decrypted: PKCS #1 v1.5, or OAEP with this hash (its
    /// MGF1 the same).
    fn rsa_decrypt(&mut self, wrapped: &[u8], oaep: Option<Hash>) -> Result<Vec<u8>, Error>;
    /// An EC key's Diffie-Hellman secret with the public `point` (SEC 1): the shared point's x.
    fn ecdh(&mut self, point: &[u8]) -> Result<Vec<u8>, Error>;
}

/// A key held in memory (PKCS #8 DER), with the random numbers RSA's blinding and ECDSA use.
pub struct Software<'a, R> {
    pub key: &'a [u8],
    pub rng: &'a mut R,
}

fn unreadable() -> Error {
    Error::Malformed("the private key can't be read")
}

impl<R: rand_core::CryptoRngCore> PrivateKey for Software<'_, R> {
    fn sign_sha256(&mut self, digest: &[u8]) -> Result<Vec<u8>, Error> {
        use signature::hazmat::RandomizedPrehashSigner as _;
        let info = pkcs8::PrivateKeyInfo::from_der(self.key).map_err(|_| unreadable())?;
        let failed = |_| Error::Malformed("can't be signed");
        if info.algorithm.oid == oids::RSA_ENCRYPTION {
            let k = rsa::RsaPrivateKey::from_pkcs8_der(self.key).map_err(|_| unreadable())?;
            // Blinded: the time it takes says nothing about the key.
            return k.sign_with_rng(self.rng, rsa::Pkcs1v15Sign::new::<sha2::Sha256>(), digest).map_err(|_| Error::Malformed("can't be signed"));
        }
        if let Ok(k) = p256::SecretKey::from_pkcs8_der(self.key) {
            let s: p256::ecdsa::Signature = p256::ecdsa::SigningKey::from(k).sign_prehash_with_rng(self.rng, digest).map_err(failed)?;
            return Ok(s.to_der().as_bytes().to_vec());
        }
        if let Ok(k) = p384::SecretKey::from_pkcs8_der(self.key) {
            let s: p384::ecdsa::Signature = p384::ecdsa::SigningKey::from(k).sign_prehash_with_rng(self.rng, digest).map_err(failed)?;
            return Ok(s.to_der().as_bytes().to_vec());
        }
        Err(Error::Unsupported("signing with this kind of key".into()))
    }

    fn rsa_decrypt(&mut self, wrapped: &[u8], oaep: Option<Hash>) -> Result<Vec<u8>, Error> {
        let k = rsa::RsaPrivateKey::from_pkcs8_der(self.key).map_err(|_| Error::Decrypt)?;
        let rng = &mut *self.rng;
        let done = match oaep {
            None => k.decrypt_blinded(rng, rsa::Pkcs1v15Encrypt, wrapped),
            Some(Hash::Sha1) => k.decrypt_blinded(rng, rsa::Oaep::new::<sha1::Sha1>(), wrapped),
            Some(Hash::Sha256) => k.decrypt_blinded(rng, rsa::Oaep::new::<sha2::Sha256>(), wrapped),
            Some(Hash::Sha384) => k.decrypt_blinded(rng, rsa::Oaep::new::<sha2::Sha384>(), wrapped),
            Some(Hash::Sha512) => k.decrypt_blinded(rng, rsa::Oaep::new::<sha2::Sha512>(), wrapped),
        };
        done.map_err(|_| Error::Decrypt)
    }

    fn ecdh(&mut self, point: &[u8]) -> Result<Vec<u8>, Error> {
        if let Ok(k) = p256::SecretKey::from_pkcs8_der(self.key) {
            let them = p256::PublicKey::from_sec1_bytes(point).map_err(|_| Error::Decrypt)?;
            return Ok(p256::ecdh::diffie_hellman(k.to_nonzero_scalar(), them.as_affine()).raw_secret_bytes().to_vec());
        }
        if let Ok(k) = p384::SecretKey::from_pkcs8_der(self.key) {
            let them = p384::PublicKey::from_sec1_bytes(point).map_err(|_| Error::Decrypt)?;
            return Ok(p384::ecdh::diffie_hellman(k.to_nonzero_scalar(), them.as_affine()).raw_secret_bytes().to_vec());
        }
        Err(Error::Decrypt)
    }
}

/// An ECDSA signature as a card gives it (`r ‖ s`, each the curve's size) in DER, as CMS has it.
pub fn ecdsa_der(raw: &[u8]) -> Result<Vec<u8>, Error> {
    let bad = || Error::Malformed("the card's signature can't be read");
    match raw.len() {
        64 => Ok(p256::ecdsa::Signature::from_slice(raw).map_err(|_| bad())?.to_der().as_bytes().to_vec()),
        96 => Ok(p384::ecdsa::Signature::from_slice(raw).map_err(|_| bad())?.to_der().as_bytes().to_vec()),
        _ => Err(bad()),
    }
}

/// The DigestInfo of a SHA-256 `digest` (RFC 8017 §9.2), what an RSA PKCS #1 v1.5 signature
/// signs: for a key that signs what it's given as it is (a card's `CKM_RSA_PKCS`).
pub fn sha256_digest_info(digest: &[u8]) -> Vec<u8> {
    const PREFIX: [u8; 19] = [0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04, 0x20];
    let mut out = PREFIX.to_vec();
    out.extend_from_slice(digest);
    out
}
