//! Issue #105: a mail server elsewhere than this computer whose certificate no authority vouches
//! for, on real servers (never a real account). Run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_CERTIFICATES="172.28.128.1 <folder> 127.0.0.1:13143" \
//!   cargo test -p mailcraft-cli --test real_certificates -- --ignored --nocapture --test-threads 1
//! ```
//!
//! The first word is an address of this computer that isn't a loopback one (on Windows, the
//! host's address on WSL's network, which only this computer reaches), the servers' ports bound
//! to it alone. `<folder>` holds `ca.pem`, an authority made for the test, and `a/`, `b/`,
//! `hand/` and `old/`, each a `tls.crt` and `tls.key`, for four Dovecot 2.4 servers (any user,
//! password `pw`, its `ssl_server` certificate mounted at `/etc/dovecot/ssl`):
//! - `a` (IMAPS on 31993, IMAP with STARTTLS on 31143, POP3S on 31990 with `protocols { pop3 =
//!   yes }`, submissions on 31465) and `b` (IMAPS on 32993): each a certificate the authority
//!   issued for that address, with keys of their own;
//! - `hand` (IMAPS on 33993): one made by hand for `mail.home.test` alone, not that address;
//! - `old` (IMAPS on 34993): one for that address that ran out in 2021.
//!
//! `a`'s submission relays to a GreenMail (`submission_relay_host`) whose plain IMAP is the third
//! word, to see what was sent arrive.

#[path = "support/servers.rs"]
mod servers;

use std::time::{Duration, Instant};

use mailcraft_engine::model::{Event, FailureKind, SyncStatus, UntrustedCertificate};
use serde_json::json;
use servers::{App, Imap};

struct Setup {
    host: String,
    folder: std::path::PathBuf,
    sent_to: String,
}

fn setup() -> Option<Setup> {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_CERTIFICATES") else {
        eprintln!("MAILCRAFT_REAL_CERTIFICATES isn't set: skipped");
        return None;
    };
    let mut words = v.split_whitespace().map(str::to_string);
    let (host, folder, sent_to) = (words.next()?, words.next()?, words.next()?);
    Some(Setup { host, folder: folder.into(), sent_to })
}

/// A certificate's SHA-256, in hex, from its PEM file.
fn fingerprint(pem: &std::path::Path) -> String {
    let text = std::fs::read_to_string(pem).unwrap();
    let body: String = text.lines().filter(|l| !l.starts_with("-----")).collect();
    let der = mailcraft_engine::mime::decode_base64(&body).unwrap();
    mailcraft_engine::smime::cert::info(&der).unwrap().fingerprint.replace(':', "").to_ascii_lowercase()
}

/// A MailCraft with one account on `imap` (port, TLS from the start or by STARTTLS) and `a`'s
/// submissions.
fn account(s: &Setup, tag: &str, imap_port: u16, imap_security: &str) -> (App, String) {
    let mut app = App::open(&format!("certificates-{tag}"), "pw");
    let id = app.add(json!({
        "email": format!("riley.{tag}@home.test"), "password": "pw",
        "imapHost": s.host, "imapPort": imap_port, "imapSecurity": imap_security,
        "smtpHost": s.host, "smtpPort": 31465, "smtpSecurity": "tls",
    }));
    (app, id)
}

/// The account's next failure or its being online: the certificate it was refused for, if any.
fn outcome(app: &mut App, id: &str) -> Result<(), (String, Option<Box<UntrustedCertificate>>)> {
    let mut seen = None;
    app.pump(60, "online or refused", |_, e| match e {
        Event::Status { account, status } if account == id => match status {
            SyncStatus::Online { .. } => {
                seen = Some(Ok(()));
                true
            }
            SyncStatus::Error { error, kind } => {
                let shown = match kind {
                    FailureKind::UntrustedCertificate(c) => Some(c.clone()),
                    _ => None,
                };
                seen = Some(Err((error.clone(), shown)));
                true
            }
            _ => false,
        },
        _ => false,
    });
    seen.unwrap_or(Ok(()))
}

/// Not trusted yet: refused before signing in, the refusal saying what the certificate is, and
/// asked about once. Trusted by its fingerprint, the account syncs, over TLS from the start and
/// by STARTTLS; mail is sent through the same server's submission (its certificate trusted the
/// same way) and arrives.
#[test]
#[ignore = "needs servers elsewhere than loopback in MAILCRAFT_REAL_CERTIFICATES"]
fn real_a_server_elsewhere_is_asked_about_then_trusted() {
    let Some(s) = setup() else { return };
    let print = fingerprint(&s.folder.join("a").join("tls.crt"));
    let (mut app, id) = account(&s, "asked", 31993, "tls");
    let Err((error, Some(shown))) = outcome(&mut app, &id) else { panic!("not refused for its certificate") };
    eprintln!("refused: {error}");
    assert_eq!((shown.host.as_str(), shown.fingerprint.as_str(), shown.remote), (s.host.as_str(), print.as_str(), true));
    assert_eq!((shown.subject.as_str(), shown.issuer.as_str()), ("mail.home.test", "MailCraft Test Home CA"));
    assert!(shown.names.contains(&s.host) && shown.not_after > shown.not_before, "{shown:?}");
    assert_eq!(app.s.ask_certificate.len(), 1, "asked about once");

    app.s.run("account.trustCertificate", &json!({"account": id, "fingerprint": print})).unwrap();
    assert_eq!(outcome(&mut app, &id), Ok(()), "online once trusted");

    // By STARTTLS, the same certificate.
    app.s.run("account.update", &json!({"account": id, "imapHost": s.host, "imapPort": 31143, "imapSecurity": "starttls"})).unwrap();
    assert_eq!(outcome(&mut app, &id), Ok(()), "online by STARTTLS");

    // Sent through submissions, and arrived.
    let to = format!("kim.{}@home.test", std::process::id());
    let subject = format!("Through a trusted certificate {}", std::process::id());
    app.s.run("mail.new", &json!({"account": id})).unwrap();
    app.s.run("compose.update", &json!({"to": to, "subject": subject, "body": "Sent over TLS to a server of its own."})).unwrap();
    app.s.run("compose.send", &json!({})).unwrap();
    app.pump(60, "sent", |_, e| matches!(e, Event::Sent { .. }));
    let mut gm = Imap::connect(&s.sent_to, &to, "pw");
    let end = Instant::now() + Duration::from_secs(30);
    while !gm.subjects("INBOX").contains(&subject) {
        assert!(Instant::now() < end, "not delivered");
        std::thread::sleep(Duration::from_millis(500));
    }

    // A POP account on the same server, over POP3S: asked about, then trusted the same way.
    let pop = app.add(json!({
        "email": "riley.pop@home.test", "password": "pw", "pop": true,
        "popHost": s.host, "popPort": 31990, "popSecurity": "tls",
        "smtpHost": s.host, "smtpPort": 31465, "smtpSecurity": "tls",
    }));
    assert!(matches!(outcome(&mut app, &pop), Err((_, Some(c))) if c.fingerprint == print), "POP3S asked about");
    app.s.run("account.trustCertificate", &json!({"account": pop, "fingerprint": print})).unwrap();
    assert_eq!(outcome(&mut app, &pop), Ok(()), "POP3S online once trusted");
    eprintln!("trusted when asked: IMAPS, STARTTLS, POP3S and submissions on {}", s.host);
}

/// The authority's file trusted: the servers it issued certificates for are trusted, each with a
/// key of its own; its server's own file trusts that server alone. One trusted by fingerprint
/// alone, another server showing another certificate is asked about again.
#[test]
#[ignore = "needs servers elsewhere than loopback in MAILCRAFT_REAL_CERTIFICATES"]
fn real_certificate_files_are_trusted() {
    let Some(s) = setup() else { return };
    let (a, b) = (fingerprint(&s.folder.join("a").join("tls.crt")), fingerprint(&s.folder.join("b").join("tls.crt")));
    let (mut app, id) = account(&s, "files", 32993, "tls");
    assert!(matches!(outcome(&mut app, &id), Err((_, Some(_)))));
    // Another certificate trusted when asked: still asked, for this one.
    app.s.run("account.trustCertificate", &json!({"account": id, "fingerprint": a})).unwrap();
    let Err((_, Some(shown))) = outcome(&mut app, &id) else { panic!("b's certificate taken for a's") };
    assert_eq!(shown.fingerprint, b);

    // a's own file: still b refused.
    let path = s.folder.join("a").join("tls.crt");
    app.s.run("account.trustCertificateFile", &json!({"account": id, "path": path.to_string_lossy()})).unwrap();
    assert!(matches!(outcome(&mut app, &id), Err((_, Some(_)))), "a's file trusts b");
    // The authority's: online.
    let path = s.folder.join("ca.pem");
    app.s.run("account.trustCertificateFile", &json!({"account": id, "path": path.to_string_lossy()})).unwrap();
    assert_eq!(outcome(&mut app, &id), Ok(()), "online through the authority");
    // Forgotten: refused again.
    let authority = fingerprint(&path);
    app.s.run("account.forgetCertificate", &json!({"account": id, "fingerprint": authority})).unwrap();
    assert!(matches!(outcome(&mut app, &id), Err((_, Some(_)))), "the authority forgotten");
    eprintln!("certificate files: the authority's trusts both servers, a server's its own");
}

/// What anyone can check is checked, trusted or not, and nothing is asked: a certificate for
/// another name than the server's, or out of date, is refused saying so.
#[test]
#[ignore = "needs servers elsewhere than loopback in MAILCRAFT_REAL_CERTIFICATES"]
fn real_a_wrong_name_or_old_certificate_is_never_asked_about() {
    let Some(s) = setup() else { return };
    for (tag, port, why, file) in [("hand", 33993, "NotValidForName", "hand"), ("old", 34993, "Expired", "old")] {
        let (mut app, id) = account(&s, tag, port, "tls");
        let Err((error, None)) = outcome(&mut app, &id) else { panic!("{tag}: asked about, or taken") };
        assert!(error.contains(why), "{tag}: {error}");
        assert!(app.s.ask_certificate.is_empty(), "{tag}");
        // Trusted anyway, by its fingerprint and its file: still refused.
        let path = s.folder.join(file).join("tls.crt");
        app.s.run("account.trustCertificate", &json!({"account": id, "fingerprint": fingerprint(&path)})).unwrap();
        app.s.run("account.trustCertificateFile", &json!({"account": id, "path": path.to_string_lossy()})).unwrap();
        let Err((error, None)) = outcome(&mut app, &id) else { panic!("{tag}: taken once trusted") };
        assert!(error.contains(why), "{tag}: {error}");
        eprintln!("{tag}: {error}");
    }
}
