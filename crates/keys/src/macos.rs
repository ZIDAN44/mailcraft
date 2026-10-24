//! Identities in the macOS Keychain (a certificate with its private key: one imported there, a
//! company's, or a smart card macOS reads itself), used where they are through the Security
//! framework: `SecKeyCreateSignature` (PKCS #1 v1.5 or X9.62 over the SHA-256 digest) and
//! `SecKeyCreateDecryptedData` (PKCS #1 v1.5 or OAEP). macOS asks the user before a key is used,
//! as it does for every program.
//!
//! An EC key's Diffie-Hellman (`SecKey::key_exchange`, standard, no KDF) needs the sender's
//! one-time key as a `SecKey`, which only an unsafe call makes: that's [`sec`], the one file here
//! that allows it.

use mailcraft_smime::PrivateKey;
use mailcraft_smime::cert::Hash;
use security_framework::certificate::SecCertificate;
use security_framework::identity::SecIdentity;
use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};
use security_framework::key::{Algorithm, SecKey};

use crate::{Error, Found, KeyRef, hex};

mod sec;

fn identities() -> Result<Vec<SecIdentity>, Error> {
    let results = ItemSearchOptions::new().class(ItemClass::identity()).load_refs(true).limit(Limit::All).search();
    let results = match results {
        Ok(r) => r,
        // None at all is "not found".
        Err(e) if e.code() == -25300 => Vec::new(),
        Err(e) => return Err(Error::Keychain(e.to_string())),
    };
    Ok(results
        .into_iter()
        .filter_map(|r| match r {
            SearchResult::Ref(Reference::Identity(id)) => Some(id),
            _ => None,
        })
        .collect())
}

fn sha1(der: &[u8]) -> String {
    use sha1::Digest as _;
    hex(&sha1::Sha1::digest(der))
}

/// The Keychain's identities whose certificates are for email (an address in them).
pub fn certificates() -> Result<Vec<Found>, Error> {
    let mut found = Vec::new();
    for id in identities()? {
        let Ok(cert) = id.certificate() else { continue };
        let der = cert.to_der();
        if cert.email_addresses().map(|e| e.is_empty()).unwrap_or(true) {
            continue;
        }
        found.push(Found { key: KeyRef::Keychain { sha1: sha1(&der) }, label: cert.subject_summary(), cert: der });
    }
    Ok(found)
}

/// A key in the Keychain.
pub struct KeychainKey {
    key: SecKey,
    ec: bool,
}

/// Find the key `key` refers to (a [`KeyRef::Keychain`]).
pub fn open(key: &KeyRef) -> Result<KeychainKey, Error> {
    let KeyRef::Keychain { sha1: want } = key else { return Err(Error::Missing) };
    for id in identities()? {
        let Ok(cert) = id.certificate() else { continue };
        if sha1(&cert.to_der()) != *want {
            continue;
        }
        let ec = is_ec(&cert);
        let key = id.private_key().map_err(|e| Error::Keychain(e.to_string()))?;
        return Ok(KeychainKey { key, ec });
    }
    Err(Error::Missing)
}

fn is_ec(cert: &SecCertificate) -> bool {
    mailcraft_smime::cert::info(&cert.to_der()).map(|i| i.key.starts_with("EC")).unwrap_or(false)
}

fn failed(e: impl std::fmt::Display) -> mailcraft_smime::Error {
    mailcraft_smime::Error::Unsupported(format!("the keychain: {e}"))
}

impl PrivateKey for KeychainKey {
    fn sign_sha256(&mut self, digest: &[u8]) -> Result<Vec<u8>, mailcraft_smime::Error> {
        if self.ec {
            // X9.62: already DER, as CMS has it.
            return self.key.create_signature(Algorithm::ECDSASignatureDigestX962SHA256, digest).map_err(failed);
        }
        self.key.create_signature(Algorithm::RSASignatureDigestPKCS1v15SHA256, digest).map_err(failed)
    }

    fn rsa_decrypt(&mut self, wrapped: &[u8], oaep: Option<Hash>) -> Result<Vec<u8>, mailcraft_smime::Error> {
        let alg = match oaep {
            None => Algorithm::RSAEncryptionPKCS1,
            Some(Hash::Sha1) => Algorithm::RSAEncryptionOAEPSHA1,
            Some(Hash::Sha256) => Algorithm::RSAEncryptionOAEPSHA256,
            Some(Hash::Sha384) => Algorithm::RSAEncryptionOAEPSHA384,
            Some(Hash::Sha512) => Algorithm::RSAEncryptionOAEPSHA512,
        };
        self.key.decrypt_data(alg, wrapped).map_err(|_| mailcraft_smime::Error::Decrypt)
    }

    fn ecdh(&mut self, point: &[u8]) -> Result<Vec<u8>, mailcraft_smime::Error> {
        if !self.ec {
            return Err(mailcraft_smime::Error::Decrypt);
        }
        // The secret is the curve's size: P-256's 32 bytes, P-384's 48.
        let size = match point.len() {
            65 => 32,
            97 => 48,
            _ => return Err(mailcraft_smime::Error::Decrypt),
        };
        let theirs = sec::public_ec_key(point).map_err(|_| mailcraft_smime::Error::Decrypt)?;
        let z = self.key.key_exchange(Algorithm::ECDHKeyExchangeStandard, &theirs, size, None).map_err(|_| mailcraft_smime::Error::Decrypt)?;
        if z.len() != size {
            return Err(mailcraft_smime::Error::Decrypt);
        }
        Ok(z)
    }
}
