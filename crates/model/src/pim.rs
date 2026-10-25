//! What an account keeps of its calendars and address books: the server's (CalDAV, CardDAV)
//! with what's needed to sync them, or ones kept on this computer only.

use serde::{Deserialize, Serialize};

use crate::ical::CalEvent;
use crate::vcard::Card;

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
    /// Notes (Outlook's Notes): the server's (Exchange) and those kept here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<StoredNote>,
    /// Where the server keeps the user's calendars and address books (found once); empty when
    /// it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_home: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carddav_home: Option<String>,
    /// When the server was last asked where they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dav_checked: Option<i64>,
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
}

impl Pim {
    /// Make sure there is somewhere to put events and people.
    pub fn ensure_defaults(&mut self) {
        if self.calendars.is_empty() {
            self.calendars.push(CalendarInfo { id: LOCAL.into(), name: "Calendar".into(), color: 0, read_only: false, ctag: String::new() });
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
                self.calendars.insert(0, CalendarInfo { id: LOCAL.into(), name: "Calendar".into(), color: 0, read_only: false, ctag: String::new() });
            }
            if !self.books.iter().any(|b| b.id == LOCAL) {
                self.books.insert(0, BookInfo { id: LOCAL.into(), name: "Contacts".into(), read_only: false, ctag: String::new() });
            }
        }
        self.caldav_home = None;
        self.carddav_home = None;
        self.dav_checked = None;
    }

    /// The first calendar events can be added to.
    pub fn default_calendar(&self) -> Option<&CalendarInfo> {
        self.calendars.iter().find(|c| !c.read_only)
    }

    pub fn default_book(&self) -> Option<&BookInfo> {
        self.books.iter().find(|b| !b.read_only)
    }

    /// Every event and instance with this uid.
    pub fn event_indices(&self, uid: &str) -> Vec<usize> {
        self.events.iter().enumerate().filter(|(_, e)| e.event.uid == uid).map(|(i, _)| i).collect()
    }
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
