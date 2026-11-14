//! MailCraft's network layer (native builds only).
//!
//! * [`stream`]: TCP with timeouts, TLS through rustls (Mozilla's roots and the system's, less
//!   what the system distrusts), STARTTLS.
//! * [`imap`]: a small, strict IMAP4rev1 client (RFC 3501) on top of `imap-proto`'s parser,
//!   with the extensions sync relies on: UIDPLUS, MOVE, CONDSTORE, IDLE, SPECIAL-USE, SASL-IR.
//! * [`sync`]: the folder synchronisation algorithm (RFC 4549, RFC 7162).
//! * [`jmap`]: the second transport, JMAP (RFC 8620/8621) over HTTPS, synced into the same cache.
//! * [`graph`]: the third, Microsoft Graph, for Exchange Online accounts.
//! * [`dav`]: calendars and address books, CalDAV and CardDAV.
//! * [`pst`]: importing Outlook data files (`.pst`) into an account on this computer.
//! * [`sieve`]: automatic replies on IMAP servers, through ManageSieve (RFC 5804).
//! * [`smtp`]: message submission through `lettre` (RFC 6409, implicit TLS per RFC 8314).
//! * [`worker`]: one thread per account that runs the engine's [`Job`](mailcraft_model::Job)s
//!   and reports [`Event`](mailcraft_model::Event)s, keeping a connection open, idling on the
//!   Inbox for new mail and replaying changes made offline.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod autoconfig;
mod data_files;
pub mod dns;
mod exchange;
#[cfg(test)]
mod fakes;
pub mod http;
pub mod oauth;
mod protocols;
pub mod stream;
pub mod sync;
pub mod updates;
pub mod worker;
mod xml;

// Each module by its own name, wherever its file sits (`mailcraft_net::ews`, `crate::imap`).
use data_files::pst_pim;
pub use data_files::{msg, pst, pst_export, space};
use exchange::ews_pim;
pub use exchange::{ews, graph, graph_pim, kerberos, ntlm};
#[cfg(test)]
use fakes::{fake, fake_graph, waiting};
pub use protocols::{dav, imap, jmap, pop, sieve, smtp, web_calendar};

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("network: {0}")]
    Io(#[from] io::Error),
    #[error("secure connection: {0}")]
    Tls(String),
    /// A mail server on this computer showed a certificate of its own, which no authority vouches
    /// for and the user hasn't trusted: the server, and the certificate's SHA-256 in hex, for the
    /// user to be asked about it (issue #77). Otherwise a TLS failure.
    #[error("secure connection: {}", mailcraft_model::local_certificate_refusal(.host, .fingerprint))]
    Untrusted { host: String, fingerprint: String },
    /// The server's response could not be understood.
    #[error("unexpected server response: {0}")]
    Protocol(String),
    /// The server refused a command (`NO`).
    #[error("the server refused: {0}")]
    No(String),
    /// The server refused because the folder isn't there, and may be made (`NO [TRYCREATE]`,
    /// RFC 3501 §6.3.11).
    #[error("the server refused: {0}")]
    NoFolder(String),
    /// The server rejected a command as malformed (`BAD`).
    #[error("the server rejected the command: {0}")]
    Bad(String),
    /// Login failed: wrong user name or password, or the method isn't allowed.
    #[error("sign-in failed: {0}")]
    Auth(String),
    /// The server closed the connection (`BYE` or end of stream).
    #[error("the server closed the connection: {0}")]
    Closed(String),
    #[error("sending failed: {0}")]
    Smtp(String),
    /// The server asked to try again later (SMTP 4xx).
    #[error("the server asked to try again later: {0}")]
    Later(String),
    #[error("{0}")]
    Store(#[from] mailcraft_store::StoreError),
    /// Can't go on, for a reason said in full (what to do included): not a server's words.
    #[error("{0}")]
    Refused(String),
    /// Microsoft signed the account in, but its server won't open the mailbox over IMAP: why,
    /// and what helps, in full (starting with [`mailcraft_model::MICROSOFT_IMAP_CLOSED`]).
    /// Otherwise a refusal.
    #[error("{0}")]
    MailboxClosed(String),
    /// The whole message went to the server, which never said whether it took it: it may have
    /// been sent. Never sent again without the user saying so (that's how mail arrives twice).
    #[error(
        "the server didn't say whether it sent the message ({0}). It may have gone: look in Sent Items, or ask who it was for, before sending it again"
    )]
    Unconfirmed(String),
}

impl NetError {
    /// Whether the connection is unusable after this error (reconnect needed).
    pub fn is_fatal(&self) -> bool {
        matches!(self, NetError::Io(_) | NetError::Tls(_) | NetError::Untrusted { .. } | NetError::Closed(_) | NetError::Protocol(_))
    }

    /// Whether sending may work later (keep the message in the Outbox rather than give up): a
    /// refused sign-in is the account's problem, not the message's, and clears once signed in.
    pub fn is_retryable(&self) -> bool {
        self.is_fatal() || matches!(self, NetError::Later(_) | NetError::Auth(_))
    }

    /// The kind of failure it is, for what helps to be offered without reading its words.
    pub fn failure(&self) -> mailcraft_model::FailureKind {
        use mailcraft_model::FailureKind;
        match self {
            NetError::Untrusted { host, fingerprint } => FailureKind::LocalCertificate { host: host.clone(), fingerprint: fingerprint.clone() },
            NetError::MailboxClosed(_) => FailureKind::MicrosoftImapClosed,
            _ => FailureKind::Other,
        }
    }

    /// An account's status after this error: its words and its kind.
    pub fn status(&self) -> mailcraft_model::SyncStatus {
        mailcraft_model::SyncStatus::Error { error: self.to_string(), kind: self.failure() }
    }
}

pub type Result<T> = std::result::Result<T, NetError>;

/// Bytes in lowercase hex, two digits each (a digest, a random mark).
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Every byte gives what each place that wrote its own hex gave.
#[test]
fn hex_is_each_bytes_two_lowercase_digits() {
    let all: Vec<u8> = (0..=255).collect();
    assert_eq!(hex(&all), all.iter().map(|b| format!("{b:02x}")).collect::<String>());
    assert_eq!(hex(&[]), "");
}

/// The smart cards' PKCS #11 modules installed here, the likeliest first (S/MIME with a card).
pub fn card_modules() -> Vec<String> {
    mailcraft_keys::pkcs11::known_modules()
}

/// The root certificates this computer trusts (DER): what a signed message's chain must reach
/// to be trusted (S/MIME). The system's own store, as the user and their organisation keep it:
/// Windows' and macOS' stores, Linux's and the BSDs' bundle. None found, none: signatures still
/// hold, they just don't reach a trusted root.
pub fn trust_roots() -> Vec<Vec<u8>> {
    let found = rustls_native_certs::load_native_certs();
    for e in found.errors.iter().take(3) {
        log::info!("S/MIME roots: {e}");
    }
    found.certs.into_iter().map(|c| c.as_ref().to_vec()).collect()
}
