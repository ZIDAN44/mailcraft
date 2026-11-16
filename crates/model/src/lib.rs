//! MailCraft data model.
//!
//! Pure data and algorithms shared by every layer: accounts and servers, folders (with their
//! RFC 6154 special-use roles), message keys and summaries, flags, the jobs the engine asks the
//! network layer to run and the events it gets back, conversation threading (JWZ), search queries
//! and Outlook-style date grouping. Nothing here does I/O, so it builds for wasm.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod addresses;
pub mod dates;
pub mod encoding;
pub mod job;
pub mod mailto;
pub mod messages;
pub mod pim;
pub mod search;

mod account;
mod folder;

pub use account::{
    AccountConfig, DavSettings, EWS_AUTODISCOVER, GRAPH_URL, MAX_AUTHORITY_BYTES, MAX_CHECK_MINUTES, MAX_TRUSTED_AUTHORITIES,
    MAX_TRUSTED_CERTIFICATES, MAX_USER_AGENT, MICROSOFT_IMAP_CLOSED, MICROSOFT_NO_PASSWORDS, Security, ServerConfig, SignInMethod,
    files_sent_mail_itself, is_certificate_fingerprint, microsoft_consumer, microsoft_password_account, oauth_provider_for, user_agent,
};
pub use folder::{Folder, FolderRole, child_path, is_within, rebase_path, sort_folders};

pub use account::signature;
pub use addresses::{addr, contacts, lookalike};
pub use messages::{organize, summary, thread};
pub use pim::{ical, vcard};

pub use addr::EmailAddress;
pub use addr::identity::Identity;
pub use dates::zone;
pub use encoding::path;
pub use encoding::{base64, utf7};
pub use job::{
    Event, FailureKind, HeldKey, Job, LOCAL_CERTIFICATE, Op, PgpSignature, PgpWork, Private, REMOTE_CERTIFICATE, Revocation, Secret, SyncStatus,
    UntrustedCertificate,
};
pub use path::file_name;
pub use signature::Signature;
pub use summary::{FOLLOW_UP_COMPLETE, Flag, Flags, MessageKey, PINNED, SharedSummary, Smime, Summary, is_keyword, keyword_for, sort_newest_first};
// Clean Up is a part of conversations, kept under its old path.
pub use thread::cleanup;
// The work passes over hostile text do, counted for the tests.
pub use job::work;
// FNV-1a, for keys and fingerprints (with the message keys it mostly makes).
pub use summary::hash;
