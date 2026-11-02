//! Contacts, contact groups, calendar items, tasks, notes and journal entries as they come in.
//! A repeating appointment's changed occurrences come in as events of their own, each replacing
//! its occurrence (`RECURRENCE-ID`).

use super::super::item::EXCEPTION;
use super::*;

impl Out<'_> {
    /// A contact, contact group, calendar item, note or journal entry (anything else in their
    /// folders is left out).
    pub(super) fn pim_of(&mut self, mut it: Item) {
        let Some(x) = it.extra.take() else { return };
        let n = self.pim.events.len() + self.pim.cards.len() + self.pim.notes.len();
        let uid = format!("pst-{}-{n}@mailcraft.invalid", self.account);
        let class = it.class.to_ascii_lowercase();
        if class.starts_with("ipm.activity") {
            // MailCraft has no journal: an entry is a note.
            let text = crate::pst_pim::journal_text(&x, &it.subject, &it.body);
            if text.is_empty() {
                self.skip(SkipReason::ItemNotUsable, 1);
                return;
            }
            let note = Note { uid, text, color: NOTE_YELLOW, modified: x.modified.unwrap_or(it.date) };
            self.pim.notes.push(StoredNote { note, ..Default::default() });
        } else if class.starts_with("ipm.distlist") {
            let card = crate::pst_pim::group_of(&x, &it.subject, &it.body, uid);
            self.pim.cards.push(StoredCard { book: LOCAL.into(), card, ..Default::default() });
        } else if class.starts_with("ipm.stickynote") {
            // A note's text is its body (its subject is the first line of it).
            let text = if it.body.trim().is_empty() { it.subject.clone() } else { it.body.replace("\r\n", "\n") };
            if text.trim().is_empty() {
                self.skip(SkipReason::ItemNotUsable, 1);
                return;
            }
            let note =
                Note { uid, text: text.trim_end().to_string(), color: x.note_color.unwrap_or(NOTE_YELLOW), modified: x.modified.unwrap_or(it.date) };
            self.pim.notes.push(StoredNote { note, ..Default::default() });
        } else if class.starts_with("ipm.task") {
            // MailCraft has no task list: a task with a due date shows in the calendar that day.
            let Some(due) = x.due else {
                self.skip(SkipReason::ItemNotUsable, 1);
                return;
            };
            let day = crate::pst_pim::Extra { start: Some(due), end: Some(due), all_day: true, ..Default::default() };
            let summary = if x.done { format!("✓ {}", it.subject) } else { it.subject.clone() };
            if let Some(event) = crate::pst_pim::event_of(&day, &summary, &it.body, &EmailAddress::default(), &[], uid) {
                self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event, ..Default::default() });
            }
        } else if class.starts_with("ipm.contact") {
            let card = crate::pst_pim::card_of(&x, &it.body, uid);
            self.pim.cards.push(StoredCard { book: LOCAL.into(), card, ..Default::default() });
        } else {
            let people: Vec<EmailAddress> = it.to.iter().chain(&it.cc).cloned().collect();
            let Some(event) = crate::pst_pim::event_of(&x, &it.subject, &it.body, &it.from, &people, uid.clone()) else {
                self.skip(SkipReason::ItemNotUsable, 1);
                return;
            };
            // Its changed occurrences are attached to it as items of their own.
            let own: Vec<&Item> = it
                .files
                .iter()
                .filter_map(|f| match f {
                    File::Message(m) if m.class.eq_ignore_ascii_case(EXCEPTION) => Some(&**m),
                    _ => None,
                })
                .collect();
            let guests: Vec<Vec<EmailAddress>> = own.iter().map(|o| o.to.iter().chain(&o.cc).cloned().collect()).collect();
            let none = crate::pst_pim::Extra::default();
            let items: Vec<crate::pst_pim::Own<'_>> = own
                .iter()
                .zip(&guests)
                .map(|(o, guests)| crate::pst_pim::Own { x: o.extra.as_ref().unwrap_or(&none), body: &o.body, people: guests })
                .collect();
            let changed = crate::pst_pim::changed_events(&x, &it.subject, &it.body, &it.from, &people, &uid, &items);
            self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event, ..Default::default() });
            for (event, _) in changed {
                self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event, ..Default::default() });
            }
        }
    }
}
