//! MailCraft's network layer (native builds only).
//!
//! * [`stream`]: TCP with timeouts, TLS through rustls (Mozilla's root store), STARTTLS.
//! * [`imap`]: a small, strict IMAP4rev1 client (RFC 3501) on top of `imap-proto`'s parser,
//!   with the extensions sync relies on: UIDPLUS, MOVE, CONDSTORE, IDLE, SPECIAL-USE, SASL-IR.
//! * [`sync`]: the folder synchronisation algorithm (RFC 4549, RFC 7162).
//! * [`smtp`]: message submission through `lettre` (RFC 6409, implicit TLS per RFC 8314).
//! * [`worker`]: one thread per account that runs the engine's [`Job`](mailcraft_model::Job)s
//!   and reports [`Event`](mailcraft_model::Event)s, keeping a connection open, idling on the
//!   Inbox for new mail and replaying changes made offline.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod autoconfig;
pub mod dns;
pub mod http;
pub mod imap;
pub mod smtp;
pub mod stream;
pub mod sync;
pub mod worker;

#[cfg(test)]
mod fake;

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
}

impl NetError {
    /// Whether the connection is unusable after this error (reconnect needed).
    pub fn is_fatal(&self) -> bool {
        matches!(self, NetError::Io(_) | NetError::Tls(_) | NetError::Closed(_) | NetError::Protocol(_))
    }

    /// Whether sending may work later (keep the message in the Outbox rather than give up).
    pub fn is_retryable(&self) -> bool {
        self.is_fatal() || matches!(self, NetError::Later(_))
    }
}

pub type Result<T> = std::result::Result<T, NetError>;
