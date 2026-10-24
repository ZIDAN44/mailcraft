//! Revocation: whether a certificate's authority withdrew it before it ran out (a stolen key, a
//! mistaken issue). The certificate says where to ask: a list of the ones withdrawn (CRL, RFC 5280
//! §5, its `cRLDistributionPoints`) or a responder that answers for one certificate (OCSP, RFC 6960,
//! its `authorityInfoAccess`). Nothing here fetches anything: the network layer does, and hands
//! the answer to [`crl_status`] or [`ocsp_status`], which check the authority signed it, it's for
//! this certificate, and it's current.
//!
//! An answer that can't be had or trusted is [`Status::Unknown`], never "revoked": the caller
//! says it wasn't checked (soft fail, as other mail programs do).

use der::asn1::ObjectIdentifier as Oid;
use der::{Decode, Encode};
use x509_cert::Certificate;
use x509_cert::crl::CertificateList;
use x509_cert::ext::pkix::name::{DistributionPointName, GeneralName};
use x509_cert::ext::pkix::{AuthorityInfoAccessSyntax, CrlDistributionPoints, ExtendedKeyUsage};
use x509_ocsp::{BasicOcspResponse, CertId, CertStatus, OcspRequest, OcspResponse, OcspResponseStatus, Request, TbsRequest};

use crate::cert::{self, Hash};
use crate::{Error, oids};

const CRL_POINTS: Oid = Oid::new_unwrap("2.5.29.31");
const AUTHORITY_INFO: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.1.1");
const OCSP_METHOD: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.48.1");
const OCSP_BASIC: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.48.1.1");
const OCSP_SIGNING: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.9");

/// Clocks differ: an answer this far in the future still counts as now.
const SKEW: i64 = 300;
/// An OCSP answer without a next update counts this long.
const OCSP_FRESH: i64 = 7 * 86_400;

/// What an authority says about a certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not withdrawn, as of the answer (until `until`, when it says).
    Good { until: Option<i64> },
    /// Withdrawn at `at`.
    Revoked { at: i64 },
    /// No answer to go by, and why.
    Unknown(String),
}

/// Where a certificate says to ask: its CRLs' and OCSP responders' web addresses.
pub fn points(cert_der: &[u8]) -> (Vec<String>, Vec<String>) {
    let Ok(c) = cert::parse(cert_der) else { return (Vec::new(), Vec::new()) };
    let ext =
        |oid: Oid| c.tbs_certificate.extensions.as_ref().and_then(|e| e.iter().find(|x| x.extn_id == oid)).map(|x| x.extn_value.as_bytes().to_vec());
    let web = |n: &GeneralName| match n {
        GeneralName::UniformResourceIdentifier(u) => {
            Some(u.as_str().trim().to_string()).filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        }
        _ => None,
    };
    let mut crls = Vec::new();
    if let Some(CrlDistributionPoints(points)) = ext(CRL_POINTS).and_then(|b| CrlDistributionPoints::from_der(&b).ok()) {
        for p in points {
            if let Some(DistributionPointName::FullName(names)) = &p.distribution_point {
                crls.extend(names.iter().filter_map(web));
            }
        }
    }
    let mut ocsp = Vec::new();
    if let Some(AuthorityInfoAccessSyntax(access)) = ext(AUTHORITY_INFO).and_then(|b| AuthorityInfoAccessSyntax::from_der(&b).ok()) {
        ocsp.extend(access.iter().filter(|a| a.access_method == OCSP_METHOD).filter_map(|a| web(&a.access_location)));
    }
    (crls, ocsp)
}

fn secs(t: &x509_cert::time::Time) -> i64 {
    i64::try_from(t.to_unix_duration().as_secs()).unwrap_or(i64::MAX)
}

/// What the CRL `crl` (DER) says about `cert`, issued by `issuer` (both DER), at `now`.
pub fn crl_status(crl: &[u8], cert_der: &[u8], issuer_der: &[u8], now: i64) -> Status {
    let (Ok(c), Ok(issuer)) = (cert::parse(cert_der), cert::parse(issuer_der)) else { return Status::Unknown("a certificate can't be read".into()) };
    let Ok(list) = CertificateList::from_der(crl) else { return Status::Unknown("the list of withdrawn certificates can't be read".into()) };
    let tbs = &list.tbs_cert_list;
    if tbs.issuer != issuer.tbs_certificate.subject || tbs.issuer != c.tbs_certificate.issuer {
        return Status::Unknown("the list isn't from the certificate's authority".into());
    }
    let signed = cert::signed_part(crl);
    let holds = signed.is_some_and(|s| {
        list.signature
            .as_bytes()
            .is_some_and(|sig| cert::signature_holds(&issuer.tbs_certificate.subject_public_key_info, &list.signature_algorithm, None, s, sig))
    });
    if !holds {
        return Status::Unknown("the list's signature doesn't hold".into());
    }
    if secs(&tbs.this_update) > now + SKEW {
        return Status::Unknown("the list is from the future".into());
    }
    let until = tbs.next_update.as_ref().map(secs);
    if until.is_some_and(|u| u < now) {
        return Status::Unknown("the list is out of date".into());
    }
    let revoked = tbs.revoked_certificates.iter().flatten().find(|r| r.serial_number == c.tbs_certificate.serial_number);
    match revoked {
        Some(r) => Status::Revoked { at: secs(&r.revocation_date) },
        None => Status::Good { until },
    }
}

/// An OCSP request (DER) for `cert`, issued by `issuer`.
pub fn ocsp_request(cert_der: &[u8], issuer_der: &[u8]) -> Result<Vec<u8>, Error> {
    let c = cert::parse(cert_der)?;
    let issuer = cert::parse(issuer_der)?;
    let id = CertId::from_cert::<sha1::Sha1>(&issuer, &c).map_err(|_| Error::Malformed("an OCSP request can't be made"))?;
    let req = OcspRequest {
        tbs_request: TbsRequest { request_list: vec![Request { req_cert: id, single_request_extensions: None }], ..Default::default() },
        optional_signature: None,
    };
    req.to_der().map_err(|_| Error::Malformed("an OCSP request can't be made"))
}

/// Whether `id` is `cert`'s, by the hash it was made with.
fn same_cert(id: &CertId, cert: &Certificate, issuer: &Certificate) -> bool {
    let ours = match Hash::from_oid(&id.hash_algorithm.oid) {
        Some(Hash::Sha1) => CertId::from_cert::<sha1::Sha1>(issuer, cert),
        Some(Hash::Sha256) => CertId::from_cert::<sha2::Sha256>(issuer, cert),
        Some(Hash::Sha384) => CertId::from_cert::<sha2::Sha384>(issuer, cert),
        Some(Hash::Sha512) => CertId::from_cert::<sha2::Sha512>(issuer, cert),
        None => return false,
    };
    ours.is_ok_and(|o| o.issuer_name_hash == id.issuer_name_hash && o.issuer_key_hash == id.issuer_key_hash && o.serial_number == id.serial_number)
}

/// What an OCSP answer (DER) says about `cert`, issued by `issuer`, at `now`. The answer is signed
/// by the authority itself, or by a responder it gave a certificate for OCSP signing.
pub fn ocsp_status(answer: &[u8], cert_der: &[u8], issuer_der: &[u8], now: i64) -> Status {
    let (Ok(c), Ok(issuer)) = (cert::parse(cert_der), cert::parse(issuer_der)) else { return Status::Unknown("a certificate can't be read".into()) };
    let Ok(resp) = OcspResponse::from_der(answer) else { return Status::Unknown("the OCSP answer can't be read".into()) };
    if resp.response_status != OcspResponseStatus::Successful {
        return Status::Unknown("the OCSP responder didn't answer".into());
    }
    let Some(bytes) = resp.response_bytes.filter(|b| b.response_type == OCSP_BASIC) else {
        return Status::Unknown("the OCSP answer is of another kind".into());
    };
    let basic_der = bytes.response.as_bytes();
    let Ok(basic) = BasicOcspResponse::from_der(basic_der) else { return Status::Unknown("the OCSP answer can't be read".into()) };
    let Some(signed) = cert::signed_part(basic_der) else { return Status::Unknown("the OCSP answer can't be read".into()) };
    let Some(sig) = basic.signature.as_bytes() else { return Status::Unknown("the OCSP answer can't be read".into()) };
    let by = |spki: &spki::SubjectPublicKeyInfoOwned| cert::signature_holds(spki, &basic.signature_algorithm, None, signed, sig);
    let delegated = basic.certs.iter().flatten().any(|r| {
        let Ok(r_der) = r.to_der() else { return false };
        let for_ocsp = r
            .tbs_certificate
            .extensions
            .as_ref()
            .and_then(|e| e.iter().find(|x| x.extn_id == oids::EXT_KEY_USAGE))
            .and_then(|x| ExtendedKeyUsage::from_der(x.extn_value.as_bytes()).ok())
            .is_some_and(|ExtendedKeyUsage(u)| u.contains(&OCSP_SIGNING));
        for_ocsp && cert::issued_by(&r_der, issuer_der) && by(&r.tbs_certificate.subject_public_key_info)
    });
    if !by(&issuer.tbs_certificate.subject_public_key_info) && !delegated {
        return Status::Unknown("the OCSP answer isn't signed by the certificate's authority".into());
    }
    let Some(single) = basic.tbs_response_data.responses.iter().find(|r| same_cert(&r.cert_id, &c, &issuer)) else {
        return Status::Unknown("the OCSP answer is about another certificate".into());
    };
    let this = i64::try_from(single.this_update.0.to_unix_duration().as_secs()).unwrap_or(i64::MAX);
    if this > now + SKEW {
        return Status::Unknown("the OCSP answer is from the future".into());
    }
    let until =
        single.next_update.map(|n| i64::try_from(n.0.to_unix_duration().as_secs()).unwrap_or(i64::MAX)).unwrap_or(this.saturating_add(OCSP_FRESH));
    if until < now {
        return Status::Unknown("the OCSP answer is out of date".into());
    }
    match &single.cert_status {
        CertStatus::Good(_) => Status::Good { until: Some(until) },
        CertStatus::Revoked(info) => Status::Revoked { at: i64::try_from(info.revocation_time.0.to_unix_duration().as_secs()).unwrap_or(0) },
        CertStatus::Unknown(_) => Status::Unknown("the OCSP responder doesn't know the certificate".into()),
    }
}

#[cfg(test)]
mod tests;
