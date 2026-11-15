//! Issue #105: a mail server elsewhere than this computer whose certificate no authority vouches
//! for (a company's or a home server's), trusted as other mail apps let you: when asked, by its
//! fingerprint, or from a file (the server's certificate, or the authority that issued it).

use super::super::test_certs::{email_only, expired, hand_made, internal_ca, issuing_ca, server, via_issuing};
use super::super::tests::check;
use super::*;

/// What an account trusts: these fingerprints, and these certificates imported from files.
fn own(host: &str, here: bool, pinned: &[String], imported: &[CertificateDer<'static>], distrusted: &[CertificateDer<'static>]) -> Arc<Own> {
    let files: Vec<String> = imported.iter().map(|c| mailcraft_model::base64::encode(c)).collect();
    Own::new(check(vec![], distrusted), Trusted { certificates: pinned, authorities: &files }, host, here)
}

fn chain(certs: &[CertificateDer<'static>]) -> Vec<Vec<u8>> {
    certs.iter().map(|c| c.to_vec()).collect()
}

/// Not trusted yet: refused, the refusal saying what the certificate is (for whom, by whom, until
/// when, its fingerprint), so the user can be asked. Trusted by its fingerprint, or imported (the
/// server's own, or the authority above it), it's taken.
#[test]
fn a_server_elsewhere_is_asked_about_then_trusted() {
    let host = "mail.corp.test";
    let leaf = server();
    let print = fingerprint(&leaf);
    let asked = own(host, false, &[], &[], &[]);
    assert!(asked.check(&chain(std::slice::from_ref(&leaf))).is_err());
    let shown = asked.refused().unwrap_or_default();
    assert_eq!((shown.host.as_str(), shown.fingerprint.as_str(), shown.remote), (host, print.as_str(), true));
    assert_eq!((shown.subject.as_str(), shown.issuer.as_str()), ("mail.corp.test", "MailCraft Test Internal CA"));
    assert_eq!(shown.names, ["mail.corp.test", "127.0.0.1"]);
    assert!(shown.not_before > 1_700_000_000 && shown.not_after > 4_900_000_000, "{shown:?}");

    assert_eq!(own(host, false, std::slice::from_ref(&print), &[], &[]).check(&chain(std::slice::from_ref(&leaf))), Ok(()));
    assert_eq!(own(host, false, &[print.to_ascii_uppercase()], &[], &[]).check(&chain(std::slice::from_ref(&leaf))), Ok(()));
    assert_eq!(
        own(host, false, &[], std::slice::from_ref(&leaf), &[]).check(&chain(std::slice::from_ref(&leaf))),
        Ok(()),
        "the server's own, from a file"
    );
    assert_eq!(own(host, false, &[], &[internal_ca()], &[]).check(&chain(std::slice::from_ref(&leaf))), Ok(()), "its authority, from a file");
    // Through an intermediate the server sends along.
    let through = chain(&[via_issuing(), issuing_ca()]);
    assert_eq!(own(host, false, &[], &[internal_ca()], &[]).check(&through), Ok(()));
    // Another certificate trusted, or imported: asked about.
    let other = own(host, false, &[fingerprint(&via_issuing())], &[issuing_ca()], &[]);
    assert!(other.check(&chain(std::slice::from_ref(&leaf))).is_err());
    assert_eq!(other.refused().map(|s| s.fingerprint), Some(print));
}

/// What can be checked of a certificate nobody vouches for is checked on a server elsewhere,
/// trusted or not: it's for this server's name, and in date. Wrong there, nothing is asked: it's
/// for whoever runs the server to put right.
#[test]
fn a_server_elsewhere_must_be_named_and_in_date() {
    let leaf = server();
    let print = fingerprint(&leaf);
    for host in ["other.corp.test", "corp.test", "mail.corp.test.evil.example", "10.0.0.1"] {
        let check = own(host, false, std::slice::from_ref(&print), std::slice::from_ref(&leaf), &[]);
        let why = check.check(&chain(std::slice::from_ref(&leaf))).unwrap_err();
        assert_eq!(why, rustls::Error::InvalidCertificate(CertificateError::NotValidForName), "{host}");
        assert_eq!(check.refused(), None, "{host}");
    }
    // By the address it names, it's for that server.
    assert_eq!(own("127.0.0.1", false, std::slice::from_ref(&print), &[], &[]).check(&chain(std::slice::from_ref(&leaf))), Ok(()));
    // Out of date, trusted or imported: refused as such.
    let old = expired();
    let check = own("old.corp.test", false, &[fingerprint(&old)], std::slice::from_ref(&old), &[]);
    assert_eq!(check.check(&chain(std::slice::from_ref(&old))), Err(rustls::Error::InvalidCertificate(CertificateError::Expired)));
    assert_eq!(check.refused(), None);
    // On this computer (a bridge to a mail service, issue #77) it's the user's say alone.
    assert_eq!(own("localhost", true, &[fingerprint(&old)], &[], &[]).check(&chain(&[old])), Ok(()));
}

/// A certificate made by hand names its server in its common name alone, and says it's an
/// authority (as `openssl req -x509` makes one): trusted when asked, or from its file.
#[test]
fn a_hand_made_certificate_is_trusted_by_its_common_name() {
    let made = hand_made();
    assert_eq!(mailcraft_smime::cert::server_names(&made), ["home.lan"]);
    let asked = own("home.lan", false, &[], &[], &[]);
    assert!(asked.check(&chain(std::slice::from_ref(&made))).is_err());
    assert_eq!(asked.refused().map(|s| s.names), Some(vec!["home.lan".to_string()]));
    assert_eq!(own("Home.Lan", false, &[fingerprint(&made)], &[], &[]).check(&chain(std::slice::from_ref(&made))), Ok(()));
    assert_eq!(own("home.lan", false, &[], std::slice::from_ref(&made), &[]).check(&chain(std::slice::from_ref(&made))), Ok(()));
    let elsewhere = own("mail.home.lan", false, &[fingerprint(&made)], std::slice::from_ref(&made), &[]);
    assert!(elsewhere.check(&chain(&[made])).is_err());
}

/// A major mail service's server always shows a certificate an authority vouches for: one that
/// doesn't is someone in between, whatever the account trusts, and nothing is asked.
#[test]
fn a_major_service_s_server_is_never_trusted_by_hand() {
    for host in
        ["imap.gmail.com", "outlook.office365.com", "smtp.mail.yahoo.com", "imap.mail.me.com", "GMAIL.COM.", "imap.secureserver.net", "imap.mail.com"]
    {
        assert!(public_service(host), "{host}");
    }
    for host in ["mail.corp.test", "notgmail.com", "gmail.com.corp.test", "127.0.0.1", ""] {
        assert!(!public_service(host), "{host}");
    }
    let made = hand_made();
    // Named for it, as someone in between would make it.
    let check = own("gmail.com", false, &[fingerprint(&made)], std::slice::from_ref(&made), &[]);
    assert!(check.check(&chain(&[made])).is_err());
    assert_eq!(check.refused(), None);
}

/// A certificate with alternative names, none a server's, is for no server: its common name
/// counts only when it has none at all.
#[test]
fn a_common_name_counts_only_without_alternative_names() {
    let only = email_only();
    assert!(mailcraft_smime::cert::server_names(&only).is_empty());
    let check = own("mail.corp.test", false, &[fingerprint(&only)], std::slice::from_ref(&only), &[]);
    assert_eq!(check.check(&chain(std::slice::from_ref(&only))), Err(rustls::Error::InvalidCertificate(CertificateError::NotValidForName)));
}

/// What the system distrusts stays so, whatever the account trusts: an authority it distrusts,
/// imported (whether or not the server sends it along), and a chain ending at a root it
/// distrusts, which is refused, never asked about.
#[test]
fn an_authority_the_system_distrusts_is_never_trusted() {
    let leaf = server();
    let imported = own("mail.corp.test", false, &[], &[internal_ca()], &[internal_ca()]);
    assert_eq!(imported.check(&chain(std::slice::from_ref(&leaf))), Err(rustls::Error::InvalidCertificate(CertificateError::Revoked)));
    assert_eq!(imported.refused(), None);
    let asked = own("mail.corp.test", false, &[], &[], &[internal_ca()]);
    assert_eq!(asked.check(&chain(std::slice::from_ref(&leaf))), Err(rustls::Error::InvalidCertificate(CertificateError::Revoked)));
    assert_eq!(asked.refused(), None, "not asked about");
    let pinned = own("mail.corp.test", false, &[fingerprint(&leaf)], &[], &[internal_ca()]);
    assert!(pinned.check(&chain(std::slice::from_ref(&leaf))).is_err(), "nor taken trusted by hand");
    // Through the issuing CA, its root distrusted.
    let through = own("mail.corp.test", false, &[], &[issuing_ca()], &[internal_ca()]);
    assert_eq!(through.check(&chain(&[via_issuing(), issuing_ca()])), Err(rustls::Error::InvalidCertificate(CertificateError::Revoked)));
    // Something else distrusted: no matter.
    let other = own("mail.corp.test", false, &[], &[internal_ca()], &[hand_made()]);
    assert_eq!(other.check(&chain(std::slice::from_ref(&leaf))), Ok(()));
}

/// One the system distrusts stays refused, trusted or imported, and so does one an imported
/// authority issued through an intermediate the system distrusts.
#[test]
fn what_the_system_distrusts_stays_refused() {
    let leaf = server();
    let pinned = own("mail.corp.test", false, &[fingerprint(&leaf)], std::slice::from_ref(&leaf), std::slice::from_ref(&leaf));
    assert_eq!(pinned.check(&chain(&[leaf])), Err(rustls::Error::InvalidCertificate(CertificateError::Revoked)));
    let through = own("mail.corp.test", false, &[], &[internal_ca()], &[issuing_ca()]);
    assert_eq!(through.check(&chain(&[via_issuing(), issuing_ca()])), Err(rustls::Error::InvalidCertificate(CertificateError::Revoked)));
    assert_eq!(through.refused(), None);
}

/// Names as RFC 6125 matches them: the whole name in any case, a wildcard for one leftmost label
/// only, an address only as an address.
#[test]
fn names_match_as_the_rfc_says() {
    let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(names_host(&names(&["mail.corp.test"]), "Mail.Corp.Test."));
    assert!(names_host(&names(&["*.corp.test"]), "mail.corp.test"));
    assert!(!names_host(&names(&["*.corp.test"]), "a.mail.corp.test"));
    assert!(!names_host(&names(&["*.corp.test"]), "corp.test"));
    assert!(!names_host(&names(&["*.test"]), "corp.test"), "a wildcard over a whole top-level name");
    assert!(!names_host(&names(&["m*.corp.test"]), "mail.corp.test"));
    assert!(names_host(&names(&["10.0.0.1"]), "10.0.0.1"));
    assert!(names_host(&names(&["::1"]), "[::1]"));
    assert!(!names_host(&names(&["10.0.0.1"]), "10.0.0.2"));
    assert!(!names_host(&names(&["10.0.0.1"]), "mail.corp.test"));
    assert!(!names_host(&names(&["*.0.0.1"]), "10.0.0.1"), "never a wildcard for an address");
    assert!(!names_host(&[], "mail.corp.test"));
}
