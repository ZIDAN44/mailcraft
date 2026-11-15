//! What an account trusts besides the authorities everyone does (issues #77 and #105), as other
//! mail apps let you: a certificate trusted when asked, by its fingerprint, and certificates
//! imported from files. For the account's mail servers alone, and less widely elsewhere than on
//! this computer.

use super::*;
use mailcraft_model::UntrustedCertificate;

/// What an account trusts besides the authorities everyone does (issues #77 and #105): the
/// certificates the user trusted when asked, by their fingerprints, and those imported from
/// files (base64 DER). For the account's mail servers only: never a web address.
#[derive(Clone, Copy, Debug, Default)]
pub struct Trusted<'a> {
    pub certificates: &'a [String],
    pub authorities: &'a [String],
}

impl<'a> Trusted<'a> {
    pub fn of(account: &'a mailcraft_model::AccountConfig) -> Trusted<'a> {
        Trusted { certificates: &account.trusted_certificates, authorities: &account.trusted_authorities }
    }

    /// Only certificates trusted by fingerprint.
    pub fn pinned(certificates: &'a [String]) -> Trusted<'a> {
        Trusted { certificates, authorities: &[] }
    }
}

/// Mail services whose servers always show a certificate a public authority issued: one that
/// doesn't is someone in between, never a server to trust by hand (issue #105). Each with the
/// names under it. By name: a server typed as an address is judged as any other.
const PUBLIC_SERVICES: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "google.com",
    "outlook.com",
    "office365.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    "microsoft.com",
    "yahoo.com",
    "yahoo.co.jp",
    "aol.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "apple.com",
    "yandex.ru",
    "yandex.com",
    "mail.ru",
    "gmx.net",
    "gmx.com",
    "web.de",
    "zoho.com",
    "zoho.eu",
    "fastmail.com",
    "proton.me",
    "protonmail.ch",
    "mailbox.org",
    "posteo.de",
    "qq.com",
    "163.com",
    "126.com",
    "yeah.net",
    "mail.com",
    "gmx.de",
    "t-online.de",
    "comcast.net",
    "att.net",
    "naver.com",
    "secureserver.net",
    "office.com",
    "ymail.com",
    "rocketmail.com",
    "rambler.ru",
    "seznam.cz",
    "orange.fr",
    "free.fr",
    "libero.it",
];

/// Whether `host` is one of [`PUBLIC_SERVICES`]' servers.
pub(crate) fn public_service(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    PUBLIC_SERVICES.iter().any(|d| host == *d || host.strip_suffix(d).is_some_and(|rest| rest.ends_with('.')))
}

/// Whether a certificate for `names` ([`mailcraft_smime::cert::server_names`]) is for `host`
/// (RFC 6125 §6.4): the whole name in any case, or `*` standing for its leftmost label alone
/// (never over a name with no dot left, as `*.com`); an address only as that address.
pub(crate) fn names_host(names: &[String], host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if let Ok(ip) = host.trim_start_matches('[').trim_end_matches(']').parse::<std::net::IpAddr>() {
        return names.iter().any(|n| n.parse::<std::net::IpAddr>().is_ok_and(|a| a.to_canonical() == ip.to_canonical()));
    }
    names.iter().map(|n| n.trim_end_matches('.').to_ascii_lowercase()).any(|n| match n.strip_prefix("*.") {
        Some(rest) => rest.contains('.') && host.split_once('.').is_some_and(|(first, after)| !first.is_empty() && after == rest),
        None => n == host,
    })
}

/// The check for an account's own mail servers (IMAP, POP, SMTP, ManageSieve): the usual one,
/// and when that refuses, what the user chose for the account:
/// - a certificate trusted when asked, by its fingerprint, or imported from a file (a server's
///   own), or one an authority imported from a file issued, or not trusted yet (issues #77 and
///   #105). The refusal names that certificate, for the user to be asked about it.
/// - On a server elsewhere than this computer, a certificate nobody vouches for is only ever
///   taken in date and for the server's name, and never on a major mail service's server
///   ([`PUBLIC_SERVICES`]), where it can only be someone in between. One on this computer (a
///   bridge to a mail service, issue #77) makes its own for whatever name.
/// - One the system distrusts stays refused.
#[derive(Debug)]
pub(crate) struct Own {
    usual: Arc<Distrusting>,
    pinned: Vec<String>,
    imported: Vec<Vec<u8>>,
    /// The check trusting the imported certificates as authorities.
    anchored: Option<Arc<Distrusting>>,
    /// The check trusting what the system distrusts as authorities alone: a chain it vouches
    /// for ends at a root the system distrusts (which its roots leave out, so the usual check
    /// says no authority vouches for it), and stays refused as one.
    beneath_distrusted: Option<Arc<Distrusting>>,
    host: String,
    here: bool,
    refused: std::sync::Mutex<Option<UntrustedCertificate>>,
}

impl Own {
    /// For a connection to `host`; `here`: it's on this computer (a loopback address).
    pub(crate) fn new(usual: Arc<Distrusting>, trusted: Trusted<'_>, host: &str, here: bool) -> Arc<Own> {
        let imported: Vec<Vec<u8>> = trusted
            .authorities
            .iter()
            .take(mailcraft_model::MAX_TRUSTED_AUTHORITIES)
            .filter_map(|b| mailcraft_mime::decode_base64(b))
            .filter(|der| !der.is_empty() && der.len() <= mailcraft_model::MAX_AUTHORITY_BYTES)
            // Only what's shown in Server settings (and can be taken back there), and nothing the
            // system distrusts.
            .filter(|der| mailcraft_smime::cert::info(der).is_ok() && !usual.distrusted.contains(&der[..]))
            .collect();
        let mut roots = RootCertStore::empty();
        roots.add_parsable_certificates(imported.iter().map(|der| CertificateDer::from(der.clone())));
        without_distrusted(&mut roots, &usual.distrusted);
        let anchored = (!roots.is_empty()).then(|| verifier(roots, Arc::clone(&usual.distrusted)).ok()).flatten();
        let mut bad = RootCertStore::empty();
        bad.add_parsable_certificates(usual.distrusted.iter().map(|d| CertificateDer::from(d.clone())));
        let beneath_distrusted = (!bad.is_empty()).then(|| verifier(bad, Arc::new(Distrusted::new())).ok()).flatten();
        let pinned = trusted.certificates.iter().take(mailcraft_model::MAX_TRUSTED_CERTIFICATES).cloned().collect();
        let refused = std::sync::Mutex::new(None);
        Arc::new(Own { usual, pinned, imported, anchored, beneath_distrusted, host: host.to_string(), here, refused })
    }

    /// Whether the user chose to trust the certificate with this fingerprint.
    fn trusts(&self, fingerprint: &str) -> bool {
        self.pinned.iter().any(|t| t.eq_ignore_ascii_case(fingerprint))
    }

    /// The certificate this refused, as it was shown, if it refused one to ask about.
    pub(crate) fn refused(&self) -> Option<UntrustedCertificate> {
        self.refused.lock().ok().and_then(|r| r.clone())
    }

    /// `chain` (the server's certificate, then the CAs it sent along) as a connection made
    /// elsewhere was given it, judged as this judges its own. For lettre (SMTP).
    pub(crate) fn check(&self, chain: &[Vec<u8>]) -> Result<(), rustls::Error> {
        check_chain(self, chain, &self.host)
    }

    /// The usual check's verdict on a certificate, or what the user chose for this one.
    fn judge(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        now: UnixTime,
        verdict: Result<(), rustls::Error>,
    ) -> Result<(), rustls::Error> {
        let refusal = match verdict {
            Ok(()) => return Ok(()),
            Err(e @ rustls::Error::InvalidCertificate(CertificateError::Revoked)) => return Err(e),
            Err(e) => e,
        };
        let revoked = rustls::Error::InvalidCertificate(CertificateError::Revoked);
        if let Some(beneath) = &self.beneath_distrusted {
            match beneath.verify_server_cert(end_entity, intermediates, server_name, &[], now) {
                Ok(_)
                | Err(rustls::Error::InvalidCertificate(CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. })) => {
                    return Err(revoked);
                }
                Err(_) => {}
            }
        }
        if !self.here && public_service(&self.host) {
            return Err(refusal);
        }
        let info = mailcraft_smime::cert::info(end_entity).ok();
        let names = mailcraft_smime::cert::server_names(end_entity);
        if !self.here {
            // What anyone can check of a certificate nobody vouches for: that it's in date, and
            // for this server. Wrong there, it's for whoever runs the server to put right.
            let at = i64::try_from(now.as_secs()).unwrap_or(i64::MAX);
            match &info {
                Some(i) if at > i.not_after => return Err(rustls::Error::InvalidCertificate(CertificateError::Expired)),
                Some(i) if at < i.not_before => return Err(rustls::Error::InvalidCertificate(CertificateError::NotValidYet)),
                Some(_) => {}
                None => return Err(refusal),
            }
            if !names_host(&names, &self.host) {
                return Err(rustls::Error::InvalidCertificate(CertificateError::NotValidForName));
            }
        }
        let found = fingerprint(end_entity);
        if self.trusts(&found) || self.imported.iter().any(|der| der[..] == end_entity[..]) {
            return Ok(());
        }
        if let Some(anchored) = &self.anchored {
            match anchored.verify_server_cert(end_entity, intermediates, server_name, &[], now) {
                Ok(_) => return Ok(()),
                // The chain holds; its name was looked at above (a server elsewhere), with the
                // common name for one that has no other (as many made by hand do).
                Err(rustls::Error::InvalidCertificate(CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. })) => {
                    return Ok(());
                }
                Err(e @ rustls::Error::InvalidCertificate(CertificateError::Revoked)) => return Err(e),
                Err(_) => {}
            }
        }
        let shown = UntrustedCertificate {
            host: self.host.clone(),
            fingerprint: found,
            remote: !self.here,
            subject: info.as_ref().map(|i| i.name.clone()).unwrap_or_default(),
            issuer: info.as_ref().map(|i| i.issuer.clone()).unwrap_or_default(),
            names,
            not_before: info.as_ref().map_or(0, |i| i.not_before),
            not_after: info.as_ref().map_or(0, |i| i.not_after),
        };
        if let Ok(mut refused) = self.refused.lock() {
            *refused = Some(shown);
        }
        Err(refusal)
    }
}

impl ServerCertVerifier for Own {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let verdict = self.usual.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now).map(|_| ());
        self.judge(end_entity, intermediates, server_name, now, verdict).map(|()| ServerCertVerified::assertion())
    }

    // The handshake is still signed with the certificate's key: whoever shows a trusted
    // certificate has to hold its key.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.usual.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.usual.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.usual.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests;
