//! What an account trusts besides the authorities everyone does, as other mail apps let you
//! (issues #77 and #105): a mail server's certificate no authority vouches for, asked about once
//! and trusted by its fingerprint, and certificates imported from files (an authority a company
//! runs, or a server's own). For the account's mail servers only; the network layer says how far
//! (`net/src/stream/trust/own.rs`).

use super::*;

/// A certificate an account trusts, for showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedCertificate {
    /// Its SHA-256, in lower-case hex: what forgetting it names.
    pub fingerprint: String,
    /// Whom it's for, and who issued it, when it came from a file (one trusted when asked is
    /// known by its fingerprint alone).
    pub name: Option<(String, String)>,
}

impl Session {
    /// A connection was refused, of a `kind` that says a mail server showed a certificate no
    /// authority vouches for: the window is to ask about it, once for each certificate.
    pub(crate) fn note_untrusted_certificate(&mut self, account: &str, kind: &FailureKind) {
        let FailureKind::UntrustedCertificate(shown) = kind else { return };
        // Only a server's certificate, by its SHA-256, is asked about (and trusted).
        if shown.host.trim().is_empty() || !mailcraft_model::is_certificate_fingerprint(&shown.fingerprint) {
            return;
        }
        let mut shown = (**shown).clone();
        shown.host = shown.host.trim().to_string();
        shown.fingerprint = shown.fingerprint.to_ascii_lowercase();
        if self.asked_certificate.len() < 64 && self.asked_certificate.insert((account.to_string(), shown.fingerprint.clone())) {
            self.ask_certificate.push((account.to_string(), shown));
        }
    }

    /// The account, if it connects to mail servers of its own (IMAP or POP, and SMTP): what it
    /// trusts is for those alone, never a web address (JMAP, Exchange, calendars).
    fn mail_servers_of(&self, account: &str) -> Result<AccountConfig, CmdError> {
        let config = self.config_of(account)?;
        let web = config.jmap.is_some() || config.graph.is_some() || config.ews.is_some() || config.is_calendar_only();
        if !config.has_server() || web {
            return Err(CmdError::Params("only an account's IMAP, POP and SMTP servers can be trusted this way".into()));
        }
        Ok(config)
    }

    /// Trust the certificate with this fingerprint for an account's mail servers, and connect
    /// again. The newest [`mailcraft_model::MAX_TRUSTED_CERTIFICATES`] are kept.
    pub fn trust_certificate(&mut self, account: &str, fingerprint: &str) -> CmdResult {
        if self.org.no_certificate_exceptions {
            return Err(CmdError::Disabled(super::super::organisation::NO_EXCEPTIONS.into()));
        }
        let fingerprint = fingerprint.trim().to_ascii_lowercase();
        if !mailcraft_model::is_certificate_fingerprint(&fingerprint) {
            return Err(CmdError::Params("`fingerprint` is a certificate's SHA-256 in hex: 64 characters".into()));
        }
        let mut config = self.mail_servers_of(account)?;
        config.trusted_certificates.retain(|t| !t.eq_ignore_ascii_case(&fingerprint));
        config.trusted_certificates.push(fingerprint.clone());
        let over = config.trusted_certificates.len().saturating_sub(mailcraft_model::MAX_TRUSTED_CERTIFICATES);
        config.trusted_certificates.drain(..over);
        self.update_account(config, None)?;
        Ok(serde_json::json!({ "account": account, "trusted": fingerprint }))
    }

    /// Trust the certificates in a file (`data`: PEM, one or several, or DER) for an account's
    /// mail servers, and connect again (issue #105). The newest
    /// [`mailcraft_model::MAX_TRUSTED_AUTHORITIES`] are kept.
    pub fn trust_certificate_file(&mut self, account: &str, data: &[u8]) -> CmdResult {
        if self.org.no_certificate_exceptions {
            return Err(CmdError::Disabled(super::super::organisation::NO_EXCEPTIONS.into()));
        }
        let found = certificates_in(data);
        if found.is_empty() {
            return Err(CmdError::Params("there's no certificate in that file: a .pem, .crt or .cer file has one".into()));
        }
        let mut config = self.mail_servers_of(account)?;
        let mut names = Vec::new();
        for (der, info) in found {
            let print = fingerprint_of(&info);
            config.trusted_authorities.retain(|b| file_fingerprint(b).as_deref() != Some(print.as_str()));
            config.trusted_authorities.push(mailcraft_model::base64::encode(&der));
            names.push(serde_json::json!({ "name": info.name, "issuer": info.issuer, "fingerprint": print }));
        }
        let over = config.trusted_authorities.len().saturating_sub(mailcraft_model::MAX_TRUSTED_AUTHORITIES);
        config.trusted_authorities.drain(..over);
        self.update_account(config, None)?;
        Ok(serde_json::json!({ "account": account, "trusted": names }))
    }

    /// Stop trusting a certificate for an account, trusted when asked or from a file, by its
    /// fingerprint; it connects again.
    pub fn forget_certificate(&mut self, account: &str, fingerprint: &str) -> CmdResult {
        let fingerprint = fingerprint.trim().to_ascii_lowercase();
        let mut config = self.config_of(account)?;
        let before = config.trusted_certificates.len() + config.trusted_authorities.len();
        config.trusted_certificates.retain(|t| !t.eq_ignore_ascii_case(&fingerprint));
        config.trusted_authorities.retain(|b| file_fingerprint(b).as_deref() != Some(fingerprint.as_str()));
        if config.trusted_certificates.len() + config.trusted_authorities.len() == before {
            return Err(CmdError::Params("that account doesn't trust that certificate".into()));
        }
        // It may be the one an existing connection rests on: asked about again if it's shown.
        self.asked_certificate.retain(|(a, f)| !(a == account && *f == fingerprint));
        self.update_account(config, None)?;
        Ok(serde_json::json!({ "account": account, "forgotten": fingerprint }))
    }

    /// The certificates an account trusts besides the usual authorities: from files, then those
    /// trusted when asked.
    pub fn trusted_certificates(&self, account: &str) -> Vec<TrustedCertificate> {
        let Some(a) = self.account(account) else { return Vec::new() };
        let files = a.config.trusted_authorities.iter().filter_map(|b| {
            let der = mailcraft_mime::decode_base64(b)?;
            let info = mailcraft_smime::cert::info(&der).ok()?;
            Some(TrustedCertificate { fingerprint: fingerprint_of(&info), name: Some((info.name, info.issuer)) })
        });
        let asked = a.config.trusted_certificates.iter().map(|f| TrustedCertificate { fingerprint: f.to_ascii_lowercase(), name: None });
        files.chain(asked).collect()
    }
}

/// A certificate's SHA-256 in lower-case hex, from what it says.
fn fingerprint_of(info: &mailcraft_smime::CertInfo) -> String {
    info.fingerprint.replace(':', "").to_ascii_lowercase()
}

/// The fingerprint of a certificate kept from a file (base64): of its bytes, so that one this
/// can't read is still taken back by it.
fn file_fingerprint(kept: &str) -> Option<String> {
    mailcraft_mime::decode_base64(kept).map(|der| mailcraft_smime::cert::fingerprint(&der))
}

/// The certificates in a file, as each can be read: PEM's `CERTIFICATE` blocks (the first
/// [`mailcraft_model::MAX_TRUSTED_AUTHORITIES`]), or the file itself as DER.
fn certificates_in(data: &[u8]) -> Vec<(Vec<u8>, mailcraft_smime::CertInfo)> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let read = |der: Vec<u8>| {
        let ok = !der.is_empty() && der.len() <= mailcraft_model::MAX_AUTHORITY_BYTES;
        ok.then(|| mailcraft_smime::cert::info(&der).ok().map(|info| (der, info))).flatten()
    };
    let text = String::from_utf8_lossy(data);
    if !text.contains(BEGIN) {
        return read(data.to_vec()).into_iter().collect();
    }
    let mut found = Vec::new();
    let mut rest = text.as_ref();
    while let Some(start) = rest.find(BEGIN) {
        let body = &rest[start + BEGIN.len()..];
        let Some(end) = body.find(END) else { break };
        if let Some(cert) = mailcraft_mime::decode_base64(&body[..end]).and_then(read) {
            found.push(cert);
        }
        if found.len() >= mailcraft_model::MAX_TRUSTED_AUTHORITIES {
            break;
        }
        rest = &body[end + END.len()..];
    }
    found
}
