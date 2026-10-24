//! `.p12` / `.pfx` files (PKCS #12, RFC 7292): the private key and the certificates with it.
//! Read by `p12-keystore`, which checks the file's MAC with the password and opens PBES2 (PBKDF2
//! with AES, what OpenSSL 3 and newer Firefox write) and the older SHA-1 3DES and RC2-40 bags
//! (Windows' default export, older Firefox, `openssl -legacy`).

use der::{Decode, Encode};
use x509_cert::Certificate;

use crate::{Error, Identity, cert, oids};

/// The largest certificate file read.
const P12_MAX: usize = 1 << 20;

/// The key and chain in a `.p12` file opened with `password`. The certificate that goes with
/// the key comes first, then each one that issued the one before it (as far as the file has
/// them); a file with several keys gives the first that has a certificate for email.
pub fn read_p12(bytes: &[u8], password: &str) -> Result<Identity, Error> {
    if bytes.len() > P12_MAX {
        return Err(Error::Malformed("too big for a certificate file"));
    }
    let store = p12_keystore::KeyStore::from_pkcs12(bytes, password).map_err(|_| Error::Password)?;
    let mut found: Vec<Identity> = Vec::new();
    for (_, entry) in store.entries() {
        let p12_keystore::KeyStoreEntry::PrivateKeyChain(c) = entry else { continue };
        let certs: Vec<Vec<u8>> = c.chain().iter().map(|x| x.as_der().to_vec()).collect();
        let Some(leaf) = certs.iter().find(|d| matches_key(d, c.key())) else { continue };
        found.push(Identity { key: c.key().to_vec(), chain: ordered(leaf, &certs) });
    }
    // The one for email first, when there are several.
    found.sort_by_key(|id| !cert::parse(id.certificate()).is_ok_and(|c| !cert::emails(&c).is_empty()));
    found.into_iter().next().ok_or(Error::NoKey)
}

/// Whether the certificate `cert` (DER) is the public half of `key` (PKCS #8 DER).
pub(crate) fn matches_key(cert_der: &[u8], key: &[u8]) -> bool {
    let Ok(c) = Certificate::from_der(cert_der) else { return false };
    let spki = &c.tbs_certificate.subject_public_key_info;
    let Ok(ours) = public_of(key) else { return false };
    spki.to_der().ok() == Some(ours)
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
