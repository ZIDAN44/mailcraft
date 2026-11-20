//! The settings the user changes (View tab, File › Options), saved by the app.

use super::invite::AnsweredInvites;
use super::*;

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

/// What Ctrl+Enter does in a message being written (issue #115): it sent at once, and a slip of
/// the keys sent a message unfinished. It asks first, as Outlook does the first time, until the
/// answer is kept ("Don't ask again", or File › Options › Mail); Alt+S and Send are as ever.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CtrlEnter {
    #[default]
    Ask,
    Send,
    Nothing,
}

/// What opens at the start (issue #172): the first account's Inbox (Outlook's first mailbox), All
/// Inboxes, the folder open when MailCraft last closed, or a folder chosen.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "open")]
pub enum StartIn {
    #[default]
    FirstInbox,
    AllInboxes,
    Last,
    #[serde(rename_all = "camelCase")]
    Folder {
        account: String,
        folder: String,
    },
}

/// How tightly the folder pane's rows sit (View › Folder density, issue #184): each step a
/// smaller text, a shorter row and less indent a level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderDensity {
    /// As it always was: room to read.
    #[default]
    Spacious,
    Medium,
    /// About as many folders as Outlook for Mac shows in the same height.
    Compact,
}

impl FolderDensity {
    pub const ALL: [FolderDensity; 3] = [FolderDensity::Spacious, FolderDensity::Medium, FolderDensity::Compact];

    /// As `view.folderDensity` takes it.
    pub fn code(self) -> &'static str {
        match self {
            FolderDensity::Spacious => "spacious",
            FolderDensity::Medium => "medium",
            FolderDensity::Compact => "compact",
        }
    }

    pub fn from_code(code: &str) -> Option<FolderDensity> {
        FolderDensity::ALL.into_iter().find(|d| d.code() == code)
    }
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
    /// How tasks are listed, and whether the list leaves out those done.
    pub tasks_order: pim::TasksOrder,
    pub tasks_hide_done: bool,
    /// Calendars left out of the calendar (`account/calendar id`), as Outlook's unticked ones.
    pub hidden_calendars: BTreeSet<String>,
    /// Day and week show each calendar in a column of its own (Outlook's side-by-side view)
    /// rather than all of them together.
    pub calendars_side_by_side: bool,
    /// Months shown one under another in the calendar's side pane (1 to 6).
    pub calendar_months: u8,
    /// The days and hours worked (issue #185): the work week shows those days, and the day and
    /// week views shade the hours outside.
    pub work_time: mailcraft_model::pim::work_hours::WorkPeriod,
    /// How long a time free for everyone is looked for (issue #186), in minutes.
    pub free_time_minutes: u16,
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
    /// How tightly the folder pane's rows sit (issue #184).
    pub folder_density: FolderDensity,
    /// Folders in an order of the user's own, by account: their paths in that order (issue
    /// #183). An account without one has them A to Z.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub folder_order: BTreeMap<String, Vec<String>>,
    /// The message list's headings folded away (issue #189), by the folder they're in: each
    /// folder's, by arrangement and heading.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub folded_groups: BTreeMap<String, BTreeSet<String>>,
    /// The list under headings, as it's arranged (Today, Yesterday…; a sender's name): off, its
    /// messages follow one another with nothing between.
    pub groups: bool,
    /// A message on one line, as a table's row: the sender, the subject, the time.
    pub single_line: bool,
    /// The classic ribbon (issue #8): big icons over their labels, in named groups, instead of
    /// one row of small ones.
    pub classic_ribbon: bool,
    /// The ribbon's buttons hidden, its tabs kept on the top bar: one clicked shows its buttons
    /// until a click elsewhere.
    pub ribbon_hidden: bool,
    /// Send / Receive asked for (F9) opens its progress window. Off unless asked for, as
    /// Outlook's: the status bar says how it goes, and opens the window when clicked.
    pub open_send_receive: bool,
    /// The accent colour (issue #8): empty for MailCraft's teal, a named one ([`ACCENTS`]), or
    /// any colour as `#rrggbb`. The window adjusts it to read well in light and dark.
    pub accent: String,
    /// The top bar in plain pane colours instead of the accent colour (issue #8, as Office's
    /// white theme).
    pub plain_top_bar: bool,
    /// In the dark theme, messages are still shown on a light background, with the colours they
    /// were written in (issue #109, as Outlook can keep them).
    pub light_messages: bool,
    /// Windows: the system's title bar above MailCraft's top bar, instead of the top bar being the
    /// title bar (issue #22). Read as MailCraft starts.
    pub system_title_bar: bool,
    /// How dates and times are written: the order of a date's parts, the clock, and whether the
    /// list gives every message its whole date and time (issue #46).
    pub date_style: mailcraft_model::dates::DateStyle,
    /// Ask before deleting permanently.
    pub confirm_permanent_delete: bool,
    /// Desktop notifications for new mail in an Inbox.
    pub notify: bool,
    /// With the Focused Inbox on, new mail in Other is announced too (issue #199): off, only
    /// Focused mail is, as Outlook does.
    pub notify_other: bool,
    /// Check spelling as you type (issue #174): misspelled words underlined in the message
    /// editor, as Outlook does (on unless turned off).
    pub spelling: bool,
    /// The words added to the dictionary (Add to dictionary), taken as spelled right in every
    /// language.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty", deserialize_with = "checked_words")]
    pub dictionary: BTreeSet<String>,
    /// The typeface new messages, replies and forwards are written in (issue #166), as Outlook's
    /// "New mail messages" font; `None`: MailCraft's own.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "checked_font")]
    pub compose_font: Option<String>,
    /// Closing the window leaves MailCraft running in the notification area, where the app
    /// has one (issue #67): mail is still checked and announced.
    pub close_to_tray: bool,
    /// With `close_to_tray`, MailCraft starts with its window put away as a close puts it
    /// (issue #116): at sign-in, it checks for mail and announces it with no window coming up.
    /// Read as MailCraft starts.
    pub start_in_tray: bool,
    /// The notification area's icon shows the Inboxes' unread mail (issue #117).
    pub tray_unread: bool,
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
    /// Replies and forwards open in a window of their own too, not in the reading pane (issue
    /// #11: Pop out each time).
    pub reply_window: bool,
    /// What Ctrl+Enter does in a message being written.
    pub ctrl_enter: CtrlEnter,
    /// Seconds a message sent waits in the Outbox, to be undone (issue #135); 0: none.
    pub undo_send_secs: u32,
    /// What opens at the start (issue #172).
    pub start_in: StartIn,
    /// The folder open last (All Inboxes or an account's), for [`StartIn::Last`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_open: Option<StartIn>,
    /// The folder pane shows Favorites at its top (issue #11: it could only be collapsed).
    pub show_favorites: bool,
    /// Meeting invitations go into the calendar as tentative when they arrive, before they're
    /// answered, as Outlook shows them (issue #20).
    pub tentative_invites: bool,
    /// What becomes of a meeting invitation's mail (and its earlier versions in the Inbox) once
    /// it's answered, in the message or the calendar, and of a cancellation once it's taken in
    /// (issue #31), as Outlook's "Delete meeting requests and notifications from Inbox after
    /// responding". Deleted Items at first, as there; settings saved before the option came
    /// leave the mail where it is, as it was.
    #[serde(default = "AnsweredInvites::kept")]
    pub answered_invites: AnsweredInvites,
    /// With [`AnsweredInvites::Move`]: the folder each account's go to (account id → path).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub answered_folders: BTreeMap<String, String>,
    /// Guests' answers to the user's meetings go the same way once recorded.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub answered_replies: bool,
    /// How the message list is arranged (View › Arrange by), and turned the other way round.
    pub arrange: Arrange,
    pub reverse_sort: bool,
    /// Work offline (Send / Receive › Work Offline): no account connects; kept across starts.
    pub work_offline: bool,
    /// Ignored conversations (Delete › Ignore): their messages' Message-IDs; replies to any of
    /// them go to Deleted Items as they arrive.
    pub ignored: IgnoredIds,
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
    /// The To-Do Bar beside the mail (View › To-Do Bar): its parts and its width.
    pub todo_bar: pim::TodoBar,
    /// When flagged messages are due (Follow Up › Today, Tomorrow…): Message-ID → local day.
    /// IMAP has nowhere for it, so it's kept here, as Outlook keeps it for IMAP accounts.
    #[serde(default, deserialize_with = "checked_days")]
    pub follow_up: BTreeMap<String, i64>,
    /// When follow-ups were marked complete (Follow Up › Mark Complete): Message-ID → local day.
    /// The mark itself is the message's [`FOLLOW_UP_COMPLETE`](mailcraft_model::FOLLOW_UP_COMPLETE)
    /// keyword; the day goes into an Outlook data file with it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "checked_days")]
    pub follow_up_done: BTreeMap<String, i64>,
    /// When flagged messages remind (Follow Up › Add Reminder…): Message-ID → Unix seconds. IMAP
    /// has nowhere for it, so it's kept here as the days are; Exchange and Microsoft 365 keep it
    /// on the message.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "checked_reminders")]
    pub mail_reminders: BTreeMap<String, i64>,
    /// The imports' due-day notes taken into `follow_up`, by account: each note's fingerprint
    /// ([`Session::imported_follow_up_at_start`]). Saved with `follow_up`, it says at the next
    /// start that the note's days are kept here, so the note can go.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub follow_up_taken: BTreeMap<String, u64>,
    /// Quick Steps: actions done to the selection in one go ([`quick_steps`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub quick_steps: Vec<quick_steps::QuickStep>,
    /// Play animations: hovers and selections easing, panes sliding, notices sliding in (off:
    /// everything changes at once).
    pub animations: bool,
    /// Compact data files as MailCraft closes, when one has room to give back and that takes
    /// no longer than closing waits for (File › Options › Data files; off unless chosen: it's
    /// done when MailCraft is left alone, or by hand, either way).
    pub compact_on_close: bool,
    /// AutoArchive (issue #180): when it runs, and what it does with each folder's old mail.
    pub auto_archive: AutoArchive,
    /// Search Folders: saved searches shown as folders ([`search_folders`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub search_folders: Vec<search_folders::SearchFolder>,
    /// Reminders answered ([`Session::due_reminders`]), by occurrence
    /// (`<account>|<uid>|<occurrence>`): 0 dismissed, else snoozed until that Unix time.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reminders: BTreeMap<String, i64>,
    /// Where the search box looks unless told (its menu, as Outlook's; Current Mailbox at first).
    pub search_scope: SearchScope,
    /// The last searches, newest first (the search box's list; cleared from it).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent_searches: Vec<String>,
    /// Photos go with where they were taken (the place a phone writes into them). Off, as at
    /// first: it's taken out of every JPEG and PNG attached as the message is sent.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub keep_photo_location: bool,
    /// Links from a message open where they really go: out of redirect wrappers, without the
    /// tags that say who clicked ([`mime::clean_link`]). On at first.
    pub clean_links: bool,
    /// Web pictures load as a message opens, unless it looks suspicious
    /// ([`Session::pictures_load_alone`]). Off at first: every message asks.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub auto_pictures: bool,
    /// Ask a signer's certificate's authority whether it was withdrawn (OCSP or its CRL), as
    /// signed mail is shown. Off at first: the authority learns whose mail is being read.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub check_revocation: bool,
    /// When the message list shows a sender's address after their name (issue #29): Outlook
    /// shows names only, which lets anyone pass for anyone.
    pub sender_addresses: SenderAddresses,
    /// Tag mail from outside the organisation "External" (issue #29). Off at first: outside a
    /// company nearly every message is, and a tag on everything is a tag nobody reads.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub external_tag: bool,
    /// The organisation's domains, for the External tag (empty: those of the user's accounts,
    /// mail services left out).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub org_domains: Vec<String>,
    /// Online meetings (issue #25): the user's own rooms (a BigBlueButton or Jitsi room's
    /// permanent link and access code), added to a meeting with Add online meeting; the first is
    /// the one added unless another is picked.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub meeting_rooms: Vec<mailcraft_model::ical::MeetingRoom>,
    /// More hosts whose links join a meeting (`jitsi.example.org`, `example.edu/rooms/`), for
    /// self-hosted servers other organisations run ([`mailcraft_model::ical::host_entry`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub meeting_hosts: Vec<String>,
    /// Add the first room's meeting to every new meeting with guests (Outlook's "Add online
    /// meeting to all meetings").
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub meeting_for_all: bool,
}

/// When the message list shows a sender's address after their name (issue #29).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SenderAddresses {
    Always,
    /// For senders the user doesn't know (not a contact, never written to, not in the
    /// directory): where an unknown address hides behind a familiar name.
    #[default]
    Strangers,
    Never,
}

impl SenderAddresses {
    pub fn code(self) -> &'static str {
        match self {
            SenderAddresses::Always => "always",
            SenderAddresses::Strangers => "strangers",
            SenderAddresses::Never => "never",
        }
    }

    pub fn from_code(code: &str) -> Option<SenderAddresses> {
        [SenderAddresses::Always, SenderAddresses::Strangers, SenderAddresses::Never]
            .into_iter()
            .find(|a| a.code() == code.trim().to_ascii_lowercase())
    }
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            reading_pane: ReadingPane::Right,
            conversations: false,
            mark_read_on_open: true,
            read_in_pane: ReadInPane::Shown,
            notes_order: NotesOrder::Modified,
            tasks_order: pim::TasksOrder::Due,
            tasks_hide_done: false,
            hidden_calendars: BTreeSet::new(),
            calendars_side_by_side: false,
            calendar_months: 1,
            work_time: mailcraft_model::pim::work_hours::WorkPeriod::default(),
            free_time_minutes: 30,
            address_book: "all".into(),
            keep_messages: 1000,
            keep_offline: 40,
            read_after_secs: 0,
            show_preview: true,
            compact: false,
            folder_density: FolderDensity::Spacious,
            folder_order: BTreeMap::new(),
            folded_groups: BTreeMap::new(),
            groups: true,
            single_line: false,
            classic_ribbon: false,
            ribbon_hidden: false,
            open_send_receive: false,
            accent: String::new(),
            plain_top_bar: false,
            light_messages: false,
            system_title_bar: false,
            date_style: mailcraft_model::dates::DateStyle::default(),
            confirm_permanent_delete: true,
            ctrl_enter: CtrlEnter::Ask,
            undo_send_secs: 0,
            start_in: StartIn::FirstInbox,
            last_open: None,
            favorites: None,
            notify: true,
            notify_other: false,
            spelling: true,
            dictionary: BTreeSet::new(),
            compose_font: None,
            close_to_tray: false,
            start_in_tray: false,
            tray_unread: true,
            categories: mailcraft_model::organize::default_categories(),
            focused_inbox: false,
            focus_overrides: std::collections::BTreeMap::new(),
            show_html: true,
            trusted_senders: BTreeSet::new(),
            blocked_senders: BTreeSet::new(),
            safe_senders: BTreeSet::new(),
            new_mail_window: true,
            reply_window: false,
            show_favorites: true,
            tentative_invites: true,
            answered_invites: AnsweredInvites::Delete,
            answered_folders: BTreeMap::new(),
            answered_replies: false,
            arrange: Arrange::Date,
            reverse_sort: false,
            work_offline: false,
            ignored: IgnoredIds::default(),
            follow_up: BTreeMap::new(),
            follow_up_done: BTreeMap::new(),
            mail_reminders: BTreeMap::new(),
            follow_up_taken: BTreeMap::new(),
            reminders: BTreeMap::new(),
            search_folders: Vec::new(),
            animations: true,
            compact_on_close: false,
            auto_archive: AutoArchive::default(),
            search_scope: SearchScope::Mailbox,
            recent_searches: Vec::new(),
            keep_photo_location: false,
            clean_links: true,
            auto_pictures: false,
            check_revocation: false,
            sender_addresses: SenderAddresses::Strangers,
            external_tag: false,
            org_domains: Vec::new(),
            meeting_rooms: Vec::new(),
            meeting_hosts: Vec::new(),
            meeting_for_all: false,
            quick_steps: Vec::new(),
            folder_pane_minimized: false,
            collapsed: BTreeSet::new(),
            pane_order: Vec::new(),
            todo_bar: pim::TodoBar::default(),
        }
    }
}

impl Prefs {
    /// The preferences as the app saved them, from this version or another that names or types
    /// a setting otherwise: a setting that can't be read starts as it does for everyone (its name
    /// is in the answer), the others are kept. `None` when `saved` isn't preferences at all.
    pub fn from_saved(saved: &serde_json::Value) -> Option<(Prefs, Vec<String>)> {
        if let Ok(p) = serde_json::from_value::<Prefs>(saved.clone()) {
            return Some((p, Vec::new()));
        }
        // One at a time: each read alone (the rest as they start), those that can be kept.
        let mut kept = serde_json::Map::new();
        let mut unread = Vec::new();
        for (name, value) in saved.as_object()? {
            let alone = serde_json::Value::Object(std::iter::once((name.clone(), value.clone())).collect());
            if serde_json::from_value::<Prefs>(alone).is_ok() {
                kept.insert(name.clone(), value.clone());
            } else {
                unread.push(name.clone());
            }
        }
        serde_json::from_value(serde_json::Value::Object(kept)).ok().map(|p| (p, unread))
    }
}

/// The accent colours offered by name (issue #8), each `(name, #rrggbb)`: MailCraft's teal
/// first. Any other colour can be given as `#rrggbb`.
pub const ACCENTS: [(&str, &str); 8] = [
    ("teal", "#0e7c86"),
    ("blue", "#2563c9"),
    ("indigo", "#4f52b8"),
    ("violet", "#7a4cc2"),
    ("rose", "#b8326a"),
    ("red", "#b83a2c"),
    ("orange", "#b35a14"),
    ("green", "#2a7d3a"),
];

/// Follow-up days read from disk: only what are days (a key too long or empty, a day out of
/// [`FOLLOW_UP_DAYS`](mailcraft_model::organize::FOLLOW_UP_DAYS), past
/// [`MAX_FOLLOW_UP`](mailcraft_model::organize::MAX_FOLLOW_UP) are left out).
fn checked_days<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, i64>, D::Error> {
    let read = <BTreeMap<String, i64> as serde::Deserialize>::deserialize(d)?;
    let mut kept = BTreeMap::new();
    mailcraft_model::organize::merge_follow_up(&mut kept, read);
    Ok(kept)
}

/// Flags' reminders read from disk: only times on the days a follow-up may have, by a key that
/// may be a follow-up's ([`merge_reminders`](mailcraft_model::organize::merge_reminders)).
fn checked_reminders<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, i64>, D::Error> {
    let read = <BTreeMap<String, i64> as serde::Deserialize>::deserialize(d)?;
    let mut kept = BTreeMap::new();
    mailcraft_model::organize::merge_reminders(&mut kept, read);
    Ok(kept)
}

/// The most words the dictionary keeps, and the longest word.
pub const DICTIONARY_MAX: usize = 10_000;
pub const DICTIONARY_WORD_MAX: usize = 100;

/// A word as the dictionary keeps it, when it's one: trimmed, one word (no space or control
/// character), at most [`DICTIONARY_WORD_MAX`] characters.
pub fn dictionary_word(w: &str) -> Option<String> {
    let w = w.trim();
    let ok = !w.is_empty() && w.chars().count() <= DICTIONARY_WORD_MAX && !w.chars().any(|c| c.is_whitespace() || c.is_control());
    ok.then(|| w.to_string())
}

/// The font for new messages read from disk: only a name a style can carry
/// ([`mime::rich::font_name`]).
fn checked_font<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let read = <Option<String> as serde::Deserialize>::deserialize(d)?;
    Ok(read.as_deref().and_then(mailcraft_mime::rich::font_name))
}

/// The dictionary's words read from disk: only what are words, up to [`DICTIONARY_MAX`].
fn checked_words<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeSet<String>, D::Error> {
    let read = <Vec<String> as serde::Deserialize>::deserialize(d)?;
    Ok(read.iter().filter_map(|w| dictionary_word(w)).take(DICTIONARY_MAX).collect())
}

/// An accent as it's kept: a name from [`ACCENTS`] or `#rrggbb` (lower case); `""` for
/// MailCraft's own. `None` when it's neither.
pub fn accent_code(given: &str) -> Option<String> {
    let g = given.trim().to_ascii_lowercase();
    if g.is_empty() || g == "teal" || g == "default" {
        return Some(String::new());
    }
    if ACCENTS.iter().any(|(n, _)| *n == g) {
        return Some(g);
    }
    let hex = g.strip_prefix('#').unwrap_or(&g);
    (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| format!("#{hex}"))
}

/// The colour an accent code stands for (`None`: MailCraft's own).
pub fn accent_rgb(code: &str) -> Option<[u8; 3]> {
    let hex = ACCENTS.iter().find(|(n, _)| *n == code).map_or(code, |(_, h)| h);
    let hex = hex.strip_prefix('#')?;
    let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// The Message-IDs of ignored conversations, each with when it was last added: past
/// [`IgnoredIds::MAX`] the oldest go (never the ones just added), and they're kept oldest first.
#[derive(Clone, Debug, Default)]
pub struct IgnoredIds {
    ids: HashMap<String, u64>,
    added: u64,
}

/// The same ids in the same order (the counts they were added at may differ: read back from a
/// file, they start again from one).
impl PartialEq for IgnoredIds {
    fn eq(&self, other: &Self) -> bool {
        self.ids.len() == other.ids.len() && self.in_order() == other.in_order()
    }
}

impl Eq for IgnoredIds {}

impl IgnoredIds {
    /// The most ids kept.
    pub const MAX: usize = 5_000;

    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains_key(id)
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.ids.remove(id).is_some()
    }

    /// The ids, oldest first.
    pub fn in_order(&self) -> Vec<&str> {
        let mut all: Vec<(&String, &u64)> = self.ids.iter().collect();
        all.sort_unstable_by_key(|(_, at)| **at);
        all.into_iter().map(|(id, _)| id.as_str()).collect()
    }

    /// Past [`IgnoredIds::MAX`], the oldest go.
    fn trim(&mut self) {
        let excess = self.ids.len().saturating_sub(Self::MAX);
        if excess == 0 {
            return;
        }
        let mut ats: Vec<u64> = self.ids.values().copied().collect();
        ats.sort_unstable();
        // Each id was added at its own count: those up to the excess-th oldest go.
        if let Some(&last_gone) = ats.get(excess - 1) {
            self.ids.retain(|_, at| *at > last_gone);
        }
    }
}

/// Added, or added again, as the newest; the oldest go past [`IgnoredIds::MAX`].
impl Extend<String> for IgnoredIds {
    fn extend<I: IntoIterator<Item = String>>(&mut self, ids: I) {
        for id in ids {
            self.added = self.added.saturating_add(1);
            self.ids.insert(id, self.added);
        }
        self.trim();
    }
}

/// Saved as a list, oldest first (as a set was saved before, in its own order).
impl Serialize for IgnoredIds {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.in_order())
    }
}

impl<'de> Deserialize<'de> for IgnoredIds {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut ids = IgnoredIds::default();
        ids.extend(Vec::<String>::deserialize(d)?);
        Ok(ids)
    }
}
