//! Certificates: what one says (whose it is, who issued it, until when), whether a signature
//! made with its key holds, and whether it can be trusted for a sender's mail (RFC 8550 §3,
//! RFC 5280 §6): a chain to a trusted root, each certificate signed by the next and in date, for
//! email, and naming the address the message is from.

use der::asn1::ObjectIdentifier;
use der::{Decode, Encode, Tag, Tagged};
use sha2::Digest as _;
use x509_cert::Certificate;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{BasicConstraints, ExtendedKeyUsage, KeyUsage, KeyUsages, SubjectAltName};

use crate::{Error, oids};

/// What a certificate says, for showing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CertInfo {
    /// Whose it is: its common name, else its first address.
    pub name: String,
    /// The addresses it's for (its rfc822Name alternative names and its subject's emailAddress),
    /// lower-case.
    pub emails: Vec<String>,
    /// Who issued it: the issuer's common name, else organisation.
    pub issuer: String,
    /// Valid from and until (Unix seconds).
    pub not_before: i64,
    pub not_after: i64,
    /// SHA-256 of the certificate, as hex pairs (`AB:CD:…`).
    pub fingerprint: String,
    /// `RSA 2048`, `EC P-256`…
    pub key: String,
}

/// Why a certificate can't be trusted for a message (there may be several).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// No chain to a trusted root (self-signed, or issued by an authority this computer doesn't
    /// trust, or a certificate in between missing).
    UnknownIssuer,
    /// Out of date (the time it ran out), or not yet valid.
    Expired(i64),
    NotYetValid(i64),
    /// It's for other addresses than the message's sender.
    OtherAddress(Vec<String>),
    /// Its key may not be used for signing mail (key usage or extended key usage).
    NotForEmail,
    /// Its authority withdrew it, at this time ([`crate::revocation`]).
    Revoked(i64),
}

pub(crate) fn parse(der_bytes: &[u8]) -> Result<Certificate, Error> {
    Certificate::from_der(der_bytes).map_err(|_| Error::Malformed("a certificate can't be read"))
}

/// What `cert` (DER) says.
pub fn info(cert: &[u8]) -> Result<CertInfo, Error> {
    let c = parse(cert)?;
    let tbs = &c.tbs_certificate;
    let emails = emails(&c);
    let name = name_part(&tbs.subject, oids::COMMON_NAME).or_else(|| emails.first().cloned()).unwrap_or_default();
    let issuer = name_part(&tbs.issuer, oids::COMMON_NAME).or_else(|| name_part(&tbs.issuer, oids::ORGANIZATION)).unwrap_or_default();
    let secs = |t: &x509_cert::time::Time| i64::try_from(t.to_unix_duration().as_secs()).unwrap_or(i64::MAX);
    let fingerprint = sha2::Sha256::digest(cert).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
    Ok(CertInfo {
        name,
        emails,
        issuer,
        not_before: secs(&tbs.validity.not_before),
        not_after: secs(&tbs.validity.not_after),
        fingerprint,
        key: key_description(&c),
    })
}

/// The addresses a certificate is for, lower-case, each once.
pub(crate) fn emails(c: &Certificate) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(SubjectAltName(names)) = extension::<SubjectAltName>(c, oids::SUBJECT_ALT_NAME) {
        for n in names {
            if let GeneralName::Rfc822Name(a) = n {
                out.push(a.as_str().trim().to_lowercase());
            }
        }
    }
    for rdn in &c.tbs_certificate.subject.0 {
        for atv in rdn.0.iter().filter(|a| a.oid == oids::EMAIL_ADDRESS) {
            if let Some(t) = text(&atv.value) {
                out.push(t.trim().to_lowercase());
            }
        }
    }
    out.retain(|e| !e.is_empty());
    out.dedup();
    let mut seen = Vec::new();
    out.retain(|e| {
        let new = !seen.contains(e);
        seen.push(e.clone());
        new
    });
    out
}

fn extension<T: for<'a> Decode<'a>>(c: &Certificate, oid: ObjectIdentifier) -> Option<T> {
    let e = c.tbs_certificate.extensions.as_ref()?.iter().find(|e| e.extn_id == oid)?;
    T::from_der(e.extn_value.as_bytes()).ok()
}

/// The first value of an attribute in a name (`CN`, `O`…), as text.
fn name_part(name: &x509_cert::name::Name, oid: ObjectIdentifier) -> Option<String> {
    name.0.iter().flat_map(|rdn| rdn.0.iter()).find(|a| a.oid == oid).and_then(|a| text(&a.value)).filter(|t| !t.trim().is_empty())
}

/// A directory string as text, whichever string type it was written in.
fn text(v: &der::Any) -> Option<String> {
    let b = v.value();
    match v.tag() {
        Tag::Utf8String | Tag::PrintableString | Tag::Ia5String | Tag::VisibleString => Some(String::from_utf8_lossy(b).into_owned()),
        // T.61 in practice holds Latin-1.
        Tag::TeletexString => Some(b.iter().map(|c| char::from(*c)).collect()),
        Tag::BmpString => {
            let units: Vec<u16> =
                b.chunks(2).map(|p| u16::from_be_bytes([p.first().copied().unwrap_or(0), p.get(1).copied().unwrap_or(0)])).collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

fn key_description(c: &Certificate) -> String {
    let spki = &c.tbs_certificate.subject_public_key_info;
    if spki.algorithm.oid == oids::RSA_ENCRYPTION {
        let bits = public_rsa(spki).map(|k| rsa::traits::PublicKeyParts::size(&k) * 8).unwrap_or(0);
        return format!("RSA {bits}");
    }
    if spki.algorithm.oid == oids::EC_PUBLIC_KEY {
        let curve = spki.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<ObjectIdentifier>().ok());
        return match curve {
            Some(o) if o == oids::P256 => "EC P-256".into(),
            Some(o) if o == oids::P384 => "EC P-384".into(),
            _ => "EC".into(),
        };
    }
    spki.algorithm.oid.to_string()
}

pub(crate) fn public_rsa(spki: &spki::SubjectPublicKeyInfoOwned) -> Option<rsa::RsaPublicKey> {
    use rsa::pkcs1::DecodeRsaPublicKey as _;
    rsa::RsaPublicKey::from_pkcs1_der(spki.subject_public_key.raw_bytes()).ok()
}

/// The digests signatures are made over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    pub(crate) fn from_oid(o: &ObjectIdentifier) -> Option<Hash> {
        match *o {
            oids::SHA1 => Some(Hash::Sha1),
            oids::SHA256 => Some(Hash::Sha256),
            oids::SHA384 => Some(Hash::Sha384),
            oids::SHA512 => Some(Hash::Sha512),
            _ => None,
        }
    }

    pub(crate) fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            Hash::Sha1 => sha1::Sha1::digest(data).to_vec(),
            Hash::Sha256 => sha2::Sha256::digest(data).to_vec(),
            Hash::Sha384 => sha2::Sha384::digest(data).to_vec(),
            Hash::Sha512 => sha2::Sha512::digest(data).to_vec(),
        }
    }
}

/// Whether `signature` over `message` holds for the key in `spki`, made with the signature
/// algorithm `alg` (its OID and parameters); `digest` is the digest algorithm to use when `alg`
/// names only the key's kind (CMS writes `rsaEncryption` or `id-ecPublicKey` there).
pub(crate) fn signature_holds(
    spki: &spki::SubjectPublicKeyInfoOwned,
    alg: &spki::AlgorithmIdentifierOwned,
    digest: Option<Hash>,
    message: &[u8],
    signature: &[u8],
) -> bool {
    let named = match alg.oid {
        oids::SHA1_WITH_RSA | oids::ECDSA_WITH_SHA1 => Some(Hash::Sha1),
        oids::SHA256_WITH_RSA | oids::ECDSA_WITH_SHA256 => Some(Hash::Sha256),
        oids::SHA384_WITH_RSA | oids::ECDSA_WITH_SHA384 => Some(Hash::Sha384),
        oids::SHA512_WITH_RSA | oids::ECDSA_WITH_SHA512 => Some(Hash::Sha512),
        _ => None,
    };
    let is_pss = alg.oid == oids::RSASSA_PSS;
    let hash = if is_pss { pss_hash(alg) } else { named.or(digest) };
    let Some(hash) = hash else { return false };
    let hashed = hash.digest(message);
    let key = &spki.algorithm.oid;
    if *key == oids::RSA_ENCRYPTION {
        let Some(k) = public_rsa(spki) else { return false };
        let ok = match (is_pss, hash) {
            (false, Hash::Sha1) => k.verify(rsa::Pkcs1v15Sign::new::<sha1::Sha1>(), &hashed, signature),
            (false, Hash::Sha256) => k.verify(rsa::Pkcs1v15Sign::new::<sha2::Sha256>(), &hashed, signature),
            (false, Hash::Sha384) => k.verify(rsa::Pkcs1v15Sign::new::<sha2::Sha384>(), &hashed, signature),
            (false, Hash::Sha512) => k.verify(rsa::Pkcs1v15Sign::new::<sha2::Sha512>(), &hashed, signature),
            (true, Hash::Sha1) => k.verify(rsa::Pss::new::<sha1::Sha1>(), &hashed, signature),
            (true, Hash::Sha256) => k.verify(rsa::Pss::new::<sha2::Sha256>(), &hashed, signature),
            (true, Hash::Sha384) => k.verify(rsa::Pss::new::<sha2::Sha384>(), &hashed, signature),
            (true, Hash::Sha512) => k.verify(rsa::Pss::new::<sha2::Sha512>(), &hashed, signature),
        };
        return ok.is_ok();
    }
    if *key == oids::EC_PUBLIC_KEY {
        use signature::hazmat::PrehashVerifier as _;
        let curve = spki.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<ObjectIdentifier>().ok());
        let point = spki.subject_public_key.raw_bytes();
        return match curve {
            Some(c) if c == oids::P256 => {
                let (Ok(k), Ok(s)) = (p256::ecdsa::VerifyingKey::from_sec1_bytes(point), p256::ecdsa::Signature::from_der(signature)) else {
                    return false;
                };
                k.verify_prehash(&hashed, &s).is_ok()
            }
            Some(c) if c == oids::P384 => {
                let (Ok(k), Ok(s)) = (p384::ecdsa::VerifyingKey::from_sec1_bytes(point), p384::ecdsa::Signature::from_der(signature)) else {
                    return false;
                };
                k.verify_prehash(&hashed, &s).is_ok()
            }
            _ => false,
        };
    }
    false
}

/// The digest an RSASSA-PSS signature names in its parameters (SHA-1 when it names none,
/// RFC 4055 §3.1).
fn pss_hash(alg: &spki::AlgorithmIdentifierOwned) -> Option<Hash> {
    use der::Reader as _;
    // RSASSA-PSS-params ::= SEQUENCE { hashAlgorithm [0] EXPLICIT HashAlgorithm DEFAULT sha1, … }
    let Some(p) = alg.parameters.as_ref() else { return Some(Hash::Sha1) };
    let content = p.value();
    if content.first() != Some(&0xA0) {
        return Some(Hash::Sha1);
    }
    let mut r = der::SliceReader::new(content).ok()?;
    let explicit: der::Any = r.decode().ok()?;
    let alg = spki::AlgorithmIdentifierOwned::from_der(explicit.value()).ok()?;
    Hash::from_oid(&alg.oid)
}

/// Bytes of the `tbsCertificate` exactly as the issuer signed them (not re-encoded).
pub(crate) fn signed_part(cert: &[u8]) -> Option<&[u8]> {
    let (_, outer) = header(cert)?;
    let inner = cert.get(outer..)?;
    let (len, head) = header(inner)?;
    inner.get(..head.checked_add(len)?)
}

/// A DER element's content length and header length.
fn header(b: &[u8]) -> Option<(usize, usize)> {
    let len_byte = *b.get(1)?;
    if len_byte & 0x80 == 0 {
        return Some((usize::from(len_byte), 2));
    }
    let n = usize::from(len_byte & 0x7F);
    if n == 0 || n > 4 {
        return None;
    }
    let mut len = 0usize;
    for i in 0..n {
        len = (len << 8) | usize::from(*b.get(2 + i)?);
    }
    Some((len, 2 + n))
}

/// Which of two of one's own certificates (DER) signs and encrypts: one in date before one that
/// isn't, then the one issued later (a renewal), then the one that runs longer. The other is kept
/// to read mail sent to it.
pub fn newer(a: &[u8], b: &[u8], now: i64) -> bool {
    let rank = |c: &[u8]| info(c).ok().map(|i| (i.not_before <= now && now <= i.not_after, i.not_before, i.not_after));
    rank(a) > rank(b)
}

/// Whether a certificate (DER) is a root: issued by itself.
pub fn self_signed(cert: &[u8]) -> bool {
    parse(cert).is_ok_and(|c| same_name(&c.tbs_certificate.subject, &c.tbs_certificate.issuer))
}

/// Whether the certificate `issuer` (DER) signed `cert` (DER).
pub(crate) fn issued_by(cert: &[u8], issuer: &[u8]) -> bool {
    match (parse(cert), parse(issuer)) {
        (Ok(c), Ok(i)) => same_name(&i.tbs_certificate.subject, &c.tbs_certificate.issuer) && issued(cert, &c, &i),
        _ => false,
    }
}

/// Whether `issuer` signed `cert` (both DER).
fn issued(cert: &[u8], c: &Certificate, issuer: &Certificate) -> bool {
    let Some(tbs) = signed_part(cert) else { return false };
    let Some(sig) = c.signature.as_bytes() else { return false };
    signature_holds(&issuer.tbs_certificate.subject_public_key_info, &c.signature_algorithm, None, tbs, sig)
}

fn same_name(a: &x509_cert::name::Name, b: &x509_cert::name::Name) -> bool {
    matches!((a.to_der(), b.to_der()), (Ok(x), Ok(y)) if x == y)
}

fn is_ca(c: &Certificate) -> bool {
    extension::<BasicConstraints>(c, oids::BASIC_CONSTRAINTS).is_some_and(|b| b.ca)
}

pub(crate) fn in_date(c: &Certificate, now: i64) -> Result<(), Problem> {
    let v = &c.tbs_certificate.validity;
    let secs = |t: &x509_cert::time::Time| i64::try_from(t.to_unix_duration().as_secs()).unwrap_or(i64::MAX);
    if now < secs(&v.not_before) {
        return Err(Problem::NotYetValid(secs(&v.not_before)));
    }
    if now > secs(&v.not_after) {
        return Err(Problem::Expired(secs(&v.not_after)));
    }
    Ok(())
}

/// The longest chain looked for.
const CHAIN_MAX: usize = 8;

/// What's wrong with trusting `signer` (DER) for mail from `from` at `now`: a chain to one of
/// `roots` through `others` (the certificates the message carried), every certificate on it in
/// date and the ones above it authorities, made for email, and for that address. Empty: trusted.
pub fn check(signer: &[u8], others: &[Vec<u8>], roots: &[Vec<u8>], now: i64, from: &str) -> Result<Vec<Problem>, Error> {
    check_with_path(signer, others, roots, now, from).map(|(p, _)| p)
}

/// [`check`], and the chain it found: the signer first, each certificate's issuer after it, up to
/// the trusted root when there is one (what revocation is checked along).
pub fn check_with_path(signer: &[u8], others: &[Vec<u8>], roots: &[Vec<u8>], now: i64, from: &str) -> Result<(Vec<Problem>, Vec<Vec<u8>>), Error> {
    let leaf = parse(signer)?;
    let mut path = vec![signer.to_vec()];
    let mut problems = Vec::new();
    // For this sender's mail.
    let from = from.trim().to_lowercase();
    let addresses = emails(&leaf);
    if !addresses.contains(&from) {
        problems.push(Problem::OtherAddress(addresses));
    }
    let for_signing = extension::<KeyUsage>(&leaf, oids::KEY_USAGE)
        .is_none_or(|KeyUsage(u)| u.contains(KeyUsages::DigitalSignature) || u.contains(KeyUsages::NonRepudiation));
    let for_mail = extension::<ExtendedKeyUsage>(&leaf, oids::EXT_KEY_USAGE)
        .is_none_or(|ExtendedKeyUsage(u)| u.iter().any(|o| *o == oids::EMAIL_PROTECTION || *o == oids::ANY_EXTENDED_KEY_USAGE));
    if !for_signing || !for_mail {
        problems.push(Problem::NotForEmail);
    }
    if let Err(p) = in_date(&leaf, now) {
        problems.push(p);
    }
    // The chain: up, issuer by issuer, to a root this computer trusts.
    let pool: Vec<(&Vec<u8>, Certificate, bool)> = others
        .iter()
        .map(|d| (d, false))
        .chain(roots.iter().map(|d| (d, true)))
        .filter_map(|(d, root)| parse(d).ok().map(|c| (d, c, root)))
        .collect();
    let is_root = |d: &[u8]| roots.iter().any(|r| r.as_slice() == d);
    let mut current: (Vec<u8>, Certificate) = (signer.to_vec(), leaf);
    let mut trusted = is_root(signer);
    for _ in 0..CHAIN_MAX {
        if trusted {
            break;
        }
        let (der_now, c) = &current;
        let up = pool.iter().find(|(d, cand, _)| {
            d.as_slice() != der_now.as_slice() && same_name(&cand.tbs_certificate.subject, &c.tbs_certificate.issuer) && issued(der_now, c, cand)
        });
        let Some((d, cand, _)) = up else { break };
        // A root is one of `roots` however it was reached: messages often carry the root too.
        let root = is_root(d);
        if !root && !is_ca(cand) {
            break;
        }
        if let Err(p) = in_date(cand, now)
            && !problems.contains(&p)
        {
            problems.push(p);
        }
        trusted = root;
        path.push((*d).clone());
        current = ((*d).clone(), cand.clone());
    }
    if !trusted {
        problems.push(Problem::UnknownIssuer);
    }
    Ok((problems, path))
}
