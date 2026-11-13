//! A mail server on this computer with a certificate of its own (issue #77).

use super::*;

/// The refusal names the certificate: it's asked about once, trusting it is kept with the
/// account and starts the account again, and what isn't a fingerprint is refused.
#[test]
fn a_local_server_s_certificate_is_asked_about_once_then_trusted() {
    let mut s = Session::open(Some(temp("trust-certificate")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "riley@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    let print = "ab".repeat(32);
    let refused = SyncStatus::Error { error: format!("secure connection: {}", mailcraft_model::local_certificate_refusal("127.0.0.1", &print)) };
    s.apply(Event::Status { account: id.clone(), status: refused.clone() });
    assert_eq!(s.ask_certificate, [(id.clone(), "127.0.0.1".to_string(), print.clone())]);
    // Refused again as it tries again: asked once.
    s.ask_certificate.clear();
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::Connecting });
    s.apply(Event::Status { account: id.clone(), status: refused });
    assert!(s.ask_certificate.is_empty());
    // Another refusal isn't one to ask about.
    s.apply(Event::Status {
        account: id.clone(),
        status: SyncStatus::Error { error: "secure connection: invalid peer certificate: UnknownIssuer".into() },
    });
    assert!(s.ask_certificate.is_empty());

    // Trusted (typed in capitals, with room around it): kept, and the account starts again with it.
    s.run("account.trustCertificate", &json!({"account": id, "fingerprint": format!(" {} ", print.to_ascii_uppercase())})).unwrap();
    let trusted = |s: &Session| s.account(&id).map(|a| a.config.trusted_certificates.clone()).unwrap_or_default();
    assert_eq!(trusted(&s), std::slice::from_ref(&print));
    let jobs = s.take_jobs();
    assert!(
        jobs.iter().any(|j| matches!(j, Job::Start { account, .. } if account.id == id && account.trusted_certificates == [print.clone()])),
        "{jobs:?}"
    );
    let saved: Vec<mailcraft_model::AccountConfig> = s.store.as_ref().unwrap().load_accounts().unwrap();
    assert_eq!(saved.iter().find(|c| c.id == id).map(|c| c.trusted_certificates.clone()), Some(vec![print.clone()]), "kept for next time");
    // Trusted twice, it's there once; the newest few are kept.
    s.run("account.trustCertificate", &json!({"account": id, "fingerprint": print})).unwrap();
    assert_eq!(trusted(&s).len(), 1);
    for n in 0..12u8 {
        s.run("account.trustCertificate", &json!({"account": id, "fingerprint": format!("{n:02x}").repeat(32)})).unwrap();
    }
    let kept = trusted(&s);
    assert_eq!(kept.len(), mailcraft_model::MAX_TRUSTED_CERTIFICATES);
    assert_eq!(kept.last().map(String::as_str), Some("0b".repeat(32).as_str()));
    assert!(!kept.contains(&print), "the oldest gave way");

    for bad in [
        json!({"account": id}),
        json!({"account": id, "fingerprint": "abc"}),
        json!({"account": id, "fingerprint": "zz".repeat(32)}),
        json!({"account": "nobody", "fingerprint": print}),
        json!({"account": "sample", "fingerprint": print}),
        json!({"fingerprint": print}),
    ] {
        assert!(matches!(s.run("account.trustCertificate", &bad), Err(CmdError::Params(_))), "{bad}");
    }
    done(s);
}
