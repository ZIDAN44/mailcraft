//! Against OpenSSL 3's answers (`testdata/rev-*`, made with `openssl ca` and `openssl ocsp`): the
//! test CA issued Kim and Lee certificates naming a CRL and a responder, then revoked Lee's.

use super::*;

/// After the CA withdrew Lee's certificate (2026-10-24 14:00:01).
const NOW: i64 = 1_792_900_000;
const LEE_REVOKED: i64 = 1_792_850_401;

fn data(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn pem(name: &str) -> Vec<u8> {
    let text = String::from_utf8(data(name)).unwrap();
    der::pem::decode_vec(text.as_bytes()).unwrap().1
}

#[test]
fn a_certificate_says_where_to_ask() {
    let (crls, ocsp) = points(&pem("rev-kim.pem"));
    assert_eq!(crls, ["http://crl.test.example/ca.crl"]);
    assert_eq!(ocsp, ["http://ocsp.test.example/"]);
    assert_eq!(points(&pem("riley.pem")), (Vec::new(), Vec::new()), "nothing to ask");
    assert_eq!(points(b"junk"), (Vec::new(), Vec::new()));
}

#[test]
fn the_crl_withdraws_lee_and_not_kim() {
    let (ca, crl) = (pem("ca.pem"), data("rev-ca-crl.der"));
    assert!(matches!(crl_status(&crl, &pem("rev-kim.pem"), &ca, NOW), Status::Good { until: Some(_) }));
    assert_eq!(crl_status(&crl, &pem("rev-lee.pem"), &ca, NOW), Status::Revoked { at: LEE_REVOKED });
}

#[test]
fn a_crl_that_cant_be_trusted_says_nothing() {
    let (ca, lee) = (pem("ca.pem"), pem("rev-lee.pem"));
    let mut crl = data("rev-ca-crl.der");
    // Another authority's "issuer", a damaged signature, a list too old, a list from the future.
    assert!(matches!(crl_status(&crl, &lee, &pem("riley.pem"), NOW), Status::Unknown(_)));
    let last = crl.len() - 1;
    crl[last] ^= 1;
    assert!(matches!(crl_status(&crl, &lee, &ca, NOW), Status::Unknown(_)));
    let crl = data("rev-ca-crl.der");
    assert!(matches!(crl_status(&crl, &lee, &ca, NOW + 20 * 365 * 86_400), Status::Unknown(_)));
    assert!(matches!(crl_status(&crl, &lee, &ca, NOW - 86_400), Status::Unknown(_)));
    assert!(matches!(crl_status(b"junk", &lee, &ca, NOW), Status::Unknown(_)));
}

#[test]
fn the_responder_says_lee_is_withdrawn_and_kim_isnt() {
    let ca = pem("ca.pem");
    assert!(matches!(ocsp_status(&data("rev-kim-ocsp.der"), &pem("rev-kim.pem"), &ca, NOW), Status::Good { until: Some(_) }));
    assert_eq!(ocsp_status(&data("rev-lee-ocsp.der"), &pem("rev-lee.pem"), &ca, NOW), Status::Revoked { at: LEE_REVOKED });
}

#[test]
fn an_ocsp_answer_about_someone_else_or_tampered_says_nothing() {
    let (ca, lee) = (pem("ca.pem"), pem("rev-lee.pem"));
    // Kim's good answer replayed for Lee.
    assert!(matches!(ocsp_status(&data("rev-kim-ocsp.der"), &lee, &ca, NOW), Status::Unknown(_)));
    let mut answer = data("rev-lee-ocsp.der");
    let at = answer.len() / 2;
    answer[at] ^= 1;
    assert!(matches!(ocsp_status(&answer, &lee, &ca, NOW), Status::Unknown(_)));
    let answer = data("rev-lee-ocsp.der");
    assert!(matches!(ocsp_status(&answer, &lee, &pem("riley.pem"), NOW), Status::Unknown(_)));
    assert!(matches!(ocsp_status(&answer, &lee, &ca, NOW + 20 * 365 * 86_400), Status::Unknown(_)));
    assert!(matches!(ocsp_status(b"junk", &lee, &ca, NOW), Status::Unknown(_)));
}

#[test]
fn the_request_names_the_certificate_as_openssl_does() {
    let req = ocsp_request(&pem("rev-lee.pem"), &pem("ca.pem")).unwrap();
    assert_eq!(req, data("rev-lee-req.der"), "same CertID (SHA-1), no nonce");
    assert!(ocsp_request(b"junk", &pem("ca.pem")).is_err());
}
