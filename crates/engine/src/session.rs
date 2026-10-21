//! The session: accounts, the current view and its message list, the open message, the message
//! being written, preferences, and the queue of jobs for the network layer.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use mailcraft_mime::{self as mime, OutAttachment, Parsed, ReplyMode};
use mailcraft_model::contacts::{Contact, ContactBook};
use mailcraft_model::dates::{Clock, DateGroup};
use mailcraft_model::job::{Secret, Vacation};
use mailcraft_model::search::{Doc, Index, Query, summary_text};
use mailcraft_model::thread::{Thread, threads};
use mailcraft_model::{
    AccountConfig, EmailAddress, Event, Flag, Folder, FolderRole, Job, MessageKey, Op, Summary, SyncStatus, addr, sort_folders, sort_newest_first,
};
use mailcraft_store::{FolderState, OutboxItem, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CmdError, CmdResult, Registry};

pub mod invite;
pub mod junk;
mod organize;
pub mod pim;
pub mod quick_steps;
pub mod search_folders;

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
    /// Every folder of every account.
    #[default]
    All,
    /// The folder (or unified view) on screen.
    Current,
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
    pub query: String,
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

/// Settings the user changes from the View tab (saved by the app).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub reading_pane: ReadingPane,
    /// Group the list by conversation.
    pub conversations: bool,
    /// Mark messages read in the reading pane at all (when off, only opening one in its own
    /// window does).
    pub mark_read_on_open: bool,
    pub read_in_pane: ReadInPane,
    pub notes_order: NotesOrder,
    /// Calendars left out of the calendar (`account/calendar id`), as Outlook's unticked ones.
    pub hidden_calendars: BTreeSet<String>,
    /// Day and week show each calendar in a column of its own (Outlook's side-by-side view)
    /// rather than all of them together.
    pub calendars_side_by_side: bool,
    /// Months shown one under another in the calendar's side pane (1 to 6).
    pub calendar_months: u8,
    /// The address book Select Names opens with (Outlook's "Show this address list first"):
    /// `all` (everyone written to or heard from, and contacts), `people` (every contact), or
    /// one address book (`book:<account>:<book id>`).
    pub address_book: String,
    /// The newest messages of each folder kept on this computer (0: all of them), as Outlook's
    /// "Mail to keep offline".
    pub keep_messages: u32,
    /// Of those, the newest downloaded in full ahead of reading (the rest when opened).
    pub keep_offline: u32,
    /// With `ReadInPane::Shown`: after it's been shown this many seconds (0: at once).
    pub read_after_secs: u32,
    /// Show the first line of text under each message.
    pub show_preview: bool,
    /// Tighter rows.
    pub compact: bool,
    /// Ask before deleting permanently.
    pub confirm_permanent_delete: bool,
    /// Desktop notifications for new mail in an Inbox.
    pub notify: bool,
    /// The categories offered, in order.
    pub categories: Vec<mailcraft_model::organize::Category>,
    /// Split the Inbox into Focused and Other.
    pub focused_inbox: bool,
    /// Senders the user moved: `true` always Focused, `false` always Other.
    pub focus_overrides: std::collections::BTreeMap<String, bool>,
    /// Show HTML mail formatted (`false`: as plain text).
    pub show_html: bool,
    /// Senders whose pictures from the web load without asking (lower-case addresses).
    pub trusted_senders: BTreeSet<String>,
    /// Junk Email: senders (a lower-case address, or `@domain`) whose mail goes to Junk as it
    /// arrives ([`junk`]).
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub blocked_senders: BTreeSet<String>,
    /// Junk Email: senders whose mail is never junk (taken back out of Junk; pictures load).
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub safe_senders: BTreeSet<String>,
    /// Folders the user pinned to Favorites, in order. `None` until the user changes them: the
    /// Inbox, Sent Items and Drafts of a single account are shown.
    pub favorites: Option<Vec<FolderRef>>,
    /// New messages open in a window of their own (replies start in the reading pane).
    pub new_mail_window: bool,
    /// How the message list is arranged (View › Arrange by), and turned the other way round.
    pub arrange: Arrange,
    pub reverse_sort: bool,
    /// Work offline (Send / Receive › Work Offline): no account connects; kept across starts.
    pub work_offline: bool,
    /// Ignored conversations (Delete › Ignore): their messages' Message-IDs; replies to any of
    /// them go to Deleted Items as they arrive.
    pub ignored: BTreeSet<String>,
    /// The folder pane minimized to a narrow strip (View › Folder pane, or its ‹ button).
    pub folder_pane_minimized: bool,
    /// What's collapsed in the folder pane, kept from one start to the next as Outlook keeps it:
    /// `favorites`, `account:<id>`, `folder:<account>/<path>`.
    #[serde(default)]
    pub collapsed: BTreeSet<String>,
    /// The folder pane's order of mailboxes, moved by the user ([`Session::pane_sections`]):
    /// account ids, and shared mailboxes as `<account>|<address>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pane_order: Vec<String>,
    /// When flagged messages are due (Follow Up › Today, Tomorrow…): Message-ID → local day.
    /// IMAP has nowhere for it, so it's kept here, as Outlook keeps it for IMAP accounts.
    #[serde(default)]
    pub follow_up: BTreeMap<String, i64>,
    /// Quick Steps: actions done to the selection in one go ([`quick_steps`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub quick_steps: Vec<quick_steps::QuickStep>,
    /// Search Folders: saved searches shown as folders ([`search_folders`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub search_folders: Vec<search_folders::SearchFolder>,
    /// Reminders answered ([`Session::due_reminders`]), by occurrence
    /// (`<account>|<uid>|<occurrence>`): 0 dismissed, else snoozed until that Unix time.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reminders: BTreeMap<String, i64>,
}

/// What the message list is sorted and grouped by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Arrange {
    /// Newest first, under Today, Yesterday… (conversations, when on, apply here).
    #[default]
    Date,
    /// By sender, A to Z.
    From,
    /// By the first recipient, A to Z.
    To,
    /// By subject (without Re:, Fwd:…), A to Z.
    Subject,
    /// Largest first, under size ranges.
    Size,
    /// Flagged first.
    Flag,
    /// With attachments first.
    Attachments,
}

impl Arrange {
    pub const ALL: [Arrange; 7] = [Arrange::Date, Arrange::From, Arrange::To, Arrange::Subject, Arrange::Size, Arrange::Flag, Arrange::Attachments];

    pub fn code(self) -> &'static str {
        match self {
            Arrange::Date => "date",
            Arrange::From => "from",
            Arrange::To => "to",
            Arrange::Subject => "subject",
            Arrange::Size => "size",
            Arrange::Flag => "flag",
            Arrange::Attachments => "attachments",
        }
    }

    pub fn from_code(s: &str) -> Option<Arrange> {
        Arrange::ALL.into_iter().find(|a| a.code() == s)
    }

    /// The name shown in the View tab and above the list.
    pub fn label(self) -> &'static str {
        match self {
            Arrange::Date => "Date",
            Arrange::From => "From",
            Arrange::To => "To",
            Arrange::Subject => "Subject",
            Arrange::Size => "Size",
            Arrange::Flag => "Flag status",
            Arrange::Attachments => "Attachments",
        }
    }
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

/// Who a message is shown under: a name, else the address, else nothing.
fn person(a: Option<&mailcraft_model::EmailAddress>) -> String {
    a.map(|a| if a.name.trim().is_empty() { a.email.trim().to_string() } else { a.name.trim().to_string() }).unwrap_or_default()
}

/// The fixed headings an arranged list shows (for translations).
pub const ARRANGE_HEADINGS: [&str; 13] = [
    "Under 10 KB",
    "10 to 25 KB",
    "25 to 100 KB",
    "100 to 500 KB",
    "500 KB to 1 MB",
    "1 to 5 MB",
    "Over 5 MB",
    "Flagged",
    "Not flagged",
    "With attachments",
    "Without attachments",
    "(no subject)",
    "(nobody)",
];

/// The size range a message is shown under.
fn size_group(size: u32) -> &'static str {
    match size {
        0..10_240 => "Under 10 KB",
        10_240..25_600 => "10 to 25 KB",
        25_600..102_400 => "25 to 100 KB",
        102_400..512_000 => "100 to 500 KB",
        512_000..1_048_576 => "500 KB to 1 MB",
        1_048_576..5_242_880 => "1 to 5 MB",
        _ => "Over 5 MB",
    }
}

/// Sort a list as `arrange` says (each one's natural order; `reverse` the other way), newest
/// first among equals.
pub fn arrange_list(list: &mut [Summary], arrange: Arrange, reverse: bool) {
    let text = |s: String| s.to_lowercase();
    list.sort_by(|a, b| {
        let primary = match arrange {
            Arrange::Date => b.date.cmp(&a.date),
            Arrange::From => text(person(Some(&a.from))).cmp(&text(person(Some(&b.from)))),
            Arrange::To => text(person(a.to.first())).cmp(&text(person(b.to.first()))),
            Arrange::Subject => text(a.base_subject()).cmp(&text(b.base_subject())),
            Arrange::Size => b.size.cmp(&a.size),
            Arrange::Flag => b.flags.flagged.cmp(&a.flags.flagged),
            Arrange::Attachments => b.has_attachments.cmp(&a.has_attachments),
        };
        let primary = if reverse { primary.reverse() } else { primary };
        primary.then_with(|| b.date.cmp(&a.date)).then_with(|| b.key.cmp(&a.key))
    });
}

/// A folder of an account, by path.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderRef {
    pub account: String,
    pub folder: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            reading_pane: ReadingPane::Right,
            conversations: false,
            mark_read_on_open: true,
            read_in_pane: ReadInPane::Shown,
            notes_order: NotesOrder::Modified,
            hidden_calendars: BTreeSet::new(),
            calendars_side_by_side: false,
            calendar_months: 1,
            address_book: "all".into(),
            keep_messages: 1000,
            keep_offline: 40,
            read_after_secs: 0,
            show_preview: true,
            compact: false,
            confirm_permanent_delete: true,
            favorites: None,
            notify: true,
            categories: mailcraft_model::organize::default_categories(),
            focused_inbox: false,
            focus_overrides: std::collections::BTreeMap::new(),
            show_html: true,
            trusted_senders: BTreeSet::new(),
            blocked_senders: BTreeSet::new(),
            safe_senders: BTreeSet::new(),
            new_mail_window: true,
            arrange: Arrange::Date,
            reverse_sort: false,
            work_offline: false,
            ignored: BTreeSet::new(),
            follow_up: BTreeMap::new(),
            reminders: BTreeMap::new(),
            search_folders: Vec::new(),
            quick_steps: Vec::new(),
            folder_pane_minimized: false,
            collapsed: BTreeSet::new(),
            pane_order: Vec::new(),
        }
    }
}

/// A line in the message list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A date heading (`Today`, `Last week`…), or another heading of a fixed text.
    Header(&'static str),
    /// A heading that's a name (a sender, a recipient, a subject), shown as it is.
    Group(String),
    /// A message (index into [`Session::list`]); with conversations on, the newest message of a
    /// conversation and how many messages (and unread ones) it holds.
    Message { index: usize, count: usize, unread: usize },
}

/// The most messages open in their own windows at once.
pub const MAX_WINDOWS: usize = 8;

/// The message open in the reading pane (or in a window of its own).
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub key: MessageKey,
    pub parsed: Option<Parsed>,
    pub error: Option<String>,
    /// The other messages of its conversation in this list, oldest first.
    pub thread: Vec<MessageKey>,
}

/// A message being written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compose {
    pub id: u64,
    pub account: String,
    /// The signature in the text as it was put there (`-- ` and the signature): replaced when
    /// another goes in, while it's still there unchanged.
    #[serde(default)]
    pub signature: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<OutAttachment>,
    pub in_reply_to: String,
    pub references: Vec<String>,
    pub mode: Option<ReplyMode>,
    /// The message replied to or forwarded (it gets `\Answered` / `$Forwarded` once sent).
    pub source: Option<MessageKey>,
    pub sending: bool,
    pub error: Option<String>,
    /// The copy saved in Drafts, replaced on the next save.
    pub draft: Option<MessageKey>,
    /// The Bcc row is shown.
    pub show_cc: bool,
    /// Send as plain text only (lists written out, the rest of the formatting dropped).
    #[serde(default)]
    pub plain: bool,
    /// The pictures placed in the text (each where a [`mime::rich::PICTURE`] stands).
    #[serde(default)]
    pub inline: Vec<mime::InlinePicture>,
    /// How the text looks: a style for each character and paragraph.
    #[serde(default)]
    pub styles: mime::rich::Styles,
    /// Written in a window of its own rather than the reading pane.
    #[serde(default)]
    pub window: bool,
    /// High (1) or low (-1) importance.
    #[serde(default)]
    pub importance: i8,
    /// Ask the recipients for a read receipt.
    #[serde(default)]
    pub read_receipt: bool,
    /// Where replies go, when not to the sender (Outlook's "Have replies sent to").
    #[serde(default)]
    pub reply_to: String,
    /// Hold it in the Outbox until then (Unix seconds): Outlook's Delay Delivery.
    #[serde(default)]
    pub send_after: Option<i64>,
    /// The Replies to row is shown.
    #[serde(default)]
    pub show_reply_to: bool,
}

impl Compose {
    /// The text, HTML and pictures to send: formatted when the text uses formatting or has
    /// pictures (and the message isn't set to plain text), else plain text alone.
    pub fn bodies(&self) -> (String, Option<String>, Vec<mime::InlinePicture>) {
        let mut styles = self.styles.clone();
        styles.fit(&self.body);
        let used = mime::rich::pictures(&self.body, &styles);
        let pictures: Vec<mime::InlinePicture> = self.inline.iter().filter(|p| used.contains(&p.cid)).cloned().collect();
        let text = mime::rich::to_text(&self.body, &styles);
        if self.plain || (styles.is_plain() && pictures.is_empty()) {
            (text, None, Vec::new())
        } else {
            (text, Some(mime::rich::to_html(&self.body, &styles)), pictures)
        }
    }

    /// Set the text (and forget its formatting).
    pub fn set_text(&mut self, text: String) {
        self.styles = mime::rich::Styles::plain(&text);
        self.body = text;
    }
}

/// A status-bar message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub at: i64,
    pub error: bool,
}

struct SearchIndex {
    index: Index,
    keys: Vec<MessageKey>,
    /// Whether each message's body words are in the index (the words themselves aren't kept: a
    /// big mailbox's would fill the memory; they're read again for the messages a search finds).
    has_body: Vec<bool>,
    scope: SearchScope,
    view: View,
}

/// File for the Replied/Forwarded flags still owed by replies and forwards in the Outbox.
const PENDING_MARKS: &str = "pending-marks.json";

/// A local account with more messages than this keeps the counts it saved when it opens, rather
/// than reading every folder to count them.
const RECOUNT_MAX: u64 = 50_000;

/// Message summaries kept in memory at most, besides the folders on screen (a big data file's
/// whole mailbox is several times this).
const CACHE_MESSAGES: usize = 300_000;

/// Body words kept for search at most (bytes): enough for the messages a search looks at again.
const BODY_WORDS_BYTES: usize = 48 << 20;

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
    pub selection: Vec<MessageKey>,
    pub focus: Option<MessageKey>,
    pub reading: Option<Reading>,
    /// Messages opened in their own windows (double-click), oldest first.
    pub windows: Vec<Reading>,
    pub compose: Option<Compose>,
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
    /// Flags owed to originals, keyed by the Outbox id of the reply or forward that earns them:
    /// set when the server accepts that message. Saved to disk so a restart keeps them.
    pending_marks: BTreeMap<u64, (MessageKey, Flag)>,
    /// Summaries kept cached at most ([`CACHE_MESSAGES`]; tests make it small).
    pub(crate) cache_limit: usize,
    /// Built on first use from the cached mail; dropped when mail arrives.
    contacts: Option<ContactBook>,
    /// The highest UID seen per Inbox, so only mail that arrives later is announced.
    seen_uid: HashMap<(String, String), u32>,
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
    /// Mail, Calendar or People.
    pub module: pim::Module,
    pub cal_mode: pim::CalMode,
    /// The local day the calendar shows (`None`: today).
    pub cal_day: Option<i64>,
    index: Option<SearchIndex>,
    /// The message in the reading pane to mark read, and when (`ReadInPane::Shown` with a wait).
    read_due: Option<(MessageKey, i64)>,
    /// The message to mark read once another is chosen (`ReadInPane::Left`).
    read_on_leave: Option<MessageKey>,
    /// Each downloaded message's words for search (its text and attachment names), read from
    /// the cache once: the index is rebuilt whenever mail changes, and parsing every message
    /// again each time made typing in the search box hang.
    body_words: HashMap<MessageKey, Arc<str>>,
    /// Their size, kept under [`BODY_WORDS_BYTES`].
    body_words_bytes: usize,
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
            showing_more: None,
            ask_password: Vec::new(),
            asked_password: HashSet::new(),
            list: Vec::new(),
            threads: Vec::new(),
            selection: Vec::new(),
            focus: None,
            reading: None,
            windows: Vec::new(),
            compose: None,
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
            pending_marks: BTreeMap::new(),
            cache_limit: CACHE_MESSAGES,
            contacts: None,
            seen_uid: HashMap::new(),
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
            module: pim::Module::Mail,
            cal_mode: pim::CalMode::Week,
            cal_day: None,
            index: None,
            read_due: None,
            read_on_leave: None,
            body_words: HashMap::new(),
            body_words_bytes: 0,
            registry: Arc::new(crate::cmd::registry()),
            // Compose ids also name Outbox entries, which outlive a session: start from the clock.
            next_id: u64::try_from(clock.now.max(0)).unwrap_or(0).saturating_mul(1000),
            seed: (clock.now as u64) ^ 0x9E37_79B9_7F4A_7C15,
            build_sha: None,
            update: UpdateCheck::NotAsked,
        };
        if let Some(store) = &s.store {
            s.rules = store.load_doc("rules.json").unwrap_or_default();
            s.snoozes = store.load_doc("snoozes.json").unwrap_or_default();
            s.pending_marks = store
                .load_doc::<Vec<(u64, MessageKey, Flag)>>(PENDING_MARKS)
                .unwrap_or_default()
                .into_iter()
                .map(|(id, k, f)| (id, (k, f)))
                .collect();
            let accounts = store.load_accounts().unwrap_or_else(|e| {
                log::warn!("accounts: {e}");
                Vec::new()
            });
            for config in accounts {
                // Online only: held in memory from the start (anything left on disk goes).
                if config.keeps_online()
                    && let Err(e) = store.set_online_only(&config.id, true)
                {
                    log::warn!("{}: couldn't be kept online only: {e}", config.id);
                }
                let mut folders = store.load_folders(&config.id).unwrap_or_default();
                // A folder belongs to the account it's kept with (a data file brought in from
                // another account once kept that one's name).
                for f in &mut folders {
                    if f.account != config.id {
                        f.account.clone_from(&config.id);
                    }
                }
                sort_folders(&mut folders);
                s.accounts.push(AccountState { config, folders, status: SyncStatus::Offline, quota: None, vacation: VacationState::Unknown });
            }
        }
        let mut local = Vec::new();
        for a in &s.accounts {
            if a.is_local() {
                local.push(a.config.id.clone());
            } else {
                s.jobs.push(Job::Start { account: a.config.clone(), password: None });
            }
        }
        // Counted again on opening, unless big: a big data file's counts were kept as it came in
        // and with every change since (counting meant reading its every folder's list).
        for id in local {
            let kept: u64 = s.account(&id).map(|a| a.folders.iter().map(|f| u64::from(f.total)).sum()).unwrap_or(0);
            if kept <= RECOUNT_MAX {
                s.recount_local(&id, None);
            }
        }
        // What's already cached isn't new.
        let inboxes: Vec<(String, String)> =
            s.accounts.iter().filter_map(|a| a.folder(FolderRole::Inbox).map(|f| (a.config.id.clone(), f.path.clone()))).collect();
        for (a, f) in inboxes {
            s.note_new_mail(&a, &f);
        }
        s.refresh_outbox();
        s.load_pim();
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
        if let Some(why) = (spec.enabled)(self) {
            return Err(CmdError::Disabled(why.to_string()));
        }
        let run = spec.run;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self, params)));
        match result {
            Ok(r) => r,
            Err(_) => {
                log::error!("command {id} panicked");
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
        self.clock = Clock::new(now, offset);
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

    // ----- accounts and folders -----

    pub fn account(&self, id: &str) -> Option<&AccountState> {
        self.accounts.iter().find(|a| a.config.id == id)
    }

    fn account_mut(&mut self, id: &str) -> Option<&mut AccountState> {
        self.accounts.iter_mut().find(|a| a.config.id == id)
    }

    pub fn folder(&self, account: &str, path: &str) -> Option<&Folder> {
        self.account(account).and_then(|a| a.folders.iter().find(|f| f.path == path))
    }

    /// What opens at the start: the first account's Inbox, as Outlook opens on its first mailbox
    /// (All Inboxes is a click away in Favorites; the accounts can be put in any order).
    fn default_view(&self) -> View {
        match self.accounts.first() {
            Some(a) => {
                let folder = a.folder(FolderRole::Inbox).map(|f| f.path.clone()).unwrap_or_else(|| "INBOX".to_string());
                View::Folder { account: a.config.id.clone(), folder }
            }
            None => View::Empty,
        }
    }

    /// The account the view belongs to (or the first one, for unified views).
    pub fn current_account(&self) -> Option<&AccountState> {
        match &self.view {
            View::Folder { account, .. } => self.account(account),
            _ => self.accounts.first(),
        }
    }

    /// The role of the folder on screen.
    pub fn view_role(&self) -> Option<FolderRole> {
        match &self.view {
            View::Folder { account, folder } => self.folder(account, folder).map(|f| f.role),
            View::Unified { role } => Some(*role),
            View::Empty | View::Outbox => None,
        }
    }

    /// The view's title for the list header.
    pub fn view_title(&self) -> String {
        if let Some((q, _)) = &self.search {
            return format!("Results for “{q}”");
        }
        match &self.view {
            View::Folder { account, folder } => {
                self.folder(account, folder).map(Folder::display_name).unwrap_or_else(|| mailcraft_model::utf7::decode(folder))
            }
            View::Unified { role } => match role {
                FolderRole::Inbox => "All Inboxes".to_string(),
                r => format!("All {}", r.label().unwrap_or("folders")),
            },
            View::Outbox => "Outbox".to_string(),
            View::Empty => "MailCraft".to_string(),
        }
    }

    /// The cached state of a folder, loaded from disk on first use.
    pub fn folder_state(&mut self, account: &str, folder: &str) -> &FolderState {
        let k = (account.to_string(), folder.to_string());
        if !self.cache.contains_key(&k) {
            let st = self.store.as_ref().and_then(|s| s.load_folder(account, folder).ok()).unwrap_or_default();
            self.cache.insert(k.clone(), st);
            self.cache_order.retain(|o| *o != k);
            self.cache_order.push(k.clone());
            self.trim_cache(&k);
        } else if self.cache_order.last() != Some(&k) {
            self.cache_order.retain(|o| *o != k);
            self.cache_order.push(k.clone());
        }
        static EMPTY: std::sync::OnceLock<FolderState> = std::sync::OnceLock::new();
        self.cache.get(&k).unwrap_or_else(|| EMPTY.get_or_init(FolderState::default))
    }

    /// Keep the cached lists under `cache_limit` summaries: the least recently used folders
    /// go first (read again when wanted), never the ones on screen nor `keep`. Looking through a
    /// big mailbox (the address book, a search of everything) no longer leaves all of it in memory.
    fn trim_cache(&mut self, keep: &(String, String)) {
        let mut total: usize = self.cache.values().map(|st| st.messages.len()).sum();
        if total <= self.cache_limit {
            return;
        }
        let shown = self.view_folders();
        let mut i = 0;
        while total > self.cache_limit && i < self.cache_order.len() {
            let k = self.cache_order.get(i).cloned().unwrap_or_default();
            if k == *keep || shown.contains(&k) {
                i += 1;
                continue;
            }
            if let Some(st) = self.cache.remove(&k) {
                total = total.saturating_sub(st.messages.len());
            }
            self.cache_order.remove(i);
        }
    }

    /// Change how many summaries the cache keeps, letting go of what's past it now.
    #[cfg(test)]
    pub(crate) fn set_cache_limit(&mut self, n: usize) {
        self.cache_limit = n;
        self.trim_cache(&(String::new(), String::new()));
    }

    fn reload_folder(&mut self, account: &str, folder: &str) {
        let k = (account.to_string(), folder.to_string());
        self.cache.remove(&k);
        self.index = None;
        self.contacts = None;
    }

    /// Remember what an Inbox holds; unread mail above the last high-water mark is new. The first
    /// look at a folder only sets the mark, so a first sync doesn't announce the backlog.
    fn note_new_mail(&mut self, account: &str, folder: &str) {
        let role = self.folder(account, folder).map(|f| f.role);
        if !matches!(role, Some(FolderRole::Inbox | FolderRole::Junk)) {
            return;
        }
        let st = self.folder_state(account, folder);
        let (validity, max) = (st.validity, st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0));
        let k = (account.to_string(), folder.to_string());
        let arrived: Vec<Summary> = match self.seen_uid.get(&k) {
            Some(&seen) => {
                self.folder_state(account, folder).messages.iter().filter(|m| m.key.uid > seen && m.key.validity == validity).cloned().collect()
            }
            None => Vec::new(),
        };
        self.seen_uid.insert(k, max);
        // Junk from safe senders goes back to the Inbox (and is seen arriving there).
        if role == Some(FolderRole::Junk) {
            self.rescue_arrived(&arrived);
            return;
        }
        // Blocked senders' mail goes to Junk, replies in ignored conversations to Deleted Items,
        // then rules; what they file away or mark read isn't announced.
        let mut quiet = self.block_arrived(&arrived);
        let rest: Vec<Summary> = arrived.iter().filter(|m| !quiet.contains(&m.key)).cloned().collect();
        quiet.extend(self.ignore_arrived(&rest));
        let rest: Vec<Summary> = arrived.iter().filter(|m| !quiet.contains(&m.key)).cloned().collect();
        quiet.extend(self.run_rules(&rest));
        // Mail older than a day (a resync after a long time away) isn't "new" any more.
        let cutoff = self.clock.now - 86_400;
        let fresh: Vec<Summary> = arrived.into_iter().filter(|m| !m.flags.seen && m.date >= cutoff && !quiet.contains(&m.key)).collect();
        if self.prefs.notify {
            self.new_mail.extend(fresh.into_iter().rev());
            // A burst (a first sync, a long time offline) is summed up by the UI; keep it bounded.
            let excess = self.new_mail.len().saturating_sub(50);
            self.new_mail.drain(..excess);
        }
    }

    /// New mail to announce since the last call.
    pub fn take_new_mail(&mut self) -> Vec<Summary> {
        std::mem::take(&mut self.new_mail)
    }

    /// Unread mail in every Inbox (for the window title and badges).
    pub fn inbox_unread(&self) -> u32 {
        self.accounts.iter().filter_map(|a| a.folder(FolderRole::Inbox)).map(|f| f.unread).sum()
    }

    /// People to suggest for a recipient box, best first.
    pub fn suggest_contacts(&mut self, query: &str, limit: usize) -> Vec<Contact> {
        self.ensure_contacts();
        self.contacts.as_ref().map(|b| b.suggest(query, limit, self.clock.now)).unwrap_or_default()
    }

    /// Whether anything was written in the message being written (more than its signature):
    /// closing it then asks whether to keep it.
    pub fn compose_written(&self) -> bool {
        let Some(c) = &self.compose else { return false };
        let signature = self.signature(&c.account).trim().to_string();
        let body = c.body.trim();
        !(c.to.trim().is_empty()
            && c.cc.trim().is_empty()
            && c.bcc.trim().is_empty()
            && c.subject.trim().is_empty()
            && c.attachments.is_empty()
            && (body.is_empty() || body.trim_start_matches("--").trim() == signature))
    }

    /// The address book, as Outlook's Select Names dialog lists it: `people` for the people
    /// saved in Contacts, anything else for everyone known (Contacts and everyone mail was
    /// exchanged with). Filtered by `query` (name or address), sorted by name.
    pub fn address_book(&mut self, source: &str, query: &str) -> Vec<EmailAddress> {
        const MAX: usize = 2000;
        let book = source.strip_prefix("book:").and_then(|b| b.split_once(':'));
        if source == "people" || book.is_some() {
            let mut v: Vec<EmailAddress> = self
                .people(query)
                .into_iter()
                .filter(|p| book.is_none_or(|(a, b)| p.account == a && p.book == b))
                .flat_map(|p| p.card.emails.iter().map(|e| EmailAddress::new(p.card.display(), e.value.clone())).collect::<Vec<_>>())
                .collect();
            v.truncate(MAX);
            return v;
        }
        self.ensure_contacts();
        self.contacts.as_ref().map(|b| b.list(query, MAX).into_iter().map(|c| c.address).collect()).unwrap_or_default()
    }

    /// The address books Select Names can show besides everyone (`all`) and every contact
    /// (`people`): each account's, as (source, name, the account's address).
    pub fn address_books(&self) -> Vec<(String, String, String)> {
        self.accounts
            .iter()
            .filter_map(|a| self.pim.get(&a.config.id).map(|p| (a, p)))
            .flat_map(|(a, p)| p.books.iter().map(move |b| (format!("book:{}:{}", a.config.id, b.id), b.name.clone(), a.config.email.clone())))
            .collect()
    }

    /// Build the address book from the cache if it isn't built.
    fn ensure_contacts(&mut self) {
        if self.contacts.is_none() {
            let mine: Vec<String> = self.accounts.iter().flat_map(|a| [a.config.email.clone(), a.config.username.clone()]).collect();
            let folders: Vec<(String, String)> =
                self.accounts.iter().flat_map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| (a.config.id.clone(), f.path.clone()))).collect();
            let mut book = ContactBook::new();
            for (a, f) in folders {
                for m in &self.folder_state(&a, &f).messages {
                    book.add(m, &mine);
                }
            }
            // People saved in an address book come first.
            for p in self.pim.values() {
                for c in &p.cards {
                    for e in &c.card.emails {
                        book.add_saved(&EmailAddress::new(c.card.display(), e.value.clone()), self.clock.now);
                    }
                }
            }
            self.contacts = Some(book);
        }
    }

    /// The folders the view draws from.
    fn view_folders(&self) -> Vec<(String, String)> {
        match &self.view {
            View::Folder { account, folder } => vec![(account.clone(), folder.clone())],
            View::Unified { role } => self.accounts.iter().filter_map(|a| a.folder(*role).map(|f| (a.config.id.clone(), f.path.clone()))).collect(),
            View::Empty | View::Outbox => Vec::new(),
        }
    }

    /// Rebuild the list from the view, search and filter, keeping the selection where possible.
    pub fn refresh_list(&mut self) {
        let mut list: Vec<Summary> = if let Some((q, scope)) = self.search.clone() {
            let mut found = self.search_results(&q, scope);
            // What the servers found: its words may be in mail not downloaded here.
            let hits: Vec<MessageKey> = self.server_search.iter().filter(|ss| ss.query == q).flat_map(|ss| ss.found.iter().cloned()).collect();
            let have: HashSet<MessageKey> = found.iter().map(|m| m.key.clone()).collect();
            for key in hits.into_iter().filter(|k| !have.contains(k)) {
                if let Some(m) = self.folder_state(&key.account, &key.folder).messages.iter().find(|m| m.key.uid == key.uid).cloned() {
                    found.push(m);
                }
            }
            // A Search Folder leaves out Deleted Items and Junk, as Outlook's.
            if self.open_search_folder_name().is_some() {
                found.retain(|m| self.folder(&m.key.account, &m.key.folder).is_none_or(|f| !matches!(f.role, FolderRole::Trash | FolderRole::Junk)));
            }
            sort_newest_first(&mut found);
            found
        } else if self.view == View::Outbox {
            self.outbox_summaries()
        } else {
            let mut v = Vec::new();
            for (a, f) in self.view_folders() {
                v.extend(self.folder_state(&a, &f).messages.iter().filter(|m| !m.flags.deleted).cloned());
            }
            v
        };
        let filter = self.filter;
        list.retain(|m| self.filter_keeps(filter, m));
        if self.focus_split() {
            self.ensure_contacts();
            let other = self.focus_other;
            list.retain(|m| self.is_focused(m) != other);
        }
        arrange_list(&mut list, self.prefs.arrange, self.prefs.reverse_sort);
        self.threads = if self.prefs.conversations && self.prefs.arrange == Arrange::Date { threads(&list) } else { Vec::new() };
        let alive: BTreeSet<&MessageKey> = list.iter().map(|m| &m.key).collect();
        self.selection.retain(|k| alive.contains(k));
        if self.focus.as_ref().is_some_and(|k| !alive.contains(k)) {
            self.focus = None;
        }
        if let Some(r) = &self.reading
            && !alive.contains(&r.key)
            && self.search.is_none()
        {
            self.reading = None;
        }
        self.list = list;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Whether a message passes a filter.
    fn filter_keeps(&self, filter: ListFilter, m: &Summary) -> bool {
        match filter {
            ListFilter::All => true,
            ListFilter::Unread => !m.flags.seen,
            ListFilter::Flagged => m.flags.flagged,
            ListFilter::Attachments => m.has_attachments,
            ListFilter::Categorized => self.prefs.categories.iter().any(|c| m.flags.has(&c.keyword)),
            ListFilter::Important => m.importance > 0,
            ListFilter::ToMe => self.account(&m.key.account).is_some_and(|a| {
                let me = a.config.email.trim();
                !me.is_empty() && m.to.iter().chain(&m.cc).any(|r| r.email.trim().eq_ignore_ascii_case(me))
            }),
        }
    }

    /// The list as rows with date headings (and conversations, when on).
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::with_capacity(self.list.len() + 8);
        let mut last: Option<Row> = None;
        let arrange = self.prefs.arrange;
        let mut push = |out: &mut Vec<Row>, index: usize, count: usize, unread: usize, date: i64| {
            // The heading this message goes under, as the list is arranged.
            let m = self.list.get(index);
            let heading = match arrange {
                Arrange::Date => Row::Header(DateGroup::of(date, &self.clock).label()),
                Arrange::From => Row::Group(m.map(|m| person(Some(&m.from))).unwrap_or_default()),
                Arrange::To => Row::Group(m.map(|m| person(m.to.first())).unwrap_or_default()),
                Arrange::Subject => Row::Group(m.map(Summary::base_subject).unwrap_or_default()),
                Arrange::Size => Row::Header(size_group(m.map_or(0, |m| m.size))),
                Arrange::Flag => Row::Header(if m.is_some_and(|m| m.flags.flagged) { "Flagged" } else { "Not flagged" }),
                Arrange::Attachments => Row::Header(if m.is_some_and(|m| m.has_attachments) { "With attachments" } else { "Without attachments" }),
            };
            let heading = match heading {
                Row::Group(g) if g.trim().is_empty() => Row::Header(if arrange == Arrange::Subject { "(no subject)" } else { "(nobody)" }),
                h => h,
            };
            if last.as_ref() != Some(&heading) {
                out.push(heading.clone());
                last = Some(heading);
            }
            out.push(Row::Message { index, count, unread });
        };
        // Pinned mail first, under its own heading (searches keep plain date order).
        let pins = self.search.is_none();
        let pinned = |i: usize| pins && self.list.get(i).is_some_and(|m| m.flags.pinned());
        let mut pinned_rows = Vec::new();
        if self.prefs.conversations && !self.threads.is_empty() {
            for t in &self.threads {
                let Some(&newest) = t.messages.iter().max_by_key(|&&i| self.list.get(i).map_or(0, |m| m.date)) else { continue };
                let unread = t.messages.iter().filter(|&&i| self.list.get(i).is_some_and(|m| !m.flags.seen)).count();
                if t.messages.iter().any(|&i| pinned(i)) {
                    pinned_rows.push(Row::Message { index: newest, count: t.messages.len(), unread });
                } else {
                    push(&mut out, newest, t.messages.len(), unread, t.latest);
                }
            }
        } else {
            for (i, m) in self.list.iter().enumerate() {
                if pinned(i) {
                    pinned_rows.push(Row::Message { index: i, count: 1, unread: usize::from(!m.flags.seen) });
                } else {
                    push(&mut out, i, 1, usize::from(!m.flags.seen), m.date);
                }
            }
        }
        if !pinned_rows.is_empty() {
            pinned_rows.insert(0, Row::Header("Pinned"));
            pinned_rows.extend(out);
            return pinned_rows;
        }
        out
    }

    pub fn summary(&self, key: &MessageKey) -> Option<&Summary> {
        self.list.iter().find(|m| &m.key == key)
    }

    /// The selected messages (in list order).
    pub fn selected(&self) -> Vec<Summary> {
        if self.selection.len() < 8 {
            return self.list.iter().filter(|m| self.selection.contains(&m.key)).cloned().collect();
        }
        let chosen: HashSet<&MessageKey> = self.selection.iter().collect();
        self.list.iter().filter(|m| chosen.contains(&m.key)).cloned().collect()
    }

    /// The conversation a message belongs to in the current list.
    /// The open message's conversation again (conversations turned on or off).
    pub(crate) fn rethread_reading(&mut self) {
        let Some(key) = self.reading.as_ref().map(|r| r.key.clone()) else { return };
        let thread = self.thread_of(&key);
        if let Some(r) = self.reading.as_mut() {
            r.thread = thread;
        }
    }

    /// The conversation a message is in, shown above it in the reading pane: only while the list
    /// is grouped by conversation (as Outlook's reading pane shows one message otherwise).
    fn thread_of(&self, key: &MessageKey) -> Vec<MessageKey> {
        if !self.prefs.conversations {
            return Vec::new();
        }
        let Some(i) = self.list.iter().position(|m| &m.key == key) else { return Vec::new() };
        let ts;
        let threads = if self.threads.is_empty() {
            ts = threads(&self.list);
            &ts
        } else {
            &self.threads
        };
        threads
            .iter()
            .find(|t| t.messages.contains(&i))
            .map(|t| t.messages.iter().filter_map(|&j| self.list.get(j).map(|m| m.key.clone())).collect())
            .unwrap_or_default()
    }

    // ----- opening and selecting -----

    pub fn open_view(&mut self, view: View) {
        self.view = view;
        self.search = None;
        self.index = None;
        self.selection.clear();
        self.focus = None;
        self.reading = None;
        self.leave_reading(None);
        self.refresh_list();
    }

    /// Mark a message read (when it's there and isn't yet).
    fn mark_seen(&mut self, key: &MessageKey) {
        if key.folder != OUTBOX && self.summary(key).is_some_and(|m| !m.flags.seen) {
            self.apply_op(&key.account.clone(), Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true });
        }
    }

    /// The reading pane moves on from what it showed (to `next`, or to nothing): one waiting for
    /// that is marked read now, and one waiting for its time no longer is.
    fn leave_reading(&mut self, next: Option<&MessageKey>) {
        if self.read_due.as_ref().is_some_and(|(k, _)| Some(k) != next) {
            self.read_due = None;
        }
        if let Some(prev) = self.read_on_leave.take() {
            if Some(&prev) == next {
                self.read_on_leave = Some(prev);
            } else {
                self.mark_seen(&prev);
            }
        }
    }

    /// A message waiting in the reading pane for its time to be marked read, once it's come.
    pub(crate) fn read_when_due(&mut self) {
        let Some((key, at)) = self.read_due.clone() else { return };
        if self.clock.now >= at {
            self.read_due = None;
            if self.reading.as_ref().is_some_and(|r| r.key == key) {
                self.mark_seen(&key);
            }
        }
    }

    /// Select one message and show it in the reading pane (marked read as the options say).
    pub fn open_message(&mut self, key: &MessageKey) {
        self.show_message(key, true);
    }

    /// Select one message and show it; `mark`: as the reading options say (else left as it is).
    fn show_message(&mut self, key: &MessageKey, mark: bool) {
        self.leave_reading(Some(key));
        self.selection = vec![key.clone()];
        self.focus = Some(key.clone());
        let thread = self.thread_of(key);
        self.reading = Some(Reading { key: key.clone(), parsed: None, error: None, thread });
        let cached = if key.folder == OUTBOX {
            self.outbox_entry(key).and_then(|(a, i)| self.store.as_ref().and_then(|s| s.outbox_raw(&a, i.id)))
        } else {
            self.store.as_ref().and_then(|s| s.read_body(key))
        };
        match cached {
            Some(raw) => self.show_body(key, &raw),
            None => self.jobs.push(Job::FetchBody { key: key.clone() }),
        }
        let unread = key.folder != OUTBOX && self.summary(key).is_some_and(|m| !m.flags.seen);
        if mark && unread && self.prefs.mark_read_on_open {
            match (self.prefs.read_in_pane, self.prefs.read_after_secs) {
                (ReadInPane::Shown, 0) => self.mark_seen(key),
                (ReadInPane::Shown, secs) => self.read_due = Some((key.clone(), self.clock.now.saturating_add(i64::from(secs.min(3600))))),
                (ReadInPane::Left, _) => self.read_on_leave = Some(key.clone()),
            }
        }
    }

    /// Open a message in its own window (it stays open while the list changes). Already open:
    /// nothing changes. At most [`MAX_WINDOWS`]; the oldest closes first.
    pub fn pop_out(&mut self, key: &MessageKey) -> CmdResult {
        if !self.windows.iter().any(|w| &w.key == key) {
            if self.windows.len() >= MAX_WINDOWS {
                self.windows.remove(0);
            }
            let cached = if key.folder == OUTBOX {
                self.outbox_entry(key).and_then(|(a, i)| self.store.as_ref().and_then(|s| s.outbox_raw(&a, i.id)))
            } else {
                self.store.as_ref().and_then(|s| s.read_body(key))
            };
            let mut w = Reading { key: key.clone(), parsed: None, error: None, thread: Vec::new() };
            match cached {
                Some(raw) => {
                    w.parsed = mime::parse(&raw);
                    w.error = w.parsed.is_none().then(|| "this message could not be read".to_string());
                }
                None => self.jobs.push(Job::FetchBody { key: key.clone() }),
            }
            self.windows.push(w);
            // Opened on its own (a double-click): read, whatever the reading pane does.
            self.mark_seen(key);
        }
        Ok(serde_json::json!({"key": key.to_id(), "windows": self.windows.len()}))
    }

    pub fn close_window(&mut self, key: &MessageKey) {
        self.windows.retain(|w| &w.key != key);
    }

    fn show_body(&mut self, key: &MessageKey, raw: &[u8]) {
        for w in self.windows.iter_mut().filter(|w| &w.key == key && w.parsed.is_none()) {
            w.parsed = mime::parse(raw);
            w.error = w.parsed.is_none().then(|| "this message could not be read".to_string());
        }
        // Only the open message is laid out (bodies downloaded ahead aren't, each costing time).
        if let Some(r) = self.reading.as_mut()
            && &r.key == key
        {
            let parsed = mime::parse(raw);
            r.error = if parsed.is_none() { Some("this message could not be read".into()) } else { None };
            r.parsed = parsed;
        }
        self.request_pictures();
    }

    /// A body that came in joins the search index there is (rather than throwing it away).
    fn index_body(&mut self, key: &MessageKey, raw: &[u8]) {
        // No index yet: read when one is built (downloading ahead costs nothing extra then).
        if self.index.is_none() {
            return;
        }
        let words: Arc<str> = Arc::from(mime::search_text(raw));
        self.keep_words(key, words.clone());
        if let Some(ix) = self.index.as_mut()
            && let Some(doc) = ix.keys.iter().position(|k| k == key)
            && ix.has_body.get(doc) == Some(&false)
        {
            ix.index.add(u32::try_from(doc).unwrap_or(u32::MAX), &words);
            if let Some(b) = ix.has_body.get_mut(doc) {
                *b = true;
            }
        }
    }

    // ----- pictures from the web -----

    /// Web pictures of the open message (none for plain-text mail).
    pub fn remote_pictures(&self) -> Vec<String> {
        self.reading
            .as_ref()
            .and_then(|r| r.parsed.as_ref())
            .map(|p| p.images.iter().filter(|u| u.starts_with("https://") || u.starts_with("http://")).take(MAX_PICTURES).cloned().collect())
            .unwrap_or_default()
    }

    /// Whether the open message's web pictures may load: the user asked, for this message or
    /// for everything from its sender.
    pub fn pictures_allowed(&self) -> bool {
        let Some(r) = &self.reading else { return false };
        let sender = r.parsed.as_ref().map(|p| p.from.email.trim().to_ascii_lowercase()).unwrap_or_default();
        self.pictures_for.contains(&r.key) || (!sender.is_empty() && (self.prefs.trusted_senders.contains(&sender) || self.sender_safe(&sender)))
    }

    /// Ask the network layer for the open message's pictures, when allowed.
    fn request_pictures(&mut self) {
        if !self.pictures_allowed() {
            return;
        }
        if self.pictures.len() > 400 {
            self.pictures.retain(|_, p| matches!(p, Picture::Loading));
        }
        for url in self.remote_pictures() {
            if !self.pictures.contains_key(&url) {
                self.pictures.insert(url.clone(), Picture::Loading);
                self.jobs.push(Job::FetchUrl { url });
            }
        }
    }

    /// Show the open message's pictures (`always`: and everything from its sender from now on).
    pub fn show_pictures(&mut self, always: bool) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        self.pictures_for.insert(r.key.clone());
        let sender = r.parsed.as_ref().map(|p| p.from.email.trim().to_ascii_lowercase()).unwrap_or_default();
        if always && !sender.is_empty() && self.prefs.trusted_senders.len() < 10_000 {
            self.prefs.trusted_senders.insert(sender.clone());
        }
        self.request_pictures();
        Ok(serde_json::json!({ "pictures": self.remote_pictures().len(), "trusted": always.then_some(sender) }))
    }

    /// Stop loading pictures from the open message's sender automatically.
    pub fn block_pictures(&mut self) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        self.pictures_for.remove(&r.key);
        let sender = r.parsed.as_ref().map(|p| p.from.email.trim().to_ascii_lowercase()).unwrap_or_default();
        let was = self.prefs.trusted_senders.remove(&sender);
        Ok(serde_json::json!({ "untrusted": was }))
    }

    // ----- changing mail -----

    /// Apply a change: to the in-memory list at once, then to the cache and server (through a
    /// job), or straight to the cache for local accounts.
    pub fn apply_op(&mut self, account: &str, op: Op) {
        // Outbox entries aren't on a server: mail commands leave them alone.
        let op = match op {
            Op::SetFlag { keys, flag, on } => Op::SetFlag { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), flag, on },
            Op::SetKeyword { keys, keyword, on } => Op::SetKeyword { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), keyword, on },
            Op::Move { keys, to } => Op::Move { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect(), to },
            Op::Expunge { keys } => Op::Expunge { keys: keys.into_iter().filter(|k| k.folder != OUTBOX).collect() },
            other => other,
        };
        if !op.is_folder_op() && !matches!(op, Op::EmptyFolder { .. }) && op.keys().is_empty() {
            return;
        }
        let local = self.account(account).is_none_or(AccountState::is_local);
        self.apply_op_memory(account, &op);
        if local {
            self.apply_op_local(account, &op);
        } else {
            self.jobs.push(Job::Apply { account: account.to_string(), op });
        }
        self.index = None;
        self.refresh_list();
    }

    fn apply_op_memory(&mut self, account: &str, op: &Op) {
        if op.is_folder_op() {
            self.apply_folder_op_memory(account, op);
            return;
        }
        if let Op::EmptyFolder { path } = op {
            if let Some(st) = self.cache.get_mut(&(account.to_string(), path.clone())) {
                st.messages.clear();
            }
            if let Some(f) = self.account_mut(account).and_then(|a| a.folders.iter_mut().find(|f| &f.path == path)) {
                f.unread = 0;
                f.total = 0;
            }
            self.selection.retain(|k| !(k.account == account && &k.folder == path));
            if self.reading.as_ref().is_some_and(|r| r.key.account == account && &r.key.folder == path) {
                self.reading = None;
            }
            return;
        }
        let mut unread_delta: HashMap<String, i64> = HashMap::new();
        for ((a, f), st) in self.cache.iter_mut() {
            if a != account {
                continue;
            }
            // A set: every message of a big folder may change at once.
            let mine: HashSet<u32> = op.keys().iter().filter(|k| &k.folder == f && k.validity == st.validity).map(|k| k.uid).collect();
            if mine.is_empty() {
                continue;
            }
            match op {
                Op::SetFlag { flag, on, .. } => {
                    for m in st.messages.iter_mut().filter(|m| mine.contains(&m.key.uid)) {
                        if *flag == Flag::Seen && m.flags.seen != *on {
                            *unread_delta.entry(f.clone()).or_default() += if *on { -1 } else { 1 };
                        }
                        flag.set(&mut m.flags, *on);
                    }
                }
                Op::SetKeyword { keyword, on, .. } => {
                    for m in st.messages.iter_mut().filter(|m| mine.contains(&m.key.uid)) {
                        m.flags.set_keyword(keyword, *on);
                    }
                }
                Op::Move { to, .. } if to == f => {}
                Op::Move { .. } | Op::Expunge { .. } => {
                    let unread = st.messages.iter().filter(|m| mine.contains(&m.key.uid) && !m.flags.seen).count();
                    *unread_delta.entry(f.clone()).or_default() -= i64::try_from(unread).unwrap_or(0);
                    st.messages.retain(|m| !mine.contains(&m.key.uid));
                }
                Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } | Op::EmptyFolder { .. } => {}
            }
        }
        if let Some(a) = self.account_mut(account) {
            for (f, d) in unread_delta {
                if let Some(folder) = a.folders.iter_mut().find(|x| x.path == f) {
                    folder.unread = u32::try_from((i64::from(folder.unread) + d).max(0)).unwrap_or(0);
                }
            }
        }
    }

    /// Local accounts: change the cache directly, including filing moved messages in their
    /// destination with new UIDs.
    fn apply_op_local(&mut self, account: &str, op: &Op) {
        let Some(store) = self.store.clone() else { return };
        // The folders it touches (counted again after; all of them for a folder change).
        let mut touched: Vec<String> = op.keys().iter().map(|k| k.folder.clone()).collect();
        match op {
            Op::Move { to, .. } => touched.push(to.clone()),
            Op::EmptyFolder { path } => touched.push(path.clone()),
            _ => {}
        }
        touched.sort();
        touched.dedup();
        // A flag or category: the lists in memory have it already (`apply_op_memory`), so they're
        // saved as they are (a big folder's changed page only), not read from disk, changed and
        // read again. One not in memory is changed on disk.
        if matches!(op, Op::SetFlag { .. } | Op::SetKeyword { .. })
            && touched.iter().all(|f| self.cache.contains_key(&(account.to_string(), f.clone())))
        {
            let changed: HashSet<u32> = op.keys().iter().map(|k| k.uid).collect();
            for f in &touched {
                if let Some(st) = self.cache.get(&(account.to_string(), f.clone()))
                    && let Err(e) = store.save_folder_changed(account, f, st, &changed)
                {
                    self.notice(e.to_string(), true);
                }
            }
            self.recount_local(account, Some(&touched));
            return;
        }
        if let Op::Move { keys, to } = op {
            let mut dest = store.load_folder(account, to).unwrap_or_default();
            if dest.validity == 0 {
                dest.validity = 1;
            }
            // Each source folder read once, its messages found by number; the bodies written
            // together (a folder's every message may go at once: per message, all of this was
            // read again, and each body was a write of its own).
            let mut sources: HashMap<String, HashMap<u32, Summary>> = HashMap::new();
            let mut uid = dest.uid_next.max(dest.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1);
            let mut bodies: Vec<(MessageKey, Vec<u8>)> = Vec::new();
            let mut pending = 0usize;
            for k in keys {
                if k.folder == *to {
                    continue;
                }
                let src = sources.entry(k.folder.clone()).or_insert_with(|| {
                    store.load_folder(account, &k.folder).map(|st| st.messages.into_iter().map(|m| (m.key.uid, m)).collect()).unwrap_or_default()
                });
                let Some(m) = src.get(&k.uid) else { continue };
                let mut moved = m.clone();
                moved.key = MessageKey { account: account.to_string(), folder: to.clone(), validity: dest.validity, uid };
                uid += 1;
                if let Some(raw) = store.read_body(k) {
                    pending += raw.len();
                    bodies.push((moved.key.clone(), raw));
                }
                if bodies.len() >= 1000 || pending >= 16 << 20 {
                    let _ = store.write_bodies(&std::mem::take(&mut bodies));
                    pending = 0;
                }
                dest.messages.push(moved);
            }
            if !bodies.is_empty() {
                let _ = store.write_bodies(&bodies);
            }
            dest.uid_next = uid;
            sort_newest_first(&mut dest.messages);
            if let Err(e) = store.save_folder(account, to, &dest) {
                self.notice(e.to_string(), true);
            }
            self.reload_folder(account, to);
        }
        match store.apply_op(account, op) {
            Ok(changed) => {
                for f in changed {
                    self.reload_folder(account, &f);
                }
            }
            Err(e) => self.notice(e.to_string(), true),
        }
        if op.is_folder_op()
            && let Ok(mut folders) = store.load_folders(account)
        {
            sort_folders(&mut folders);
            if let Some(a) = self.account_mut(account) {
                a.folders = folders;
            }
        }
        self.recount_local(account, (!op.is_folder_op()).then_some(&touched));
    }

    /// A folder-list change, in memory: the account's folders, the open view, cached folder
    /// contents and Favorites follow a renamed folder; a deleted one disappears.
    fn apply_folder_op_memory(&mut self, account: &str, op: &Op) {
        let delimiter = self.delimiter(account);
        if let Some(a) = self.account_mut(account) {
            match op {
                Op::CreateFolder { path } => {
                    if !a.folders.iter().any(|f| &f.path == path) {
                        // Made in another mailbox (a shared one), it's in that mailbox too.
                        let parent = delimiter.and_then(|d| path.rsplit_once(d)).map(|(p, _)| p);
                        let mailbox = a.folders.iter().find(|f| Some(f.path.as_str()) == parent).and_then(|f| f.mailbox.clone());
                        a.folders.push(Folder {
                            account: account.to_string(),
                            path: path.clone(),
                            delimiter,
                            role: FolderRole::Other,
                            selectable: true,
                            mailbox,
                            ..Default::default()
                        });
                    }
                }
                Op::RenameFolder { from, to } => {
                    for f in a.folders.iter_mut() {
                        if let Some(new) = mailcraft_model::rebase_path(&f.path, from, to, f.delimiter.or(delimiter)) {
                            f.path = new;
                        }
                    }
                }
                Op::DeleteFolder { path } => a.folders.retain(|f| !mailcraft_model::is_within(&f.path, path, f.delimiter.or(delimiter))),
                Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Move { .. } | Op::Expunge { .. } | Op::EmptyFolder { .. } => {}
            }
            sort_folders(&mut a.folders);
        }
        match op {
            Op::CreateFolder { .. } => {}
            Op::RenameFolder { from, to } => {
                self.cache.retain(|(acc, f), _| acc != account || !mailcraft_model::is_within(f, from, delimiter));
                if let View::Folder { account: va, folder } = &mut self.view
                    && va == account
                    && let Some(new) = mailcraft_model::rebase_path(folder, from, to, delimiter)
                {
                    *folder = new;
                }
                if let Some(favs) = self.prefs.favorites.as_mut() {
                    for fav in favs.iter_mut().filter(|f| f.account == account) {
                        if let Some(new) = mailcraft_model::rebase_path(&fav.folder, from, to, delimiter) {
                            fav.folder = new;
                        }
                    }
                }
            }
            Op::DeleteFolder { path } => {
                self.cache.retain(|(acc, f), _| acc != account || !mailcraft_model::is_within(f, path, delimiter));
                if let Some(favs) = self.prefs.favorites.as_mut() {
                    favs.retain(|f| f.account != account || !mailcraft_model::is_within(&f.folder, path, delimiter));
                }
                if matches!(&self.view, View::Folder { account: va, folder } if va == account && mailcraft_model::is_within(folder, path, delimiter))
                {
                    self.view = self.default_view();
                    self.reading = None;
                    self.selection.clear();
                    self.focus = None;
                }
            }
            Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Move { .. } | Op::Expunge { .. } | Op::EmptyFolder { .. } => {}
        }
        self.index = None;
    }

    // ----- mailbox settings -----

    /// Delete everything in a folder for good (only Deleted Items and Junk Email).
    pub fn empty_folder(&mut self, account: &str, folder: &str) -> CmdResult {
        let f = self.folder(account, folder).ok_or_else(|| CmdError::Params(format!("no folder `{folder}`")))?;
        if !matches!(f.role, FolderRole::Trash | FolderRole::Junk) {
            // Any other folder: everything goes to Deleted Items, as Outlook's Delete All.
            let trash = self
                .account(account)
                .and_then(|a| a.folder(FolderRole::Trash))
                .map(|t| t.path.clone())
                .ok_or_else(|| CmdError::Params("the account has no Deleted Items folder".into()))?;
            let keys: Vec<MessageKey> = self.folder_state(account, folder).messages.iter().map(|m| m.key.clone()).collect();
            let n = keys.len();
            if n > 0 {
                self.apply_op(account, Op::Move { keys, to: trash });
            }
            return Ok(serde_json::json!({ "moved": n, "to": "trash" }));
        }
        self.apply_op(account, Op::EmptyFolder { path: folder.to_string() });
        Ok(serde_json::json!({ "emptied": folder }))
    }

    /// Ask the server for an account's automatic replies.
    pub fn load_vacation(&mut self, account: &str) -> CmdResult {
        let a = self.account_mut(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if a.is_local() {
            a.vacation = VacationState::Failed("the sample mailbox has no server".into());
        } else {
            a.vacation = VacationState::Loading;
            self.jobs.push(Job::GetVacation { account: account.to_string() });
        }
        Ok(self.vacation_json(account))
    }

    /// Change an account's automatic replies on the server.
    pub fn set_vacation(&mut self, account: &str, v: Vacation) -> CmdResult {
        for d in [&v.from, &v.until].into_iter().flatten() {
            if !mailcraft_model::job::is_day(d) {
                return Err(CmdError::Params(format!("`{d}` is not a date (YYYY-MM-DD)")));
            }
        }
        if let (Some(a), Some(b)) = (&v.from, &v.until)
            && a > b
        {
            return Err(CmdError::Params("the last day is before the first".into()));
        }
        if v.enabled && v.text.trim().is_empty() {
            return Err(CmdError::Params("write the reply first".into()));
        }
        let a = self.account_mut(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if a.is_local() {
            return Err(CmdError::Params("the sample mailbox has no server".into()));
        }
        a.vacation = VacationState::Loading;
        self.jobs.push(Job::SetVacation { account: account.to_string(), vacation: v });
        Ok(self.vacation_json(account))
    }

    pub fn vacation_json(&self, account: &str) -> serde_json::Value {
        match self.account(account).map(|a| &a.vacation) {
            Some(VacationState::Loaded(v)) => serde_json::json!({"vacation": v}),
            Some(VacationState::Loading) => serde_json::json!({"pending": true}),
            Some(VacationState::Failed(e)) => serde_json::json!({"error": e}),
            _ => serde_json::json!({}),
        }
    }

    /// Bytes of mail cached on this computer for an account.
    pub fn cache_size(&self, account: &str) -> u64 {
        self.store.as_ref().map_or(0, |s| s.account_size(account))
    }

    // ----- folder management -----

    /// The account's hierarchy delimiter (from any folder that has one).
    fn delimiter(&self, account: &str) -> Option<char> {
        self.account(account).and_then(|a| a.folders.iter().find_map(|f| f.delimiter))
    }

    /// Where top-level folders go: the root, or `INBOX` on servers that keep every folder under
    /// it (`INBOX.Sent`, `INBOX.Projects`).
    fn top_parent(&self, account: &str) -> Option<String> {
        let a = self.account(account)?;
        let d = self.delimiter(account)?;
        let others: Vec<&Folder> = a.folders.iter().filter(|f| !f.path.eq_ignore_ascii_case("INBOX")).collect();
        let inbox = a.folders.iter().find(|f| f.path.eq_ignore_ascii_case("INBOX"))?;
        (!others.is_empty() && others.iter().all(|f| f.path.starts_with(&format!("{}{d}", inbox.path)))).then(|| inbox.path.clone())
    }

    fn parent_of(path: &str, delimiter: Option<char>) -> Option<&str> {
        delimiter.and_then(|d| path.rsplit_once(d)).map(|(p, _)| p)
    }

    /// A user folder (not Inbox or another special folder) that exists, for changing.
    fn user_folder(&self, account: &str, path: &str) -> Result<Folder, CmdError> {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        let f = self.folder(account, path).cloned().ok_or_else(|| CmdError::Params(format!("no folder `{path}`")))?;
        if f.is_fixed() {
            return Err(CmdError::Failed(format!("{} can't be renamed, moved or deleted", f.display_name())));
        }
        Ok(f)
    }

    /// Create a folder called `name` inside `parent` (`None`: at the top).
    pub fn create_folder(&mut self, account: &str, parent: Option<&str>, name: &str) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if let Some(p) = parent
            && self.folder(account, p).is_none()
        {
            return Err(CmdError::Params(format!("no folder `{p}`")));
        }
        let parent = parent.map(str::to_string).or_else(|| self.top_parent(account));
        let path = mailcraft_model::child_path(parent.as_deref(), self.delimiter(account), name).map_err(CmdError::Params)?;
        if self.folder(account, &path).is_some() {
            return Err(CmdError::Params(format!("there is already a folder called {}", name.trim())));
        }
        self.apply_op(account, Op::CreateFolder { path: path.clone() });
        Ok(serde_json::json!({ "created": path }))
    }

    /// Rename a folder (it stays where it is).
    pub fn rename_folder(&mut self, account: &str, path: &str, name: &str) -> CmdResult {
        let f = self.user_folder(account, path)?;
        let d = f.delimiter.or(self.delimiter(account));
        let to = mailcraft_model::child_path(Self::parent_of(path, d), d, name).map_err(CmdError::Params)?;
        if to == path {
            return Ok(serde_json::json!({ "renamed": to }));
        }
        if self.folder(account, &to).is_some() {
            return Err(CmdError::Params(format!("there is already a folder called {}", name.trim())));
        }
        self.apply_op(account, Op::RenameFolder { from: path.to_string(), to: to.clone() });
        Ok(serde_json::json!({ "renamed": to }))
    }

    /// Move a folder inside another (`None`: to the top).
    pub fn move_folder(&mut self, account: &str, path: &str, parent: Option<&str>) -> CmdResult {
        let f = self.user_folder(account, path)?;
        let d = f.delimiter.or(self.delimiter(account));
        let d = d.ok_or_else(|| CmdError::Failed("this server has no subfolders".into()))?;
        let parent = match parent {
            Some(p) => {
                if self.folder(account, p).is_none() {
                    return Err(CmdError::Params(format!("no folder `{p}`")));
                }
                if mailcraft_model::is_within(p, path, Some(d)) {
                    return Err(CmdError::Params("a folder can't be moved into itself".into()));
                }
                Some(p.to_string())
            }
            None => self.top_parent(account),
        };
        let leaf = path.rsplit(d).next().unwrap_or(path);
        let to = match &parent {
            Some(p) => format!("{p}{d}{leaf}"),
            None => leaf.to_string(),
        };
        if to == path {
            return Ok(serde_json::json!({ "moved": to }));
        }
        if self.folder(account, &to).is_some() {
            return Err(CmdError::Params(format!("there is already a folder called {} there", f.display_name())));
        }
        self.apply_op(account, Op::RenameFolder { from: path.to_string(), to: to.clone() });
        Ok(serde_json::json!({ "moved": to }))
    }

    /// Delete a folder: into Deleted Items, or for good when it is already there (or
    /// `permanent`, or there is no Deleted Items folder). Subfolders go with it.
    pub fn delete_folder(&mut self, account: &str, path: &str, permanent: bool) -> CmdResult {
        let f = self.user_folder(account, path)?;
        let d = f.delimiter.or(self.delimiter(account));
        let trash = self.account(account).and_then(|a| a.folder(FolderRole::Trash)).map(|t| t.path.clone());
        let in_trash = trash.as_deref().is_some_and(|t| mailcraft_model::is_within(path, t, d));
        if let (Some(trash), Some(d), false, false) = (&trash, d, in_trash, permanent) {
            // Like deleting mail: the folder waits in Deleted Items.
            let leaf = path.rsplit(d).next().unwrap_or(path);
            let mut to = format!("{trash}{d}{leaf}");
            let mut n = 2;
            while self.folder(account, &to).is_some() {
                to = format!("{trash}{d}{leaf} ({n})");
                n += 1;
            }
            self.apply_op(account, Op::RenameFolder { from: path.to_string(), to: to.clone() });
            return Ok(serde_json::json!({ "moved": to }));
        }
        // Deepest first: servers refuse to delete a folder that still has children.
        let mut doomed: Vec<String> = self
            .account(account)
            .map(|a| a.folders.iter().filter(|x| mailcraft_model::is_within(&x.path, path, d)).map(|x| x.path.clone()).collect())
            .unwrap_or_default();
        doomed.sort_by_key(|p| std::cmp::Reverse(p.len()));
        let n = doomed.len();
        for p in doomed {
            self.apply_op(account, Op::DeleteFolder { path: p });
        }
        Ok(serde_json::json!({ "deleted": n }))
    }

    /// Favorites as shown: the user's list, or the defaults.
    pub fn favorites(&self) -> Vec<FolderRef> {
        match &self.prefs.favorites {
            Some(list) => list.iter().filter(|r| self.folder(&r.account, &r.folder).is_some()).cloned().collect(),
            None if self.accounts.len() == 1 => self
                .accounts
                .first()
                .map(|a| {
                    [FolderRole::Inbox, FolderRole::Sent, FolderRole::Drafts]
                        .iter()
                        .filter_map(|r| a.folder(*r))
                        .map(|f| FolderRef { account: a.config.id.clone(), folder: f.path.clone() })
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// Add a folder to Favorites or take it off (`on: None` toggles).
    pub fn set_favorite(&mut self, account: &str, folder: &str, on: Option<bool>) -> CmdResult {
        if self.folder(account, folder).is_none() {
            return Err(CmdError::Params(format!("no folder `{folder}` in account `{account}`")));
        }
        let mut list = self.favorites();
        let r = FolderRef { account: account.to_string(), folder: folder.to_string() };
        let present = list.contains(&r);
        let on = on.unwrap_or(!present);
        if on && !present {
            list.push(r);
        } else if !on {
            list.retain(|x| *x != r);
        }
        self.prefs.favorites = Some(list);
        Ok(serde_json::json!({ "on": on }))
    }

    /// Count a local account's folders again (`only` these, or all of them).
    fn recount_local(&mut self, account: &str, only: Option<&[String]>) {
        let paths: Vec<String> = match only {
            Some(list) => list.to_vec(),
            None => self.account(account).map(|a| a.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default(),
        };
        let mut counts = Vec::new();
        for p in paths {
            let st = self.folder_state(account, &p);
            counts.push((p, st.unread(), u32::try_from(st.messages.len()).unwrap_or(u32::MAX)));
        }
        let mut changed = false;
        if let Some(a) = self.account_mut(account) {
            for (p, unread, total) in counts {
                if let Some(f) = a.folders.iter_mut().find(|f| f.path == p)
                    && (f.unread != unread || f.total != total)
                {
                    f.unread = unread;
                    f.total = total;
                    changed = true;
                }
            }
        }
        // Local accounts have no server to report counts: keep them on disk.
        if changed
            && let (Some(store), Some(a)) = (&self.store, self.account(account))
            && let Err(e) = store.save_folders(account, &a.folders)
        {
            log::warn!("{account}: folder counts not saved: {e}");
        }
    }

    /// The message to show after `gone` leave the list: the next one down, else the one above.
    fn successor(&self, gone: &[MessageKey]) -> Option<MessageKey> {
        let gone: HashSet<&MessageKey> = gone.iter().collect();
        let last = self.list.iter().rposition(|m| gone.contains(&m.key))?;
        self.list
            .iter()
            .skip(last + 1)
            .find(|m| !gone.contains(&m.key))
            .or_else(|| self.list.iter().take(last).rev().find(|m| !gone.contains(&m.key)))
            .map(|m| m.key.clone())
    }

    /// Move the selection to a folder of the same role in each message's account (Archive,
    /// Deleted Items, Junk), or to an explicit folder.
    pub fn move_selection(&mut self, target: MoveTarget) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let next = self.successor(&self.selection.clone());
        let mut by_account: HashMap<String, Vec<MessageKey>> = HashMap::new();
        for m in &sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
        }
        let mut moved = 0usize;
        let mut accounts: Vec<String> = by_account.keys().cloned().collect();
        accounts.sort();
        for a in accounts {
            let keys = by_account.remove(&a).unwrap_or_default();
            let dest = match &target {
                MoveTarget::Folder(f) => Some(f.clone()),
                MoveTarget::Role(r) => self.account(&a).and_then(|acc| acc.folder(*r)).map(|f| f.path.clone()),
            };
            let Some(dest) = dest else {
                let what = match target {
                    MoveTarget::Role(r) => r.label().unwrap_or("that folder"),
                    MoveTarget::Folder(_) => "that folder",
                };
                return Err(CmdError::Failed(format!("this account has no {what} folder")));
            };
            if self.folder(&a, &dest).is_none() {
                return Err(CmdError::Params(format!("no folder `{dest}`")));
            }
            let keys: Vec<MessageKey> = keys.into_iter().filter(|k| k.folder != dest).collect();
            moved += keys.len();
            if !keys.is_empty() {
                // Undo finds them again by Message-ID (those without one can't come back).
                let back: Vec<(String, String)> = keys
                    .iter()
                    .filter_map(|k| self.summary(k).filter(|m| !m.message_id.is_empty()).map(|m| (m.message_id.clone(), k.folder.clone())))
                    .collect();
                if !back.is_empty() {
                    self.remember(Undo::Move { account: a.clone(), to: dest.clone(), back });
                }
                self.apply_op(&a, Op::Move { keys, to: dest });
            }
        }
        self.after_removal(next);
        Ok(serde_json::json!({ "moved": moved }))
    }

    /// Delete permanently.
    pub fn expunge_selection(&mut self) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let next = self.successor(&self.selection.clone());
        let mut by_account: HashMap<String, Vec<MessageKey>> = HashMap::new();
        for m in &sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
        }
        let n = sel.len();
        for (a, keys) in by_account {
            self.apply_op(&a, Op::Expunge { keys });
        }
        self.after_removal(next);
        Ok(serde_json::json!({ "deleted": n }))
    }

    fn after_removal(&mut self, next: Option<MessageKey>) {
        self.reading = None;
        self.selection.clear();
        self.focus = None;
        if let Some(k) = next
            && self.list.iter().any(|m| m.key == k)
        {
            // Show the next message, but don't mark it read just because something was deleted.
            self.show_message(&k, false);
        }
    }

    /// Set or clear a flag on the selection (`on: None` toggles, following the first message).
    pub fn flag_selection(&mut self, flag: Flag, on: Option<bool>) -> CmdResult {
        let sel = self.selected();
        let first = sel.first().ok_or_else(|| CmdError::Disabled("no message selected".into()))?;
        let current = flag.get(&first.flags);
        let on = on.unwrap_or(!current);
        let mut by_account: HashMap<String, Vec<MessageKey>> = HashMap::new();
        // Undo puts back only the ones that changed.
        let mut changed: HashMap<String, Vec<MessageKey>> = HashMap::new();
        for m in &sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
            if flag.get(&m.flags) != on {
                changed.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
        }
        for (a, keys) in changed {
            self.remember(Undo::Ops { account: a, ops: vec![Op::SetFlag { keys, flag, on: !on }] });
        }
        for (a, keys) in by_account {
            self.apply_op(&a, Op::SetFlag { keys, flag, on });
        }
        // Unflagged: no longer due.
        if flag == Flag::Flagged && !on {
            for m in &sel {
                self.prefs.follow_up.remove(&follow_up_id(m));
            }
        }
        Ok(serde_json::json!({ "on": on, "count": sel.len() }))
    }

    /// Flag the selection for follow-up by a day (`None`: no date), or (`complete`) mark it
    /// done: unflagged, its date forgotten.
    pub fn follow_up_selection(&mut self, due: Option<i64>, complete: bool) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        self.flag_selection(Flag::Flagged, Some(!complete))?;
        for m in &sel {
            let id = follow_up_id(m);
            match due.filter(|_| !complete) {
                Some(d) if self.prefs.follow_up.len() < 10_000 || self.prefs.follow_up.contains_key(&id) => {
                    self.prefs.follow_up.insert(id, d);
                }
                _ => {
                    self.prefs.follow_up.remove(&id);
                }
            }
        }
        Ok(serde_json::json!({ "flagged": !complete, "due": due.filter(|_| !complete).map(mailcraft_model::dates::day_string), "count": sel.len() }))
    }

    /// When a flagged message is due, if a day was given.
    pub fn follow_up_due(&self, m: &Summary) -> Option<i64> {
        if !m.flags.flagged {
            return None;
        }
        self.prefs.follow_up.get(&follow_up_id(m)).copied()
    }

    fn remember(&mut self, u: Undo) {
        self.undo.push(u);
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
    }

    /// Put the last change back (Undo).
    pub fn undo_last(&mut self) -> CmdResult {
        let u = self.undo.pop().ok_or_else(|| CmdError::Disabled("nothing to undo".into()))?;
        match u {
            Undo::Ops { account, ops } => {
                for op in ops {
                    self.apply_op(&account, op);
                }
                Ok(serde_json::json!({ "undone": "changes" }))
            }
            Undo::Move { account, to, back } => {
                // Where each one is now: the destination's messages with that Message-ID.
                let here = self.folder_state(&account, &to).messages.clone();
                let mut by_folder: std::collections::BTreeMap<String, Vec<MessageKey>> = std::collections::BTreeMap::new();
                let mut used: BTreeSet<MessageKey> = BTreeSet::new();
                for (id, from) in &back {
                    if let Some(m) = here.iter().find(|m| &m.message_id == id && !used.contains(&m.key)) {
                        used.insert(m.key.clone());
                        by_folder.entry(from.clone()).or_default().push(m.key.clone());
                    }
                }
                if by_folder.is_empty() {
                    // Not there yet (the server is still moving them): try again in a moment.
                    self.undo.push(Undo::Move { account, to, back });
                    return Err(CmdError::Failed("still being moved; try Undo again in a moment".into()));
                }
                let n = used.len();
                for (from, keys) in by_folder {
                    self.apply_op(&account, Op::Move { keys, to: from });
                }
                Ok(serde_json::json!({ "undone": "move", "count": n }))
            }
        }
    }

    // ----- search -----

    fn ensure_index(&mut self, scope: SearchScope) {
        if self.index.as_ref().is_some_and(|ix| ix.scope == scope && (scope == SearchScope::All || ix.view == self.view)) {
            return;
        }
        let folders: Vec<(String, String)> = match scope {
            SearchScope::Current => self.view_folders(),
            SearchScope::All => {
                self.accounts.iter().flat_map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| (a.config.id.clone(), f.path.clone()))).collect()
            }
        };
        let mut index = Index::new();
        let mut keys = Vec::new();
        let mut has_body = Vec::new();
        for (a, f) in folders {
            // A thousand at a time: a big folder isn't copied whole while it's read.
            let n = self.folder_state(&a, &f).messages.len();
            let mut at = 0;
            while at < n {
                let msgs: Vec<Summary> = self.folder_state(&a, &f).messages.get(at..(at + 1000).min(n)).map(<[Summary]>::to_vec).unwrap_or_default();
                if msgs.is_empty() {
                    break;
                }
                at += msgs.len();
                for m in msgs {
                    let doc = u32::try_from(keys.len()).unwrap_or(u32::MAX);
                    index.add(doc, &summary_text(&m));
                    let words = self.words_of(&m.key);
                    if let Some(words) = &words {
                        index.add(doc, words);
                    }
                    has_body.push(words.is_some());
                    keys.push(m.key);
                }
            }
        }
        self.index = Some(SearchIndex { index, keys, has_body, scope, view: self.view.clone() });
    }

    /// A downloaded message's words for search, read once (`None`: not downloaded).
    fn words_of(&mut self, key: &MessageKey) -> Option<Arc<str>> {
        if let Some(w) = self.body_words.get(key) {
            return Some(w.clone());
        }
        let raw = self.store.as_ref().and_then(|s| s.read_body(key))?;
        let words: Arc<str> = Arc::from(mime::search_text(&raw));
        self.keep_words(key, words.clone());
        Some(words)
    }

    /// Keep a message's body words, the oldest kept going first past [`BODY_WORDS_BYTES`] (all
    /// at once: they're cheap to read again).
    fn keep_words(&mut self, key: &MessageKey, words: Arc<str>) {
        if self.body_words_bytes + words.len() > BODY_WORDS_BYTES {
            self.body_words.clear();
            self.body_words_bytes = 0;
        }
        self.body_words_bytes += words.len();
        if let Some(old) = self.body_words.insert(key.clone(), words) {
            self.body_words_bytes = self.body_words_bytes.saturating_sub(old.len());
        }
    }

    fn search_results(&mut self, q: &str, scope: SearchScope) -> Vec<Summary> {
        let mut query = Query::parse(q);
        // `category:Work` means the Work category's keyword (`$label2`).
        // `is:categorized` means one of the categories.
        for c in &mut query.clauses {
            if let mailcraft_model::search::Term::Keyword(k) = &mut c.term
                && let Some(cat) = self.category(k)
            {
                *k = cat.keyword.clone();
            }
            if let mailcraft_model::search::Term::Categorized(ks) = &mut c.term {
                *ks = self.prefs.categories.iter().map(|c| c.keyword.clone()).collect();
            }
        }
        if query.is_empty() {
            return Vec::new();
        }
        self.ensure_index(scope);
        let Some(ix) = self.index.take() else { return Vec::new() };
        let mut out = Vec::new();
        // Each folder's messages by number, made once: looked up one by one in the list, a search
        // finding most of a big folder was quadratic.
        let mut by_uid: HashMap<(String, String), HashMap<u32, usize>> = HashMap::new();
        let needs_body = query.needs_body();
        for doc in ix.index.candidates(&query) {
            let Some(key) = ix.keys.get(doc as usize) else { continue };
            let folder = (key.account.clone(), key.folder.clone());
            if !by_uid.contains_key(&folder) {
                // Where each message is in its folder's list (not a copy of every summary).
                let map = self.folder_state(&key.account, &key.folder).messages.iter().enumerate().map(|(i, m)| (m.key.uid, i)).collect();
                by_uid.insert(folder.clone(), map);
            }
            let Some(&at) = by_uid.get(&folder).and_then(|f| f.get(&key.uid)) else { continue };
            let Some(m) = self.folder_state(&key.account, &key.folder).messages.get(at).filter(|m| m.key.uid == key.uid).cloned() else { continue };
            // The body read again only when the index can't settle the query.
            let body = if needs_body && ix.has_body.get(doc as usize) == Some(&true) { self.words_of(key) } else { None };
            if query.matches(&Doc { summary: &m, body: body.as_deref(), indexed: true }) {
                out.push(m);
            }
        }
        self.index = Some(ix);
        out
    }

    pub fn run_search(&mut self, query: &str, scope: SearchScope) -> usize {
        self.search_as(query, scope, None)
    }

    /// A search, or (`folder`) a Search Folder's: a search typed in is never one, whatever it
    /// asks.
    pub(crate) fn search_as(&mut self, query: &str, scope: SearchScope, folder: Option<String>) -> usize {
        self.search_folder = folder;
        let q = query.trim();
        self.search = if q.is_empty() { None } else { Some((q.to_string(), scope)) };
        if self.server_search.as_ref().is_some_and(|ss| ss.query != q) {
            self.server_search = None;
        }
        self.selection.clear();
        self.focus = None;
        self.reading = None;
        self.refresh_list();
        self.list.len()
    }

    // ----- writing -----

    fn me(&self, account: &str) -> Vec<String> {
        let mut v: Vec<String> = self.accounts.iter().map(|a| a.config.email.clone()).collect();
        if let Some(a) = self.account(account) {
            v.push(a.config.username.clone());
        }
        v
    }

    fn signature(&self, account: &str) -> String {
        self.signature_rich(account).0
    }

    /// An account's signature with its look: its text and styles (plain when it has none).
    pub fn signature_rich(&self, account: &str) -> (String, mime::rich::Styles) {
        let Some(a) = self.account(account) else { return (String::new(), mime::rich::Styles::default()) };
        if !a.config.signature_html.trim().is_empty() {
            let (text, styles) = mime::pasted_html(&a.config.signature_html);
            if !text.trim().is_empty() {
                return (text, styles);
            }
        }
        (a.config.signature.clone(), mime::rich::Styles::plain(&a.config.signature))
    }

    /// The signature in the message being made, given its look (the text went in plain).
    fn style_signature(&mut self) {
        let Some(account) = self.compose.as_ref().map(|c| c.account.clone()) else { return };
        let sig = self.signature_rich(&account);
        if let Some(c) = self.compose.as_mut() {
            style_signature(c, &sig);
        }
    }

    pub fn new_compose(&mut self, account: Option<&str>) -> Result<&mut Compose, CmdError> {
        let account = match account {
            Some(a) => self.account(a).map(|a| a.config.id.clone()),
            None => self.current_account().map(|a| a.config.id.clone()),
        }
        .ok_or_else(|| CmdError::Disabled("add an account first".into()))?;
        let sig = self.signature(&account);
        let id = self.fresh_id();
        let body = if sig.trim().is_empty() { String::new() } else { format!("\n\n{}", signature_block(&sig)) };
        let window = self.prefs.new_mail_window;
        let signature = if sig.trim().is_empty() { String::new() } else { signature_block(&sig) };
        let styles = mime::rich::Styles::plain(&body);
        self.compose = Some(Compose { id, account, body, styles, window, signature, ..Default::default() });
        self.style_signature();
        self.compose.as_mut().ok_or_else(|| CmdError::Failed("compose".into()))
    }

    pub fn reply_compose(&mut self, mode: ReplyMode) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let parsed = r.parsed.ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
        let account = r.key.account.clone();
        let sig = self.signature(&account);
        let (d, styles) = mime::reply_doc(&parsed, mode, &self.me(&account), &self.clock, &sig);
        let id = self.fresh_id();
        let mut attachments = Vec::new();
        if mode == ReplyMode::Forward
            && let Some(raw) = self.store.as_ref().and_then(|s| s.read_body(&r.key))
        {
            for a in parsed.attachments.iter().filter(|a| !a.inline) {
                if let Some(data) = mime::attachment_data(&raw, a.index) {
                    attachments.push(OutAttachment { name: a.name.clone(), mime: a.mime.clone(), data });
                }
            }
        }
        self.compose = Some(Compose {
            id,
            account,
            signature: if sig.trim().is_empty() { String::new() } else { signature_block(&sig) },
            to: addr::format_field(&d.to),
            cc: addr::format_field(&d.cc),
            subject: d.subject,
            body: d.body,
            styles,
            in_reply_to: d.in_reply_to,
            references: d.references,
            attachments,
            mode: Some(mode),
            source: Some(r.key),
            ..Default::default()
        });
        self.style_signature();
        Ok(serde_json::json!({ "compose": id }))
    }

    /// Forward the open message as an attachment, as Outlook's More › Forward as Attachment: the
    /// original goes along whole (an `.eml`), the new message holds only the signature.
    pub fn forward_as_attachment(&mut self) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let raw =
            self.store.as_ref().and_then(|s| s.read_body(&r.key)).ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
        let out = self.reply_compose(ReplyMode::Forward)?;
        let sig = self.signature(&r.key.account);
        let subject = r.parsed.as_ref().map(|p| p.subject.trim().to_string()).unwrap_or_default();
        let name = format!("{}.eml", if subject.is_empty() { "message" } else { subject.as_str() });
        if let Some(c) = self.compose.as_mut() {
            c.set_text(if sig.trim().is_empty() { String::new() } else { format!("\n\n{}", signature_block(&sig)) });
            c.signature = if sig.trim().is_empty() { String::new() } else { signature_block(&sig) };
            c.attachments = vec![OutAttachment { name, mime: "message/rfc822".into(), data: raw }];
        }
        self.style_signature();
        Ok(out)
    }

    fn compose_draft(&self, c: &Compose) -> Result<mime::Draft, CmdError> {
        let parse = |field: &str, s: &str| addr::parse_list(s).map_err(|bad| CmdError::Params(format!("{field}: not an email address: {bad}")));
        let (body, html, inline) = c.bodies();
        Ok(mime::Draft {
            reply_to: parse("Replies to", &c.reply_to)?,
            to: parse("To", &c.to)?,
            cc: parse("Cc", &c.cc)?,
            bcc: parse("Bcc", &c.bcc)?,
            subject: c.subject.trim().to_string(),
            body,
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
            attachments: c.attachments.clone(),
            html,
            inline,
            importance: c.importance,
            read_receipt: c.read_receipt,
        })
    }

    fn compose_bytes(&mut self, c: &Compose, d: &mime::Draft) -> Result<(Vec<u8>, String), CmdError> {
        let acc = self.account(&c.account).ok_or_else(|| CmdError::Failed("the sending account is gone".into()))?;
        let from = mailcraft_model::EmailAddress::new(acc.config.name.clone(), acc.config.email.clone());
        let email = acc.config.email.clone();
        let now = self.clock.now;
        let mid = mime::new_message_id(&email, now, self.random());
        let raw = mime::build(d, &from, now, &mid).map_err(CmdError::Failed)?;
        Ok((raw, email))
    }

    pub fn send_compose(&mut self, allow_empty_subject: bool) -> CmdResult {
        let c = self.compose.clone().ok_or_else(|| CmdError::Disabled("nothing to send".into()))?;
        if c.sending {
            return Err(CmdError::Disabled("already sending".into()));
        }
        let d = self.compose_draft(&c)?;
        if d.recipients().is_empty() {
            return Err(CmdError::Params("add at least one recipient".into()));
        }
        if d.subject.is_empty() && !allow_empty_subject {
            return Err(CmdError::Params("the subject is empty (send with {\"allowEmptySubject\": true} to send anyway)".into()));
        }
        let (raw, from) = self.compose_bytes(&c, &d)?;
        let local = self.account(&c.account).is_none_or(AccountState::is_local);
        let sent_folder = self.account(&c.account).and_then(|a| a.folder(FolderRole::Sent)).map(|f| f.path.clone());
        // Delay Delivery: into the Outbox, sent once it's time (the app has to be running then).
        if let Some(at) = c.send_after.filter(|at| *at > self.clock.now && !local) {
            let item = OutboxItem {
                id: c.id,
                from,
                recipients: d.recipients(),
                sent_folder,
                queued: self.clock.now,
                attempts: 0,
                error: None,
                not_before: at,
            };
            let store = self.store.as_ref().ok_or_else(|| CmdError::Failed("there is nowhere to keep it".into()))?;
            store.outbox_add(&c.account, &item, &raw).map_err(|e| CmdError::Failed(format!("the Outbox couldn't keep it: {e}")))?;
            self.after_outbox_change();
            self.finish_sent(&c, false);
            // finish_sent says "sent"; it isn't yet.
            self.notices.pop();
            self.notice(format!("It will be sent {}", mailcraft_model::dates::full_time(at, &self.clock)), false);
            return Ok(serde_json::json!({ "queued": c.id, "sendAfter": at }));
        }
        if local {
            if let Some(folder) = &sent_folder {
                self.file_locally(&c.account, folder, &raw, true);
            }
            self.finish_sent(&c, true);
            return Ok(serde_json::json!({ "sent": c.id, "local": true }));
        }
        self.jobs.push(Job::Send { account: c.account.clone(), id: c.id, raw, from, recipients: d.recipients(), sent_folder });
        if let Some(cm) = self.compose.as_mut() {
            cm.sending = true;
            cm.error = None;
        }
        Ok(serde_json::json!({ "sending": c.id }))
    }

    fn save_pending_marks(&self) {
        let list: Vec<(u64, &MessageKey, &Flag)> = self.pending_marks.iter().map(|(id, (k, f))| (*id, k, f)).collect();
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc(PENDING_MARKS, &list)
        {
            log::warn!("{PENDING_MARKS}: {e}");
        }
    }

    /// The compose is finished. `delivered`: the server accepted it, so the original gets its
    /// Replied or Forwarded flag now; otherwise it sits in the Outbox and the flag is owed until
    /// delivery.
    fn finish_sent(&mut self, c: &Compose, delivered: bool) {
        if let (Some(src), Some(mode)) = (&c.source, c.mode) {
            let flag = if mode == ReplyMode::Forward { Flag::Forwarded } else { Flag::Answered };
            if delivered {
                self.apply_op(&src.account.clone(), Op::SetFlag { keys: vec![src.clone()], flag, on: true });
            } else if self.pending_marks.len() < 1000 {
                self.pending_marks.insert(c.id, (src.clone(), flag));
                self.save_pending_marks();
            }
        }
        if let Some(draft) = &c.draft {
            self.apply_op(&draft.account.clone(), Op::Expunge { keys: vec![draft.clone()] });
        }
        if self.compose.as_ref().is_some_and(|x| x.id == c.id) {
            self.compose = None;
        }
        self.notice("Message sent", false);
    }

    /// Save the message being written to Drafts.
    pub fn save_draft(&mut self) -> CmdResult {
        let c = self.compose.clone().ok_or_else(|| CmdError::Disabled("nothing to save".into()))?;
        // Drafts may hold unfinished addresses: keep only the valid ones.
        let lenient = |s: &str| s.split([',', ';']).filter_map(|p| addr::parse_list(p).ok()).flatten().collect::<Vec<_>>();
        let (body, html, inline) = c.bodies();
        let d = mime::Draft {
            reply_to: lenient(&c.reply_to),
            to: lenient(&c.to),
            cc: lenient(&c.cc),
            bcc: lenient(&c.bcc),
            subject: c.subject.clone(),
            body,
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
            attachments: c.attachments.clone(),
            html,
            inline,
            importance: c.importance,
            read_receipt: c.read_receipt,
        };
        let folder = self
            .account(&c.account)
            .and_then(|a| a.folder(FolderRole::Drafts))
            .map(|f| f.path.clone())
            .ok_or_else(|| CmdError::Failed("this account has no Drafts folder".into()))?;
        let (raw, _) = self.compose_bytes(&c, &d)?;
        if self.account(&c.account).is_none_or(AccountState::is_local) {
            if let Some(old) = &c.draft {
                self.apply_op(&c.account.clone(), Op::Expunge { keys: vec![old.clone()] });
            }
            let key = self.file_locally(&c.account, &folder, &raw, false);
            if let Some(cm) = self.compose.as_mut() {
                cm.draft = key;
            }
        } else {
            self.jobs.push(Job::SaveDraft { account: c.account.clone(), raw, folder, replaces: c.draft.clone() });
            if let Some(cm) = self.compose.as_mut() {
                // The server assigns the new copy's UID; the old one is replaced by the worker.
                cm.draft = None;
            }
        }
        self.notice("Draft saved", false);
        Ok(serde_json::json!({ "saved": c.id }))
    }

    /// File a message in a local account's folder; returns its key.
    fn file_locally(&mut self, account: &str, folder: &str, raw: &[u8], seen: bool) -> Option<MessageKey> {
        let store = self.store.clone()?;
        let mut st = store.load_folder(account, folder).unwrap_or_default();
        if st.validity == 0 {
            st.validity = 1;
        }
        let uid = st.uid_next.max(st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1);
        st.uid_next = uid + 1;
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: st.validity, uid };
        let flags = mailcraft_model::Flags { seen, draft: !seen, ..Default::default() };
        let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        let s = mime::summary_from_full(raw, key.clone(), flags, size, self.clock.now);
        let _ = store.write_body(&key, raw);
        st.messages.push(s);
        sort_newest_first(&mut st.messages);
        if let Err(e) = store.save_folder(account, folder, &st) {
            self.notice(e.to_string(), true);
            return None;
        }
        self.reload_folder(account, folder);
        self.recount_local(account, Some(&[folder.to_string()]));
        self.refresh_list();
        Some(key)
    }

    // ----- outbox -----

    /// Re-read the Outbox from disk.
    pub fn refresh_outbox(&mut self) {
        let Some(store) = &self.store else { return };
        self.outbox = self.accounts.iter().flat_map(|a| store.outbox(&a.config.id).into_iter().map(|i| (a.config.id.clone(), i))).collect();
    }

    fn after_outbox_change(&mut self) {
        self.refresh_outbox();
        if self.view == View::Outbox {
            self.refresh_list();
        }
    }

    /// Queued messages, as list rows. Their key is `(account, OUTBOX, 0, n)`, `n` counting from 1
    /// in Outbox order.
    fn outbox_summaries(&self) -> Vec<Summary> {
        let Some(store) = &self.store else { return Vec::new() };
        let mut out = Vec::new();
        for (n, (account, item)) in self.outbox.iter().enumerate() {
            let key = MessageKey { account: account.clone(), folder: OUTBOX.to_string(), validity: 0, uid: u32::try_from(n + 1).unwrap_or(u32::MAX) };
            let raw = store.outbox_raw(account, item.id).unwrap_or_default();
            let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
            let flags = mailcraft_model::Flags { seen: true, ..Default::default() };
            let mut m = mime::summary_from_full(&raw, key, flags, size, item.queued);
            m.preview = match &item.error {
                Some(e) if e != "offline" => format!("Not sent yet: {e}"),
                _ if item.not_before > self.clock.now => format!("Will be sent {}", mailcraft_model::dates::full_time(item.not_before, &self.clock)),
                _ => "Waiting to be sent".to_string(),
            };
            out.push(m);
        }
        out
    }

    /// The Outbox entry behind an Outbox key.
    pub fn outbox_entry(&self, key: &MessageKey) -> Option<(String, OutboxItem)> {
        if key.folder != OUTBOX {
            return None;
        }
        let n = usize::try_from(key.uid).ok()?.checked_sub(1)?;
        self.outbox.get(n).filter(|(a, _)| *a == key.account).cloned()
    }

    /// Send one held-back Outbox message now (Delay Delivery off), as its Send now button.
    pub fn send_outbox_now(&mut self, key: &MessageKey) -> CmdResult {
        let (account, mut item) = self.outbox_entry(key).ok_or_else(|| CmdError::Params("not an Outbox message".into()))?;
        if item.not_before != 0 {
            item.not_before = 0;
            if let Some(store) = &self.store {
                store.outbox_update(&account, &item).map_err(|e| CmdError::Failed(e.to_string()))?;
            }
            self.after_outbox_change();
        }
        self.jobs.push(Job::FlushOutbox { account });
        Ok(serde_json::json!({ "sending": item.id }))
    }

    /// Ask the accounts holding Outbox mail whose time has come to send it. Called with
    /// [`Session::tick`]; each message is asked for once.
    pub(crate) fn flush_due_outbox(&mut self) {
        let now = self.clock.now;
        let due: Vec<(String, u64)> = self
            .outbox
            .iter()
            .filter(|(_, i)| i.not_before != 0 && i.not_before <= now && !self.flushed.contains(&i.id))
            .map(|(a, i)| (a.clone(), i.id))
            .collect();
        let mut accounts: Vec<String> = Vec::new();
        for (a, id) in due {
            self.flushed.insert(id);
            if !accounts.contains(&a) {
                accounts.push(a);
            }
        }
        for account in accounts {
            self.jobs.push(Job::FlushOutbox { account });
        }
    }

    /// Try to send everything in the Outbox now.
    pub fn send_outbox(&mut self) -> CmdResult {
        let mut accounts: Vec<String> = self.outbox.iter().map(|(a, _)| a.clone()).collect();
        accounts.dedup();
        for a in &accounts {
            self.jobs.push(Job::FlushOutbox { account: a.clone() });
        }
        Ok(serde_json::json!({ "accounts": accounts.len(), "messages": self.outbox.len() }))
    }

    /// Delete the selected Outbox messages (they are never sent).
    pub fn delete_outbox_selection(&mut self) -> CmdResult {
        let gone: Vec<(String, OutboxItem)> = self.selection.iter().filter_map(|k| self.outbox_entry(k)).collect();
        if let Some(store) = &self.store {
            for (a, i) in &gone {
                store.outbox_remove(a, i.id);
            }
        }
        // Removed from the Outbox undelivered: whatever flags they owed are dropped.
        let before = self.pending_marks.len();
        self.pending_marks.retain(|id, _| !gone.iter().any(|(_, i)| i.id == *id));
        if self.pending_marks.len() != before {
            self.save_pending_marks();
        }
        self.reading = None;
        self.selection.clear();
        self.focus = None;
        self.after_outbox_change();
        Ok(serde_json::json!({ "deleted": gone.len() }))
    }

    /// Take a message back out of the Outbox to change it: it opens for editing.
    pub fn edit_outbox(&mut self, key: &MessageKey) -> CmdResult {
        let (account, item) = self.outbox_entry(key).ok_or_else(|| CmdError::Params("not an Outbox message".into()))?;
        let store = self.store.clone().ok_or_else(|| CmdError::Failed("no mail cache".into()))?;
        let raw = store.outbox_raw(&account, item.id).ok_or_else(|| CmdError::Failed("the message is gone".into()))?;
        let p = mime::parse(&raw).ok_or_else(|| CmdError::Failed("the message can't be read".into()))?;
        let mut attachments = Vec::new();
        for a in &p.attachments {
            if let Some(data) = mime::attachment_data(&raw, a.index) {
                attachments.push(OutAttachment { name: a.name.clone(), mime: a.mime.clone(), data });
            }
        }
        // Bcc isn't written into a message: it's the envelope recipients not in To or Cc.
        let shown: Vec<String> = p.to.iter().chain(&p.cc).map(|a| a.email.to_ascii_lowercase()).collect();
        let bcc: Vec<String> = item.recipients.iter().filter(|r| !shown.contains(&r.to_ascii_lowercase())).cloned().collect();
        let id = self.fresh_id();
        self.compose = Some(Compose {
            id,
            account: account.clone(),
            to: addr::format_field(&p.to),
            cc: addr::format_field(&p.cc),
            bcc: if bcc.is_empty() { String::new() } else { format!("{}; ", bcc.join("; ")) },
            show_cc: !bcc.is_empty(),
            subject: p.subject.clone(),
            body: p.text.clone(),
            in_reply_to: p.in_reply_to.clone(),
            references: p.references.clone(),
            attachments,
            ..Default::default()
        });
        // Pulled back for editing: the owed flag moves onto the compose, for its delivery.
        if let Some((src, flag)) = self.pending_marks.remove(&item.id) {
            self.save_pending_marks();
            if let Some(c) = self.compose.as_mut() {
                c.source = Some(src);
                c.mode = Some(if flag == Flag::Forwarded { ReplyMode::Forward } else { ReplyMode::Reply });
            }
        }
        store.outbox_remove(&account, item.id);
        self.reading = None;
        self.selection.clear();
        self.after_outbox_change();
        Ok(serde_json::json!({ "compose": id }))
    }

    // ----- accounts -----

    /// Add an account (the password goes to the keychain through the `Start` job).
    pub fn add_account(&mut self, mut config: AccountConfig, password: Option<String>) -> Result<String, CmdError> {
        config.email = config.email.trim().to_string();
        if !addr::is_valid_email(&config.email) {
            return Err(CmdError::Params(format!("`{}` is not an email address", config.email)));
        }
        if self.accounts.iter().any(|a| a.config.email.eq_ignore_ascii_case(&config.email)) {
            return Err(CmdError::Params(format!("{} is already added", config.email)));
        }
        if config.username.trim().is_empty() {
            config.username = config.email.clone();
        }
        config.id = self.new_account_id();
        config.color = u8::try_from(self.accounts.len() % usize::from(ACCOUNT_COLORS)).unwrap_or(0);
        let local = config.is_local();
        let incoming = config.pop.as_ref().unwrap_or(&config.imap);
        if !local
            && !config.is_api()
            && (incoming.host.trim().is_empty() || incoming.port == 0 || config.smtp.host.trim().is_empty() || config.smtp.port == 0)
        {
            return Err(CmdError::Params("incoming and outgoing server names and ports are required".into()));
        }
        let id = config.id.clone();
        if config.online_only && !config.can_be_online_only() {
            return Err(CmdError::Params("a POP account keeps its mail on this computer: it can't be online only".into()));
        }
        if let Some(store) = &self.store
            && config.keeps_online()
        {
            store.set_online_only(&id, true).map_err(|e| CmdError::Failed(e.to_string()))?;
        }
        self.accounts.push(AccountState {
            config: config.clone(),
            folders: Vec::new(),
            status: SyncStatus::Connecting,
            quota: None,
            vacation: VacationState::Unknown,
        });
        self.save_accounts();
        if config.oauth.is_some() {
            self.jobs.push(Job::SignIn { account: config });
        } else if !local {
            self.jobs.push(Job::Start { account: config, password: password.map(Secret) });
        }
        if self.accounts.len() == 1 || self.view == View::Empty {
            self.view = View::Folder { account: id.clone(), folder: "INBOX".into() };
            self.refresh_list();
        }
        Ok(id)
    }

    fn new_account_id(&self) -> String {
        let mut n = self.accounts.len() + 1;
        while self.accounts.iter().any(|a| a.config.id == format!("a{n}")) {
            n += 1;
        }
        format!("a{n}")
    }

    /// Open an Outlook data file (`.pst`): a new account on this computer, named after the file,
    /// that the import fills.
    pub fn import_pst(&mut self, path: &str) -> CmdResult {
        let path = path.trim();
        if path.is_empty() {
            return Err(CmdError::Params("`path` is required".into()));
        }
        let file = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
        let stem = file.rsplit_once('.').map_or(file.as_str(), |(s, _)| s).trim().to_string();
        let id = self.new_account_id();
        let config = AccountConfig {
            id: id.clone(),
            name: if stem.is_empty() { file.clone() } else { stem },
            // Shown where an account's address would be; never an address, so never "me".
            email: file,
            color: u8::try_from(self.accounts.len() % usize::from(ACCOUNT_COLORS)).unwrap_or(0),
            ..Default::default()
        };
        self.accounts.push(AccountState {
            config,
            folders: Vec::new(),
            status: SyncStatus::Syncing { folder: String::new() },
            quota: None,
            vacation: VacationState::Unknown,
        });
        self.save_accounts();
        self.jobs.push(Job::ImportPst { account: id.clone(), path: path.to_string() });
        Ok(serde_json::json!({ "account": id, "importing": true }))
    }

    /// Write an account's mail to an Outlook data file at `path`.
    pub fn export_pst(&mut self, account: &str, path: &str) -> CmdResult {
        let path = path.trim();
        if path.is_empty() {
            return Err(CmdError::Params("`path` is required".into()));
        }
        let a = self.account(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        let name = if a.config.name.trim().is_empty() { a.config.email.clone() } else { a.config.name.clone() };
        self.jobs.push(Job::ExportPst { account: account.to_string(), path: path.to_string(), name });
        Ok(serde_json::json!({ "account": account, "exporting": true }))
    }

    fn exported(&mut self, account: &str, messages: usize, skipped: usize, done: bool, error: Option<String>) {
        let name = self.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
        match (done, error) {
            (false, _) => self.notice(format!("{name}: {messages} messages exported so far…"), false),
            (true, Some(e)) => self.notice(format!("{name} could not be exported: {e}"), true),
            (true, None) => {
                let more = if skipped > 0 { format!(" ({skipped} not on this computer were left out)") } else { String::new() };
                self.notice(format!("{name}: {messages} messages exported{more}"), false);
            }
        }
    }

    /// Bring an existing data file's mail into a new account before it first connects (Outlook's
    /// "use an existing data file" for POP accounts).
    pub fn attach_data_file(&mut self, account: &str, path: &str) -> CmdResult {
        let path = path.trim();
        if path.is_empty() {
            return Err(CmdError::Params("`path` is required".into()));
        }
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        // Connecting waits for the file, so the two don't write the Inbox at once.
        if let Some(i) = self.jobs.iter().position(|j| matches!(j, Job::Start { .. } | Job::SignIn { .. }) && j.account() == account) {
            let start = self.jobs.remove(i);
            self.held.insert(account.to_string(), start);
        }
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Syncing { folder: String::new() };
        }
        self.jobs.push(Job::ImportPst { account: account.to_string(), path: path.to_string() });
        Ok(serde_json::json!({ "account": account, "importing": true }))
    }

    fn imported(&mut self, account: &str, messages: usize, skipped: usize, done: bool, error: Option<String>) {
        let Some(name) = self.account(account).map(|a| a.config.email.clone()) else { return };
        if !done {
            self.notice(format!("{name}: {messages} messages imported so far…"), false);
            return;
        }
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Offline;
        }
        // An account that was waiting for its data file connects now.
        if let Some(start) = self.held.remove(account) {
            self.jobs.push(start);
            if let Some(e) = &error {
                self.notice(format!("{name}: the data file could not be opened: {e}"), true);
                return;
            }
        }
        match error {
            Some(e) => {
                // Nothing came in: the account goes again, and the file it was starting.
                let _ = self.drop_account(account, false);
                self.notice(format!("{name} could not be opened: {e}"), true);
            }
            None => {
                // The file's contacts and calendar came in too.
                if let Some(store) = self.store.clone()
                    && let Ok(mut p) = store.load_account_doc::<mailcraft_model::pim::Pim>(account, mailcraft_model::pim::PIM_DOC)
                {
                    p.ensure_defaults();
                    self.pim.insert(account.to_string(), p);
                    self.contacts = None;
                    self.pim_revision = self.pim_revision.wrapping_add(1);
                }
                let more = if skipped > 0 { format!(" ({skipped} could not be read)") } else { String::new() };
                self.notice(format!("{name}: {messages} messages imported{more}"), false);
                if let Some(inbox) = self.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()) {
                    self.open_view(View::Folder { account: account.to_string(), folder: inbox });
                }
            }
        }
    }

    /// Move an account up (`by` < 0) or down the folder pane, as Outlook moves a mailbox.
    pub fn move_account(&mut self, id: &str, by: i64) -> CmdResult {
        if self.account(id).is_none() {
            return Err(CmdError::Params(format!("no account `{id}`")));
        }
        let to = self.move_pane_section(id, by)?;
        Ok(serde_json::json!({ "account": id, "position": to }))
    }

    /// The folder pane's mailboxes in order (issue #1): each account, and each shared mailbox an
    /// account opens as a mailbox of its own (`<account>|<address>`), placed where the user moved
    /// them; new ones after the mailbox they came with.
    pub fn pane_sections(&self) -> Vec<PaneSection> {
        let mut natural = Vec::new();
        for a in &self.accounts {
            natural.push(PaneSection { key: a.config.id.clone(), account: a.config.id.clone(), mailbox: None });
            for top in a.folders.iter().filter(|f| f.depth() == 0) {
                if let Some(address) = &top.mailbox {
                    let key = format!("{}|{}", a.config.id, address.to_ascii_lowercase());
                    if !natural.iter().any(|s: &PaneSection| s.key == key) {
                        natural.push(PaneSection { key, account: a.config.id.clone(), mailbox: Some(top.path.clone()) });
                    }
                }
            }
        }
        let mut out: Vec<PaneSection> = self.prefs.pane_order.iter().filter_map(|k| natural.iter().find(|s| &s.key == k).cloned()).collect();
        for (i, s) in natural.iter().enumerate() {
            if out.iter().any(|o| o.key == s.key) {
                continue;
            }
            let after = natural.get(..i).unwrap_or_default().iter().rev().find_map(|p| out.iter().position(|o| o.key == p.key));
            out.insert(after.map_or(0, |at| at + 1), s.clone());
        }
        out
    }

    /// Move a mailbox in the folder pane `by` places (an account's own order follows, as the
    /// accounts' order is theirs everywhere else). Returns where it is now.
    pub fn move_pane_section(&mut self, key: &str, by: i64) -> Result<usize, CmdError> {
        let mut order: Vec<String> = self.pane_sections().into_iter().map(|s| s.key).collect();
        let from = order.iter().position(|k| k == key).ok_or_else(|| CmdError::Params(format!("no mailbox `{key}` in the folder pane")))?;
        let last = order.len().saturating_sub(1);
        let to = if by < 0 { from.saturating_sub(by.unsigned_abs() as usize) } else { from.saturating_add(by as usize).min(last) };
        if to != from {
            let k = order.remove(from);
            order.insert(to, k);
            let ids: Vec<String> = order.iter().filter(|k| !k.contains('|')).cloned().collect();
            let mut accounts = std::mem::take(&mut self.accounts);
            accounts.sort_by_key(|a| ids.iter().position(|id| *id == a.config.id).unwrap_or(usize::MAX));
            self.accounts = accounts;
            self.prefs.pane_order = order;
            self.save_accounts();
            self.revision = self.revision.wrapping_add(1);
        }
        Ok(to)
    }

    /// Remove an account as Outlook does: gone from here, its data file kept (closed) where it
    /// is, so it can be opened again from File › Open & Export; nothing changes on the server.
    pub fn remove_account(&mut self, id: &str) -> CmdResult {
        self.drop_account(id, true)
    }

    /// An account gone; `keep` keeps its data file (an account the user removes), otherwise the
    /// file goes too (one that never held anyone's mail, or an account starting again).
    fn drop_account(&mut self, id: &str, keep: bool) -> CmdResult {
        let Some(email) = self.account(id).map(|a| a.config.email.clone()) else {
            return Err(CmdError::Params(format!("no account `{id}`")));
        };
        self.accounts.retain(|a| a.config.id != id);
        self.cache.retain(|(a, _), _| a != id);
        self.pim.remove(id);
        let mut kept = None;
        if let Some(store) = &self.store {
            let gone = if keep { store.remove_account(id).map(|f| kept = f) } else { store.delete_account(id) };
            if let Err(e) = gone {
                self.notice(e.to_string(), true);
            }
        }
        if let Some(f) = &kept {
            self.notice(format!("{email} removed. Its data file stays at {}", f.display()), false);
        }
        self.save_accounts();
        self.jobs.push(Job::Remove { account: id.to_string() });
        if self.compose.as_ref().is_some_and(|c| c.account == id) {
            self.compose = None;
        }
        let view = self.default_view();
        self.open_view(view);
        Ok(serde_json::json!({ "removed": id, "dataFile": kept.map(|f| f.display().to_string()) }))
    }

    /// An account that signs in to Microsoft with a password (which Microsoft refuses) switched to
    /// Sign in with Microsoft, keeping its mail: the browser opens, and the token replaces the
    /// password in the keychain.
    pub fn use_microsoft_sign_in(&mut self, id: &str) -> CmdResult {
        let a = self.account_mut(id).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
        if !mailcraft_model::microsoft_password_account(&a.config) {
            return Err(CmdError::Params("this account doesn't sign in to Microsoft with a password".into()));
        }
        a.config.oauth = Some("microsoft".into());
        // Outlook.com's own servers, where the address is one of Microsoft's.
        if a.config.imap.host.trim().is_empty() || mailcraft_model::microsoft_consumer(&a.config.email) && a.config.pop.is_none() {
            let guess = AccountConfig::guess(&a.config.email);
            a.config.imap = guess.imap;
            a.config.smtp = guess.smtp;
        }
        a.status = SyncStatus::Connecting;
        let config = a.config.clone();
        self.save_accounts();
        self.jobs.push(Job::SignIn { account: config });
        Ok(serde_json::json!({ "account": id, "signingIn": true }))
    }

    /// A Microsoft account that signs in with Microsoft moved from IMAP and SMTP to Microsoft
    /// Graph (for a mailbox Microsoft won't open over IMAP). Graph names folders its own way, so
    /// it starts again as a new account in the same place: its mail comes down from the server,
    /// and what was only on this computer for it (unsent mail in the Outbox) goes.
    pub fn use_graph(&mut self, id: &str) -> CmdResult {
        let a = self.account(id).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
        let c = a.config.clone();
        let microsoft = c.oauth.as_deref() == Some("microsoft") || mailcraft_model::microsoft_password_account(&c);
        if c.graph.is_some() || c.jmap.is_some() || c.ews.is_some() || c.pop.is_some() || !microsoft {
            return Err(CmdError::Params("only a Microsoft account connected over IMAP can move to Microsoft Graph".into()));
        }
        if !self.oauth_available.iter().any(|p| p == "microsoft") {
            return Err(CmdError::Params("signing in with Microsoft isn't set up in this copy of MailCraft".into()));
        }
        let at = self.accounts.iter().position(|x| x.config.id == id).unwrap_or(0);
        // Starting again from the server: its old file isn't kept beside the new one.
        self.drop_account(id, false)?;
        let config = AccountConfig {
            name: c.name,
            email: c.email,
            username: String::new(),
            signature: c.signature,
            oauth: Some("microsoft".into()),
            graph: Some(mailcraft_model::GRAPH_URL.to_string()),
            ..Default::default()
        };
        let new = self.add_account(config, None)?;
        if let Some(from) = self.accounts.iter().position(|x| x.config.id == new) {
            let mut moved = self.accounts.remove(from);
            moved.config.color = c.color;
            self.accounts.insert(at.min(self.accounts.len()), moved);
            self.save_accounts();
        }
        self.open_view(View::Folder { account: new.clone(), folder: "INBOX".into() });
        Ok(serde_json::json!({ "account": new, "graph": true }))
    }

    /// What closing MailCraft would lose (issue #4): online-only accounts' mail still waiting to
    /// be sent, and their changes not yet on the server, held in memory only.
    pub fn lost_on_close(&self) -> (usize, usize) {
        let online: Vec<&str> = self.accounts.iter().filter(|a| a.config.keeps_online()).map(|a| a.config.id.as_str()).collect();
        if online.is_empty() {
            return (0, 0);
        }
        let unsent = self.outbox.iter().filter(|(a, _)| online.contains(&a.as_str())).count();
        let changes = online.iter().map(|a| self.store.as_ref().and_then(|s| s.load_ops(a).ok()).map_or(0, |o| o.len())).sum();
        (unsent, changes)
    }

    /// How many more of the folder on view its server has than are kept here (issue #4), with
    /// the folder: shown at the end of its list, to show more.
    pub fn more_on_server(&self) -> Option<(String, String, u32)> {
        let View::Folder { account, folder } = &self.view else { return None };
        if self.search.is_some() {
            return None;
        }
        let a = self.account(account)?;
        if a.is_local() || a.config.pop.is_some() {
            return None;
        }
        let total = a.folders.iter().find(|f| &f.path == folder)?.total;
        // The folder on view is loaded (its list is on screen).
        let kept = self.cache.get(&(account.clone(), folder.clone())).map_or(0, |st| st.messages.len());
        let kept = u32::try_from(kept).unwrap_or(u32::MAX);
        (total > kept).then(|| (account.clone(), folder.clone(), total - kept))
    }

    /// Keep `more` older messages of a folder than it keeps now, fetched from its server.
    pub fn show_more(&mut self, account: &str, folder: &str, more: u32) -> CmdResult {
        let a = self.account(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if a.is_local() || a.config.pop.is_some() {
            return Err(CmdError::Disabled("this account keeps all its mail here".into()));
        }
        self.jobs.push(Job::MoreMail { account: account.to_string(), folder: folder.to_string(), more });
        self.showing_more = Some((account.to_string(), folder.to_string()));
        Ok(serde_json::json!({ "account": account, "folder": folder, "more": more }))
    }

    /// Ask the servers for the search box's query (issue #4: mail past what's kept here): the
    /// folder on view, or every folder of every account with a server.
    pub fn search_server(&mut self) -> CmdResult {
        let (q, scope) = self.search.clone().ok_or_else(|| CmdError::Disabled("search for something first".into()))?;
        let mut by_account: Vec<(String, Vec<String>)> = Vec::new();
        let folders: Vec<(String, String)> = match scope {
            SearchScope::Current => self.view_folders(),
            SearchScope::All => self.accounts.iter().flat_map(|a| a.folders.iter().map(|f| (a.config.id.clone(), f.path.clone()))).collect(),
        };
        for (a, f) in folders {
            if self.account(&a).is_none_or(|x| x.is_local() || x.config.pop.is_some()) {
                continue;
            }
            match by_account.iter_mut().find(|(x, _)| *x == a) {
                Some((_, v)) => v.push(f),
                None => by_account.push((a, vec![f])),
            }
        }
        if by_account.is_empty() {
            return Err(CmdError::Disabled("no account here has its mail on a server".into()));
        }
        self.server_search = Some(ServerSearch { query: q.clone(), pending: by_account.len(), ..Default::default() });
        for (account, folders) in by_account {
            self.jobs.push(Job::SearchServer { account, folders, query: q.clone() });
        }
        Ok(serde_json::json!({ "query": q, "searching": true }))
    }

    /// Ask for MailCraft's releases (File › About › Check for updates).
    pub(crate) fn check_updates(&mut self) {
        self.update = UpdateCheck::Checking;
        self.jobs.push(Job::CheckUpdates);
    }

    /// Keep an account's outgoing server's own password (its `smtp_username`'s), in the keychain.
    pub(crate) fn keep_smtp_password(&mut self, account: &str, password: String) {
        if let Some(a) = self.account(account) {
            self.jobs.push(Job::SmtpPassword { account: a.config.clone(), password: Secret(password) });
        }
    }

    pub fn update_account(&mut self, config: AccountConfig, password: Option<String>) -> CmdResult {
        let id = config.id.clone();
        let Some(a) = self.account_mut(&id) else { return Err(CmdError::Params(format!("no account `{id}`"))) };
        let reconnect = a.config.imap != config.imap
            || a.config.smtp != config.smtp
            || a.config.jmap != config.jmap
            || a.config.graph != config.graph
            || a.config.ews != config.ews
            || a.config.pop != config.pop
            || a.config.pop_delete != config.pop_delete
            || a.config.pop_keep_days != config.pop_keep_days
            || a.config.pop_delete_with_trash != config.pop_delete_with_trash
            || a.config.username != config.username
            || a.config.smtp_username != config.smtp_username
            || a.config.shared_mailboxes != config.shared_mailboxes
            || a.config.online_only != config.online_only
            || password.is_some();
        if config.online_only && !config.can_be_online_only() {
            return Err(CmdError::Params("a POP account keeps its mail on this computer: it can't be online only".into()));
        }
        // Online only, or kept here again: what's kept moves between memory and the disk first.
        if a.config.online_only != config.online_only
            && let Some(store) = self.store.clone()
        {
            store.set_online_only(&id, config.keeps_online()).map_err(|e| CmdError::Failed(e.to_string()))?;
        }
        let Some(a) = self.account_mut(&id) else { return Err(CmdError::Params(format!("no account `{id}`"))) };
        a.config = config.clone();
        let local = a.is_local();
        self.save_accounts();
        if reconnect && !local {
            self.jobs.push(Job::Start { account: config, password: password.map(Secret) });
        }
        Ok(serde_json::json!({ "updated": id }))
    }

    fn save_accounts(&mut self) {
        let configs: Vec<AccountConfig> = self.accounts.iter().map(|a| a.config.clone()).collect();
        if let Some(store) = &self.store
            && let Err(e) = store.save_accounts(&configs)
        {
            self.notice(format!("account settings could not be saved: {e}"), true);
        }
    }

    /// Ask every account to sync now.
    /// Work offline, or connect again: every account's connection closes (changes, sent mail and
    /// drafts wait on this computer) or opens. Ahead of anything queued, so what follows sees it.
    #[cfg(test)]
    pub(crate) fn jobs_push_for_test(&mut self, j: Job) {
        self.jobs.push(j);
    }

    /// Tell the network side how much mail to keep (ahead of anything queued, so accounts
    /// start with it).
    pub fn apply_sync_prefs(&mut self) {
        self.jobs.retain(|j| !matches!(j, Job::SetSync { .. }));
        self.jobs.insert(0, Job::SetSync { messages: self.prefs.keep_messages, offline: self.prefs.keep_offline });
    }

    pub fn set_work_offline(&mut self, on: bool) {
        self.prefs.work_offline = on;
        self.jobs.retain(|j| !matches!(j, Job::WorkOffline { .. }));
        self.jobs.insert(0, Job::WorkOffline { on });
        if on {
            for a in self.accounts.iter_mut().filter(|a| !a.is_local()) {
                a.status = SyncStatus::Offline;
            }
        }
    }

    pub fn sync_all(&mut self) -> usize {
        let ids: Vec<String> = self.accounts.iter().filter(|a| !a.is_local()).map(|a| a.config.id.clone()).collect();
        for id in &ids {
            self.jobs.push(Job::SyncAll { account: id.clone() });
        }
        ids.len()
    }

    // ----- events -----

    /// Apply an event from the network layer.
    pub fn apply(&mut self, e: Event) {
        match e {
            Event::Status { account, status } => {
                // Show more ended (with nothing new, or failing): its footer can be asked again.
                if !matches!(status, SyncStatus::Syncing { .. } | SyncStatus::Connecting)
                    && self.showing_more.as_ref().is_some_and(|(a, _)| *a == account)
                {
                    self.showing_more = None;
                }
                // Refusals that need something done say so once, as they come.
                if let SyncStatus::Error { error } = &status
                    && (error.starts_with(mailcraft_model::MICROSOFT_IMAP_CLOSED) || error.starts_with(mailcraft_model::MICROSOFT_NO_PASSWORDS))
                    && self.account(&account).is_some_and(|a| a.status != status)
                {
                    let who = self.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
                    self.notice(format!("{who}: {error}"), true);
                }
                if let SyncStatus::NeedsPassword { error } = &status {
                    let (who, browser) = self.account(&account).map(|a| (a.config.email.clone(), a.config.oauth.is_some())).unwrap_or_default();
                    let why = if error.is_empty() { String::new() } else { format!(": {error}") };
                    // An account that signs in in the browser has no password to ask for.
                    let what = if browser { "needs you to sign in again" } else { "needs its password" };
                    self.notice(format!("{who} {what}{why}"), true);
                    if !browser && self.asked_password.insert(account.clone()) {
                        self.ask_password.push(account.clone());
                    }
                }
                if let Some(a) = self.account_mut(&account) {
                    a.status = status;
                }
            }
            Event::Folders { account, mut folders } => {
                // They're this account's, whatever name they were kept under.
                for f in &mut folders {
                    if f.account != account {
                        f.account.clone_from(&account);
                    }
                }
                sort_folders(&mut folders);
                let first = self.account(&account).is_some_and(|a| a.folders.is_empty());
                if let Some(a) = self.account_mut(&account) {
                    a.folders = folders;
                }
                // A view on a folder that no longer exists falls back to the Inbox.
                let missing = matches!(&self.view, View::Folder { account: va, folder } if *va == account && self.folder(va, folder).is_none());
                if missing {
                    let inbox =
                        self.account(&account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()).unwrap_or_else(|| "INBOX".into());
                    self.open_view(View::Folder { account: account.clone(), folder: inbox });
                } else if first {
                    self.refresh_list();
                }
                self.index = None;
            }
            Event::FolderChanged { account, folder } => {
                if self.showing_more.as_ref().is_some_and(|(a, f)| *a == account && *f == folder) {
                    self.showing_more = None;
                }
                self.reload_folder(&account, &folder);
                self.note_new_mail(&account, &folder);
                let visible = self.search.is_some() || self.view_folders().iter().any(|(a, f)| *a == account && *f == folder);
                if visible {
                    self.refresh_list();
                }
                if let Some(r) = &self.reading
                    && r.key.account == account
                    && r.key.folder == folder
                {
                    let key = r.key.clone();
                    let thread = self.thread_of(&key);
                    if let Some(r) = self.reading.as_mut() {
                        r.thread = thread;
                    }
                }
            }
            Event::ServerSearched { account, query, keys, error } => {
                if let Some(ss) = self.server_search.as_mut().filter(|ss| ss.query == query) {
                    ss.pending = ss.pending.saturating_sub(1);
                    ss.found.extend(keys);
                    if let Some(e) = error {
                        let who = self.accounts.iter().find(|a| a.config.id == account).map(|a| a.config.email.clone()).unwrap_or(account);
                        if let Some(ss) = self.server_search.as_mut() {
                            ss.errors.push(format!("{who}: {e}"));
                        }
                    }
                    self.refresh_list();
                }
            }
            Event::DavSynced { account, sync } => self.apply_dav(&account, &sync),
            Event::Imported { account, messages, skipped, done, error } => self.imported(&account, messages, skipped, done, error),
            Event::Exported { account, messages, skipped, done, error } => self.exported(&account, messages, skipped, done, error),
            Event::Quota { account, used, limit } => {
                if let Some(a) = self.account_mut(&account) {
                    a.quota = Some((used, limit));
                }
            }
            Event::Vacation { account, vacation, error } => {
                let mut failed = None;
                if let Some(a) = self.account_mut(&account) {
                    a.vacation = match (vacation, error) {
                        (Some(v), None) => VacationState::Loaded(v),
                        (Some(v), Some(e)) => {
                            // Read back after a change that failed: show what the server has.
                            failed = Some(e);
                            VacationState::Loaded(v)
                        }
                        (None, e) => VacationState::Failed(e.unwrap_or_else(|| "this server doesn't offer automatic replies".into())),
                    };
                }
                if let Some(e) = failed {
                    self.notice(format!("automatic replies weren't changed: {e}"), true);
                }
            }
            Event::Body { key, raw } => {
                self.show_body(&key, &raw);
                self.index_body(&key, &raw);
            }
            Event::BodyFailed { key, error } => {
                for w in self.windows.iter_mut().filter(|w| w.key == key && w.parsed.is_none()) {
                    w.error = Some(error.clone());
                }
                if let Some(r) = self.reading.as_mut()
                    && r.key == key
                {
                    r.error = Some(error);
                }
            }
            Event::Sent { id, .. } => {
                if let Some(c) = self.compose.clone().filter(|c| c.id == id) {
                    self.finish_sent(&c, true);
                } else {
                    self.notice("Message sent", false);
                }
                // Delivered from the Outbox: set the flag it owed.
                if let Some((key, flag)) = self.pending_marks.remove(&id) {
                    self.save_pending_marks();
                    self.apply_op(&key.account.clone(), Op::SetFlag { keys: vec![key], flag, on: true });
                }
                self.after_outbox_change();
            }
            Event::Queued { id, error, .. } => {
                if let Some(c) = self.compose.clone().filter(|c| c.id == id) {
                    // It's safe in the Outbox: as far as the user is concerned, the message is done
                    // (a Replied/Forwarded flag stays owed until delivery).
                    self.finish_sent(&c, false);
                    self.notices.pop();
                    self.notice("Saved in the Outbox: it will be sent when you're back online", false);
                } else {
                    log::info!("outbox {id}: still waiting ({error})");
                }
                self.after_outbox_change();
            }
            Event::SendFailed { id, error, .. } => {
                if let Some(c) = self.compose.as_mut().filter(|c| c.id == id) {
                    c.sending = false;
                    c.error = Some(error.clone());
                }
                self.notice(format!("Not sent: {error}"), true);
                self.after_outbox_change();
            }
            Event::OpFailed { error, .. } => self.notice(format!("The server refused a change: {error}"), true),
            Event::OpenUrl { url } => {
                if self.open_urls.len() < 8 {
                    self.open_urls.push(url);
                }
            }
            Event::SignedIn { account, user } => {
                // Who the browser signed in as, kept when it isn't the account's address: tried
                // when the mailbox won't open under that, and named when it still won't.
                let user = user.map(|u| u.trim().to_string()).filter(|u| !u.is_empty() && u.len() < 320);
                if let Some(a) = self.account_mut(&account) {
                    let other = user.filter(|u| !u.eq_ignore_ascii_case(&a.config.email));
                    let changed = a.config.oauth_user != other;
                    a.config.oauth_user = other.clone();
                    let config = a.config.clone();
                    let who = config.email.clone();
                    if changed {
                        self.save_accounts();
                    }
                    self.jobs.push(Job::Start { account: config, password: None });
                    match other {
                        Some(o) => self.notice(format!("{who} is signed in (as {o})"), false),
                        None => self.notice(format!("{who} is signed in"), false),
                    }
                }
            }
            Event::Discovered { email, config, source } => {
                self.discovered.insert(email.trim().to_ascii_lowercase(), Some((config.map(|c| *c), source)));
            }
            Event::Releases { releases, error } => {
                self.update = match error {
                    Some(e) => UpdateCheck::Failed(e),
                    None => match newer_release(env!("CARGO_PKG_VERSION"), self.build_sha.as_deref(), &releases) {
                        Some(r) => UpdateCheck::Available(r.clone()),
                        None => UpdateCheck::UpToDate(releases.first().map(|r| r.tag.clone()).unwrap_or_default()),
                    },
                };
            }
            Event::Fetched { url, data, error } => {
                let p = match (data, error) {
                    (Some(d), _) => Picture::Ready(Arc::new(d)),
                    (None, e) => Picture::Failed(e.unwrap_or_default()),
                };
                self.pictures.insert(url, p);
            }
            Event::Notice { account, message } => {
                let who = self.account(&account).map(|a| a.config.email.clone()).unwrap_or(account);
                self.notice(format!("{who}: {message}"), true);
            }
        }
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

/// What a follow-up date is kept under: the Message-ID, or (without one) the message's place.
fn follow_up_id(m: &Summary) -> String {
    if m.message_id.trim().is_empty() { m.key.to_id() } else { m.message_id.trim().to_string() }
}

/// A signature as it goes into a message: after the usual `-- ` line (RFC 3676's delimiter).
pub fn signature_block(sig: &str) -> String {
    format!("-- \n{}", sig.trim_end())
}

/// Give the signature in a message (`c.signature`, where it still stands as put there) the look
/// it has: `sig` is its text and styles.
pub fn style_signature(c: &mut Compose, sig: &(String, mime::rich::Styles)) {
    let (text, styles) = sig;
    if c.signature.is_empty() || styles.is_plain() {
        return;
    }
    let mut block = "-- \n".to_string();
    let mut block_styles = mime::rich::Styles::plain(&block);
    mime::rich::append(&mut block, &mut block_styles, text.trim_end(), styles);
    if block != c.signature {
        return;
    }
    let Some(b) = c.body.rfind(&c.signature) else { return };
    let start = c.body.get(..b).unwrap_or("").chars().count();
    let n = block.chars().count();
    c.styles.fit(&c.body);
    mime::rich::splice(&mut c.body, &mut c.styles, start..start + n, &block, &block_styles);
}
