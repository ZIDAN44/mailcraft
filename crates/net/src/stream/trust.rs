//! Which servers' certificates are trusted: the bundled Mozilla roots and the system's own (an
//! internal CA that IT installed for an on-premises server), less what the system distrusts.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

/// Certificates this computer explicitly distrusts (DER).
pub(crate) type Distrusted = HashSet<Vec<u8>>;

/// The root certificates this computer trusts for servers, read once: the Windows certificate
/// store, the macOS Keychain, the Linux and BSD bundle (or `SSL_CERT_FILE` / `SSL_CERT_DIR`).
/// An internal CA that IT installed for an on-premises server (Exchange, a company IMAP server)
/// is then trusted next to the bundled Mozilla roots. For SMTP too ([`crate::protocols::smtp`]).
pub(crate) fn system_roots() -> &'static [CertificateDer<'static>] {
    static ROOTS: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let found = rustls_native_certs::load_native_certs();
        for e in found.errors.iter().take(3) {
            log::info!("system roots: {e}");
        }
        let usable = usable_roots(found.certs, distrusted());
        log::info!("TLS: {} system root certificates, {} distrusted certificates", usable.len(), distrusted().len());
        usable
    })
}

/// What the system distrusts, read once: Windows' Untrusted Certificates (`Disallowed`), where
/// Windows Update and administrators put CAs that must not be trusted, even ones still among the
/// roots. Elsewhere a distrusted root is already left out of what the system lists.
pub(crate) fn distrusted() -> &'static Arc<Distrusted> {
    static SET: OnceLock<Arc<Distrusted>> = OnceLock::new();
    SET.get_or_init(|| Arc::new(load_distrusted()))
}

/// Windows' Untrusted Certificates, the user's and the computer's, opened read-only. A store
/// that can't be opened is logged and counts as empty.
#[cfg(windows)]
fn load_distrusted() -> Distrusted {
    use schannel::cert_store::CertStoreBuilder;
    let mut set = Distrusted::new();
    let mut open = CertStoreBuilder::new();
    open.read_only();
    for (whose, store) in [("user", open.open_current_user("Disallowed")), ("computer", open.open_local_machine("Disallowed"))] {
        match store {
            Ok(store) => set.extend(store.certs().map(|c| c.to_der().to_vec())),
            Err(e) => log::info!("distrusted certificates ({whose}): {e}"),
        }
    }
    set
}

/// macOS and the Linux and BSD bundles list no distrusted certificates of their own: a root
/// the system distrusts is left out of its roots already.
#[cfg(not(windows))]
fn load_distrusted() -> Distrusted {
    Distrusted::new()
}

/// The certificates rustls can take as roots, less those the system distrusts. A store can hold
/// one it can't read; given to lettre, that one would fail every message sent, to every server.
pub(crate) fn usable_roots(certs: Vec<CertificateDer<'static>>, distrusted: &Distrusted) -> Vec<CertificateDer<'static>> {
    certs.into_iter().filter(|c| !distrusted.contains(&c[..]) && RootCertStore::empty().add(c.clone()).is_ok()).collect()
}

/// The bundled Mozilla roots and `extra` (the system's), without any root the system distrusts.
pub(super) fn root_store(extra: &[CertificateDer<'static>], distrusted: &Distrusted) -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots.add_parsable_certificates(extra.iter().cloned());
    let mut bad = RootCertStore::empty();
    bad.add_parsable_certificates(distrusted.iter().map(|d| CertificateDer::from(d.clone())));
    roots.roots.retain(|a| !bad.roots.iter().any(|b| b.subject == a.subject && b.subject_public_key_info == a.subject_public_key_info));
    roots
}

/// The check every connection's certificates get, built once: the bundled Mozilla roots and the
/// system's, less what the system distrusts.
pub(crate) fn system_verifier() -> Result<Arc<Distrusting>, String> {
    static VERIFIER: OnceLock<Result<Arc<Distrusting>, String>> = OnceLock::new();
    VERIFIER.get_or_init(|| verifier(root_store(system_roots(), distrusted()), Arc::clone(distrusted()))).clone()
}

/// A check trusting `roots` that refuses a server whose chain holds a certificate in
/// `distrusted`.
pub(crate) fn verifier(roots: RootCertStore, distrusted: Arc<Distrusted>) -> Result<Arc<Distrusting>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider).build().map_err(|e| e.to_string())?;
    Ok(Arc::new(Distrusting { inner, distrusted }))
}

/// A client configuration whose server certificates `verifier` checks.
pub(super) fn client_config(verifier: Arc<Distrusting>) -> Result<Arc<ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().map_err(|e| e.to_string())?;
    Ok(Arc::new(builder.dangerous().with_custom_certificate_verifier(verifier).with_no_client_auth()))
}

/// Checks the chain a TLS connection made elsewhere was given as `verifier` checks its own
/// connections': the server's certificate first, then the CAs it sent along. For lettre (SMTP),
/// which takes no verifier of ours: called after the handshake, before signing in.
pub(crate) fn check_chain(verifier: &Distrusting, chain: &[Vec<u8>], host: &str) -> Result<(), rustls::Error> {
    let Some((leaf, rest)) = chain.split_first() else {
        return Err(rustls::Error::NoCertificatesPresented);
    };
    let leaf = CertificateDer::from(leaf.as_slice());
    let rest: Vec<CertificateDer<'_>> = rest.iter().map(|c| CertificateDer::from(c.as_slice())).collect();
    let name = ServerName::try_from(host).map_err(|_| rustls::Error::InvalidCertificate(CertificateError::NotValidForName))?;
    verifier.verify_server_cert(&leaf, &rest, &name, &[], UnixTime::now()).map(|_| ())
}

/// rustls's own checks, after refusing a chain the system distrusts part of: a server, or an
/// intermediate CA, that Windows lists as untrusted (WebPKI doesn't look there).
#[derive(Debug)]
pub(crate) struct Distrusting {
    inner: Arc<WebPkiServerVerifier>,
    distrusted: Arc<Distrusted>,
}

impl ServerCertVerifier for Distrusting {
    /// Refused as revoked when the server's certificate or an intermediate is distrusted;
    /// otherwise WebPKI's verdict.
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if std::iter::once(end_entity).chain(intermediates).any(|c| self.distrusted.contains(&c[..])) {
            return Err(rustls::Error::InvalidCertificate(CertificateError::Revoked));
        }
        self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

#[cfg(test)]
pub(crate) mod test_certs;
#[cfg(test)]
pub(crate) mod tests;
