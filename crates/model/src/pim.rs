//! What an account keeps of its calendars and address books: the server's (CalDAV, CardDAV)
//! with what's needed to sync them, or ones kept on this computer only; and its notes and tasks.

use serde::{Deserialize, Serialize};

use crate::ical::CalEvent;
use crate::vcard::Card;

pub mod free_busy;
pub mod ical;
pub mod signature;
pub mod vcard;
pub mod work_hours;

mod kept_file;
mod task;
pub use kept_file::{KeptFile, kept_file};
pub use task::*;

/// The id of a calendar or address book that lives on this computer only.
pub const LOCAL: &str = "local";

/// Where an account's calendars and address books are kept in its cache.
pub const PIM_DOC: &str = "pim.json";

/// A calendar.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarInfo {
    /// The collection's address on the server, or [`LOCAL`].
    pub id: String,
    pub name: String,
    /// An index into the UI's palette.
    #[serde(default)]
    pub color: u8,
    #[serde(default)]
    pub read_only: bool,
    /// The collection's version at the last sync (`getctag` / sync token).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ctag: String,
    /// Someone else's, listed with the account's own (its `DAV:owner` isn't the account's):
    /// SOGo lists the calendars subscribed to in the user's own, writable ones among them
    /// (issue #30). New events and invitations don't go there by themselves.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub foreign: bool,
    /// A colleague's who shares only their free/busy time (issue #140): what it holds is their
    /// busy times, asked of the server as they're shown, never kept here nor changed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub free_busy: bool,
}

impl CalendarInfo {
    /// The calendar on this computer ([`LOCAL`]).
    pub fn local() -> CalendarInfo {
        CalendarInfo { id: LOCAL.into(), name: "Calendar".into(), ..Default::default() }
    }
}

/// An event with where it lives.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEvent {
    /// The calendar's id.
    pub calendar: String,
    /// The resource's address on the server (empty until stored there).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    pub event: CalEvent,
    /// Changed here and not yet on the server.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

/// An address book.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ctag: String,
}

impl BookInfo {
    /// The address book on this computer ([`LOCAL`]).
    pub fn local() -> BookInfo {
        BookInfo { id: LOCAL.into(), name: "Contacts".into(), read_only: false, ctag: String::new() }
    }
}

/// A card with where it lives.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCard {
    pub book: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    pub card: Card,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

/// What the id of someone else's calendar opened by their address starts with.
const SHARED: &str = "shared:";

/// The id of someone else's calendar opened by their address.
pub fn shared_calendar_id(email: &str) -> String {
    format!("{SHARED}{}", email.trim().to_ascii_lowercase())
}

/// Whose calendar a shared calendar's id is (`shared:<address>`).
pub fn shared_calendar_owner(id: &str) -> Option<&str> {
    id.strip_prefix(SHARED).filter(|e| !e.is_empty())
}

/// A calendar group: its name, and the people in it by their addresses (lower-case), in the order
/// added.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarGroup {
    pub name: String,
    pub members: Vec<String>,
}

/// A calendar subscribed to by its link (an iCal or `webcal://` address, issue #7), as Outlook's
/// Add Calendar › From Internet: read-only, read again from the link now and then.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebCalendar {
    /// Its calendar's id ([`web_calendar_id`]).
    pub id: String,
    /// The link it's read from (`https://` or `http://`; a `webcal://` one as `https://`).
    pub url: String,
    /// Named by the user: the calendar's own name (`X-WR-CALNAME`) doesn't replace it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub named: bool,
    /// When it was last read, and last tried (Unix seconds; 0: never).
    #[serde(default)]
    pub fetched: i64,
    #[serde(default)]
    pub tried: i64,
    /// What the server named this version (`ETag`, `Last-Modified`), so an unchanged calendar
    /// isn't sent again.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub modified: String,
    /// How often the calendar asks to be read (`REFRESH-INTERVAL`, `X-PUBLISHED-TTL`), seconds.
    #[serde(default)]
    pub every: i64,
    /// Why the last read failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The id of a calendar subscribed to by its link.
pub fn web_calendar_id(url: &str) -> String {
    format!("web:{url}")
}

/// Whether a calendar is one subscribed to by its link.
pub fn is_web_calendar(id: &str) -> bool {
    id.starts_with("web:")
}

/// Longest calendar link kept.
pub const MAX_CALENDAR_LINK: usize = 2048;

/// A calendar link as it's read: `webcal://` and `webcals://` (how calendars are published to
/// subscribe to) as `https://`, `http://` and `https://` as given; anything else (another scheme,
/// no host, spaces, a user name and password in it), `None`.
pub fn calendar_link(s: &str) -> Option<String> {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    let (scheme, rest) = ["webcals://", "webcal://", "https://", "http://"]
        .iter()
        .find(|p| lower.starts_with(*p))
        .map(|p| (if p.starts_with("http://") { "http://" } else { "https://" }, s.get(p.len()..).unwrap_or("")))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let bad = |c: char| c.is_whitespace() || c.is_control();
    if host.is_empty() || host.contains('@') || host.starts_with(':') || rest.chars().any(bad) {
        return None;
    }
    // A `#fragment` is the browser's, never sent.
    let rest = rest.split('#').next().unwrap_or(rest);
    let link = format!("{scheme}{rest}");
    (link.len() <= MAX_CALENDAR_LINK).then_some(link)
}

/// A note, as Outlook's Notes keeps them: text, a colour, and the files it carries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub uid: String,
    pub text: String,
    /// Outlook's note colours: 0 blue, 1 green, 2 pink, 3 yellow (the usual), 4 white.
    #[serde(default = "yellow")]
    pub color: u8,
    /// When it was last changed (Unix seconds).
    #[serde(default)]
    pub modified: i64,
    /// Its files (a note from an Outlook data file can carry some), each the line an event and a
    /// task keep one in ([`kept_file`]): `ATTACH` with its bytes inline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

fn yellow() -> u8 {
    NOTE_YELLOW
}

/// The colour a new note takes, as in Outlook.
pub const NOTE_YELLOW: u8 = 3;
/// How many note colours there are.
pub const NOTE_COLORS: u8 = 5;

impl Note {
    /// Its first line: what a list of notes shows as its title.
    pub fn title(&self) -> &str {
        self.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
    }

    /// Its files, in order (a line that isn't one left out).
    pub fn kept_files(&self) -> Vec<KeptFile<'_>> {
        self.files.iter().filter_map(|l| kept_file(l)).collect()
    }
}

/// A note with where it lives: `href` is its item on the server (Exchange's Notes folder),
/// empty for a note kept on this computer only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNote {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    pub note: Note,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

/// Something deleted here that is still to be deleted on the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub href: String,
    #[serde(default)]
    pub etag: String,
    /// The calendar it was in, when that decides where it's deleted (someone else's calendar).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub calendar: String,
}

/// An account's calendars and address books.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pim {
    #[serde(default)]
    pub calendars: Vec<CalendarInfo>,
    #[serde(default)]
    pub events: Vec<StoredEvent>,
    #[serde(default)]
    pub books: Vec<BookInfo>,
    #[serde(default)]
    pub cards: Vec<StoredCard>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted: Vec<Tombstone>,
    /// Other people's calendars opened from this account (their addresses, lower-case), as
    /// Outlook's Open Calendar › From Address Book: read from Exchange with the rest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared: Vec<String>,
    /// Calendar groups (issue #140), as Outlook's: people whose calendars (in `shared`) are shown
    /// and hidden together, under the group's name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<CalendarGroup>,
    /// Calendars subscribed to by their links (issue #7), each with its entry in `calendars`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscribed: Vec<WebCalendar>,
    /// Notes (Outlook's Notes): the server's (Exchange) and those kept here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<StoredNote>,
    /// Tasks (Outlook's Tasks): the server's (Exchange, CalDAV) and those kept here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<StoredTask>,
    /// The CalDAV server's task lists (issue #201); none when it keeps no tasks (tasks then stay
    /// on this computer).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_lists: Vec<TaskList>,
    /// Task lists to make on the server, by name, at the next sync (issue #294: one with none kept
    /// MailCraft's tasks on this computer).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new_task_lists: Vec<String>,

    /// Where the server keeps the user's calendars and address books (found once); empty when
    /// it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_home: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carddav_home: Option<String>,
    /// When the server was last asked where they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dav_checked: Option<i64>,
    /// The last sync with the server taken in here ([`DavSync::pass`]): the next one waits for
    /// it, as it reads this to send what changed here.
    #[serde(default, skip_serializing_if = "no_pass")]
    pub dav_pass: u64,
    /// The calendar the user chose for new events and invitations (Default calendar, issue
    /// #30); without one, [`Pim::default_calendar`] picks the account's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen_calendar: Option<String>,
    /// The calendar the server takes as the user's default (RFC 6638's
    /// `schedule-default-calendar-URL`, issue #278), which [`Pim::default_calendar`] picks when
    /// the user chose none. `Some("")`: the server was asked and names none; `None`: not asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_default: Option<String>,
    /// When the server last answered which is its default: it's asked again a day on, as
    /// another program may have set another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_default_at: Option<i64>,
    /// Invitations that arrived before the server was asked where the calendars are, waiting to
    /// be put in the calendar once it's known (issue #30); kept, so a restart doesn't lose them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held: Vec<HeldInvite>,
    /// Invitations MailCraft itself put in the calendar on this computer as they arrived (their
    /// uids), because the account had no calendar on its server yet: they move there when it
    /// has one (issue #30).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub placed: Vec<String>,
    /// The user chose to keep the calendar on this computer: moving its events to the server
    /// isn't offered again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_here: bool,
    /// The calendar server sends meeting invitations and answers itself when events are stored
    /// (it says `calendar-auto-schedule`, RFC 6638): MailCraft tells it to leave them to
    /// MailCraft, and mails them once the server has the meeting ([`Notice`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_schedule: bool,
    /// Meetings (their uids) whose copy in the user's calendar on such a server MailCraft put
    /// there as a guest, from the invitation's mail (`ical::GUEST_COPY`): answers to them go by
    /// mail, as the server never had them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guest_copies: Vec<String>,
    /// Invitations, answers and cancellations to mail once the server has stored (or deleted)
    /// what they're about; kept, so a restart doesn't lose them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<Notice>,
    /// Reminders dismissed or snoozed here, for an Exchange server to learn (issue #40): Outlook
    /// shows them again otherwise. A CalDAV server learns them from the event itself (its
    /// alarm's `ACKNOWLEDGED`, RFC 9074).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reminder_answers: Vec<ReminderAnswer>,
    /// Answers to the user's meetings from someone a guest handed their place to (issue #254)
    /// that came before that guest's own answer saying so: taken in once it does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegate_answers: Vec<crate::MessageKey>,
}

/// The most notices kept waiting for the server.
pub const MAX_NOTICES: usize = 500;
/// The most delegates' answers kept waiting for the guest's ([`Pim::delegate_answers`]).
pub const MAX_DELEGATE_ANSWERS: usize = 100;
/// The most reminder answers kept waiting for the server.
pub const MAX_REMINDER_ANSWERS: usize = 500;

/// A reminder answered here, waiting for the server ([`Pim::reminder_answers`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderAnswer {
    pub calendar: String,
    /// The event's item on the server (a series' when it recurs).
    pub href: String,
    pub uid: String,
    /// The occurrence's start (Unix seconds).
    pub occurrence: i64,
    /// One occurrence of a series.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub series: bool,
    /// Snoozed until then; `None`: dismissed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snooze_until: Option<i64>,
}

/// A meeting message (iTIP) waiting for the calendar server: the organizer's invitation or
/// change goes with the version the server stored (SOGo raises its `SEQUENCE`), a cancellation
/// once the server deleted the meeting, and a guest's answer once it's known whether the server
/// takes it to the organizer itself (a meeting it put in the calendar) or MailCraft mails it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub uid: String,
    /// `REQUEST`, a change (`UPDATE`, sent as a `REQUEST` that says so), `CANCEL` or `REPLY`.
    pub method: String,
    /// A guest's answer (`ACCEPTED`, `TENTATIVE`, `DECLINED`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub answer: String,
    /// The meeting as it was (a cancelled or declined one is gone from the calendar), or the
    /// occurrence a cancellation names.
    pub event: CalEvent,
    /// The item to be deleted on the server first (empty: none).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    /// A guest's answer goes by mail whatever the server's copy says (the user's own copy was
    /// a guest copy MailCraft put there).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub by_mail: bool,
}

/// An invitation that arrived, waiting for the account's calendars to be found (issue #30).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldInvite {
    /// The message it came in.
    pub key: crate::MessageKey,
    /// When it was held (Unix seconds): after a while it goes to the calendar on this computer.
    pub since: i64,
    /// Its calendar (the message's `text/calendar` part), and the address it came from: kept,
    /// as the message may be moved by a rule meanwhile.
    pub ics: String,
    pub from: String,
}

/// What a CalDAV / CardDAV sync found, for the engine to merge into its [`Pim`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DavSync {
    /// The homes found (`None`: the server offers none).
    pub caldav_home: Option<String>,
    pub carddav_home: Option<String>,
    pub checked: i64,
    /// The server's calendars (`None`: no CalDAV).
    pub calendars: Option<Vec<CalendarInfo>>,
    /// The events of the calendars read in full (`synced_calendars`), except those unchanged
    /// (`kept`, by href).
    pub events: Vec<StoredEvent>,
    pub synced_calendars: Vec<String>,
    /// Calendars that didn't change: what the engine has stays.
    pub unchanged_calendars: Vec<String>,
    pub books: Option<Vec<BookInfo>>,
    pub cards: Vec<StoredCard>,
    pub synced_books: Vec<String>,
    pub unchanged_books: Vec<String>,
    /// Items read again unchanged (their etag matched).
    pub kept: Vec<String>,
    /// Changes stored on the server, with their new addresses and etags.
    pub pushed_events: Vec<StoredEvent>,
    pub pushed_cards: Vec<StoredCard>,
    /// The server's notes, read in full (`None`: it keeps none, so notes stay on this
    /// computer), and the notes stored there this time.
    #[serde(default)]
    pub notes: Option<Vec<StoredNote>>,
    #[serde(default)]
    pub pushed_notes: Vec<StoredNote>,
    /// The server's tasks (Exchange's Tasks folder, issue #161), read in full (`None`: it keeps
    /// none, so tasks stay on this computer), and the tasks stored there this time (Exchange's,
    /// or in a CalDAV task list).
    #[serde(default)]
    pub tasks: Option<Vec<StoredTask>>,
    #[serde(default)]
    pub pushed_tasks: Vec<StoredTask>,
    /// The CalDAV server's task lists (issue #201; `None`: no CalDAV), the tasks of those read
    /// in full (`synced_task_lists`) except those unchanged (`kept`, by href).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_lists: Option<Vec<TaskList>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub list_tasks: Vec<StoredTask>,
    /// The task lists asked for ([`Pim::new_task_lists`]) this sync tried to make: made, read
    /// with the others, or said why not; not asked for again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub made_task_lists: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub synced_task_lists: Vec<String>,
    /// Deletions done (by href).
    pub deleted: Vec<String>,
    /// What couldn't be stored or deleted (tried again next time).
    pub errors: Vec<String>,
    /// Events (by uid) whose change the server refused for good (a colleague's event the user's
    /// role there doesn't let them change): what's here gives way to the server's copy, read
    /// again in this sync.
    #[serde(default)]
    pub refused: Vec<String>,
    /// Whether the calendar server schedules (`calendar-auto-schedule`); `None`: not asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_schedule: Option<bool>,
    /// The calendar the server takes as the user's default ([`Pim::server_default`]): `Some("")`
    /// when it names none; `None`: not asked this time, what was known stays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_calendar: Option<String>,
    /// Asking for it failed: what was known is forgotten, so it's asked again at the next sync,
    /// though the calendars are the same then (they changed this time, or it was never known).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default_calendar_failed: bool,
    /// Reminder answers the server took, or that no longer apply there (the event is gone): by
    /// uid and occurrence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answered: Vec<(String, i64)>,
    /// Only the calendars of people just opened ([`crate::Job::ReadShared`], issue #208): theirs
    /// taken in, everything else left as it is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub people_only: bool,
    /// Which sync with the server this is, counted up for each account: the engine keeps the
    /// last it took in ([`Pim::dav_pass`]), and the next waits for it. A sync begun before the one
    /// before was taken in read what changed here as it was before that one, and sent it again:
    /// refused (412) as "changed on the server meanwhile", and the server's copy kept.
    #[serde(default, skip_serializing_if = "no_pass")]
    pub pass: u64,
    /// The homes were taken as saved, not looked for: the last look ([`Pim::dav_checked`]) stays
    /// as it was, so a server not found is looked for again a day after that look, not a day after
    /// the last of the syncs since.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub homes_kept: bool,
}

impl Pim {
    /// Whether a calendar is someone else's: listed as theirs (a colleague's the user subscribed
    /// to), or one the user may only read. A meeting there is their copy, not the user's to
    /// answer in or keep beside their own.
    pub fn is_theirs(&self, calendar: &str) -> bool {
        self.calendars.iter().any(|c| c.id == calendar && (c.foreign || c.read_only))
    }

    /// The calendars that are someone else's ([`Pim::is_theirs`]).
    pub fn theirs(&self) -> Vec<String> {
        self.calendars.iter().filter(|c| c.foreign || c.read_only).map(|c| c.id.clone()).collect()
    }

    /// Whether it holds anything: contacts, events, notes or tasks.
    pub fn has_items(&self) -> bool {
        !self.cards.is_empty() || !self.events.is_empty() || !self.notes.is_empty() || !self.tasks.is_empty()
    }

    /// Make sure there is somewhere to put events and people.
    pub fn ensure_defaults(&mut self) {
        if self.calendars.is_empty() {
            self.calendars.push(CalendarInfo::local());
        }
        if self.books.is_empty() {
            self.books.push(BookInfo::local());
        }
    }

    /// The calendar server changed (issue #7): what came from the old one goes, except what was
    /// changed here and not yet sent, which is kept in the calendar and contacts on this computer
    /// (it would otherwise be sent to the old server with the new one's password, or lost).
    /// Where the homes are is looked for again.
    pub fn leave_server(&mut self) {
        let mut kept_any = false;
        self.events.retain_mut(|e| {
            if e.calendar == LOCAL || e.calendar.starts_with(SHARED) {
                return true;
            }
            if !e.dirty {
                return false;
            }
            e.calendar = LOCAL.into();
            e.href.clear();
            e.etag.clear();
            e.dirty = false;
            kept_any = true;
            true
        });
        self.cards.retain_mut(|c| {
            if c.book == LOCAL {
                return true;
            }
            if !c.dirty {
                return false;
            }
            c.book = LOCAL.into();
            c.href.clear();
            c.etag.clear();
            c.dirty = false;
            kept_any = true;
            true
        });
        self.deleted.clear();
        self.calendars.retain(|c| c.id == LOCAL || c.id.starts_with(SHARED));
        self.books.retain(|b| b.id == LOCAL);
        if kept_any || self.calendars.is_empty() {
            if !self.calendars.iter().any(|c| c.id == LOCAL) {
                self.calendars.insert(0, CalendarInfo::local());
            }
            if !self.books.iter().any(|b| b.id == LOCAL) {
                self.books.insert(0, BookInfo::local());
            }
        }
        self.caldav_home = None;
        self.carddav_home = None;
        self.dav_checked = None;
        // The old server's calendar can't be the default any more; the user said to keep the
        // calendar here for the old one.
        self.chosen_calendar = None;
        self.server_default = None;
        self.server_default_at = None;
        self.keep_here = false;
    }

    /// The calendar new events and invitations go to: the one the user chose, else the
    /// account's own on its server (the one the server names its default, else SOGo's
    /// `personal` or a `default`) rather than someone else's listed with it, else the first
    /// that can be added to (issues #30, #278).
    pub fn default_calendar(&self) -> Option<&CalendarInfo> {
        pick_calendar(self.calendars.iter(), self.chosen_calendar.as_deref(), self.server_default.as_deref())
    }

    /// The calendar on the server events move to from this computer: as
    /// [`Pim::default_calendar`], among the server's own calendars only.
    pub fn server_calendar(&self) -> Option<&CalendarInfo> {
        pick_calendar(self.calendars.iter().filter(|c| is_server_calendar(&c.id)), self.chosen_calendar.as_deref(), self.server_default.as_deref())
    }

    /// The user's own calendars on the server, that they may change: not someone else's listed
    /// with them (SOGo lists those the user subscribed to in their home), nor one read-only.
    pub fn own_server_calendars(&self) -> Vec<String> {
        self.calendars.iter().filter(|c| is_server_calendar(&c.id) && !c.foreign && !c.read_only).map(|c| c.id.clone()).collect()
    }

    pub fn default_book(&self) -> Option<&BookInfo> {
        self.books.iter().find(|b| !b.read_only)
    }

    /// The CalDAV task list new tasks go to (issue #201): the first made for tasks alone, else
    /// the first that takes them; the user's own and writable. `None` when the server keeps no
    /// tasks: they stay on this computer.
    pub fn default_task_list(&self) -> Option<&TaskList> {
        let mut usable = self.task_lists.iter().filter(|l| !l.read_only && !l.foreign);
        usable.clone().find(|l| l.tasks_only).or_else(|| usable.next())
    }

    /// Whether a task list is one the user may change.
    pub fn task_list_writable(&self, id: &str) -> bool {
        id.is_empty() || self.task_lists.iter().any(|l| l.id == id && !l.read_only)
    }
}

/// A calendar kept on the account's server (CalDAV, Exchange): not the one on this computer,
/// one subscribed to by its link, or someone else's opened by their address.
pub fn is_server_calendar(id: &str) -> bool {
    id != LOCAL && !is_web_calendar(id) && shared_calendar_owner(id).is_none() && !id.is_empty()
}

/// The calendar new events go to among these (see [`Pim::default_calendar`]): the one `chosen`
/// by the user, else the one the server names its default (`server`), each when it can be added
/// to.
pub fn pick_calendar<'a>(
    calendars: impl Iterator<Item = &'a CalendarInfo> + Clone,
    chosen: Option<&str>,
    server: Option<&str>,
) -> Option<&'a CalendarInfo> {
    let writable = |c: &&CalendarInfo| !c.read_only && !is_web_calendar(&c.id);
    if let Some(c) = chosen.and_then(|id| calendars.clone().filter(writable).find(|c| c.id == id)) {
        return Some(c);
    }
    let own = |c: &&CalendarInfo| writable(c) && !c.foreign && shared_calendar_owner(&c.id).is_none();
    // The server's default as it names it (issue #278), when it's among these (a trailing slash
    // aside) and the user's own.
    let same = |a: &str, b: &str| a.trim_end_matches('/') == b.trim_end_matches('/');
    if let Some(c) = server.filter(|s| !s.is_empty()).and_then(|s| calendars.clone().filter(own).find(|c| same(&c.id, s))) {
        return Some(c);
    }
    // The server's own default by its name: SOGo's `personal`, Stalwart's and Radicale-style
    // `default` (issue #278), before the order the server lists them in.
    let named_default = |c: &&CalendarInfo| {
        c.id.trim_end_matches('/').rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case("personal") || n.eq_ignore_ascii_case("default"))
    };
    calendars
        .clone()
        .filter(own)
        .find(named_default)
        .or_else(|| calendars.clone().find(own))
        .or_else(|| calendars.clone().find(|c| writable(c) && shared_calendar_owner(&c.id).is_none()))
        .or_else(|| calendars.clone().find(writable))
}

/// Make UIDs unique within each calendar. A calendar can hold several plain (non-recurring)
/// appointments with one UID, e.g. invitations a scheduling tool sends per date under its
/// series UID. The engine stores events by UID, so only one survived. Such appointments get
/// `<uid>/<start>` here. Recurring masters and exceptions are untouched, and the new UID never
/// goes back to the server.
pub fn separate_shared_uids(events: &mut [StoredEvent]) {
    // Which events share their calendar and UID with another: counted on the events' own
    // strings, before any is changed.
    let shared: Vec<bool> = {
        let mut seen: std::collections::HashMap<(&str, &str), usize> = std::collections::HashMap::new();
        for e in events.iter().filter(|e| e.event.recurrence_id.is_none()) {
            *seen.entry((e.calendar.as_str(), e.event.uid.as_str())).or_default() += 1;
        }
        events.iter().map(|e| seen.get(&(e.calendar.as_str(), e.event.uid.as_str())).is_some_and(|n| *n > 1)).collect()
    };
    let mut taken: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for (e, shared) in events.iter_mut().zip(shared) {
        if !shared || e.event.recurrence_id.is_some() || e.event.rrule.is_some() {
            continue;
        }
        let mut uid = format!("{}/{}", e.event.uid, e.event.start);
        let mut n = 1;
        while !taken.insert((e.calendar.clone(), uid.clone())) && n < 1000 {
            n += 1;
            uid = format!("{}/{}-{n}", e.event.uid, e.event.start);
        }
        e.event.uid = uid;
    }
}

/// No sync numbered yet (left out of the files).
fn no_pass(n: &u64) -> bool {
    *n == 0
}

#[cfg(test)]
mod default_calendar_tests;
#[cfg(test)]
mod shared_uid_tests;
#[cfg(test)]
mod web_calendar_tests;
