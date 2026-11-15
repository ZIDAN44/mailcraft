//! How an account's connection is doing, and what kind of failure stopped it: said where the
//! failure is made (the network layer), so what helps can be offered without reading its words.

use serde::{Deserialize, Serialize};

/// How an account's connection is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SyncStatus {
    #[default]
    Offline,
    Connecting,
    Syncing {
        folder: String,
    },
    /// Connected and up to date as of `at` (Unix seconds).
    Online {
        at: i64,
    },
    /// The server refused the login: the user must enter the password again.
    NeedsPassword {
        error: String,
    },
    /// The last attempt failed; the worker retries with back-off, unless `kind` waits for you.
    Error {
        error: String,
        #[serde(default, skip_serializing_if = "FailureKind::is_other")]
        kind: FailureKind,
    },
}

impl SyncStatus {
    pub fn is_online(&self) -> bool {
        matches!(self, SyncStatus::Online { .. } | SyncStatus::Syncing { .. })
    }

    /// A failure of no kind that needs you: said in its words, and tried again.
    pub fn error(error: impl Into<String>) -> SyncStatus {
        SyncStatus::Error { error: error.into(), kind: FailureKind::Other }
    }
}

/// What kind of failure an account's [`SyncStatus::Error`], or a message waiting in the Outbox,
/// is. Each but `Other` waits for you to do something; its words say what.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureKind {
    /// Anything else: tried again by itself.
    #[default]
    Other,
    /// A Microsoft account signing in with a password, which Microsoft refuses: it needs Sign in
    /// with Microsoft ([`crate::MICROSOFT_NO_PASSWORDS`]).
    MicrosoftNoPasswords,
    /// Microsoft took the sign-in, but its server won't open the mailbox over IMAP
    /// ([`crate::MICROSOFT_IMAP_CLOSED`]).
    MicrosoftImapClosed,
    /// A mail server showed a certificate no authority MailCraft trusts vouches for, and the user
    /// hasn't trusted it yet (issues #77 and #105): what the window asks about. Read as it was
    /// written before, when only a server on this computer was asked about.
    #[serde(alias = "localCertificate")]
    UntrustedCertificate(Box<UntrustedCertificate>),
}

/// Why a mail server on this computer was refused: it showed a certificate of its own, which no
/// authority vouches for and the user hasn't trusted. The refusal says these words, then the
/// server and the certificate's fingerprint.
pub const LOCAL_CERTIFICATE: &str = "the mail server on this computer uses a certificate of its own that isn't trusted yet";

/// Why a mail server elsewhere was refused (issue #105): no authority MailCraft or this computer
/// trusts vouches for its certificate, and the user hasn't trusted it.
pub const REMOTE_CERTIFICATE: &str = "no authority this computer trusts vouches for the mail server's certificate, and it isn't trusted yet";

/// A server's certificate that no authority vouches for, as it was shown: what the user is asked
/// whether to trust, by its fingerprint, for the account.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UntrustedCertificate {
    /// The server, as the account names it.
    pub host: String,
    /// The certificate's SHA-256 in hex: what trusting it keeps.
    pub fingerprint: String,
    /// The server isn't on this computer (one that is, a bridge to a mail service, makes its own
    /// certificate as a matter of course).
    pub remote: bool,
    /// Whom it's for (its common name), and who issued it (an authority's name, or its own).
    pub subject: String,
    pub issuer: String,
    /// The server names it's for.
    pub names: Vec<String>,
    /// Valid from and until (Unix seconds; 0 when it couldn't be read).
    pub not_before: i64,
    pub not_after: i64,
}

impl UntrustedCertificate {
    /// What its refusal says: why, then the server and the fingerprint.
    pub fn refusal(&self) -> String {
        let why = if self.remote { REMOTE_CERTIFICATE } else { LOCAL_CERTIFICATE };
        format!("{why} ({} {})", self.host, self.fingerprint)
    }
}

impl FailureKind {
    pub fn is_other(&self) -> bool {
        *self == FailureKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure of no kind is written as it always was, and read back so; a kind goes with it.
    #[test]
    fn a_status_keeps_its_kind_through_json() {
        let plain = SyncStatus::error("x");
        assert_eq!(serde_json::to_value(&plain).unwrap(), serde_json::json!({"state": "error", "error": "x"}));
        let local = FailureKind::UntrustedCertificate(Box::new(UntrustedCertificate {
            host: "mail.corp.test".into(),
            fingerprint: "ab".repeat(32),
            remote: true,
            names: vec!["mail.corp.test".into()],
            ..UntrustedCertificate::default()
        }));
        for status in [
            plain,
            SyncStatus::Error { error: "y".into(), kind: local },
            SyncStatus::Error { error: "z".into(), kind: FailureKind::MicrosoftImapClosed },
        ] {
            let back: SyncStatus = serde_json::from_value(serde_json::to_value(&status).unwrap()).unwrap();
            assert_eq!(back, status);
        }
    }

    /// A refusal written before servers elsewhere were asked about (issue #77's) reads back as
    /// one on this computer.
    #[test]
    fn an_older_refusal_reads_as_one_on_this_computer() {
        let print = "ab".repeat(32);
        let old = serde_json::json!({"state": "error", "error": "x", "kind": {"localCertificate": {"host": "127.0.0.1", "fingerprint": print}}});
        let read: SyncStatus = serde_json::from_value(old).unwrap();
        let wanted = UntrustedCertificate { host: "127.0.0.1".into(), fingerprint: print.clone(), ..UntrustedCertificate::default() };
        assert_eq!(read, SyncStatus::Error { error: "x".into(), kind: FailureKind::UntrustedCertificate(Box::new(wanted.clone())) });
        assert_eq!(wanted.refusal(), format!("{LOCAL_CERTIFICATE} (127.0.0.1 {print})"));
        let elsewhere = UntrustedCertificate { remote: true, host: "mail.corp.test".into(), ..wanted };
        assert_eq!(elsewhere.refusal(), format!("{REMOTE_CERTIFICATE} (mail.corp.test {print})"));
    }
}
