use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, ServerConfig, ServerConnection};

use super::test_certs::{internal_ca, issuing_ca, server, server_identity, unreadable, via_issuing, via_issuing_identity};
use super::{Distrusted, Distrusting, check_chain, client_config, root_store, usable_roots, verifier};

type Identity = (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>);

/// A TLS handshake, in memory, between `client` and `mail.corp.test` presenting `identity`.
/// The client's verdict on the server's certificates, as the error it gives.
fn handshake(client: Arc<ClientConfig>, identity: Identity) -> Result<(), String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let (chain, key) = identity;
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(chain, key)
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

/// The check connections make, given this system store and distrust list.
pub(crate) fn check(store: Vec<CertificateDer<'static>>, distrusted: &[CertificateDer<'static>]) -> Arc<Distrusting> {
    let distrusted: Distrusted = distrusted.iter().map(|c| c.to_vec()).collect();
    let roots = root_store(&usable_roots(store, &distrusted), &distrusted);
    verifier(roots, Arc::new(distrusted)).unwrap()
}

/// The configuration IMAP, POP, Sieve and HTTPS use, given this system store and distrust list.
fn config(store: Vec<CertificateDer<'static>>, distrusted: &[CertificateDer<'static>]) -> Arc<ClientConfig> {
    client_config(check(store, distrusted)).unwrap()
}

/// An on-premises server whose certificate comes from the company's own CA: refused with only
/// the bundled roots (the error users saw), trusted once the system trusts that CA, even with a
/// damaged entry in the store, and through an intermediate CA too.
#[test]
fn a_server_signed_by_a_ca_the_system_trusts_is_trusted() {
    let without = handshake(config(vec![], &[]), server_identity()).unwrap_err();
    assert!(without.contains("UnknownIssuer"), "{without}");
    assert_eq!(handshake(config(vec![internal_ca()], &[]), server_identity()), Ok(()));
    assert_eq!(handshake(config(vec![unreadable(), internal_ca(), unreadable()], &[]), server_identity()), Ok(()));
    assert_eq!(handshake(config(vec![internal_ca()], &[]), via_issuing_identity()), Ok(()));
}

/// What the system distrusts stays refused though it's among its roots: the CA itself, the
/// server's own certificate, or the intermediate CA it sends.
#[test]
fn a_certificate_the_system_distrusts_is_refused() {
    let ca_distrusted = handshake(config(vec![internal_ca()], &[internal_ca()]), server_identity()).unwrap_err();
    assert!(ca_distrusted.contains("UnknownIssuer"), "{ca_distrusted}");
    let server_distrusted = handshake(config(vec![internal_ca()], &[server()]), server_identity()).unwrap_err();
    assert!(server_distrusted.contains("Revoked"), "{server_distrusted}");
    let leaf_distrusted = handshake(config(vec![internal_ca()], &[via_issuing()]), via_issuing_identity()).unwrap_err();
    assert!(leaf_distrusted.contains("Revoked"), "{leaf_distrusted}");
    let intermediate_distrusted = handshake(config(vec![internal_ca()], &[issuing_ca()]), via_issuing_identity()).unwrap_err();
    assert!(intermediate_distrusted.contains("Revoked"), "{intermediate_distrusted}");
}

/// A chain another TLS stack was given (lettre's) gets the same verdicts as a handshake.
#[test]
fn a_chain_from_elsewhere_is_checked_the_same_way() {
    let chain = vec![via_issuing().to_vec(), issuing_ca().to_vec()];
    assert_eq!(check_chain(&check(vec![internal_ca()], &[]), &chain, "mail.corp.test"), Ok(()));
    assert_eq!(check_chain(&check(vec![internal_ca()], &[]), &chain, "127.0.0.1"), Ok(()));
    let distrusted = check_chain(&check(vec![internal_ca()], &[issuing_ca()]), &chain, "mail.corp.test").unwrap_err();
    assert!(distrusted.to_string().contains("Revoked"), "{distrusted}");
    let unknown = check_chain(&check(vec![], &[]), &chain, "mail.corp.test").unwrap_err();
    assert!(unknown.to_string().contains("UnknownIssuer"), "{unknown}");
    let wrong_name = check_chain(&check(vec![internal_ca()], &[]), &chain, "other.corp.test").unwrap_err();
    assert!(wrong_name.to_string().contains("not valid for name"), "{wrong_name}");
    assert!(check_chain(&check(vec![internal_ca()], &[]), &[], "mail.corp.test").is_err());
    assert!(check_chain(&check(vec![internal_ca()], &[]), &[vec![1, 2, 3]], "mail.corp.test").is_err());
}

/// The configuration connections use is built, and the system doesn't trust the test CA.
#[test]
fn connections_use_the_trusting_configuration() {
    let used = super::super::tls_config().unwrap();
    let refused = handshake(used, server_identity()).unwrap_err();
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
