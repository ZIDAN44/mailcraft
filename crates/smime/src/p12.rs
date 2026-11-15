//! `.p12` / `.pfx` files (PKCS #12, RFC 7292): the private key and the certificates with it.
//! Read by `p12-keystore`, which checks the file's MAC with the password and opens PBES2 (PBKDF2
//! with AES, what OpenSSL 3 and newer Firefox write) and the older SHA-1 3DES and RC2-40 bags
//! (Windows' default export, older Firefox, `openssl -legacy`).
//!
//! It reads DER only, and Firefox and Thunderbird write BER (issue #89): every length left open,
//! the contents cut into pieces. Those are rewritten as DER first ([`crate::ber`]).

use der::{Decode, Encode};
use p12_keystore::{KeyStore, error::Error as StoreError};
use x509_cert::Certificate;

use crate::{Error, Identity, ber, cert, oids};

/// The largest certificate file read.
const P12_MAX: usize = 1 << 20;

/// The key and chain in a `.p12` file opened with `password`. The certificate that goes with
/// the key comes first, then each one that issued the one before it (as far as the file has
/// them); a file with several keys gives the first that has a certificate for email.
pub fn read_p12(bytes: &[u8], password: &str) -> Result<Identity, Error> {
    if bytes.len() > P12_MAX {
        return Err(Error::Malformed("too big for a certificate file"));
    }
    let store = open(bytes, password)?;
    let mut found: Vec<Identity> = Vec::new();
    for (_, entry) in store.entries() {
        let p12_keystore::KeyStoreEntry::PrivateKeyChain(c) = entry else { continue };
        let certs: Vec<Vec<u8>> = c.chain().iter().map(|x| x.as_der().to_vec()).collect();
        let Some(leaf) = leaf_of(c.key(), &certs) else { continue };
        found.push(Identity { key: c.key().to_vec(), chain: ordered(leaf, &certs) });
    }
    // The one for email first, when there are several.
    found.sort_by_key(|id| !cert::parse(id.certificate()).is_ok_and(|c| !cert::emails(&c).is_empty()));
    found.into_iter().next().ok_or(Error::NoKey)
}

/// The file opened with `password`: as it is when it's DER (OpenSSL, Windows), and rewritten as
/// DER when it's BER (Firefox, Thunderbird and what else is built on NSS).
fn open(bytes: &[u8], password: &str) -> Result<KeyStore, Error> {
    match KeyStore::from_pkcs12(bytes, password) {
        Ok(store) => return Ok(store),
        // Not DER: BER, perhaps.
        Err(StoreError::DerError(_)) => {}
        Err(e) => return Err(refused(&e)),
    }
    // The outside as DER. What the file's check (its MAC) covers is inside an OCTET STRING,
    // whose bytes stay as they were written.
    let outer = ber::to_der(bytes).map_err(|_| Error::NotCertificateFile)?;
    pkcs12::pfx::Pfx::from_der(&outer).map_err(|_| Error::NotCertificateFile)?;
    match KeyStore::from_pkcs12(&outer, password) {
        Ok(store) => return Ok(store),
        // The outside is read (just above, as the library reads it), so this is the inside,
        // which is read only after the password has been checked against the file: it's BER too.
        Err(StoreError::DerError(_)) => {}
        Err(e) => return Err(refused(&e)),
    }
    // The inside as DER too. The check no longer fits it, and has been made: it's left out.
    let whole = ber::p12_to_der(bytes).map_err(|_| Error::NotCertificateFile)?;
    let mut pfx = pkcs12::pfx::Pfx::from_der(&whole).map_err(|_| Error::NotCertificateFile)?;
    pfx.mac_data = None;
    KeyStore::from_pkcs12(&pfx.to_der().map_err(|_| Error::NotCertificateFile)?, password).map_err(|e| refused(&e))
}

/// Why a file wasn't opened, as far as it can be told: the password, a way of locking the file
/// that isn't read here, or a file that isn't one.
fn refused(e: &StoreError) -> Error {
    match e {
        // The file's own check of the password; and, in a file with no check, a key or
        // certificates that don't come out whole.
        StoreError::MacError(_) | StoreError::Pkcs5Error(_) | StoreError::UnpadError => Error::Password,
        StoreError::UnsupportedEncryptionScheme | StoreError::UnsupportedMacAlgorithm => {
            Error::Unsupported("the way this file is locked (save it again from the program it came from, or with OpenSSL)".into())
        }
        StoreError::UnsupportedContentType | StoreError::UnsupportedCertificateType => Error::Unsupported("what this file holds".into()),
        _ => Error::NotCertificateFile,
    }
}

/// The certificate among `certs` (DER) that is the public half of `key` (PKCS #8 DER).
/// The key's public half is worked out once (an RSA key is checked as it's read), not for each.
pub(crate) fn leaf_of<'c>(key: &[u8], certs: &'c [Vec<u8>]) -> Option<&'c Vec<u8>> {
    let ours = public_of(key).ok()?;
    certs.iter().find(|d| Certificate::from_der(d).is_ok_and(|c| c.tbs_certificate.subject_public_key_info.to_der().is_ok_and(|s| s == ours)))
}

/// The public key (SubjectPublicKeyInfo DER) of a PKCS #8 private key.
pub(crate) fn public_of(key: &[u8]) -> Result<Vec<u8>, Error> {
    use pkcs8::EncodePublicKey as _;
    let info = pkcs8::PrivateKeyInfo::from_der(key).map_err(|_| Error::Malformed("the private key can't be read"))?;
    let alg = info.algorithm.oid;
    let doc = if alg == oids::RSA_ENCRYPTION {
        use pkcs8::DecodePrivateKey as _;
        let k = rsa::RsaPrivateKey::from_pkcs8_der(key).map_err(|_| Error::Malformed("the private key can't be read"))?;
        rsa::RsaPublicKey::from(&k).to_public_key_der()
    } else if alg == oids::EC_PUBLIC_KEY {
        use pkcs8::DecodePrivateKey as _;
        match p256::SecretKey::from_pkcs8_der(key) {
            Ok(k) => k.public_key().to_public_key_der(),
            Err(_) => {
                p384::SecretKey::from_pkcs8_der(key).map_err(|_| Error::Unsupported("this kind of key".into()))?.public_key().to_public_key_der()
            }
        }
    } else {
        return Err(Error::Unsupported(format!("a {alg} key")));
    };
    doc.map(|d| d.as_bytes().to_vec()).map_err(|_| Error::Malformed("the private key can't be read"))
}

/// `leaf` and then its issuers from `all`, each found by name, up the chain.
fn ordered(leaf: &[u8], all: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut chain = vec![leaf.to_vec()];
    let parsed: Vec<(&Vec<u8>, Certificate)> = all.iter().filter_map(|d| Certificate::from_der(d).ok().map(|c| (d, c))).collect();
    for _ in 0..all.len() {
        let Some(last) = chain.last().and_then(|d| Certificate::from_der(d).ok()) else { break };
        if last.tbs_certificate.subject == last.tbs_certificate.issuer {
            break;
        }
        let up = parsed.iter().find(|(d, c)| c.tbs_certificate.subject == last.tbs_certificate.issuer && !chain.contains(d));
        let Some((d, _)) = up else { break };
        chain.push((*d).clone());
    }
    chain
}
