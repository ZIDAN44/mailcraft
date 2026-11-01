//! The engine is sans-I/O: it never touches the network. It queues [`Job`]s, the network layer
//! runs them (one worker per account) and answers with [`Event`]s, which the engine applies.
//! The same split lets tests drive the engine with scripted events, and lets a web build plug in
//! a different transport.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{AccountConfig, Folder, MessageKey};

pub mod eta;
mod import;
mod keys;
mod op;
pub mod progress;
mod vacation;
pub use eta::Eta;
pub use import::*;
pub use keys::{HeldKey, Revocation};
pub use op::Op;
pub use progress::Work;
pub use vacation::{Vacation, is_day};

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
    /// Keep the password of an account's calendar and contacts server (its [`crate::DavSettings`]'s
    /// own user name) in the keychain.
    DavPassword { account: AccountConfig, password: Secret },
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
    /// Bring the account's search index up to its mail in these folders (none: every folder),
    /// in the background; asks close together are one pass. Progress and the end come as
    /// [`Event::Indexed`].
    Index { account: String, folders: Vec<String> },
    /// Stop the account's index pass (what it added is kept) and forget what was asked of it.
    StopIndex { account: String },
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
    /// Read a calendar subscribed to by its link (issue #7), unless it's unchanged since the
    /// version named by `etag` / `modified`; answered by [`Event::CalendarFetched`].
    FetchCalendar { account: String, calendar: String, url: String, etag: String, modified: String },
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
    /// Look people up in the account's organisation directory (Exchange's Global Address List,
    /// issue #9), asked of the server each time; answered by [`Event::Directory`].
    SearchDirectory { account: String, query: String },
    /// Ask the user, in the browser, to let an Exchange Online account search its organisation's
    /// directory (issue #9: a sign-in of its own, for the directory's permissions alone); answered
    /// by [`Event::DirectoryAllowed`].
    AllowDirectory { account: AccountConfig },
    /// Ask where MailCraft is published for its recent releases (File › About › Check for
    /// updates); answered by [`Event::Releases`].
    CheckUpdates,
    /// Read an Outlook data file (`.pst`) into an account with no server; `resume`: go on with an
    /// import that stopped, from where it stopped. Progress comes as [`Event::ImportProgress`],
    /// the end as [`Event::Imported`].
    ImportPst {
        account: String,
        path: String,
        #[serde(default)]
        resume: bool,
    },
    /// Look at a data file before importing it: what it holds, and the room its copy needs.
    /// Answered by [`Event::DataFileInspected`].
    InspectDataFile { path: String },
    /// Stop an account's import, keeping what came in or not as `how` says. Answered by
    /// [`Event::Imported`].
    StopImport { account: String, how: StopHow },
    /// Write the account's mail to an Outlook data file, which opens as `name`.
    ExportPst { account: String, path: String, name: String },
    /// Keep the certificate and private key in a `.p12` / `.pfx` file (opened with `password`)
    /// for signing and decrypting the account's mail (S/MIME): the key goes to the system's
    /// keychain only. Answered by [`Event::Identity`].
    ImportIdentity { account: AccountConfig, data: Vec<u8>, password: Secret },
    /// Forget the account's certificate and key (the keychain's copy goes). Older ones kept for
    /// reading stay.
    ForgetIdentity { account: AccountConfig },
    /// Forget an older certificate kept to read mail encrypted to it (`fingerprint`, its
    /// SHA-256 as `AB:CD:…` hex pairs), and its key.
    ForgetOlderIdentity { account: AccountConfig, fingerprint: String },
    /// The browser only (gap 7): keep the account's certificates unlocked between visits (`on`),
    /// their password encrypted with a key the browser won't let out, or stop and forget it.
    KeepUnlocked { account: AccountConfig, on: bool },
    /// Sign and encrypt with an older certificate again (`fingerprint`): the one used until now is
    /// kept for reading. Answered by [`Event::Identity`].
    UseOlderIdentity { account: AccountConfig, fingerprint: String },
    /// Sign (`sign`) and/or encrypt (to `encrypt_to`, certificates as DER, the account's own
    /// among them; empty: not encrypted) a message about to be sent, with the account's key;
    /// `gcm`: AES-GCM, every recipient reads it. Its header fields go inside too (RFC 9788);
    /// encrypted with `hide_subject`, the Subject outside is `[...]`. Answered by
    /// [`Event::Protected`].
    Protect { account: AccountConfig, id: u64, raw: Private, sign: bool, encrypt_to: Vec<Vec<u8>>, gcm: bool, hide_subject: bool },
    /// Decrypt an encrypted message (its `smime.p7m`, `cms`) with the account's key, or the older
    /// one it was encrypted to. Answered by [`Event::Decrypted`].
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
    /// The certificates whose private keys are held where MailCraft can't read them: on the smart
    /// cards the PKCS #11 module at `module` sees (logging in with `pin`), or with no module in
    /// the system's keychain (macOS). Answered by [`Event::HeldKeys`].
    FindHeldKeys { module: Option<String>, pin: Option<Secret> },
    /// Sign and decrypt the account's mail with a key [`Job::FindHeldKeys`] found (`at`, its
    /// [`HeldKey::at`]; `pin` kept in the system's keychain for it). Answered by
    /// [`Event::Identity`].
    UseHeldKey { account: AccountConfig, at: String, cert: Vec<u8>, pin: Option<Secret> },
}

impl Job {
    pub fn account(&self) -> &str {
        match self {
            Job::Start { account, .. } | Job::SmtpPassword { account, .. } | Job::DavPassword { account, .. } => &account.id,
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
            | Job::SearchDirectory { account, .. }
            | Job::FetchCalendar { account, .. }
            | Job::ImportPst { account, .. }
            | Job::StopImport { account, .. }
            | Job::Index { account, .. }
            | Job::StopIndex { account }
            | Job::ExportPst { account, .. } => account,
            Job::FetchUrl { .. }
            | Job::InspectDataFile { .. }
            | Job::Discover { .. }
            | Job::WorkOffline { .. }
            | Job::SetSync { .. }
            | Job::CheckUpdates
            | Job::StopSearch
            | Job::CheckRevocation { .. }
            | Job::FindHeldKeys { .. } => "",
            Job::SignIn { account }
            | Job::AllowDirectory { account }
            | Job::ImportIdentity { account, .. }
            | Job::ForgetIdentity { account }
            | Job::ForgetOlderIdentity { account, .. }
            | Job::KeepUnlocked { account, .. }
            | Job::UseOlderIdentity { account, .. }
            | Job::Protect { account, .. }
            | Job::SealDraft { account, .. }
            | Job::UseHeldKey { account, .. }
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
    /// What [`Job::FetchCalendar`] read: the calendar (`ics`) with the version the server named,
    /// unchanged since the one asked about, or why it couldn't be read.
    CalendarFetched {
        account: String,
        calendar: String,
        ics: Option<String>,
        etag: String,
        modified: String,
        unchanged: bool,
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
    /// How a data file import is going (the newest replaces the ones before).
    ImportProgress {
        account: String,
        progress: ImportProgress,
    },
    /// A data file import ended (`done`: finished, stopped, or failed with `error`; an earlier
    /// version also said how it was going with `done` false). The folders come as
    /// [`Event::Folders`] first. `report`: what it came to, every loss by its reason.
    Imported {
        account: String,
        messages: usize,
        skipped: usize,
        done: bool,
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        report: Option<Box<ImportReport>>,
    },
    /// How a [`Job::Index`] pass is going (the newest replaces the ones before): the messages
    /// read of those it found to add, the index's generation now (a search reads again past
    /// it), and `finished` at its end, stopped or not. `failed`: messages that couldn't be read
    /// for search (kept as having no words).
    Indexed {
        account: String,
        work: Work,
        generation: u64,
        #[serde(default)]
        failed: u64,
        finished: bool,
    },
    /// The index found which of a folder's messages carry files (`with`) and which only
    /// pictures their text shows (`without`), by uid, where their summaries said otherwise.
    AttachmentsKnown {
        account: String,
        folder: String,
        with: Vec<u32>,
        without: Vec<u32>,
    },
    /// What [`Job::InspectDataFile`] found (its `error` says why it couldn't look).
    DataFileInspected {
        path: String,
        facts: Box<DataFileFacts>,
    },
    /// How an export to a data file is going (`done`: finished, or failed with `error`).
    Exported {
        account: String,
        messages: usize,
        skipped: usize,
        /// Messages a data file already had, not written again (issue #18).
        #[serde(default)]
        already: usize,
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
    /// Who the directory found for a [`Job::SearchDirectory`]: (name, address) each, or why it
    /// couldn't look. `consent`: the account needs the user's permission first
    /// ([`Job::AllowDirectory`]).
    Directory {
        account: String,
        query: String,
        people: Vec<(String, String)>,
        error: Option<String>,
        consent: bool,
    },
    /// What [`Job::AllowDirectory`] came to: allowed, or why not.
    DirectoryAllowed {
        account: String,
        error: Option<String>,
    },
    /// What [`Job::ImportIdentity`] kept: the certificate that signs and encrypts and its issuers
    /// (DER), and the older ones kept only to read mail encrypted to them (each one's certificate);
    /// or why not. A certificate older than the one in use is kept for reading only.
    Identity {
        account: String,
        chain: Vec<Vec<u8>>,
        older: Vec<Vec<u8>>,
        /// The certificate just kept went among the older ones (it's older than the one in use).
        for_reading: bool,
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
    /// What [`Job::FindHeldKeys`] found, or why it couldn't look.
    HeldKeys {
        keys: Vec<HeldKey>,
        error: Option<String>,
    },
    /// What [`Job::CheckRevocation`] heard about `cert` (DER, the chain's first).
    Revocation {
        cert: Vec<u8>,
        status: Revocation,
    },
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
    fn jobs_serialize_tagged() {
        let j = Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() };
        let v = serde_json::to_value(&j).unwrap();
        assert_eq!(v["job"], "syncFolder");
        assert_eq!(serde_json::from_value::<Job>(v).unwrap(), j);
        assert_eq!(j.account(), "a1");
        let j = Job::Index { account: "a1".into(), folders: vec!["INBOX".into()] };
        assert_eq!((serde_json::from_str::<Job>(&serde_json::to_string(&j).unwrap()).unwrap(), j.account()), (j.clone(), "a1"));
        assert_eq!(Job::StopIndex { account: "a1".into() }.account(), "a1");
        let e =
            Event::Indexed { account: "a1".into(), work: Work { done: 1, total: 2, ..Work::default() }, generation: 3, failed: 0, finished: false };
        assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    }

    #[test]
    fn import_events_read_what_earlier_versions_wrote() {
        // As an earlier version sent them: no report, no `resume`.
        let old = r#"{"event":"imported","account":"a1","messages":3,"skipped":1,"done":true,"error":null}"#;
        let e: Event = serde_json::from_str(old).unwrap();
        assert_eq!(e, Event::Imported { account: "a1".into(), messages: 3, skipped: 1, done: true, error: None, report: None });
        assert_eq!(serde_json::to_value(&e).unwrap(), serde_json::from_str::<serde_json::Value>(old).unwrap(), "written as it was");
        let j: Job = serde_json::from_str(r#"{"job":"importPst","account":"a1","path":"a.pst"}"#).unwrap();
        assert_eq!(j, Job::ImportPst { account: "a1".into(), path: "a.pst".into(), resume: false });

        // And the new ones round trip.
        let mut report = ImportReport { messages: 3, runs: 1, ..Default::default() };
        report.skip(SkipReason::AttachmentTooBig, 2);
        let e = Event::Imported { account: "a1".into(), messages: 3, skipped: 1, done: true, error: None, report: Some(Box::new(report)) };
        assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
        let progress = ImportProgress { phase: ImportPhase::Importing, work: Work { done: 1, total: 9, ..Default::default() }, ..Default::default() };
        let e = Event::ImportProgress { account: "a1".into(), progress };
        assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
        let j = Job::StopImport { account: "a1".into(), how: StopHow::Discard };
        assert_eq!((serde_json::from_str::<Job>(&serde_json::to_string(&j).unwrap()).unwrap(), j.account()), (j.clone(), "a1"));
        assert_eq!(Job::InspectDataFile { path: "a.pst".into() }.account(), "");
    }
}
