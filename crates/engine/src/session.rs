//! The session: accounts, the current view and its message list, the open message, the message
//! being written, preferences, and the queue of jobs for the network layer.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use mailcraft_mime::{self as mime, OutAttachment, Parsed, ReplyMode};
use mailcraft_model::contacts::{Contact, ContactBook};
use mailcraft_model::dates::{Clock, DateGroup};
use mailcraft_model::job::{Secret, Vacation};
use mailcraft_model::search::{Doc, Query, SummaryText, TextField, TextSource, Tri};
use mailcraft_model::thread::{Thread, threads};
use mailcraft_model::{
    AccountConfig, EmailAddress, Event, Flag, Folder, FolderRole, Job, MessageKey, Op, Summary, SyncStatus, addr, sort_folders, sort_newest_first,
};
use mailcraft_store::{FolderMarks, FolderState, LongWrite, MarksAfter, OutboxItem, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CmdError, CmdResult, Registry};

mod accounts;
mod changing;
mod events;
mod folder_edits;
pub mod invite;
mod mailbox_settings;
mod mailboxes;
mod opening;
mod organize;
mod outbox;
mod pictures;
mod prefs;
mod searching;
mod security;
mod writing;
#[cfg(test)]
pub(crate) use accounts::REMOVE_BACKSTOP;
pub use accounts::{ImportState, Imports};
pub use mailbox_settings::{Compacting, Compaction, IDLE_AFTER_S};
#[cfg(test)]
pub(crate) use opening::source_lines;
pub use opening::{BodyWork, PreparedBody, SOURCE_SHOWN, SourceView, Toward};
pub use searching::{IndexPass, SearchProgress, SearchStats, SearchWork};
pub use security::{CardSearch, KnownCert, Lock, RevocationCheck, SMIME_DOC, Seal, Security, SignedBy, SmimeSettings};
pub use writing::{Compose, SendCheck, signature_parts};
// The message list's arrangement and rows, named here as before.
#[cfg(test)]
pub(crate) use mailboxes::arrange_list;
pub use mailboxes::{ARRANGE_HEADINGS, Arrange, Row};
// Organizing mail's own parts, named here as before.
pub use opening::SenderMarks;
pub use organize::{junk, quick_steps, search_folders};
pub use prefs::{ACCENTS, Prefs, SenderAddresses, accent_code, accent_rgb};
pub mod pim;

/// One account and what the session knows about it.
#[derive(Clone, Debug, PartialEq)]
pub struct AccountState {
    pub config: AccountConfig,
    pub folders: Vec<Folder>,
    pub status: SyncStatus,
    /// Space used on the server and its limit in bytes, once reported.
    pub quota: Option<(u64, u64)>,
    /// The automatic replies as last read from the server.
    pub vacation: VacationState,
}

/// What is known of an account's automatic replies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum VacationState {
    #[default]
    Unknown,
    /// Asked the server.
    Loading,
    Loaded(Vacation),
    /// The server offers none, or reading or changing them failed.
    Failed(String),
}

impl AccountState {
    pub fn folder(&self, role: FolderRole) -> Option<&Folder> {
        self.folders.iter().find(|f| f.role == role)
    }

    /// A local-only account (the sample mailbox): no server, changes apply to the cache alone.
    pub fn is_local(&self) -> bool {
        self.config.is_local()
    }

    /// An Outlook data file opened here (its "address" is the file's name).
    pub fn is_data_file(&self) -> bool {
        self.is_local() && !addr::is_valid_email(&self.config.email)
    }
}

/// File › About › Check for updates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdateCheck {
    #[default]
    NotAsked,
    Checking,
    /// This is the newest (the newest release's tag, when there is one).
    UpToDate(String),
    /// A newer one is out.
    Available(mailcraft_model::job::Release),
    Failed(String),
}

/// The newest release `releases` has for a build of `version` made from commit `build_sha`:
/// `Some` when it's newer. A newer version is; the same version at a later test build is when
/// this build is one of that version's earlier test builds (by its commit). A build made from
/// no release (a developer's), ahead or the same, is up to date.
pub fn newer_release<'a>(
    version: &str,
    build_sha: Option<&str>,
    releases: &'a [mailcraft_model::job::Release],
) -> Option<&'a mailcraft_model::job::Release> {
    let parse = |v: &str| -> Option<(u32, u32, u32)> {
        let mut p = v.split('.').map(|n| n.parse::<u32>().ok());
        Some((p.next()??, p.next()??, p.next()??))
    };
    let current = parse(version)?;
    let newest = releases.iter().filter_map(|r| parse(&r.version).map(|v| (v, r))).max_by_key(|(v, r)| (*v, r.build))?;
    match newest.0.cmp(&current) {
        std::cmp::Ordering::Greater => Some(newest.1),
        std::cmp::Ordering::Less => None,
        std::cmp::Ordering::Equal => {
            let sha = build_sha.map(str::trim).filter(|s| s.len() >= 7)?;
            let mine = releases.iter().find(|r| !r.commit.is_empty() && (r.commit.starts_with(sha) || sha.starts_with(&r.commit)))?;
            (mine.version == newest.1.version && mine.build < newest.1.build).then_some(newest.1)
        }
    }
}

/// What the message list shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum View {
    /// Nothing yet (no accounts).
    Empty,
    Folder {
        account: String,
        folder: String,
    },
    /// The same folder of every account (Favorites › All Inboxes).
    Unified {
        role: FolderRole,
    },
    /// Messages waiting to be sent, from every account.
    Outbox,
}

/// The pseudo-folder of Outbox message keys (NUL never appears in a server's folder names).
pub const OUTBOX: &str = "\u{0}outbox";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListFilter {
    #[default]
    All,
    Unread,
    Flagged,
    Attachments,
    /// Carrying a category.
    Categorized,
    /// Marked high importance by the sender.
    Important,
    /// Sent to the account's address, or copied to it.
    ToMe,
}

impl ListFilter {
    /// The filters shown as tabs above the list.
    pub const ALL: [ListFilter; 4] = [ListFilter::All, ListFilter::Unread, ListFilter::Flagged, ListFilter::Attachments];
    /// Every filter (the Home tab's Filter menu).
    pub const EVERY: [ListFilter; 7] = [
        ListFilter::All,
        ListFilter::Unread,
        ListFilter::Flagged,
        ListFilter::Attachments,
        ListFilter::Categorized,
        ListFilter::Important,
        ListFilter::ToMe,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ListFilter::All => "All",
            ListFilter::Unread => "Unread",
            ListFilter::Flagged => "Flagged",
            ListFilter::Attachments => "Has files",
            ListFilter::Categorized => "Categorized",
            ListFilter::Important => "Important",
            ListFilter::ToMe => "To or Cc me",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            ListFilter::All => "all",
            ListFilter::Unread => "unread",
            ListFilter::Flagged => "flagged",
            ListFilter::Attachments => "attachments",
            ListFilter::Categorized => "categorized",
            ListFilter::Important => "important",
            ListFilter::ToMe => "toMe",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchScope {
    /// Every folder of every account (Outlook's All Mailboxes).
    All,
    /// Every folder of the account on view (Outlook's Current Mailbox, its default); every
    /// account's from a unified view.
    #[default]
    Mailbox,
    /// The folder on view and the folders inside it.
    Subfolders,
    /// The folder (or unified view) on screen.
    Current,
}

impl SearchScope {
    pub const ALL: [SearchScope; 4] = [SearchScope::Current, SearchScope::Subfolders, SearchScope::Mailbox, SearchScope::All];

    /// Its name in commands (`search.run`'s `scope`).
    pub fn code(self) -> &'static str {
        match self {
            SearchScope::All => "all",
            SearchScope::Mailbox => "mailbox",
            SearchScope::Subfolders => "subfolders",
            SearchScope::Current => "current",
        }
    }

    pub fn from_code(code: &str) -> Option<SearchScope> {
        SearchScope::ALL.into_iter().find(|s| s.code().eq_ignore_ascii_case(code))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadingPane {
    #[default]
    Right,
    Bottom,
    Off,
}

/// How notes are listed (Outlook's Arrange by for notes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotesOrder {
    /// The last changed first.
    #[default]
    Modified,
    /// A to Z by their first line.
    Title,
    /// By colour, then A to Z.
    Color,
}

/// When a message shown in the reading pane is marked read (with `Prefs::mark_read_on_open`),
/// as Outlook's Reading Pane options have it. Opened in its own window, it always is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadInPane {
    /// When it's shown (after `Prefs::read_after_secs`).
    #[default]
    Shown,
    /// When another message is chosen (or the folder left).
    Left,
}

/// A search asked of the servers ([`Session::search_server`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerSearch {
    /// Which search it is (the jobs' `search`), so a newer one stops it.
    pub id: u64,
    pub query: String,
    /// The query as the servers were asked it: categories by their keywords
    /// ([`Session::server_query`]); their answers name it.
    pub sent: String,
    /// Accounts still searching.
    pub pending: usize,
    /// What the servers found (in their folders' lists now), shown whether or not the words are
    /// in what's downloaded here.
    pub found: HashSet<MessageKey>,
    /// Why an account couldn't search.
    pub errors: Vec<String>,
}

/// A mailbox in the folder pane: an account, or a shared mailbox one opens ([`Session::pane_sections`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneSection {
    /// The account's id, or `<account>|<address>` for a shared mailbox.
    pub key: String,
    pub account: String,
    /// A shared mailbox: its top folder's path.
    pub mailbox: Option<String>,
}

/// A change the user made that Undo can put back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undo {
    /// Messages moved (deleted to Deleted Items, archived, moved): found again in `to` by their
    /// Message-ID (a move gives them new keys) and moved back where each came from.
    Move { account: String, to: String, back: Vec<(String, String)> },
    /// Flags or keywords changed: the changes that put them back.
    Ops { account: String, ops: Vec<Op> },
}

/// Changes Undo remembers.
const UNDO_DEPTH: usize = 20;

/// A folder of an account, by path.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderRef {
    pub account: String,
    pub folder: String,
}

/// Said of the files of a message whose text came first, while they come (issue #24).
pub(crate) const STILL_COMING: &str = "its attachments are still downloading: a moment";

/// The most messages open in their own windows at once.
pub const MAX_WINDOWS: usize = 8;

/// The message open in the reading pane (or in a window of its own).
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub key: MessageKey,
    /// Shared, so the pane and windows copy it each frame for nothing (a reply takes its own).
    pub parsed: Option<Arc<Parsed>>,
    pub error: Option<String>,
    /// The other messages of its conversation in this list, oldest first.
    pub thread: Vec<MessageKey>,
    /// Who it says it's from doesn't add up: a lookalike of a domain the user deals with, or
    /// another address shown in the sender's name.
    pub warning: Option<mailcraft_model::lookalike::SenderWarning>,
    /// Signed or encrypted (S/MIME), and how that went.
    pub security: Option<Security>,
    /// What's shown is its text first, its files still coming (issue #24).
    pub text_first: bool,
}

/// A status-bar message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub at: i64,
    pub error: bool,
}

/// File for the Replied/Forwarded flags still owed by replies and forwards in the Outbox.
const PENDING_MARKS: &str = "pending-marks.json";
/// The mail rules, and the snoozed mail to bring back.
const RULES_DOC: &str = "rules.json";
const SNOOZES_DOC: &str = "snoozes.json";

/// Message summaries kept in memory at most, besides the folders on screen (a big data file's
/// whole mailbox is several times this).
const CACHE_MESSAGES: usize = 300_000;

/// How many command panics [`Session::run`] has caught in this process: none is the only right
/// number, so tests that drive the whole window check it (a caught panic is otherwise only a
/// line in the log).
pub static PANICS_CAUGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub struct Session {
    pub store: Option<Store>,
    pub accounts: Vec<AccountState>,
    pub view: View,
    pub filter: ListFilter,
    /// The active search (query text and scope).
    pub search: Option<(String, SearchScope)>,
    /// The Search Folder last opened ([`Session::open_search_folder_name`]: while the search is
    /// still its own).
    pub search_folder: Option<String>,
    /// The search box's query asked of the servers too (issue #4: mail past what's kept here).
    pub server_search: Option<ServerSearch>,
    /// Server searches asked so far (each one's id).
    searches: u64,
    /// The recent searches while an account is kept online only (issue #4): in memory only,
    /// since what's searched for can tell what the mail says.
    recent_in_memory: Vec<String>,
    /// A folder whose older mail is being fetched (Show more), until its list changes.
    pub showing_more: Option<(String, String)>,
    /// Accounts whose password to ask for now (issue #4: a portable copy keeps none, so each
    /// start asks), once each while MailCraft runs; the window asks when nothing else is open.
    pub ask_password: Vec<String>,
    asked_password: HashSet<String>,
    /// The messages on screen, newest first.
    pub list: Vec<Summary>,
    /// Conversations over `list` (when [`Prefs::conversations`] is on).
    pub threads: Vec<Thread>,
    /// Where each message of `list` is (built with it).
    pub(crate) nav: mailboxes::ListNav,
    pub selection: Vec<MessageKey>,
    pub focus: Option<MessageKey>,
    /// Where a range of selected messages starts (Shift+click, Shift+Down): it stays put while
    /// `focus`, the other end, moves (issue #26).
    pub anchor: Option<MessageKey>,
    pub reading: Option<Reading>,
    /// Messages opened in their own windows (double-click), oldest first.
    pub windows: Vec<Reading>,
    /// Messages' sources, each shown in a window of its own (issue #27), oldest first.
    pub sources: Vec<SourceView>,
    /// The message being written that commands act on (`compose.*`).
    pub compose: Option<Compose>,
    /// Other messages being written, each in a window of its own (Outlook keeps every one
    /// open): starting another sets the current one aside here rather than losing it.
    /// [`Session::activate_compose`] brings one back to act on.
    pub composes: Vec<Compose>,
    pub prefs: Prefs,
    pub clock: Clock,
    pub notices: Vec<Notice>,
    /// What the last changes can be put back to, newest last (Undo, Ctrl+Z).
    pub undo: Vec<Undo>,
    /// Bumped whenever `list` changes.
    pub revision: u64,
    /// Bumped whenever a calendar or address book changes.
    pub pim_revision: u64,
    /// Messages waiting in the Outbox, per account (oldest first).
    pub outbox: Vec<(String, OutboxItem)>,
    /// Held-back Outbox messages already asked to be sent when their time came.
    flushed: BTreeSet<u64>,
    jobs: Vec<Job>,
    pub(crate) cache: HashMap<(String, String), FolderState>,
    /// The cached folders, the least recently used first.
    pub(crate) cache_order: Vec<(String, String)>,
    /// Flag and category changes sent for a server account's mail, their echo still to come
    /// (`mailboxes/echo.rs`).
    pub(crate) echoes: mailboxes::Echoes,
    /// Folders not read from disk while their read marks were still being written (the read
    /// would have waited for them in the window's frame): read once they're written. True for one
    /// that changed on disk meanwhile (its list in memory kept until then, as on screen).
    pub(crate) marks_deferred: BTreeMap<(String, String), bool>,
    /// Flags owed to originals, keyed by the Outbox id of the reply or forward that earns them:
    /// set when the server accepts that message. Saved to disk so a restart keeps them.
    pending_marks: BTreeMap<u64, (MessageKey, Flag)>,
    /// Summaries kept cached at most ([`CACHE_MESSAGES`]; tests make it small).
    pub(crate) cache_limit: usize,
    /// Built on first use from the cached mail; dropped when mail arrives.
    contacts: Option<ContactBook>,
    /// The organisation directory's answers (issue #9).
    pub directory: pim::Directory,
    /// Address books counted so far (each count starts the sender check again).
    pub(crate) contacts_built: u64,
    /// The sender check ([`Session::keep_senders`]): kept a while, since the address book is
    /// counted again after every sync.
    senders: Option<opening::Senders>,
    /// S/MIME: each account's settings, the roots trusted for email, decrypted messages (in
    /// memory only: the MIME entity inside, and whether it was authenticated), the ones being
    /// decrypted and why others couldn't be, and messages waiting to be signed or encrypted.
    smime: HashMap<String, SmimeSettings>,
    trust_roots: Vec<Vec<u8>>,
    opened: HashMap<MessageKey, (Vec<u8>, bool)>,
    opening: HashSet<MessageKey>,
    decrypt_errors: HashMap<MessageKey, String>,
    protecting: HashMap<u64, security::Protecting>,
    revocation: security::Revocations,
    cards: security::CardSearch,
    /// The highest UID seen per Inbox, so only mail that arrives later is announced.
    pub(crate) seen_uid: HashMap<(String, String), u32>,
    /// Mail that arrived and may hold an invitation, waiting for its body (issue #20).
    pub(crate) arrived_invites: Vec<MessageKey>,
    /// New mail to announce (desktop notifications), oldest first.
    pub new_mail: Vec<Summary>,
    /// Inbox rules, run on mail as it arrives (`rules.json`).
    pub rules: Vec<mailcraft_model::organize::Rule>,
    /// Snoozed messages and when they come back (`snoozes.json`).
    pub snoozes: Vec<mailcraft_model::organize::Snooze>,
    /// The Other tab of a Focused/Other Inbox is showing.
    pub focus_other: bool,
    /// Pictures from the web, by address: downloaded only after the user asked.
    pub pictures: HashMap<String, Picture>,
    /// Messages whose pictures the user asked to see this time.
    pictures_for: BTreeSet<MessageKey>,
    /// Server settings looked up per address (lower case): `None` while looking.
    pub discovered: HashMap<String, Option<(Option<AccountConfig>, String)>>,
    /// Accounts waiting for a data file to come in before they connect (their start).
    held: HashMap<String, Job>,
    /// OAuth providers this copy can sign in with (set by the app).
    pub oauth_available: Vec<String>,
    /// Running portable (set by the app): everything is kept beside it, passwords only in memory.
    pub portable: bool,
    /// Only JMAP accounts can be added (the browser, which can't open IMAP or SMTP connections).
    pub jmap_only: bool,
    /// The fonts PDFs embed (regular, bold): the system's, set by the app.
    pub print_fonts: Option<(Vec<u8>, Option<Vec<u8>>)>,
    /// Pages to show in the browser (OAuth sign-in), oldest first.
    pub open_urls: Vec<String>,
    /// Each account's calendars and address books.
    pub pim: HashMap<String, mailcraft_model::pim::Pim>,
    /// The accounts whose calendars and contacts couldn't be read: never saved over.
    pub(crate) pim_unread: HashSet<String>,
    /// The documents (`rules.json`, `snoozes.json`) that couldn't be read at start: never saved
    /// over with what's shown in their place.
    pub(crate) docs_unread: HashSet<&'static str>,
    /// Mail, Calendar or People.
    pub module: pim::Module,
    pub cal_mode: pim::CalMode,
    /// The local day the calendar shows (`None`: today).
    pub cal_day: Option<i64>,
    /// Search: the one running (or done and shown), and what it keeps between searches.
    search_state: searching::SearchState,
    /// How many times a message was shown in the reading pane ([`Session::messages_shown`]).
    shown: u64,
    /// The open message's body, when it's got ready on another thread.
    pub(crate) body: opening::BodyLane,
    /// Data files imported into accounts: running, waiting on the user, or done (`imports.json`).
    /// While one is read, its account's folders' search words wait for the end.
    importing: Imports,
    /// The search indexes kept up in the background.
    indexing: searching::Indexing,
    /// Data files made smaller: how each is doing, and the compactions running.
    pub compaction: Compaction,
    /// The message in the reading pane to mark read, and when (`ReadInPane::Shown` with a wait).
    read_due: Option<(MessageKey, i64)>,
    /// The message to mark read once another is chosen (`ReadInPane::Left`).
    read_on_leave: Option<MessageKey>,
    registry: Arc<Registry>,
    next_id: u64,
    seed: u64,
    /// The commit this build was made from (release builds: `MAILCRAFT_BUILD_SHA`), for
    /// telling its test build from a later one of the same version.
    pub build_sha: Option<String>,
    /// File › About's update check.
    pub update: UpdateCheck,
}

const NOTICE_MAX: usize = 50;
const ACCOUNT_COLORS: u8 = 8;

impl Session {
    /// A session over a cache. Remote accounts get a `Start` job (password from the keychain).
    pub fn open(store: Option<Store>, clock: Clock) -> Session {
        let mut s = Session {
            store,
            accounts: Vec::new(),
            view: View::Empty,
            filter: ListFilter::All,
            search: None,
            search_folder: None,
            server_search: None,
            searches: 0,
            recent_in_memory: Vec::new(),
            showing_more: None,
            ask_password: Vec::new(),
            asked_password: HashSet::new(),
            list: Vec::new(),
            threads: Vec::new(),
            nav: Default::default(),
            selection: Vec::new(),
            focus: None,
            anchor: None,
            reading: None,
            windows: Vec::new(),
            sources: Vec::new(),
            compose: None,
            composes: Vec::new(),
            prefs: Prefs::default(),
            clock,
            notices: Vec::new(),
            undo: Vec::new(),
            revision: 0,
            pim_revision: 0,
            outbox: Vec::new(),
            flushed: BTreeSet::new(),
            jobs: Vec::new(),
            cache: HashMap::new(),
            cache_order: Vec::new(),
            echoes: mailboxes::Echoes::default(),
            marks_deferred: BTreeMap::new(),
            pending_marks: BTreeMap::new(),
            cache_limit: CACHE_MESSAGES,
            contacts: None,
            directory: pim::Directory::default(),
            contacts_built: 0,
            senders: None,
            smime: HashMap::new(),
            trust_roots: Vec::new(),
            opened: HashMap::new(),
            opening: HashSet::new(),
            decrypt_errors: HashMap::new(),
            protecting: HashMap::new(),
            revocation: Default::default(),
            cards: Default::default(),
            seen_uid: HashMap::new(),
            arrived_invites: Vec::new(),
            new_mail: Vec::new(),
            rules: Vec::new(),
            snoozes: Vec::new(),
            focus_other: false,
            pictures: HashMap::new(),
            pictures_for: BTreeSet::new(),
            discovered: HashMap::new(),
            held: HashMap::new(),
            oauth_available: Vec::new(),
            portable: false,
            jmap_only: false,
            print_fonts: None,
            open_urls: Vec::new(),
            pim: HashMap::new(),
            pim_unread: HashSet::new(),
            docs_unread: HashSet::new(),
            module: pim::Module::Mail,
            cal_mode: pim::CalMode::Week,
            cal_day: None,
            search_state: Default::default(),
            shown: 0,
            body: Default::default(),
            importing: Imports::default(),
            indexing: Default::default(),
            compaction: Compaction::default(),
            read_due: None,
            read_on_leave: None,
            registry: Arc::new(crate::cmd::registry()),
            // Compose ids also name Outbox entries, which outlive a session: start from the clock.
            next_id: u64::try_from(clock.now.max(0)).unwrap_or(0).saturating_mul(1000),
            seed: (clock.now as u64) ^ 0x9E37_79B9_7F4A_7C15,
            build_sha: None,
            update: UpdateCheck::NotAsked,
        };
        if let Some(store) = s.store.clone() {
            match store.load_doc(RULES_DOC) {
                Ok(r) => s.rules = r,
                Err(e) => {
                    log::warn!("the rules couldn't be read: {e}");
                    s.docs_unread.insert(RULES_DOC);
                    s.notice("Your rules couldn't be read: changes to them won't be saved", true);
                }
            }
            match store.load_doc(SNOOZES_DOC) {
                Ok(z) => s.snoozes = z,
                Err(e) => {
                    log::warn!("snoozed mail couldn't be read: {e}");
                    s.docs_unread.insert(SNOOZES_DOC);
                    s.notice("Snoozed mail couldn't be read: changes to it won't be saved", true);
                }
            }
            s.pending_marks = store
                .load_doc::<Vec<(u64, MessageKey, Flag)>>(PENDING_MARKS)
                .unwrap_or_default()
                .into_iter()
                .map(|(id, k, f)| (id, (k, f)))
                .collect();
        }
        s.load_accounts();
        for id in s.accounts.iter().map(|a| a.config.id.clone()).collect::<Vec<_>>() {
            s.mark_junk(&id);
        }
        s.refresh_outbox();
        s.load_pim();
        s.load_smime();
        s.view = s.default_view();
        s.refresh_list();
        s
    }

    /// A session with no cache (tests, previews).
    pub fn blank() -> Session {
        Session::open(None, Clock::default())
    }

    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }

    /// Run a command by id. Disabled commands fail with [`CmdError::Disabled`]. A panic inside a
    /// command is caught and reported as an error rather than taking the app down.
    pub fn run(&mut self, id: &str, params: &Value) -> CmdResult {
        let reg = self.registry.clone();
        let spec = reg.get(id).ok_or_else(|| CmdError::Unknown(id.to_string()))?;
        // What the command acts on is the message as shown: its body, still being got ready on
        // another thread, is got ready here first (moving about the list doesn't wait for it).
        if !crate::cmd::leaves_body_waiting(id) && std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.finish_pending_body())).is_err() {
            log::error!("the open message couldn't be shown");
            PANICS_CAUGHT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(why) = (spec.enabled)(self) {
            return Err(CmdError::Disabled(why.to_string()));
        }
        if let Some(why) = self.compaction_blocks(id, params) {
            return Err(CmdError::Disabled(why));
        }
        let run = spec.run;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self, params)));
        match result {
            Ok(r) => r,
            Err(_) => {
                log::error!("command {id} panicked");
                PANICS_CAUGHT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.refresh_list();
                Err(CmdError::Failed(format!("{id} failed unexpectedly")))
            }
        }
    }

    /// Jobs for the network layer, oldest first.
    pub fn take_jobs(&mut self) -> Vec<Job> {
        std::mem::take(&mut self.jobs)
    }

    pub fn push_job(&mut self, job: Job) {
        self.jobs.push(job);
    }

    pub fn set_clock(&mut self, now: i64, offset: i32) {
        self.clock = Clock::new(now, offset).in_zone(self.clock.zone);
    }

    /// The local time zone, by the name the system gives (`Europe/Berlin`): times then show
    /// with the offset of their own date. One it doesn't know, or that doesn't fit the clock's
    /// offset now, leaves today's offset for all dates.
    pub fn set_zone(&mut self, name: Option<&str>) {
        let (now, offset) = (self.clock.now, i64::from(self.clock.offset));
        self.clock.zone = name.and_then(mailcraft_model::zone::Zone::named).filter(|z| z.offset_at(now) == offset);
    }

    pub fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.notices.push(Notice { text: text.into(), at: self.clock.now, error });
        if self.notices.len() > NOTICE_MAX {
            self.notices.remove(0);
        }
    }

    fn fresh_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// A fresh `Content-ID` for a picture placed in a message.
    pub fn new_cid(&mut self) -> String {
        format!("{:016x}@mailcraft", self.random())
    }

    fn random(&mut self) -> u64 {
        // xorshift64*: unique Message-IDs, not cryptography.
        let mut x = self.seed.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.seed = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Most web pictures loaded for one message.
const MAX_PICTURES: usize = 60;

/// A picture from the web.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Picture {
    Loading,
    Ready(Arc<Vec<u8>>),
    Failed(String),
}

/// Where a move goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveTarget {
    Role(FolderRole),
    Folder(String),
}

/// `{"key": "a1:7:42:INBOX"}` or `{"account":…,"folder":…,"validity":…,"uid":…}`.
pub fn parse_key(v: &Value) -> Option<MessageKey> {
    if let Some(s) = v.get("key").and_then(Value::as_str) {
        return MessageKey::from_id(s);
    }
    serde_json::from_value::<MessageKey>(v.clone()).ok().filter(|k| !k.account.is_empty())
}

/// What a follow-up date is kept under ([`Summary::follow_up_id`]).
fn follow_up_id(m: &Summary) -> String {
    m.follow_up_id()
}
