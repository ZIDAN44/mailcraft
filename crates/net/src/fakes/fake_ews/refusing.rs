//! What the fake server won't give of calendars, contacts and notes, as Exchange answers.

use super::*;

impl Data {
    /// Why a calendar, contacts or notes item isn't given: one named in `unreadable` (the server
    /// fails on it), or one asked for a property its kind hasn't (an appointment's recurrence
    /// asked of a contact: Exchange answers `ErrorInvalidPropertyRequest` for each such item).
    pub(super) fn refuses_item(&self, op: &El, id: &str, item: &El) -> Option<&'static str> {
        if self.unreadable.iter().any(|u| u == id) {
            return Some("ErrorInternalServerError");
        }
        // A series of so many occurrences, every one deleted: Exchange won't give it (issue #229).
        let count = item.find("NumberOfOccurrences").and_then(|n| n.text.trim().parse::<usize>().ok());
        let deleted = item.child("DeletedOccurrences").map_or(0, |d| d.children("DeletedOccurrence").count());
        if count.is_some_and(|n| deleted >= n) {
            return Some("ErrorRecurrenceHasNoOccurrence");
        }
        let mut fields = Vec::new();
        op.all("FieldURI", &mut fields);
        let calendar = fields.iter().any(|f| f.attr("FieldURI").is_some_and(|u| u.starts_with("calendar:")));
        (calendar && item.name != "CalendarItem").then_some("ErrorInvalidPropertyRequest")
    }

    /// Whether listing a calendar or contacts folder is refused (its id is in `unreadable`).
    pub(super) fn refuses_folder(&self, folder: &str) -> bool {
        self.unreadable.iter().any(|u| u == folder)
    }
}
