//! Sending over TLS to a server whose certificates come from an internal CA: trusted when the
//! system trusts the CA, and refused, before the password goes out, when the system distrusts
//! the server's certificate or the intermediate CA it sends.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread::JoinHandle;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConnection, StreamOwned};

use super::*;
use crate::stream::trust::test_certs::{internal_ca, issuing_ca, server_identity, via_issuing, via_issuing_identity};
use crate::stream::trust::tests::check;
use crate::stream::trust::{Distrusted, usable_roots};

type Identity = (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>);

/// One line from the client, a byte at a time so nothing of a TLS handshake after it is read.
fn read_line(s: &mut impl Read) -> Option<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.last() != Some(&b'\n') {
        match s.read(&mut byte) {
            Ok(1) => line.push(byte[0]),
            _ => return None,
        }
    }
    Some(String::from_utf8_lossy(&line).into_owned())
}

/// The SMTP dialogue after the greeting: every command is logged; the message is accepted.
fn converse(s: &mut (impl Read + Write), log: &mut String) {
    let mut in_data = false;
    while let Some(line) = read_line(s) {
        log.push_str(&line);
        let cmd = line.trim_end().to_ascii_uppercase();
        let reply: &[u8] = if in_data {
            if cmd != "." {
                continue;
            }
            in_data = false;
            b"250 queued\r\n"
        } else if cmd.starts_with("EHLO") {
            b"250-test\r\n250 AUTH PLAIN LOGIN\r\n"
        } else if cmd.starts_with("AUTH") {
            b"235 ok\r\n"
        } else if cmd == "DATA" {
            in_data = true;
            b"354 go\r\n"
        } else if cmd == "QUIT" {
            let _ = s.write_all(b"221 bye\r\n");
            break;
        } else {
            b"250 ok\r\n"
        };
        if s.write_all(reply).and_then(|()| s.flush()).is_err() {
            break;
        }
    }
}

/// An SMTP server on 127.0.0.1 presenting `identity`, over implicit TLS or STARTTLS, for one
/// connection. What the client said, once it's gone.
fn serve(identity: Identity, starttls: bool) -> (u16, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let (chain, key) = identity;
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .unwrap();
    let handle = std::thread::spawn(move || {
        let mut log = String::new();
        let Ok((mut tcp, _)) = listener.accept() else { return log };
        if starttls {
            if tcp.write_all(b"220 test ESMTP\r\n").is_err() {
                return log;
            }
            loop {
                let Some(line) = read_line(&mut tcp) else { return log };
                log.push_str(&line);
                let cmd = line.trim_end().to_ascii_uppercase();
                let reply: &[u8] = if cmd.starts_with("EHLO") { b"250-test\r\n250 STARTTLS\r\n" } else { b"220 go ahead\r\n" };
                if tcp.write_all(reply).is_err() {
                    return log;
                }
                if cmd == "STARTTLS" {
                    break;
                }
            }
        }
        let Ok(conn) = ServerConnection::new(Arc::new(config)) else { return log };
        let mut tls = StreamOwned::new(conn, tcp);
        if !starttls && tls.write_all(b"220 test ESMTP\r\n").is_err() {
            return log;
        }
        converse(&mut tls, &mut log);
        log
    });
    (port, handle)
}

/// Sends one message to such a server, the system trusting `store` less `distrusted`, as
/// [`send_with`] does with the system's own. Its result, and what the server heard.
fn send_to(
    identity: Identity,
    security: Security,
    store: Vec<CertificateDer<'static>>,
    distrusted: &[CertificateDer<'static>],
) -> (Result<()>, String) {
    let (port, server) = serve(identity, security == Security::StartTls);
    let set: Distrusted = distrusted.iter().map(|c| c.to_vec()).collect();
    let roots = usable_roots(store.clone(), &set);
    let trust = Trust { roots: &roots, verifier: check(store, distrusted) };
    let config = ServerConfig { host: "127.0.0.1".into(), port, security };
    let envelope = Envelope::new(Some("ana@corp.test".parse().unwrap()), vec!["ben@corp.test".parse().unwrap()]).unwrap();
    let message = b"Subject: hi\r\n\r\nhello\r\n";
    let sign_in = SignIn { user: "ana@corp.test", password: "made-up", oauth: false };
    let result = submit(&config, sign_in, &envelope, message, &["ben@corp.test".into()], &trust);
    (result, server.join().unwrap())
}

fn refusal_text(result: Result<()>) -> String {
    match result {
        Ok(()) => String::from("sent"),
        Err(e) => e.to_string(),
    }
}

/// The server's certificates come from the company's own CA, which the system trusts: the
/// message goes, over implicit TLS and STARTTLS, directly or through an intermediate CA.
#[test]
fn sends_to_a_server_whose_ca_the_system_trusts() {
    for security in [Security::Tls, Security::StartTls] {
        for identity in [server_identity(), via_issuing_identity()] {
            let (result, heard) = send_to(identity, security, vec![internal_ca()], &[]);
            assert!(result.is_ok(), "{security:?}: {}", refusal_text(result));
            assert!(heard.contains("AUTH PLAIN") && heard.contains("DATA"), "{security:?}: {heard}");
        }
    }
}

/// A server or intermediate certificate the system distrusts is refused after the handshake,
/// before signing in: the server never sees the password or the message.
#[test]
fn a_distrusted_server_or_intermediate_never_gets_the_password() {
    for security in [Security::Tls, Security::StartTls] {
        for distrusted in [via_issuing(), issuing_ca()] {
            let (result, heard) = send_to(via_issuing_identity(), security, vec![internal_ca()], std::slice::from_ref(&distrusted));
            let why = refusal_text(result);
            assert!(why.contains("Revoked"), "{security:?}: {why}");
            assert!(!heard.contains("AUTH") && !heard.contains("MAIL FROM"), "{security:?}: {heard}");
        }
    }
}

/// A CA the system doesn't trust, or distrusts though it's among the roots: refused in the
/// handshake, as before.
#[test]
fn a_ca_the_system_does_not_trust_is_refused() {
    for (store, distrusted) in [(vec![], vec![]), (vec![internal_ca()], vec![internal_ca()])] {
        let (result, heard) = send_to(server_identity(), Security::Tls, store, &distrusted);
        let why = refusal_text(result);
        assert!(why.contains("UnknownIssuer"), "{why}");
        assert!(!heard.contains("AUTH"), "{heard}");
    }
}
