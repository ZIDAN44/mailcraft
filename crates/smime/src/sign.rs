//! Signatures: `SignedData` (RFC 5652 §5) as S/MIME uses it (RFC 8551 §2.5, §3.5).
//!
//! What's signed is the DER of the signed attributes, never the content directly: the content's
//! digest is one of them. A signature without signed attributes is refused, and so is one whose
//! content-type attribute doesn't match the content, since a signature over attributes could
//! otherwise be passed off as one over some other content (Kopp et al. 2023).

use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{CertificateSet, DigestAlgorithmIdentifiers, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos};
use der::asn1::{ObjectIdentifier, OctetString, SetOfVec};
use der::{Any, Decode, Encode};
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;
use x509_cert::attr::Attribute;

use crate::cert::{self, Hash};
use crate::{Error, Identity, der_of, oids};

/// A signature that holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Signed {
    /// The signer's certificate (DER).
    pub signer: Vec<u8>,
    /// Every certificate the signature carried (the signer's among them): the chain to check it
    /// with, and what to encrypt to the signer with.
    pub certs: Vec<Vec<u8>>,
    /// When the signer's program says it signed (Unix seconds).
    pub signed_at: Option<i64>,
    /// The content encryption the signer says it reads (SMIMECapabilities), in its order.
    pub reads: Vec<ObjectIdentifier>,
    /// The digest was SHA-1, which no longer proves much.
    pub weak: bool,
}

impl Signed {
    /// Whether the signer reads AES-GCM (so mail to them can be authenticated).
    pub fn reads_gcm(&self) -> bool {
        self.reads.iter().any(|o| *o == oids::AES256_GCM || *o == oids::AES128_GCM)
    }
}

fn any(v: &impl Encode) -> Result<Any, Error> {
    let der = v.to_der().map_err(|_| Error::Malformed("can't be written"))?;
    Any::from_der(&der).map_err(|_| Error::Malformed("can't be written"))
}

fn attribute(oid: ObjectIdentifier, value: Any) -> Result<Attribute, Error> {
    let mut values = SetOfVec::new();
    values.insert(value).map_err(|_| Error::Malformed("can't be written"))?;
    Ok(Attribute { oid, values })
}

fn algorithm(oid: ObjectIdentifier) -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned { oid, parameters: None }
}

/// What MailCraft reads, best first, as SMIMECapabilities says it (RFC 8551 §2.5.2).
fn capabilities() -> Vec<AlgorithmIdentifierOwned> {
    [oids::AES256_GCM, oids::AES128_GCM, oids::AES256_CBC, oids::AES128_CBC].into_iter().map(algorithm).collect()
}

/// A time as a signing-time attribute: UTCTime through 2049, GeneralizedTime after (RFC 5652
/// §11.3).
fn signing_time(unix: i64) -> Result<Any, Error> {
    let secs = u64::try_from(unix.max(0)).unwrap_or(0);
    let dt = der::DateTime::from_unix_duration(std::time::Duration::from_secs(secs)).map_err(|_| Error::Malformed("the time can't be written"))?;
    let t = if dt.year() < 2050 {
        x509_cert::time::Time::UtcTime(der::asn1::UtcTime::from_date_time(dt).map_err(|_| Error::Malformed("the time can't be written"))?)
    } else {
        x509_cert::time::Time::GeneralTime(der::asn1::GeneralizedTime::from_date_time(dt))
    };
    any(&t)
}

/// A detached signature (`ContentInfo` of `SignedData`, DER) over `content` (the MIME part
/// exactly as it will be sent) by `id`, signed at `now`. The chain goes with it, its root left
/// out, so a reader can check it and later encrypt to the signer.
pub fn sign(content: &[u8], id: &Identity, now: i64, rng: &mut impl rand_core::CryptoRngCore) -> Result<Vec<u8>, Error> {
    sign_with(content, &id.chain, &mut crate::key::Software { key: &id.key, rng }, now)
}

/// [`sign`] with a key held elsewhere (a smart card, the system's keychain): `chain` is its
/// certificate first, then the ones that issued it.
pub fn sign_with(content: &[u8], chain: &[Vec<u8>], key: &mut dyn crate::PrivateKey, now: i64) -> Result<Vec<u8>, Error> {
    let leaf = cert::parse(chain.first().map(Vec::as_slice).unwrap_or_default())?;
    let digest = Hash::Sha256.digest(content);
    let mut attrs = SetOfVec::new();
    let put = |set: &mut SetOfVec<Attribute>, a: Attribute| set.insert(a).map_err(|_| Error::Malformed("can't be written"));
    put(&mut attrs, attribute(oids::CONTENT_TYPE, any(&oids::DATA)?)?)?;
    put(&mut attrs, attribute(oids::SIGNING_TIME, signing_time(now)?)?)?;
    put(&mut attrs, attribute(oids::MESSAGE_DIGEST, any(&OctetString::new(digest).map_err(|_| Error::Malformed("can't be written"))?)?)?)?;
    put(&mut attrs, attribute(oids::SMIME_CAPABILITIES, any(&capabilities())?)?)?;
    let signed_bytes = attrs.to_der().map_err(|_| Error::Malformed("can't be written"))?;
    let to_sign = Hash::Sha256.digest(&signed_bytes);
    let kind = leaf.tbs_certificate.subject_public_key_info.algorithm.oid;
    let signature_alg = if kind == oids::RSA_ENCRYPTION {
        // `rsaEncryption` with NULL parameters: what every reader expects in CMS.
        AlgorithmIdentifierOwned { oid: oids::RSA_ENCRYPTION, parameters: Some(Any::null()) }
    } else if kind == oids::EC_PUBLIC_KEY {
        algorithm(oids::ECDSA_WITH_SHA256)
    } else {
        return Err(Error::Unsupported("signing with this kind of key".into()));
    };
    let signature = key.sign_sha256(&to_sign)?;
    let signer = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: leaf.tbs_certificate.issuer.clone(),
            serial_number: leaf.tbs_certificate.serial_number.clone(),
        }),
        digest_alg: algorithm(oids::SHA256),
        signed_attrs: Some(attrs),
        signature_algorithm: signature_alg,
        signature: OctetString::new(signature).map_err(|_| Error::Malformed("can't be written"))?,
        unsigned_attrs: None,
    };
    let mut digests = SetOfVec::new();
    digests.insert(algorithm(oids::SHA256)).map_err(|_| Error::Malformed("can't be written"))?;
    let mut certs = SetOfVec::new();
    for d in chain {
        let c = Certificate::from_der(d).map_err(|_| Error::Malformed("a certificate can't be read"))?;
        // The root is the reader's to have; sending it proves nothing.
        if c.tbs_certificate.subject == c.tbs_certificate.issuer && chain.len() > 1 {
            continue;
        }
        certs.insert(CertificateChoices::Certificate(c)).map_err(|_| Error::Malformed("can't be written"))?;
    }
    let mut signers = SetOfVec::new();
    signers.insert(signer).map_err(|_| Error::Malformed("can't be written"))?;
    let sd = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: DigestAlgorithmIdentifiers::from(digests),
        encap_content_info: EncapsulatedContentInfo { econtent_type: oids::DATA, econtent: None },
        certificates: Some(CertificateSet(certs)),
        crls: None,
        signer_infos: SignerInfos(signers),
    };
    let ci = ContentInfo { content_type: oids::SIGNED_DATA, content: any(&sd)? };
    ci.to_der().map_err(|_| Error::Malformed("can't be written"))
}

/// The value of the attribute `oid` among `attrs`, when it has exactly one.
fn single(attrs: &SetOfVec<Attribute>, oid: ObjectIdentifier) -> Option<&Any> {
    let a = attrs.iter().find(|a| a.oid == oid)?;
    (a.values.len() == 1).then(|| a.values.iter().next()).flatten()
}

/// Checks the signature `cms` (BER or DER `ContentInfo` of `SignedData`). `detached` is the
/// content for a detached signature (the first part of a `multipart/signed`, as received); for
/// one that carries its content (`application/pkcs7-mime; smime-type=signed-data`), it's `None`
/// and the content comes back. The first signer whose signature holds is the one returned; none
/// holding is [`Error::BadSignature`].
pub fn verify(cms: &[u8], detached: Option<&[u8]>) -> Result<(Signed, Vec<u8>), Error> {
    let der_bytes = der_of(cms)?;
    let ci = ContentInfo::from_der(&der_bytes).map_err(|_| Error::Malformed("not a signature"))?;
    if ci.content_type != oids::SIGNED_DATA {
        return Err(Error::Malformed("not a signature"));
    }
    let sd: SignedData = ci.content.decode_as().map_err(|_| Error::Malformed("the signature can't be read"))?;
    let carried = sd.encap_content_info.econtent.as_ref().map(|c| c.value().to_vec());
    let content: Vec<u8> = match (detached, carried) {
        (Some(d), _) => d.to_vec(),
        (None, Some(c)) => c,
        (None, None) => return Err(Error::Malformed("the signed content is missing")),
    };
    let certs: Vec<(Vec<u8>, Certificate)> = sd
        .certificates
        .iter()
        .flat_map(|s| s.0.iter())
        .filter_map(|c| match c {
            CertificateChoices::Certificate(c) => c.to_der().ok().map(|d| (d, c.clone())),
            CertificateChoices::Other(_) => None,
        })
        .collect();
    let mut seen_signer = false;
    for si in sd.signer_infos.0.iter() {
        let Some((signer_der, signer)) = certs.iter().find(|(_, c)| identifies(&si.sid, c)) else { continue };
        seen_signer = true;
        let Some(attrs) = &si.signed_attrs else { continue };
        let Some(hash) = Hash::from_oid(&si.digest_alg.oid) else { continue };
        let type_ok =
            single(attrs, oids::CONTENT_TYPE).and_then(|v| v.decode_as::<ObjectIdentifier>().ok()) == Some(sd.encap_content_info.econtent_type);
        let digest_ok = single(attrs, oids::MESSAGE_DIGEST)
            .and_then(|v| v.decode_as::<OctetString>().ok())
            .is_some_and(|d| d.as_bytes() == hash.digest(&content));
        if !type_ok || !digest_ok {
            continue;
        }
        let Ok(signed_bytes) = attrs.to_der() else { continue };
        if !cert::signature_holds(
            &signer.tbs_certificate.subject_public_key_info,
            &si.signature_algorithm,
            Some(hash),
            &signed_bytes,
            si.signature.as_bytes(),
        ) {
            continue;
        }
        let signed_at = single(attrs, oids::SIGNING_TIME)
            .and_then(|v| v.to_der().ok())
            .and_then(|d| x509_cert::time::Time::from_der(&d).ok())
            .and_then(|t| i64::try_from(t.to_unix_duration().as_secs()).ok());
        let reads = single(attrs, oids::SMIME_CAPABILITIES)
            .and_then(|v| v.decode_as::<Vec<AlgorithmIdentifierOwned>>().ok())
            .map(|v| v.into_iter().map(|a| a.oid).collect())
            .unwrap_or_default();
        let signed =
            Signed { signer: signer_der.clone(), certs: certs.iter().map(|(d, _)| d.clone()).collect(), signed_at, reads, weak: hash == Hash::Sha1 };
        return Ok((signed, content));
    }
    if !seen_signer {
        return Err(Error::Malformed("the signer's certificate isn't with the signature"));
    }
    Err(Error::BadSignature)
}

/// The content a signature carries (`smime-type=signed-data`), whether or not the signature holds:
/// a message whose signature is broken is still shown, saying so.
pub fn carried(cms: &[u8]) -> Result<Vec<u8>, Error> {
    let der_bytes = der_of(cms)?;
    let ci = ContentInfo::from_der(&der_bytes).map_err(|_| Error::Malformed("not a signature"))?;
    if ci.content_type != oids::SIGNED_DATA {
        return Err(Error::Malformed("not a signature"));
    }
    let sd: SignedData = ci.content.decode_as().map_err(|_| Error::Malformed("the signature can't be read"))?;
    sd.encap_content_info.econtent.map(|c| c.value().to_vec()).ok_or(Error::Malformed("the signed content is missing"))
}

/// Whether `sid` names `c` (by issuer and serial number, or by subject key identifier).
pub(crate) fn identifies(sid: &SignerIdentifier, c: &Certificate) -> bool {
    match sid {
        SignerIdentifier::IssuerAndSerialNumber(i) => i.issuer == c.tbs_certificate.issuer && i.serial_number == c.tbs_certificate.serial_number,
        SignerIdentifier::SubjectKeyIdentifier(k) => key_id(c).is_some_and(|id| id == k.0.as_bytes()),
    }
}

/// A certificate's subject key identifier.
pub(crate) fn key_id(c: &Certificate) -> Option<Vec<u8>> {
    let e = c.tbs_certificate.extensions.as_ref()?.iter().find(|e| e.extn_id == oids::SUBJECT_KEY_ID)?;
    OctetString::from_der(e.extn_value.as_bytes()).ok().map(|o| o.as_bytes().to_vec())
}
