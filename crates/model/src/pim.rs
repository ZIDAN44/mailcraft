//! What an account keeps of its calendars and address books: the server's (CalDAV, CardDAV)
//! with what's needed to sync them, or ones kept on this computer only; and its notes and tasks.

use serde::{Deserialize, Serialize};

use crate::ical::CalEvent;
use crate::vcard::Card;

mod task;
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

/// The id of someone else's calendar opened by their address.
pub fn shared_calendar_id(email: &str) -> String {
    format!("shared:{}", email.trim().to_ascii_lowercase())
}

/// Whose calendar a shared calendar's id is (`shared:<address>`).
pub fn shared_calendar_owner(id: &str) -> Option<&str> {
    id.strip_prefix("shared:").filter(|e| !e.is_empty())
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

/// A note, as Outlook's Notes keeps them: text, and a colour.
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
    /// Calendars subscribed to by their links (issue #7), each with its entry in `calendars`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscribed: Vec<WebCalendar>,
    /// Notes (Outlook's Notes): the server's (Exchange) and those kept here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<StoredNote>,
    /// Tasks (Outlook's Tasks), kept on this computer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<StoredTask>,
    /// Where the server keeps the user's calendars and address books (found once); empty when
    /// it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_home: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carddav_home: Option<String>,
    /// When the server was last asked where they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dav_checked: Option<i64>,
    /// The calendar the user chose for new events and invitations (Default calendar, issue
    /// #30); without one, [`Pim::default_calendar`] picks the account's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen_calendar: Option<String>,
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
}

/// The most notices kept waiting for the server.
pub const MAX_NOTICES: usize = 500;

/// A meeting message (iTIP) waiting for the calendar server: the organizer's invitation or
/// change goes with the version the server stored (SOGo raises its `SEQUENCE`), a cancellation
/// once the server deleted the meeting, and a guest's answer once it's known whether the server
/// takes it to the organizer itself (a meeting it put in the calendar) or MailCraft mails it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub uid: String,
    /// `REQUEST`, `CANCEL` or `REPLY`.
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
}

impl Pim {
    /// Whether it holds anything: contacts, events, notes or tasks.
    pub fn has_items(&self) -> bool {
        !self.cards.is_empty() || !self.events.is_empty() || !self.notes.is_empty() || !self.tasks.is_empty()
    }

    /// Make sure there is somewhere to put events and people.
    pub fn ensure_defaults(&mut self) {
        if self.calendars.is_empty() {
            self.calendars.push(CalendarInfo { id: LOCAL.into(), name: "Calendar".into(), ..Default::default() });
        }
        if self.books.is_empty() {
            self.books.push(BookInfo { id: LOCAL.into(), name: "Contacts".into(), read_only: false, ctag: String::new() });
        }
    }

    /// The calendar server changed (issue #7): what came from the old one goes, except what was
    /// changed here and not yet sent, which is kept in the calendar and contacts on this computer
    /// (it would otherwise be sent to the old server with the new one's password, or lost).
    /// Where the homes are is looked for again.
    pub fn leave_server(&mut self) {
        let mut kept_any = false;
        self.events.retain_mut(|e| {
            if e.calendar == LOCAL || e.calendar.starts_with("shared:") {
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
        self.calendars.retain(|c| c.id == LOCAL || c.id.starts_with("shared:"));
        self.books.retain(|b| b.id == LOCAL);
        if kept_any || self.calendars.is_empty() {
            if !self.calendars.iter().any(|c| c.id == LOCAL) {
                self.calendars.insert(0, CalendarInfo { id: LOCAL.into(), name: "Calendar".into(), ..Default::default() });
            }
            if !self.books.iter().any(|b| b.id == LOCAL) {
                self.books.insert(0, BookInfo { id: LOCAL.into(), name: "Contacts".into(), read_only: false, ctag: String::new() });
            }
        }
        self.caldav_home = None;
        self.carddav_home = None;
        self.dav_checked = None;
        // The old server's calendar can't be the default any more; the user said to keep the
        // calendar here for the old one.
        self.chosen_calendar = None;
        self.keep_here = false;
    }

    /// The calendar new events and invitations go to: the one the user chose, else the
    /// account's own on its server (SOGo's `personal` first) rather than someone else's listed
    /// with it, else the first that can be added to (issue #30).
    pub fn default_calendar(&self) -> Option<&CalendarInfo> {
        pick_calendar(&self.calendars, self.chosen_calendar.as_deref())
    }

    /// The calendar on the server events move to from this computer: as
    /// [`Pim::default_calendar`], among the server's own calendars only.
    pub fn server_calendar(&self) -> Option<&CalendarInfo> {
        let server: Vec<CalendarInfo> = self.calendars.iter().filter(|c| is_server_calendar(&c.id)).cloned().collect();
        let id = pick_calendar(&server, self.chosen_calendar.as_deref())?.id.clone();
        self.calendars.iter().find(|c| c.id == id)
    }

    /// The user's own calendars on the server, that they may change: not someone else's listed
    /// with them (SOGo lists those the user subscribed to in their home), nor one read-only.
    pub fn own_server_calendars(&self) -> Vec<String> {
        self.calendars.iter().filter(|c| is_server_calendar(&c.id) && !c.foreign && !c.read_only).map(|c| c.id.clone()).collect()
    }

    pub fn default_book(&self) -> Option<&BookInfo> {
        self.books.iter().find(|b| !b.read_only)
    }

    /// Every event and instance with this uid.
    pub fn event_indices(&self, uid: &str) -> Vec<usize> {
        self.events.iter().enumerate().filter(|(_, e)| e.event.uid == uid).map(|(i, _)| i).collect()
    }
}

/// A calendar kept on the account's server (CalDAV, Exchange): not the one on this computer,
/// one subscribed to by its link, or someone else's opened by their address.
pub fn is_server_calendar(id: &str) -> bool {
    id != LOCAL && !is_web_calendar(id) && shared_calendar_owner(id).is_none() && !id.is_empty()
}

/// The calendar new events go to among these (see [`Pim::default_calendar`]).
pub fn pick_calendar<'a>(calendars: &'a [CalendarInfo], chosen: Option<&str>) -> Option<&'a CalendarInfo> {
    let writable = |c: &&CalendarInfo| !c.read_only && !is_web_calendar(&c.id);
    if let Some(c) = chosen.and_then(|id| calendars.iter().filter(writable).find(|c| c.id == id)) {
        return Some(c);
    }
    let own = |c: &&CalendarInfo| writable(c) && !c.foreign && shared_calendar_owner(&c.id).is_none();
    let personal = |c: &&CalendarInfo| c.id.trim_end_matches('/').rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case("personal"));
    calendars
        .iter()
        .filter(own)
        .find(personal)
        .or_else(|| calendars.iter().find(own))
        .or_else(|| calendars.iter().find(|c| writable(c) && shared_calendar_owner(&c.id).is_none()))
        .or_else(|| calendars.iter().find(writable))
}

/// Make UIDs unique within each calendar. A calendar can hold several plain (non-recurring)
/// appointments with one UID, e.g. invitations a scheduling tool sends per date under its
/// series UID. The engine stores events by UID, so only one survived. Such appointments get
/// `<uid>/<start>` here. Recurring masters and exceptions are untouched, and the new UID never
/// goes back to the server.
pub fn separate_shared_uids(events: &mut [StoredEvent]) {
    let mut seen: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
    for e in events.iter().filter(|e| e.event.recurrence_id.is_none()) {
        *seen.entry((e.calendar.clone(), e.event.uid.clone())).or_default() += 1;
    }
    let mut taken: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for e in events.iter_mut() {
        let shared = seen.get(&(e.calendar.clone(), e.event.uid.clone())).is_some_and(|n| *n > 1);
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

#[cfg(test)]
mod shared_uid_tests {
    use super::*;

    /// Duplicates get their start appended (a counter when the start repeats too); a lone
    /// event and a recurring one in another calendar keep their UID.
    #[test]
    fn repeated_uids_get_their_start_appended() {
        let ev = |cal: &str, start: i64, rrule: Option<&str>| StoredEvent {
            calendar: cal.into(),
            event: CalEvent { uid: "series-1".into(), start, rrule: rrule.map(str::to_string), ..Default::default() },
            ..Default::default()
        };
        let mut list = vec![ev("cal", 100, None), ev("cal", 200, None), ev("cal", 100, None), ev("other", 100, Some("FREQ=WEEKLY"))];
        separate_shared_uids(&mut list);
        let uids: Vec<&str> = list.iter().map(|e| e.event.uid.as_str()).collect();
        assert_eq!(uids, ["series-1/100", "series-1/200", "series-1/100-2", "series-1"]);
        // Nothing to tell apart: unchanged.
        let mut one = vec![ev("cal", 100, None)];
        separate_shared_uids(&mut one);
        assert_eq!(one[0].event.uid, "series-1");
    }
}

#[cfg(test)]
mod web_calendar_tests {
    use super::*;

    /// Issue #7: the links calendars are published with, and what isn't one.
    #[test]
    fn calendar_links_are_read_as_web_addresses() {
        assert_eq!(calendar_link(" webcal://p01-calendars.example.com/x.ics ").as_deref(), Some("https://p01-calendars.example.com/x.ics"));
        assert_eq!(calendar_link("WEBCALS://cal.example/a?b=1").as_deref(), Some("https://cal.example/a?b=1"));
        assert_eq!(calendar_link("http://192.168.1.10:5232/riley/work/").as_deref(), Some("http://192.168.1.10:5232/riley/work/"));
        assert_eq!(calendar_link("https://[::1]:8443/cal.ics").as_deref(), Some("https://[::1]:8443/cal.ics"));
        assert_eq!(calendar_link("https://cal.example/a.ics#top").as_deref(), Some("https://cal.example/a.ics"));
        for bad in [
            "",
            "webcal://",
            "https:///x",
            "ftp://x.example/a.ics",
            "file:///etc/passwd",
            "https://a@b.example/x",
            "https://x.example/a b",
            "https://:80/x",
            "x.example/a.ics",
        ] {
            assert_eq!(calendar_link(bad), None, "{bad:?}");
        }
        assert_eq!(calendar_link(&format!("https://x.example/{}", "a".repeat(MAX_CALENDAR_LINK))), None);
        assert!(is_web_calendar(&web_calendar_id("https://x.example/a.ics")));
        assert!(!is_web_calendar(LOCAL) && !is_web_calendar(&shared_calendar_id("a@b.example")));
    }
}

#[cfg(test)]
mod default_calendar_tests {
    use super::*;

    fn cal(id: &str, read_only: bool, foreign: bool) -> CalendarInfo {
        CalendarInfo { id: id.into(), name: id.into(), read_only, foreign, ..Default::default() }
    }

    /// Issue #30: the user's choice, else the account's own (SOGo's `personal` first) before
    /// someone else's listed with it, else whatever can be added to; never a read-only one or
    /// one subscribed to by its link.
    #[test]
    fn the_accounts_own_calendar_is_the_default() {
        let list = vec![
            cal("https://d.example/web/", true, false),
            cal("https://d.example/u/sam_team/", false, true),
            cal("https://d.example/u/work/", false, false),
            cal("https://d.example/u/personal/", false, false),
            cal(LOCAL, false, false),
        ];
        let pick = |chosen: Option<&str>| pick_calendar(&list, chosen).map(|c| c.id.clone());
        assert_eq!(pick(None).as_deref(), Some("https://d.example/u/personal/"));
        assert_eq!(pick(Some("https://d.example/u/sam_team/")).as_deref(), Some("https://d.example/u/sam_team/"), "chosen");
        assert_eq!(pick(Some("https://d.example/web/")).as_deref(), Some("https://d.example/u/personal/"), "read-only: not taken");
        assert_eq!(pick(Some("gone")).as_deref(), Some("https://d.example/u/personal/"));
        let theirs = vec![cal("https://d.example/u/sam_team/", false, true), cal(&web_calendar_id("https://x/a.ics"), false, false)];
        assert_eq!(pick_calendar(&theirs, None).map(|c| c.id.as_str()), Some("https://d.example/u/sam_team/"), "only someone else's");
        assert!(pick_calendar(&[cal("https://d.example/ro/", true, false)], None).is_none());
        let p = Pim { calendars: list.clone(), ..Default::default() };
        assert_eq!(p.server_calendar().map(|c| c.id.as_str()), Some("https://d.example/u/personal/"));
        let only_here =
            Pim { calendars: vec![cal(LOCAL, false, false), cal(&shared_calendar_id("a@b.example"), false, false)], ..Default::default() };
        assert!(only_here.server_calendar().is_none());
        assert_eq!(only_here.default_calendar().map(|c| c.id.as_str()), Some(LOCAL));
    }
}
