//! The engine is sans-I/O: it never touches the network. It queues [`Job`]s, the network layer
//! runs them (one worker per account) and answers with [`Event`]s, which the engine applies.
//! The same split lets tests drive the engine with scripted events, and lets a web build plug in
//! a different transport.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{AccountConfig, Flag, Folder, MessageKey};

/// A password in transit. Its `Debug` form is redacted so it never reaches a log.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secret(pub String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() { "Secret(<empty>)" } else { "Secret(<redacted>)" })
    }
}

/// What a protected message says in clear (a message about to be signed or encrypted, a decrypted
/// one), in transit between the engine and the network layer. Its `Debug` form gives only its
/// size, so it never reaches a log.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Private(pub Vec<u8>);

impl fmt::Debug for Private {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Private(<{} bytes>)", self.0.len())
    }
}

/// A change to the mailbox the user makes, applied to the local cache at once and replayed on
/// the server when it can be reached (RFC 4549's "pending actions").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Op {
    SetFlag {
        keys: Vec<MessageKey>,
        flag: Flag,
        on: bool,
    },
    /// Add or remove a keyword (a category, `$Pinned`).
    SetKeyword {
        keys: Vec<MessageKey>,
        keyword: String,
        on: bool,
    },
    /// Move to another folder of the same account.
    Move {
        keys: Vec<MessageKey>,
        to: String,
    },
    /// Delete permanently (`\Deleted` + `UID EXPUNGE`).
    Expunge {
        keys: Vec<MessageKey>,
    },
    /// Create a folder (`CREATE`, then `SUBSCRIBE`). `path` is the server's name for it.
    CreateFolder {
        path: String,
    },
    /// Rename or move a folder; folders inside it move with it (`RENAME`).
    RenameFolder {
        from: String,
        to: String,
    },
    /// Delete a folder and the mail in it (`DELETE`). The engine deletes subfolders first.
    DeleteFolder {
        path: String,
    },
    /// Delete every message in a folder for good (Deleted Items, Junk Email), including those
    /// not downloaded.
    EmptyFolder {
        path: String,
    },
}

/// Automatic replies (out of office): sent once per sender while on, between the dates when
/// set. Kept on the server, so they work while this computer is off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vacation {
    pub enabled: bool,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub text: String,
    /// First and last day (`YYYY-MM-DD`), when limited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Exchange: the reply to people outside the organisation, when it isn't the same as `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<String>,
    /// Exchange: who outside the organisation gets a reply: `all` (the default, empty), `known`
    /// (the user's contacts only) or `none`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outside: String,
}

impl Vacation {
    /// The reply to people outside the organisation.
    pub fn external_text(&self) -> &str {
        self.external.as_deref().unwrap_or(&self.text)
    }
}

/// A `YYYY-MM-DD` date that is a real day.
pub fn is_day(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return false;
    }
    let num = |a: usize, z: usize| s.get(a..z).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse::<u32>().ok());
    match (num(0, 4), num(5, 7), num(8, 10)) {
        (Some(y), Some(m), Some(d)) => {
            let days = match m {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
                2 => 28,
                _ => 0,
            };
            y >= 1970 && (1..=days).contains(&d)
        }
        _ => false,
    }
}

impl Op {
    /// The messages a change touches (none for folder changes).
    pub fn keys(&self) -> &[MessageKey] {
        match self {
            Op::SetFlag { keys, .. } | Op::SetKeyword { keys, .. } | Op::Move { keys, .. } | Op::Expunge { keys } => keys,
            Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } | Op::EmptyFolder { .. } => &[],
        }
    }

    /// Whether this changes the folder list rather than messages.
    pub fn is_folder_op(&self) -> bool {
        matches!(self, Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. })
    }
}

/// A published MailCraft release.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// As tagged (`v0.19.0-test.15`).
    pub tag: String,
    /// `0.19.0`, and its test build (15; 0 for none).
    pub version: String,
    pub build: u32,
    /// The commit it was built from (empty when not said).
    pub commit: String,
    /// Its page (release notes and every file).
    pub page: String,
    /// The installer for this system, when there is one to offer (else the page).
    pub download: String,
    /// That installer's SHA-256 (lowercase hex) as GitHub records it, when it does.
    #[serde(default)]
    pub sha256: String,
    /// That installer's size in bytes (0 when not said).
    #[serde(default)]
    pub size: u64,
    /// The release's `SHA256SUMS.txt`, for checking a download GitHub gives no digest for.
    #[serde(default)]
    pub sums: String,
}

/// Work for the network layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "job", rename_all = "camelCase")]
pub enum Job {
    /// Start (or restart) the worker for an account. `password` is set when the user just typed
    /// it (the app stores it in the system keychain); otherwise the keychain is asked.
    Start { account: AccountConfig, password: Option<Secret> },
    /// Keep the outgoing server's own password (an account whose SMTP sign-in isn't the incoming
    /// server's: `smtp_username`), in the system keychain apart from the account's password.
    SmtpPassword { account: AccountConfig, password: Secret },
    /// Stop the worker and forget the stored password.
    Remove { account: String },
    /// Refresh the folder list and every folder.
    SyncAll { account: String },
    /// Refresh one folder (new mail, flag changes, deletions).
    SyncFolder { account: String, folder: String },
    /// Keep `more` of a folder's older messages than it keeps now, fetched from the server
    /// (issue #4: mail past what's kept on this computer, shown when asked for).
    MoreMail { account: String, folder: String, more: u32 },
    /// Search these folders on the server for `query` (the search box's words), for mail past
    /// what's kept here; the messages found come into their folders' lists, answered by
    /// [`Event::ServerSearched`], as they're found. `search` names the search: every account's
    /// share of one carries the same, and a newer one (or [`Job::StopSearch`]) stops it.
    SearchServer { account: String, folders: Vec<String>, query: String, search: u64 },
    /// Stop the servers' searches (the search box cleared, issue #4): what's found so far stays.
    StopSearch,
    /// Download a message's full source.
    FetchBody { key: MessageKey },
    /// Apply a mailbox change on the server.
    Apply { account: String, op: Op },
    /// Submit a message over SMTP, then file a copy in the Sent folder (when the server doesn't
    /// already, which the worker can't know: so always when a Sent folder exists).
    Send { account: String, id: u64, raw: Vec<u8>, from: String, recipients: Vec<String>, sent_folder: Option<String> },
    /// Store a draft (`APPEND` with `\Draft`), replacing an earlier copy.
    SaveDraft { account: String, raw: Vec<u8>, folder: String, replaces: Option<MessageKey> },
    /// A draft kept on this computer at once, filed in Drafts on the server at the next
    /// connection: MailCraft is closing with the message being written.
    KeepDraft { account: String, raw: Vec<u8>, folder: String, replaces: Option<MessageKey> },
    /// Try to send what waits in the Outbox now.
    FlushOutbox { account: String },
    /// Download a picture a message links to (only after the user asked to see pictures).
    FetchUrl { url: String },
    /// Look up the server settings for an address (autoconfiguration).
    Discover { email: String },
    /// Sign an OAuth account in through the browser; it starts once signed in.
    SignIn { account: AccountConfig },
    /// Work offline (every account's connection closed, changes and mail kept to send later), or
    /// connect again.
    WorkOffline { on: bool },
    /// How much mail is kept on this computer: the newest `messages` of each folder (0: all of
    /// them), and the newest `offline` of them downloaded in full ahead of reading.
    SetSync { messages: u32, offline: u32 },
    /// Read the account's automatic replies from the server.
    GetVacation { account: String },
    /// Change them.
    SetVacation { account: String, vacation: Vacation },
    /// Sync the account's calendars and address books with its CalDAV / CardDAV server (changes
    /// made here first).
    SyncDav { account: String },
    /// Ask where MailCraft is published for its recent releases (File › About › Check for
    /// updates); answered by [`Event::Releases`].
    CheckUpdates,
    /// Read an Outlook data file (`.pst`) into an account with no server.
    ImportPst { account: String, path: String },
    /// Write the account's mail to an Outlook data file, which opens as `name`.
    ExportPst { account: String, path: String, name: String },
    /// Keep the certificate and private key in a `.p12` / `.pfx` file (opened with `password`)
    /// for signing and decrypting the account's mail (S/MIME): the key goes to the system's
    /// keychain only. Answered by [`Event::Identity`].
    ImportIdentity { account: AccountConfig, data: Vec<u8>, password: Secret },
    /// Forget the account's certificate and key (the keychain's copy goes).
    ForgetIdentity { account: AccountConfig },
    /// Sign (`sign`) and/or encrypt (to `encrypt_to`, certificates as DER, the account's own
    /// among them; empty: not encrypted) a message about to be sent, with the account's key;
    /// `gcm`: AES-GCM, every recipient reads it. Its header fields go inside too (RFC 9788);
    /// encrypted with `hide_subject`, the Subject outside is `[...]`. Answered by
    /// [`Event::Protected`].
    Protect { account: AccountConfig, id: u64, raw: Private, sign: bool, encrypt_to: Vec<Vec<u8>>, gcm: bool, hide_subject: bool },
    /// Decrypt an encrypted message (its `smime.p7m`, `cms`) with the account's key. Answered by
    /// [`Event::Decrypted`].
    Decrypt { account: AccountConfig, key: MessageKey, cms: Vec<u8> },
    /// A draft of a message to be encrypted: encrypted to the account's own certificate first, so
    /// the copy in Drafts is no more readable than what will be sent, then saved as
    /// [`Job::SaveDraft`] (or kept as [`Job::KeepDraft`], `keep`).
    SealDraft { account: AccountConfig, raw: Private, folder: String, replaces: Option<MessageKey>, keep: bool },
    /// Ask a signer's certificate's authority whether it withdrew it (OCSP, else its CRL, where
    /// the certificate says): `chain` is the certificate first, then each issuer. Only with File ›
    /// Options › Safety's switch on: the authority learns whose mail is read. Answered by
    /// [`Event::Revocation`].
    CheckRevocation { chain: Vec<Vec<u8>> },
}

impl Job {
    pub fn account(&self) -> &str {
        match self {
            Job::Start { account, .. } | Job::SmtpPassword { account, .. } => &account.id,
            Job::FetchBody { key } => &key.account,
            Job::Remove { account }
            | Job::SyncAll { account }
            | Job::SyncFolder { account, .. }
            | Job::MoreMail { account, .. }
            | Job::SearchServer { account, .. }
            | Job::Apply { account, .. }
            | Job::Send { account, .. }
            | Job::SaveDraft { account, .. }
            | Job::KeepDraft { account, .. }
            | Job::FlushOutbox { account }
            | Job::GetVacation { account }
            | Job::SetVacation { account, .. }
            | Job::SyncDav { account }
            | Job::ImportPst { account, .. }
            | Job::ExportPst { account, .. } => account,
            Job::FetchUrl { .. }
            | Job::Discover { .. }
            | Job::WorkOffline { .. }
            | Job::SetSync { .. }
            | Job::CheckUpdates
            | Job::StopSearch
            | Job::CheckRevocation { .. } => "",
            Job::SignIn { account }
            | Job::ImportIdentity { account, .. }
            | Job::ForgetIdentity { account }
            | Job::Protect { account, .. }
            | Job::SealDraft { account, .. }
            | Job::Decrypt { account, .. } => &account.id,
        }
    }
}

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
    /// The last attempt failed; the worker retries with back-off.
    Error {
        error: String,
    },
}

impl SyncStatus {
    pub fn is_online(&self) -> bool {
        matches!(self, SyncStatus::Online { .. } | SyncStatus::Syncing { .. })
    }
}

/// What the network layer reports back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Event {
    Status {
        account: String,
        status: SyncStatus,
    },
    /// The folder list (with counts) changed.
    Folders {
        account: String,
        folders: Vec<Folder>,
    },
    /// A folder's cached contents changed on disk: reload it.
    FolderChanged {
        account: String,
        folder: String,
    },
    /// A message's source arrived (it is cached on disk too).
    Body {
        key: MessageKey,
        raw: Vec<u8>,
    },
    BodyFailed {
        key: MessageKey,
        error: String,
    },
    Sent {
        account: String,
        id: u64,
    },
    SendFailed {
        account: String,
        id: u64,
        error: String,
    },
    /// The message couldn't be sent yet (offline, or the server asked to try later): it waits in
    /// the Outbox and goes out when the account is back online.
    Queued {
        account: String,
        id: u64,
        error: String,
    },
    /// A mailbox change could not be applied on the server (it was rolled back locally).
    OpFailed {
        account: String,
        op: Op,
        error: String,
    },
    /// MailCraft's recent releases (newest first), or why they couldn't be had.
    Releases {
        releases: Vec<Release>,
        error: Option<String>,
    },
    /// A downloaded picture (`data`), or why it couldn't be fetched.
    Fetched {
        url: String,
        data: Option<Vec<u8>>,
        error: Option<String>,
    },
    /// Server settings found for an address (`None`: nothing published; guess instead).
    Discovered {
        email: String,
        config: Option<Box<AccountConfig>>,
        source: String,
    },
    /// Show this page in the browser (an OAuth sign-in page).
    OpenUrl {
        url: String,
    },
    /// An OAuth account is signed in (its token is in the keychain): start it. `user` is who
    /// the provider says signed in, when it said.
    SignedIn {
        account: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
    },
    /// How much of the account's space on the server is used and its limit, in bytes (`limit`
    /// 0: no limit published).
    Quota {
        account: String,
        used: u64,
        limit: u64,
    },
    /// The account's automatic replies as the server has them; `None` with an error when it
    /// offers none (or they couldn't be read or changed).
    Vacation {
        account: String,
        vacation: Option<Vacation>,
        error: Option<String>,
    },
    /// A CalDAV / CardDAV sync's result, to merge.
    DavSynced {
        account: String,
        sync: Box<crate::pim::DavSync>,
    },
    /// How a data file import is going (`done`: finished, or failed with `error`). The folders
    /// come as [`Event::Folders`] first.
    Imported {
        account: String,
        messages: usize,
        skipped: usize,
        done: bool,
        error: Option<String>,
    },
    /// How an export to a data file is going (`done`: finished, or failed with `error`).
    Exported {
        account: String,
        messages: usize,
        skipped: usize,
        done: bool,
        error: Option<String>,
    },
    /// Something the user should see in the status bar.
    Notice {
        account: String,
        message: String,
    },
    /// What the server found for a [`Job::SearchServer`]: the messages, now in their folders'
    /// lists, or why it couldn't search. Sent as they're found (more to come until `done`).
    ServerSearched {
        account: String,
        query: String,
        keys: Vec<MessageKey>,
        error: Option<String>,
        done: bool,
    },
    /// What [`Job::ImportIdentity`] kept: the certificate and its issuers (DER), or why not.
    Identity {
        account: String,
        chain: Vec<Vec<u8>>,
        error: Option<String>,
    },
    /// A message [`Job::Protect`] signed and/or encrypted, ready to send, or why it couldn't be.
    Protected {
        account: String,
        id: u64,
        raw: Option<Private>,
        error: Option<String>,
    },
    /// What [`Job::Decrypt`] found inside (a MIME entity); `authenticated`: it was encrypted with
    /// AES-GCM, so it can't have been changed on the way.
    Decrypted {
        key: MessageKey,
        content: Option<Private>,
        authenticated: bool,
        error: Option<String>,
    },
    /// What [`Job::CheckRevocation`] heard about `cert` (DER, the chain's first).
    Revocation {
        cert: Vec<u8>,
        status: Revocation,
    },
}

/// Whether a certificate's authority withdrew it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Revocation {
    /// Not withdrawn; the answer holds until `until` (Unix seconds), when it says.
    Good { until: Option<i64> },
    /// Withdrawn at `at` (Unix seconds).
    Revoked { at: i64 },
    /// No answer to go by (none given, or none to trust), and why.
    Unknown { why: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_never_print() {
        let j = Job::Start { account: AccountConfig::default(), password: Some(Secret("hunter2".into())) };
        assert!(!format!("{j:?}").contains("hunter2"));
        let e = Event::Decrypted { key: MessageKey::default(), content: Some(Private(b"the plan".to_vec())), authenticated: true, error: None };
        assert!(!format!("{e:?}").contains("the plan"), "what decrypted mail says never prints");
    }

    #[test]
    fn days() {
        for ok in ["2026-10-15", "2024-02-29", "2000-02-29"] {
            assert!(is_day(ok), "{ok}");
        }
        for bad in ["2026-02-29", "1900-02-29", "2026-13-01", "2026-1-01", "26-10-15", "2026-10-15T00", "", "2026-10-00", "abcd-ef-gh"] {
            assert!(!is_day(bad), "{bad}");
        }
    }

    #[test]
    fn jobs_serialize_tagged() {
        let j = Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() };
        let v = serde_json::to_value(&j).unwrap();
        assert_eq!(v["job"], "syncFolder");
        assert_eq!(serde_json::from_value::<Job>(v).unwrap(), j);
        assert_eq!(j.account(), "a1");
    }
}
