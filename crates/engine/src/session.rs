//! The session: accounts, the current view and its message list, the open message, the message
//! being written, preferences, and the queue of jobs for the network layer.

use std::collections::{BTreeMap, BTreeSet, HashMap};
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
mod organize;
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

/// Settings the user changes from the View tab (saved by the app).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub reading_pane: ReadingPane,
    /// Group the list by conversation.
    pub conversations: bool,
    pub mark_read_on_open: bool,
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
    /// When flagged messages are due (Follow Up › Today, Tomorrow…): Message-ID → local day.
    /// IMAP has nowhere for it, so it's kept here, as Outlook keeps it for IMAP accounts.
    #[serde(default)]
    pub follow_up: BTreeMap<String, i64>,
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
            new_mail_window: true,
            arrange: Arrange::Date,
            reverse_sort: false,
            work_offline: false,
            ignored: BTreeSet::new(),
            follow_up: BTreeMap::new(),
            folder_pane_minimized: false,
            collapsed: BTreeSet::new(),
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
    bodies: HashMap<MessageKey, String>,
    scope: SearchScope,
    view: View,
}

pub struct Session {
    pub store: Option<Store>,
    pub accounts: Vec<AccountState>,
    pub view: View,
    pub filter: ListFilter,
    /// The active search (query text and scope).
    pub search: Option<(String, SearchScope)>,
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
    cache: HashMap<(String, String), FolderState>,
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
    registry: Arc<Registry>,
    next_id: u64,
    seed: u64,
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
            jmap_only: false,
            print_fonts: None,
            open_urls: Vec::new(),
            pim: HashMap::new(),
            module: pim::Module::Mail,
            cal_mode: pim::CalMode::Week,
            cal_day: None,
            index: None,
            registry: Arc::new(crate::cmd::registry()),
            // Compose ids also name Outbox entries, which outlive a session: start from the clock.
            next_id: u64::try_from(clock.now.max(0)).unwrap_or(0).saturating_mul(1000),
            seed: (clock.now as u64) ^ 0x9E37_79B9_7F4A_7C15,
        };
        if let Some(store) = &s.store {
            s.rules = store.load_doc("rules.json").unwrap_or_default();
            s.snoozes = store.load_doc("snoozes.json").unwrap_or_default();
            let accounts = store.load_accounts().unwrap_or_else(|e| {
                log::warn!("accounts: {e}");
                Vec::new()
            });
            for config in accounts {
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
        for id in local {
            s.recount_local(&id);
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
        }
        static EMPTY: std::sync::OnceLock<FolderState> = std::sync::OnceLock::new();
        self.cache.get(&k).unwrap_or_else(|| EMPTY.get_or_init(FolderState::default))
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
        if self.folder(account, folder).is_none_or(|f| f.role != FolderRole::Inbox) {
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
        // Replies in ignored conversations go to Deleted Items, then rules; what they file away
        // or mark read isn't announced.
        let mut quiet = self.ignore_arrived(&arrived);
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
        let signature = self.account(&c.account).map(|a| a.config.signature.trim().to_string()).unwrap_or_default();
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
        if source == "people" {
            let mut v: Vec<EmailAddress> = self
                .people(query)
                .into_iter()
                .flat_map(|p| p.card.emails.iter().map(|e| EmailAddress::new(p.card.display(), e.value.clone())).collect::<Vec<_>>())
                .collect();
            v.truncate(MAX);
            return v;
        }
        self.ensure_contacts();
        self.contacts.as_ref().map(|b| b.list(query, MAX).into_iter().map(|c| c.address).collect()).unwrap_or_default()
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
            self.search_results(&q, scope)
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
        self.list.iter().filter(|m| self.selection.contains(&m.key)).cloned().collect()
    }

    /// The conversation a message belongs to in the current list.
    fn thread_of(&self, key: &MessageKey) -> Vec<MessageKey> {
        let Some(i) = self.list.iter().position(|m| &m.key == key) else { return Vec::new() };
        let ts;
        let threads = if self.prefs.conversations {
            &self.threads
        } else {
            ts = threads(&self.list);
            &ts
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
        self.refresh_list();
    }

    /// Select one message and show it in the reading pane.
    pub fn open_message(&mut self, key: &MessageKey) {
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
        if self.prefs.mark_read_on_open && key.folder != OUTBOX && self.summary(key).is_some_and(|m| !m.flags.seen) {
            self.apply_op(&key.account.clone(), Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true });
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
            if self.prefs.mark_read_on_open && key.folder != OUTBOX && self.summary(key).is_some_and(|m| !m.flags.seen) {
                self.apply_op(&key.account.clone(), Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true });
            }
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
        let parsed = mime::parse(raw);
        if let Some(r) = self.reading.as_mut()
            && &r.key == key
        {
            r.error = if parsed.is_none() { Some("this message could not be read".into()) } else { None };
            r.parsed = parsed;
        }
        self.request_pictures();
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
        self.pictures_for.contains(&r.key) || (!sender.is_empty() && self.prefs.trusted_senders.contains(&sender))
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
            let mine: Vec<u32> = op.keys().iter().filter(|k| &k.folder == f && k.validity == st.validity).map(|k| k.uid).collect();
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
        if let Op::Move { keys, to } = op {
            let mut dest = store.load_folder(account, to).unwrap_or_default();
            if dest.validity == 0 {
                dest.validity = 1;
            }
            for k in keys {
                let Ok(src) = store.load_folder(account, &k.folder) else { continue };
                if k.folder == *to {
                    continue;
                }
                let Some(m) = src.find(k.uid) else { continue };
                let uid = dest.uid_next.max(dest.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1);
                dest.uid_next = uid + 1;
                let mut moved = m.clone();
                moved.key = MessageKey { account: account.to_string(), folder: to.clone(), validity: dest.validity, uid };
                if let Some(raw) = store.read_body(k) {
                    let _ = store.write_body(&moved.key, &raw);
                }
                dest.messages.push(moved);
            }
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
        self.recount_local(account);
    }

    /// A folder-list change, in memory: the account's folders, the open view, cached folder
    /// contents and Favorites follow a renamed folder; a deleted one disappears.
    fn apply_folder_op_memory(&mut self, account: &str, op: &Op) {
        let delimiter = self.delimiter(account);
        if let Some(a) = self.account_mut(account) {
            match op {
                Op::CreateFolder { path } => {
                    if !a.folders.iter().any(|f| &f.path == path) {
                        a.folders.push(Folder {
                            account: account.to_string(),
                            path: path.clone(),
                            delimiter,
                            role: FolderRole::Other,
                            selectable: true,
                            unread: 0,
                            total: 0,
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
        if f.role != FolderRole::Other || f.path.eq_ignore_ascii_case("INBOX") {
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

    fn recount_local(&mut self, account: &str) {
        let paths: Vec<String> = self.account(account).map(|a| a.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default();
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
            let mark = self.prefs.mark_read_on_open;
            self.prefs.mark_read_on_open = false;
            self.open_message(&k);
            self.prefs.mark_read_on_open = mark;
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
        let mut bodies = HashMap::new();
        for (a, f) in folders {
            let msgs = self.folder_state(&a, &f).messages.clone();
            for m in msgs {
                let doc = u32::try_from(keys.len()).unwrap_or(u32::MAX);
                index.add(doc, &summary_text(&m));
                if let Some(raw) = self.store.as_ref().and_then(|s| s.read_body(&m.key))
                    && let Some(p) = mime::parse(&raw)
                {
                    let text: String = p.text.chars().take(50_000).collect();
                    index.add(doc, &text);
                    for att in &p.attachments {
                        index.add(doc, &att.name);
                    }
                    bodies.insert(m.key.clone(), text);
                }
                keys.push(m.key);
            }
        }
        self.index = Some(SearchIndex { index, keys, bodies, scope, view: self.view.clone() });
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
        for doc in ix.index.candidates(&query) {
            let Some(key) = ix.keys.get(doc as usize) else { continue };
            let Some(m) = self.folder_state(&key.account, &key.folder).find(key.uid).cloned() else { continue };
            if query.matches(&Doc { summary: &m, body: ix.bodies.get(key).map(String::as_str) }) {
                out.push(m);
            }
        }
        self.index = Some(ix);
        out
    }

    pub fn run_search(&mut self, query: &str, scope: SearchScope) -> usize {
        let q = query.trim();
        self.search = if q.is_empty() { None } else { Some((q.to_string(), scope)) };
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
        self.account(account).map(|a| a.config.signature.clone()).unwrap_or_default()
    }

    pub fn new_compose(&mut self, account: Option<&str>) -> Result<&mut Compose, CmdError> {
        let account = match account {
            Some(a) => self.account(a).map(|a| a.config.id.clone()),
            None => self.current_account().map(|a| a.config.id.clone()),
        }
        .ok_or_else(|| CmdError::Disabled("add an account first".into()))?;
        let sig = self.signature(&account);
        let id = self.fresh_id();
        let body = if sig.trim().is_empty() { String::new() } else { format!("\n\n-- \n{}", sig.trim_end()) };
        let window = self.prefs.new_mail_window;
        self.compose = Some(Compose { id, account, body, window, ..Default::default() });
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
            to: addr::format_list(&d.to),
            cc: addr::format_list(&d.cc),
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
            c.set_text(if sig.trim().is_empty() { String::new() } else { format!("\n\n-- \n{}", sig.trim_end()) });
            c.attachments = vec![OutAttachment { name, mime: "message/rfc822".into(), data: raw }];
        }
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
            self.finish_sent(&c);
            // finish_sent says "sent"; it isn't yet.
            self.notices.pop();
            self.notice(format!("It will be sent {}", mailcraft_model::dates::full_time(at, &self.clock)), false);
            return Ok(serde_json::json!({ "queued": c.id, "sendAfter": at }));
        }
        if local {
            if let Some(folder) = &sent_folder {
                self.file_locally(&c.account, folder, &raw, true);
            }
            self.finish_sent(&c);
            return Ok(serde_json::json!({ "sent": c.id, "local": true }));
        }
        self.jobs.push(Job::Send { account: c.account.clone(), id: c.id, raw, from, recipients: d.recipients(), sent_folder });
        if let Some(cm) = self.compose.as_mut() {
            cm.sending = true;
            cm.error = None;
        }
        Ok(serde_json::json!({ "sending": c.id }))
    }

    fn finish_sent(&mut self, c: &Compose) {
        if let (Some(src), Some(mode)) = (&c.source, c.mode) {
            let flag = if mode == ReplyMode::Forward { Flag::Forwarded } else { Flag::Answered };
            self.apply_op(&src.account.clone(), Op::SetFlag { keys: vec![src.clone()], flag, on: true });
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
        self.recount_local(account);
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
            to: addr::format_list(&p.to),
            cc: addr::format_list(&p.cc),
            bcc: bcc.join(", "),
            show_cc: !bcc.is_empty(),
            subject: p.subject.clone(),
            body: p.text.clone(),
            in_reply_to: p.in_reply_to.clone(),
            references: p.references.clone(),
            attachments,
            ..Default::default()
        });
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
                // Nothing came in: the account goes again.
                let _ = self.remove_account(account);
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
        let from = self.accounts.iter().position(|a| a.config.id == id).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
        let last = self.accounts.len().saturating_sub(1);
        let to = if by < 0 { from.saturating_sub(by.unsigned_abs() as usize) } else { from.saturating_add(by as usize).min(last) };
        if to != from {
            let a = self.accounts.remove(from);
            self.accounts.insert(to, a);
            self.save_accounts();
            self.revision = self.revision.wrapping_add(1);
        }
        Ok(serde_json::json!({ "account": id, "position": to }))
    }

    pub fn remove_account(&mut self, id: &str) -> CmdResult {
        if self.account(id).is_none() {
            return Err(CmdError::Params(format!("no account `{id}`")));
        }
        self.accounts.retain(|a| a.config.id != id);
        self.cache.retain(|(a, _), _| a != id);
        self.pim.remove(id);
        if let Some(store) = &self.store
            && let Err(e) = store.remove_account(id)
        {
            self.notice(e.to_string(), true);
        }
        self.save_accounts();
        self.jobs.push(Job::Remove { account: id.to_string() });
        if self.compose.as_ref().is_some_and(|c| c.account == id) {
            self.compose = None;
        }
        let view = self.default_view();
        self.open_view(view);
        Ok(serde_json::json!({ "removed": id }))
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
            || password.is_some();
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
                if let SyncStatus::NeedsPassword { error } = &status {
                    let (who, browser) = self.account(&account).map(|a| (a.config.email.clone(), a.config.oauth.is_some())).unwrap_or_default();
                    let why = if error.is_empty() { String::new() } else { format!(": {error}") };
                    // An account that signs in in the browser has no password to ask for.
                    let what = if browser { "needs you to sign in again" } else { "needs its password" };
                    self.notice(format!("{who} {what}{why}"), true);
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
                self.index = None;
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
                    self.finish_sent(&c);
                } else {
                    self.notice("Message sent", false);
                }
                self.after_outbox_change();
            }
            Event::Queued { id, error, .. } => {
                if let Some(c) = self.compose.clone().filter(|c| c.id == id) {
                    // It's safe in the Outbox: as far as the user is concerned, the message is done.
                    self.finish_sent(&c);
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
            Event::SignedIn { account } => {
                if let Some(a) = self.account(&account) {
                    let config = a.config.clone();
                    let who = config.email.clone();
                    self.jobs.push(Job::Start { account: config, password: None });
                    self.notice(format!("{who} is signed in"), false);
                }
            }
            Event::Discovered { email, config, source } => {
                self.discovered.insert(email.trim().to_ascii_lowercase(), Some((config.map(|c| *c), source)));
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
