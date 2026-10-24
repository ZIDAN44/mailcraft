//! MailCraft's network layer (native builds only).
//!
//! * [`stream`]: TCP with timeouts, TLS through rustls (Mozilla's root store), STARTTLS.
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

// Each module by its own name, wherever its file sits (`mailcraft_net::ews`, `crate::imap`).
use data_files::pst_pim;
pub use data_files::{pst, pst_export};
use exchange::ews_pim;
pub use exchange::{ews, graph, graph_pim, ntlm};
#[cfg(test)]
use fakes::{fake, fake_graph};
pub use protocols::{dav, imap, jmap, pop, sieve, smtp};

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("network: {0}")]
    Io(#[from] io::Error),
    #[error("secure connection: {0}")]
    Tls(String),
    /// The server's response could not be understood.
    #[error("unexpected server response: {0}")]
    Protocol(String),
    /// The server refused a command (`NO`).
    #[error("the server refused: {0}")]
    No(String),
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
}

impl NetError {
    /// Whether the connection is unusable after this error (reconnect needed).
    pub fn is_fatal(&self) -> bool {
        matches!(self, NetError::Io(_) | NetError::Tls(_) | NetError::Closed(_) | NetError::Protocol(_))
    }

    /// Whether sending may work later (keep the message in the Outbox rather than give up): a
    /// refused sign-in is the account's problem, not the message's, and clears once signed in.
    pub fn is_retryable(&self) -> bool {
        self.is_fatal() || matches!(self, NetError::Later(_) | NetError::Auth(_))
    }
}

pub type Result<T> = std::result::Result<T, NetError>;

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
