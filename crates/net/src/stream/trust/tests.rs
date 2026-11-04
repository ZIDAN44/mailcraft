use std::sync::Arc;

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, ServerConfig, ServerConnection};

use super::test_certs::{internal_ca, server, server_key, unreadable};
use super::{Distrusted, client_config, root_store, usable_roots};

/// A TLS handshake, in memory, between `client` and `mail.corp.test` (signed by the internal CA).
/// The client's verdict on the server's certificate, as the error it gives.
fn handshake(client: Arc<ClientConfig>) -> Result<(), String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![server()], server_key())
        .unwrap();
    let mut c = ClientConnection::new(client, ServerName::try_from("mail.corp.test").unwrap()).unwrap();
    let mut s = ServerConnection::new(Arc::new(config)).unwrap();
    for _ in 0..8 {
        let mut wire = Vec::new();
        while c.wants_write() {
            c.write_tls(&mut wire).unwrap();
        }
        let mut read = &wire[..];
        while !read.is_empty() {
            s.read_tls(&mut read).unwrap();
        }
        s.process_new_packets().map_err(|e| format!("server: {e}"))?;
        let mut wire = Vec::new();
        while s.wants_write() {
            s.write_tls(&mut wire).unwrap();
        }
        let mut read = &wire[..];
        while !read.is_empty() {
            c.read_tls(&mut read).unwrap();
        }
        c.process_new_packets().map_err(|e| e.to_string())?;
        if !c.is_handshaking() && !s.is_handshaking() {
            return Ok(());
        }
    }
    Err("the handshake didn't finish".into())
}

/// The configuration IMAP, POP, Sieve and HTTPS use, given this system store and distrust list.
fn config(store: Vec<CertificateDer<'static>>, distrusted: &[CertificateDer<'static>]) -> Arc<ClientConfig> {
    let distrusted: Distrusted = distrusted.iter().map(|c| c.to_vec()).collect();
    let roots = root_store(&usable_roots(store, &distrusted), &distrusted);
    client_config(roots, Arc::new(distrusted)).unwrap()
}

/// An on-premises server whose certificate comes from the company's own CA: refused with only
/// the bundled roots (the error users saw), trusted once the system trusts that CA, even with a
/// damaged entry in the store.
#[test]
fn a_server_signed_by_a_ca_the_system_trusts_is_trusted() {
    let without = handshake(config(vec![], &[])).unwrap_err();
    assert!(without.contains("UnknownIssuer"), "{without}");
    assert_eq!(handshake(config(vec![internal_ca()], &[])), Ok(()));
    assert_eq!(handshake(config(vec![unreadable(), internal_ca(), unreadable()], &[])), Ok(()));
}

/// What the system distrusts stays refused though it's among its roots: the CA itself, or the
/// server's own certificate (or an intermediate on the way).
#[test]
fn a_certificate_the_system_distrusts_is_refused() {
    let ca_distrusted = handshake(config(vec![internal_ca()], &[internal_ca()])).unwrap_err();
    assert!(ca_distrusted.contains("UnknownIssuer"), "{ca_distrusted}");
    let server_distrusted = handshake(config(vec![internal_ca()], &[server()])).unwrap_err();
    assert!(server_distrusted.contains("Revoked"), "{server_distrusted}");
}

/// The configuration connections use is built, and the system doesn't trust the test CA.
#[test]
fn connections_use_the_trusting_configuration() {
    let used = super::super::tls_config().unwrap();
    let refused = handshake(used).unwrap_err();
    assert!(refused.contains("UnknownIssuer"), "{refused}");
}

/// A certificate rustls can't read, or one the system distrusts, is left out of the system's
/// roots; the rest are kept.
#[test]
fn unreadable_and_distrusted_roots_are_left_out() {
    let none = Distrusted::new();
    assert_eq!(usable_roots(vec![unreadable(), internal_ca(), unreadable()], &none), vec![internal_ca()]);
    assert!(usable_roots(vec![], &none).is_empty());
    let ca: Distrusted = [internal_ca().to_vec()].into_iter().collect();
    assert!(usable_roots(vec![internal_ca()], &ca).is_empty());
}
