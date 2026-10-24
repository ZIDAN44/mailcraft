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

/// Lists the CA really signed that don't cover the certificate as a whole: not listing it there
/// means nothing (a man in the middle on the CRL's plain-HTTP address could serve one in place of
/// the whole list). A list scoped to the certificate's own distribution point still counts.
#[test]
fn a_partial_list_says_nothing() {
    let (ca, kim, lee) = (pem("ca.pem"), pem("rev-kim.pem"), pem("rev-lee.pem"));
    for name in ["rev-crl-delta.der", "rev-crl-partition.der", "rev-crl-elsewhere.der", "rev-crl-unknown.der"] {
        for cert in [&kim, &lee] {
            assert!(matches!(crl_status(&data(name), cert, &ca, NOW), Status::Unknown(_)), "{name}");
        }
    }
    let scoped = data("rev-crl-scoped.der");
    assert!(matches!(crl_status(&scoped, &kim, &ca, NOW), Status::Good { .. }));
    assert_eq!(crl_status(&scoped, &lee, &ca, NOW), Status::Revoked { at: LEE_REVOKED });
}

/// A responder the CA gave an OCSP-signing certificate answers for it, while that certificate is
/// in date; an expired one's answers (its key may have leaked since) count for nothing.
#[test]
fn a_delegated_responder_counts_only_in_date() {
    let (ca, kim, lee) = (pem("ca.pem"), pem("rev-kim.pem"), pem("rev-lee.pem"));
    assert!(matches!(ocsp_status(&data("rev-kim-ocsp-live.der"), &kim, &ca, NOW), Status::Good { .. }));
    assert_eq!(ocsp_status(&data("rev-lee-ocsp-live.der"), &lee, &ca, NOW), Status::Revoked { at: LEE_REVOKED });
    for (answer, cert) in [("rev-kim-ocsp-dead.der", &kim), ("rev-lee-ocsp-dead.der", &lee)] {
        assert!(matches!(ocsp_status(&data(answer), cert, &ca, NOW), Status::Unknown(_)), "{answer}");
    }
}

/// The "issuer" has to be the one that signed the certificate, whatever its name says.
#[test]
fn only_the_certificates_own_authority_answers() {
    let (ca, kim, lee) = (pem("ca.pem"), pem("rev-kim.pem"), pem("rev-lee.pem"));
    // Kim's certificate passed off as Lee's issuer: not the one that signed it.
    assert!(matches!(crl_status(&data("rev-ca-crl.der"), &lee, &kim, NOW), Status::Unknown(_)));
    assert!(matches!(ocsp_status(&data("rev-lee-ocsp.der"), &lee, &kim, NOW), Status::Unknown(_)));
    // And the real one still answers.
    assert_eq!(crl_status(&data("rev-ca-crl.der"), &lee, &ca, NOW), Status::Revoked { at: LEE_REVOKED });
}
