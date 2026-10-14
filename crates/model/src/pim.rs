//! What an account keeps of its calendars and address books: the server's (CalDAV, CardDAV)
//! with what's needed to sync them, or ones kept on this computer only.

use serde::{Deserialize, Serialize};

use crate::ical::CalEvent;
use crate::vcard::Card;

/// The id of a calendar or address book that lives on this computer only.
pub const LOCAL: &str = "local";

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

/// Something deleted here that is still to be deleted on the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub href: String,
    #[serde(default)]
    pub etag: String,
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
