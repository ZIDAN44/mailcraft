//! A mail server's certificate no authority vouches for: on this computer (issue #77), or a
//! company's or home server's elsewhere (issue #105), trusted when asked or from a file.

use super::*;
use mailcraft_model::UntrustedCertificate;

/// A refusal of this certificate, from this server.
fn shown(host: &str, fingerprint: &str) -> UntrustedCertificate {
    UntrustedCertificate { host: host.into(), fingerprint: fingerprint.into(), ..UntrustedCertificate::default() }
}

/// The refusal names the certificate: it's asked about once, trusting it is kept with the
/// account and starts the account again, and what isn't a fingerprint is refused.
#[test]
fn a_local_server_s_certificate_is_asked_about_once_then_trusted() {
    let mut s = Session::open(Some(temp("trust-certificate")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "riley@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    let print = "ab".repeat(32);
    let said = format!("secure connection: {}", shown("127.0.0.1", &print).refusal());
    let kind = FailureKind::UntrustedCertificate(Box::new(shown(" 127.0.0.1 ", &print.to_ascii_uppercase())));
    let refused = SyncStatus::Error { error: said.clone(), kind: kind.clone() };
    s.apply(Event::Status { account: id.clone(), status: refused.clone() });
    assert_eq!(s.ask_certificate, [(id.clone(), shown("127.0.0.1", &print))]);
    // Refused again as it tries again, or as it sends: asked once.
    s.ask_certificate.clear();
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::Connecting });
    s.apply(Event::Status { account: id.clone(), status: refused });
    s.apply(Event::Queued { account: id.clone(), id: 9, error: said.clone(), kind });
    assert!(s.ask_certificate.is_empty());
    // Another refusal isn't one to ask about, nor one that only has its words, nor one whose
    // fingerprint isn't one.
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::error("secure connection: invalid peer certificate: UnknownIssuer") });
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::error(said.replace(&print, &"cd".repeat(32))) });
    let odd = FailureKind::UntrustedCertificate(Box::new(shown("127.0.0.1", "zz")));
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::Error { error: said, kind: odd } });
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
}

/// A company's own authority, as its IT hands it out (MailCraft's test one; its key was thrown
/// away), and a server's certificate made by hand for `home.lan`.
const AUTHORITY: &str = "-----BEGIN CERTIFICATE-----
MIIBsDCCAVegAwIBAgIUWtdF1YzkvG0MciS3AbXMuleseWcwCgYIKoZIzj0EAwIw
JTEjMCEGA1UEAwwaTWFpbENyYWZ0IFRlc3QgSW50ZXJuYWwgQ0EwIBcNMjYxMTA0
MjAwNjU3WhgPMjEyNjEwMTEyMDA2NTdaMCUxIzAhBgNVBAMMGk1haWxDcmFmdCBU
ZXN0IEludGVybmFsIENBMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE51BeS15b
1cjK0hRCAhWrVfTQ5VZLW5tmZ7fP+cyQMuuflHYb2Wxl+JYlmFNV1JwVVBrSi/3/
rOSlYQNQLzpO7KNjMGEwHQYDVR0OBBYEFIP4fiWnxPcuij9xrsQhdlpxi/ABMB8G
A1UdIwQYMBaAFIP4fiWnxPcuij9xrsQhdlpxi/ABMA8GA1UdEwEB/wQFMAMBAf8w
DgYDVR0PAQH/BAQDAgIEMAoGCCqGSM49BAMCA0cAMEQCIHrpZf/NDu6lDDEpmWrd
lE4owBPA/KRXrV5IK/KDrNYSAiAfXXrCofoIUXVYSb3XOrQALvtYyCn5BAs6gtaJ
V78zWQ==
-----END CERTIFICATE-----";

const HAND_MADE: &str = "-----BEGIN CERTIFICATE-----
MIIBfTCCASOgAwIBAgIUPU1r+e+KwYCaKp01AKtQukNtuT0wCgYIKoZIzj0EAwIw
EzERMA8GA1UEAwwIaG9tZS5sYW4wIBcNMjYxMTE1MjExNTM5WhgPMjEyNjEwMjIy
MTE1MzlaMBMxETAPBgNVBAMMCGhvbWUubGFuMFkwEwYHKoZIzj0CAQYIKoZIzj0D
AQcDQgAEjb5bZifajgwQzypNdk/R5yVNilawdn8VkEVHLTVtmYVWWkg/mOPfdwBW
/+QN300tNlW6m28zmjSy6FtG+yPMlqNTMFEwHQYDVR0OBBYEFJOK5+J1UnapI8C2
txuPEFIP8yVOMB8GA1UdIwQYMBaAFJOK5+J1UnapI8C2txuPEFIP8yVOMA8GA1Ud
EwEB/wQFMAMBAf8wCgYIKoZIzj0EAwIDSAAwRQIhAO8WcO4D8DmWac9XDvbS+IhX
LIYgYnSA4eRWoFxUcL+sAiAMpfW2k21IPyQJY/wKLkIZzIJnZdDt00jrpFpKUPaR
hA==
-----END CERTIFICATE-----";

/// Issue #105: certificates imported from a file (PEM, one or several, or DER) are kept with the
/// account, which starts again with them; shown by whom they're for; each once; forgotten by
/// fingerprint, as one trusted when asked is. What isn't a certificate, or an account with no
/// mail servers of its own, is refused.
#[test]
fn certificates_from_a_file_are_trusted_and_forgotten() {
    let mut s = Session::open(Some(temp("trust-file")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "riley@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    let both = format!("issued by IT\r\n{AUTHORITY}\r\n{HAND_MADE}\n");
    let told = s.run("account.trustCertificateFile", &json!({"account": id, "data": both.as_bytes()})).unwrap();
    assert_eq!(told["trusted"][0]["name"], "MailCraft Test Internal CA");
    assert_eq!(told["trusted"][1]["name"], "home.lan");
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::Start { account, .. } if account.id == id && account.trusted_authorities.len() == 2)), "{jobs:?}");
    let shown = s.trusted_certificates(&id);
    let names: Vec<_> = shown.iter().map(|c| c.name.clone()).collect();
    assert_eq!(
        names,
        [
            Some(("MailCraft Test Internal CA".to_string(), "MailCraft Test Internal CA".to_string())),
            Some(("home.lan".to_string(), "home.lan".to_string()))
        ]
    );
    let saved: Vec<mailcraft_model::AccountConfig> = s.store.as_ref().unwrap().load_accounts().unwrap();
    assert_eq!(saved.iter().find(|c| c.id == id).map(|c| c.trusted_authorities.len()), Some(2), "kept for next time");

    // The same file again, or the authority alone as DER: each kept once.
    s.run("account.trustCertificateFile", &json!({"account": id, "data": both.as_bytes()})).unwrap();
    let der = mailcraft_mime::decode_base64(AUTHORITY.lines().filter(|l| !l.starts_with("-----")).collect::<String>().as_str()).unwrap();
    s.run("account.trustCertificateFile", &json!({"account": id, "data": der})).unwrap();
    assert_eq!(s.trusted_certificates(&id).len(), 2);

    // Forgotten by fingerprint, in any case, as one trusted when asked is.
    let authority = shown[0].fingerprint.clone();
    s.run("account.trustCertificate", &json!({"account": id, "fingerprint": "cd".repeat(32)})).unwrap();
    assert_eq!(s.trusted_certificates(&id).len(), 3);
    s.take_jobs();
    s.run("account.forgetCertificate", &json!({"account": id, "fingerprint": authority.to_ascii_uppercase()})).unwrap();
    s.run("account.forgetCertificate", &json!({"account": id, "fingerprint": "cd".repeat(32)})).unwrap();
    let left: Vec<String> = s.trusted_certificates(&id).into_iter().map(|c| c.fingerprint).collect();
    assert_eq!(left, [shown[1].fingerprint.clone()]);
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Start { account, .. } if account.id == id)), "starts again without them");
    assert!(matches!(s.run("account.forgetCertificate", &json!({"account": id, "fingerprint": authority})), Err(CmdError::Params(_))));
    // One kept that can't be read (the accounts' file changed by hand): not shown, and taken
    // back by its bytes' fingerprint.
    let mut config = s.account(&id).map(|a| a.config.clone()).unwrap();
    config.trusted_authorities.push(mailcraft_model::base64::encode(b"not a certificate"));
    s.update_account(config, None).unwrap();
    assert_eq!(s.trusted_certificates(&id).len(), 1);
    let junk = mailcraft_smime::cert::fingerprint(b"not a certificate");
    s.run("account.forgetCertificate", &json!({"account": id, "fingerprint": junk})).unwrap();
    assert_eq!(s.account(&id).map(|a| a.config.trusted_authorities.len()), Some(1));

    for bad in [
        json!({"account": id, "data": b"no certificate here".to_vec()}),
        json!({"account": id, "data": b"-----BEGIN CERTIFICATE-----\nnot base64!\n-----END CERTIFICATE-----".to_vec()}),
        json!({"account": id, "data": Vec::<u8>::new()}),
        json!({"account": id, "data": "-----BEGIN CERTIFICATE-----
MIIB".repeat(10_000).into_bytes()}),
        json!({"account": id, "data": format!("-----BEGIN CERTIFICATE-----
{}
-----END CERTIFICATE-----", "A".repeat(40_000)).into_bytes()}),
        json!({"account": "sample", "data": AUTHORITY.as_bytes()}),
        json!({"account": "nobody", "data": AUTHORITY.as_bytes()}),
        json!({"account": id}),
    ] {
        assert!(s.run("account.trustCertificateFile", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.trusted_certificates(&id).len(), 1);
}

/// A JMAP account connects to web addresses alone, checked as every web address is: nothing to
/// trust by hand there.
#[test]
fn an_account_on_web_addresses_trusts_nothing_by_hand() {
    let mut s = Session::open(Some(temp("trust-web")), mailcraft_model::dates::Clock::new(NOW, 0));
    let added = s.run("account.add", &json!({"email": "riley@studio.example", "password": "pw", "jmap": "https://jmap.studio.example/session"}));
    let id = added.unwrap()["account"].as_str().unwrap().to_string();
    assert!(matches!(s.run("account.trustCertificate", &json!({"account": id, "fingerprint": "ab".repeat(32)})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("account.trustCertificateFile", &json!({"account": id, "data": AUTHORITY.as_bytes()})), Err(CmdError::Params(_))));
}
