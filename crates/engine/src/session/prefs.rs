//! The settings the user changes (View tab, File › Options), saved by the app.

use super::*;

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
    /// The classic ribbon (issue #8): big icons over their labels, in named groups, instead of
    /// one row of small ones.
    pub classic_ribbon: bool,
    /// The accent colour (issue #8): empty for MailCraft's teal, a named one ([`ACCENTS`]), or
    /// any colour as `#rrggbb`. The window adjusts it to read well in light and dark.
    pub accent: String,
    /// The top bar in plain pane colours instead of the accent colour (issue #8, as Office's
    /// white theme).
    pub plain_top_bar: bool,
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
    /// Replies and forwards open in a window of their own too, not in the reading pane (issue
    /// #11: Pop out each time).
    pub reply_window: bool,
    /// The folder pane shows Favorites at its top (issue #11: it could only be collapsed).
    pub show_favorites: bool,
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
    /// Play animations: hovers and selections easing, panes sliding, notices sliding in (off:
    /// everything changes at once).
    pub animations: bool,
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
    /// Ask a signer's certificate's authority whether it was withdrawn (OCSP or its CRL), as
    /// signed mail is shown. Off at first: the authority learns whose mail is being read.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub check_revocation: bool,
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
            classic_ribbon: false,
            accent: String::new(),
            plain_top_bar: false,
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
            reply_window: false,
            show_favorites: true,
            arrange: Arrange::Date,
            reverse_sort: false,
            work_offline: false,
            ignored: BTreeSet::new(),
            follow_up: BTreeMap::new(),
            reminders: BTreeMap::new(),
            search_folders: Vec::new(),
            animations: true,
            search_scope: SearchScope::Mailbox,
            recent_searches: Vec::new(),
            keep_photo_location: false,
            clean_links: true,
            check_revocation: false,
            quick_steps: Vec::new(),
            folder_pane_minimized: false,
            collapsed: BTreeSet::new(),
            pane_order: Vec::new(),
        }
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
