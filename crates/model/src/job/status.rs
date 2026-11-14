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
    /// A mail server on this computer showed a certificate of its own the user hasn't trusted
    /// ([`crate::LOCAL_CERTIFICATE`]): the server, and the certificate's SHA-256 in hex.
    LocalCertificate { host: String, fingerprint: String },
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
        let local = FailureKind::LocalCertificate { host: "127.0.0.1".into(), fingerprint: "ab".repeat(32) };
        for status in [
            plain,
            SyncStatus::Error { error: "y".into(), kind: local },
            SyncStatus::Error { error: "z".into(), kind: FailureKind::MicrosoftImapClosed },
        ] {
            let back: SyncStatus = serde_json::from_value(serde_json::to_value(&status).unwrap()).unwrap();
            assert_eq!(back, status);
        }
    }
}
