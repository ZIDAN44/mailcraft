//! Encryption: `EnvelopedData` (RFC 5652 §6) and `AuthEnvelopedData` (RFC 5083, AES-GCM by
//! RFC 5084), each recipient's copy of the content key encrypted to their RSA key
//! (`KeyTransRecipientInfo`) or wrapped with one agreed with their EC key by ECDH
//! (`KeyAgreeRecipientInfo`, RFC 5753, in `kari.rs`).
//!
//! Sent: AES-256-CBC by default, since Thunderbird before 154 and older Outlook read nothing
//! else; AES-256-GCM, which can't be changed undetected (EFAIL), when every recipient said they
//! read it. Read: AES-CBC and AES-GCM of every key size, and 3DES for old mail; the content key
//! by RSA PKCS #1 v1.5 or OAEP, or by ECDH on P-256 or P-384. Any failure on the way is
//! [`Error::Decrypt`], one error, so nothing about the key leaks through what went wrong.

use cms::cert::IssuerAndSerialNumber;
use cms::content_info::{CmsVersion, ContentInfo};
use cms::enveloped_data::{
    EncryptedContentInfo, EnvelopedData, KeyTransRecipientInfo, OriginatorInfo, RecipientIdentifier, RecipientInfo, RecipientInfos,
};
use der::asn1::{OctetString, SetOfVec};
use der::{Any, Decode, Encode, Sequence};
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;
use x509_cert::attr::Attribute;

use crate::{Error, Identity, cert, der_of, oids};

/// How the content is encrypted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cipher {
    /// `EnvelopedData` with AES-256-CBC: read by every S/MIME program.
    Aes256Cbc,
    /// `AuthEnvelopedData` with AES-256-GCM: authenticated, for recipients who read it.
    Aes256Gcm,
}

/// RFC 5083 §2.1.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct AuthEnvelopedData {
    version: CmsVersion,
    #[asn1(context_specific = "0", tag_mode = "IMPLICIT", optional = "true")]
    originator_info: Option<OriginatorInfo>,
    recip_infos: RecipientInfos,
    auth_encrypted_content: EncryptedContentInfo,
    #[asn1(context_specific = "1", tag_mode = "IMPLICIT", optional = "true")]
    auth_attrs: Option<SetOfVec<Attribute>>,
    mac: OctetString,
    #[asn1(context_specific = "2", tag_mode = "IMPLICIT", optional = "true")]
    unauth_attrs: Option<SetOfVec<Attribute>>,
}

/// RFC 5084 §3.2. The tag length defaults to 12 bytes: a 16-byte one must be written out (OpenSSL,
/// for one, refuses the parameters otherwise). Only 16 is made or accepted.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct GcmParameters {
    nonce: OctetString,
    #[asn1(optional = "true")]
    icv_len: Option<u8>,
}

fn written<T>(r: Result<T, der::Error>) -> Result<T, Error> {
    r.map_err(|_| Error::Malformed("can't be written"))
}

/// `content` (a MIME entity) encrypted to each of `recipients` (certificates, DER: the sender's
/// own among them, so the Sent copy reads too), with `cipher`.
pub fn encrypt(content: &[u8], recipients: &[Vec<u8>], cipher: Cipher, rng: &mut impl rand_core::CryptoRngCore) -> Result<Vec<u8>, Error> {
    if recipients.is_empty() {
        return Err(Error::Unsupported("encrypting to nobody".into()));
    }
    let mut key = [0u8; 32];
    rng.fill_bytes(&mut key);
    let mut infos = SetOfVec::new();
    let mut seen: Vec<&[u8]> = Vec::new();
    for r in recipients {
        // A certificate named twice (the sender's own, to themselves) gets one copy of the key.
        if seen.contains(&r.as_slice()) {
            continue;
        }
        seen.push(r);
        let c = cert::parse(r)?;
        let spki = &c.tbs_certificate.subject_public_key_info;
        // An EC key: the content key wrapped with one agreed by ECDH (RFC 5753).
        if spki.algorithm.oid == oids::EC_PUBLIC_KEY {
            written(infos.insert(RecipientInfo::Kari(crate::kari::recipient(&c, &key, rng)?)))?;
            continue;
        }
        if spki.algorithm.oid != oids::RSA_ENCRYPTION {
            return Err(Error::Unsupported(format!("encrypting to {}'s kind of key", cert::info(r).map(|i| i.name).unwrap_or_default())));
        }
        let public = cert::public_rsa(spki).ok_or(Error::Malformed("a recipient's key can't be read"))?;
        let wrapped = public.encrypt(rng, rsa::Pkcs1v15Encrypt, &key).map_err(|_| Error::Malformed("the key can't be encrypted"))?;
        let ktri = KeyTransRecipientInfo {
            version: CmsVersion::V0,
            rid: RecipientIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
                issuer: c.tbs_certificate.issuer.clone(),
                serial_number: c.tbs_certificate.serial_number.clone(),
            }),
            key_enc_alg: AlgorithmIdentifierOwned { oid: oids::RSA_ENCRYPTION, parameters: Some(Any::null()) },
            enc_key: written(OctetString::new(wrapped))?,
        };
        written(infos.insert(RecipientInfo::Ktri(ktri)))?;
    }
    // Version 0 when every recipient is RSA by issuer and serial, else 2 (RFC 5652 §6.1).
    let version = if infos.iter().all(|r| matches!(r, RecipientInfo::Ktri(_))) { CmsVersion::V0 } else { CmsVersion::V2 };
    let (content_type, body) = match cipher {
        Cipher::Aes256Cbc => {
            use aes::cipher::{BlockEncryptMut as _, KeyIvInit as _, block_padding::Pkcs7};
            let mut iv = [0u8; 16];
            rng.fill_bytes(&mut iv);
            let sealed = cbc::Encryptor::<aes::Aes256>::new(&key.into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(content);
            let eci = EncryptedContentInfo {
                content_type: oids::DATA,
                content_enc_alg: AlgorithmIdentifierOwned {
                    oid: oids::AES256_CBC,
                    parameters: Some(written(Any::encode_from(&written(OctetString::new(iv.to_vec()))?))?),
                },
                encrypted_content: Some(written(OctetString::new(sealed))?),
            };
            let ed =
                EnvelopedData { version, originator_info: None, recip_infos: RecipientInfos(infos), encrypted_content: eci, unprotected_attrs: None };
            (oids::ENVELOPED_DATA, written(Any::encode_from(&ed))?)
        }
        Cipher::Aes256Gcm => {
            use aes_gcm::aead::{Aead as _, KeyInit as _};
            let mut nonce = [0u8; 12];
            rng.fill_bytes(&mut nonce);
            let mut sealed =
                aes_gcm::Aes256Gcm::new(&key.into()).encrypt(&nonce.into(), content).map_err(|_| Error::Malformed("can't be encrypted"))?;
            // The tag goes in `mac`, apart from the ciphertext.
            let tag = sealed.split_off(sealed.len().saturating_sub(16));
            let params = GcmParameters { nonce: written(OctetString::new(nonce.to_vec()))?, icv_len: Some(16) };
            let eci = EncryptedContentInfo {
                content_type: oids::DATA,
                content_enc_alg: AlgorithmIdentifierOwned { oid: oids::AES256_GCM, parameters: Some(written(Any::encode_from(&params))?) },
                encrypted_content: Some(written(OctetString::new(sealed))?),
            };
            let aed = AuthEnvelopedData {
                version: CmsVersion::V0,
                originator_info: None,
                recip_infos: RecipientInfos(infos),
                auth_encrypted_content: eci,
                auth_attrs: None,
                mac: written(OctetString::new(tag))?,
                unauth_attrs: None,
            };
            (oids::AUTH_ENVELOPED_DATA, written(Any::encode_from(&aed))?)
        }
    };
    written(ContentInfo { content_type, content: body }.to_der())
}

/// The content of `cms` (BER or DER `ContentInfo` of `EnvelopedData` or `AuthEnvelopedData`),
/// decrypted with `id`. The second value says whether it was authenticated (GCM): content
/// encrypted with CBC can have been changed by someone without the key, so it's shown with care.
pub fn decrypt(cms: &[u8], id: &Identity, rng: &mut impl rand_core::CryptoRngCore) -> Result<(Vec<u8>, bool), Error> {
    decrypt_with(cms, id.certificate(), &mut crate::key::Software { key: &id.key, rng })
}

/// [`decrypt`] with a key held elsewhere (a smart card, the system's keychain), whose certificate
/// is `cert` (DER).
pub fn decrypt_with(cms: &[u8], cert: &[u8], id: &mut dyn crate::PrivateKey) -> Result<(Vec<u8>, bool), Error> {
    let der_bytes = der_of(cms)?;
    let ci = ContentInfo::from_der(&der_bytes).map_err(|_| Error::Malformed("not encrypted mail"))?;
    let me = cert::parse(cert)?;
    match Envelope::of(&ci)? {
        Envelope::Plain(ed) => {
            let key = content_key(&ed.recip_infos, &me, id)?;
            open(&ed.encrypted_content, &key, None).map(|c| (c, false))
        }
        Envelope::Authenticated(aed) => {
            // Authenticated attributes would be covered by the tag as well; mail never has them.
            if aed.auth_attrs.is_some() {
                return Err(Error::Unsupported("authenticated attributes".into()));
            }
            let key = content_key(&aed.recip_infos, &me, id)?;
            open(&aed.auth_encrypted_content, &key, Some(aed.mac.as_bytes())).map(|c| (c, true))
        }
    }
}

/// What encrypted mail's `ContentInfo` holds.
enum Envelope {
    Plain(EnvelopedData),
    Authenticated(AuthEnvelopedData),
}

impl Envelope {
    fn of(ci: &ContentInfo) -> Result<Envelope, Error> {
        let unreadable = |_| Error::Malformed("the encrypted mail can't be read");
        if ci.content_type == oids::ENVELOPED_DATA {
            return ci.content.decode_as().map(Envelope::Plain).map_err(unreadable);
        }
        if ci.content_type == oids::AUTH_ENVELOPED_DATA {
            return ci.content.decode_as().map(Envelope::Authenticated).map_err(unreadable);
        }
        Err(Error::Malformed("not encrypted mail"))
    }

    fn recip_infos(&self) -> &RecipientInfos {
        match self {
            Envelope::Plain(ed) => &ed.recip_infos,
            Envelope::Authenticated(aed) => &aed.recip_infos,
        }
    }
}

/// Whether `cms` (encrypted mail) is addressed to the certificate `cert` (DER): one of its
/// recipient infos names it. Tells which of several keys to decrypt with without trying each
/// (a card's key would ask for its PIN for nothing).
pub fn addressed_to(cms: &[u8], cert: &[u8]) -> bool {
    let (Ok(der_bytes), Ok(me)) = (der_of(cms), cert::parse(cert)) else { return false };
    let Ok(ci) = ContentInfo::from_der(&der_bytes) else { return false };
    let Ok(envelope) = Envelope::of(&ci) else { return false };
    envelope.recip_infos().0.iter().any(|r| match r {
        RecipientInfo::Ktri(k) => names(&k.rid, &me),
        RecipientInfo::Kari(k) => k.recipient_enc_keys.iter().any(|e| crate::kari::names(&e.rid, &me)),
        _ => false,
    })
}

/// Whether `rid` names `me` (by issuer and serial number, or by subject key identifier).
fn names(rid: &RecipientIdentifier, me: &Certificate) -> bool {
    match rid {
        RecipientIdentifier::IssuerAndSerialNumber(i) => cert::is_named(me, &i.issuer, &i.serial_number),
        RecipientIdentifier::SubjectKeyIdentifier(s) => cert::has_key_id(me, s.0.as_bytes()),
    }
}

/// The content key from the recipient info that names `me`.
fn content_key(infos: &RecipientInfos, me: &Certificate, id: &mut dyn crate::PrivateKey) -> Result<Vec<u8>, Error> {
    // An EC key: the copy wrapped for it by key agreement.
    if me.tbs_certificate.subject_public_key_info.algorithm.oid == oids::EC_PUBLIC_KEY {
        for r in infos.0.iter() {
            if let RecipientInfo::Kari(k) = r
                && let Some(Ok(key)) = crate::kari::content_key(k, me, id)
            {
                return Ok(key);
            }
        }
        return Err(Error::Decrypt);
    }
    let mine: Vec<&KeyTransRecipientInfo> = infos
        .0
        .iter()
        .filter_map(|r| match r {
            RecipientInfo::Ktri(k) => Some(k),
            _ => None,
        })
        .filter(|k| names(&k.rid, me))
        .collect();
    if mine.is_empty() {
        return Err(Error::Decrypt);
    }
    for k in mine {
        let wrapped = k.enc_key.as_bytes();
        let key = if k.key_enc_alg.oid == oids::RSA_ENCRYPTION {
            id.rsa_decrypt(wrapped, None)
        } else if k.key_enc_alg.oid == oids::RSAES_OAEP {
            // A hash that isn't known isn't guessed at.
            let Some(hash) = cert::params_hash(&k.key_enc_alg) else { continue };
            id.rsa_decrypt(wrapped, Some(hash))
        } else {
            continue;
        };
        if let Ok(key) = key {
            return Ok(key);
        }
    }
    Err(Error::Decrypt)
}

/// The content of `eci` decrypted with `key`; `tag` is the GCM tag when it's authenticated.
fn open(eci: &EncryptedContentInfo, key: &[u8], tag: Option<&[u8]>) -> Result<Vec<u8>, Error> {
    use aes::cipher::{BlockDecryptMut as _, KeyIvInit as _, block_padding::Pkcs7};
    let sealed = eci.encrypted_content.as_ref().ok_or(Error::Malformed("the encrypted content is missing"))?.as_bytes();
    let alg = &eci.content_enc_alg;
    let iv = || alg.parameters.as_ref().and_then(|p| p.decode_as::<OctetString>().ok()).map(|o| o.as_bytes().to_vec()).ok_or(Error::Decrypt);
    let cbc = |len: usize| -> Result<Vec<u8>, Error> {
        let iv = iv()?;
        if key.len() != len {
            return Err(Error::Decrypt);
        }
        let out = match len {
            16 => cbc::Decryptor::<aes::Aes128>::new_from_slices(key, &iv).map_err(|_| Error::Decrypt)?.decrypt_padded_vec_mut::<Pkcs7>(sealed),
            24 => cbc::Decryptor::<aes::Aes192>::new_from_slices(key, &iv).map_err(|_| Error::Decrypt)?.decrypt_padded_vec_mut::<Pkcs7>(sealed),
            _ => cbc::Decryptor::<aes::Aes256>::new_from_slices(key, &iv).map_err(|_| Error::Decrypt)?.decrypt_padded_vec_mut::<Pkcs7>(sealed),
        };
        out.map_err(|_| Error::Decrypt)
    };
    let gcm = |len: usize| -> Result<Vec<u8>, Error> {
        use aes_gcm::aead::{Aead as _, KeyInit as _};
        let tag = tag.ok_or(Error::Decrypt)?;
        let params: GcmParameters = alg.parameters.as_ref().and_then(|p| p.decode_as().ok()).ok_or(Error::Decrypt)?;
        let nonce = params.nonce.as_bytes();
        if nonce.len() != 12 || tag.len() != 16 || params.icv_len.unwrap_or(12) != 16 || key.len() != len {
            return Err(Error::Unsupported("this kind of GCM encryption".into()));
        }
        let mut whole = sealed.to_vec();
        whole.extend_from_slice(tag);
        let nonce = aes_gcm::Nonce::from_slice(nonce);
        let out = match len {
            16 => aes_gcm::Aes128Gcm::new_from_slice(key).map_err(|_| Error::Decrypt)?.decrypt(nonce, whole.as_slice()),
            24 => aes_gcm::AesGcm::<aes::Aes192, aes_gcm::aead::consts::U12>::new_from_slice(key)
                .map_err(|_| Error::Decrypt)?
                .decrypt(nonce, whole.as_slice()),
            _ => aes_gcm::Aes256Gcm::new_from_slice(key).map_err(|_| Error::Decrypt)?.decrypt(nonce, whole.as_slice()),
        };
        out.map_err(|_| Error::Decrypt)
    };
    match alg.oid {
        oids::AES128_CBC if tag.is_none() => cbc(16),
        oids::AES192_CBC if tag.is_none() => cbc(24),
        oids::AES256_CBC if tag.is_none() => cbc(32),
        oids::AES128_GCM if tag.is_some() => gcm(16),
        oids::AES192_GCM if tag.is_some() => gcm(24),
        oids::AES256_GCM if tag.is_some() => gcm(32),
        oids::DES_EDE3_CBC if tag.is_none() => {
            let iv = iv()?;
            cbc::Decryptor::<des::TdesEde3>::new_from_slices(key, &iv)
                .map_err(|_| Error::Decrypt)?
                .decrypt_padded_vec_mut::<Pkcs7>(sealed)
                .map_err(|_| Error::Decrypt)
        }
        other => Err(Error::Unsupported(format!("encryption {other}"))),
    }
}
