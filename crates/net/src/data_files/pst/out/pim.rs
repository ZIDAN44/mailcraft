//! Contacts, contact groups, calendar items, tasks, notes and journal entries as they come in.
//! A repeating appointment's changed occurrences come in as events of their own, each replacing
//! its occurrence (`RECURRENCE-ID`). The files of a contact, calendar item or task come in on its
//! card or event (`pst_pim::files`); a note and a journal entry have nowhere to keep one. The
//! account's contacts and calendar are one document, written whole at each change and each
//! checkpoint, so the files kept there are held to [`MAX_FILE`] each and [`MAX_FILES`] in all, and
//! those past either are counted as too big.

use super::super::item::EXCEPTION;
use super::*;

/// A file of a contact, calendar item or task kept at most, as its line holds it (base64)…
const MAX_FILE: usize = 4 << 20;
/// …and all of them in the account's contacts and calendar.
const MAX_FILES: usize = 24 << 20;

impl Out<'_> {
    /// A contact, contact group, calendar item, note or journal entry (anything else in their
    /// folders is left out).
    pub(super) fn pim_of(&mut self, mut it: Item) {
        let Some(x) = it.extra.take() else { return };
        let n = self.pim.events.len() + self.pim.cards.len() + self.pim.notes.len();
        let uid = format!("pst-{}-{n}@mailcraft.invalid", self.account);
        let class = it.class.to_ascii_lowercase();
        let files = std::mem::take(&mut it.files);
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
            let mut card = crate::pst_pim::group_of(&x, &it.subject, &it.body, uid.clone());
            card.extra.extend(self.card_files(files, None, &uid));
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
            if let Some(mut event) = crate::pst_pim::event_of(&day, &summary, &it.body, &EmailAddress::default(), &[], uid.clone()) {
                let lines = self.event_files(files, &uid);
                with_files(&mut event, lines);
                self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event, ..Default::default() });
            }
        } else if class.starts_with("ipm.contact") {
            let mut card = crate::pst_pim::card_of(&x, &it.body, uid.clone());
            card.extra.extend(self.card_files(files, it.photo.take(), &uid));
            self.pim.cards.push(StoredCard { book: LOCAL.into(), card, ..Default::default() });
        } else {
            let people: Vec<EmailAddress> = it.to.iter().chain(&it.cc).cloned().collect();
            let Some(mut event) = crate::pst_pim::event_of(&x, &it.subject, &it.body, &it.from, &people, uid.clone()) else {
                self.skip(SkipReason::ItemNotUsable, 1);
                return;
            };
            // Its changed occurrences are attached to it as items of their own; its files besides.
            let (own, files): (Vec<File>, Vec<File>) =
                files.into_iter().partition(|f| matches!(f, File::Message(m) if m.class.eq_ignore_ascii_case(EXCEPTION)));
            let own: Vec<Item> = own.into_iter().filter_map(|f| if let File::Message(m) = f { Some(*m) } else { None }).collect();
            let lines = self.event_files(files, &uid);
            with_files(&mut event, lines);
            let changed = {
                let guests: Vec<Vec<EmailAddress>> = own.iter().map(|o| o.to.iter().chain(&o.cc).cloned().collect()).collect();
                let none = crate::pst_pim::Extra::default();
                let items: Vec<crate::pst_pim::Own<'_>> = own
                    .iter()
                    .zip(&guests)
                    .map(|(o, guests)| crate::pst_pim::Own { x: o.extra.as_ref().unwrap_or(&none), body: &o.body, people: guests })
                    .collect();
                crate::pst_pim::changed_events(&x, &it.subject, &it.body, &it.from, &people, &uid, &items)
            };
            self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event, ..Default::default() });
            // Each with the files of its own item.
            let mut own: Vec<Option<Item>> = own.into_iter().map(Some).collect();
            for (mut e, found) in changed {
                if let Some(item) = found.and_then(|i| own.get_mut(i)).and_then(Option::take) {
                    let lines = self.event_files(item.files, &uid);
                    with_files(&mut e, lines);
                }
                self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event: e, ..Default::default() });
            }
        }
    }

    /// An event's files as its lines, those that fit.
    fn event_files(&mut self, files: Vec<File>, uid: &str) -> Vec<String> {
        let files: Vec<_> = files.into_iter().enumerate().filter_map(|(i, f)| self.file_of(f, &format!("{i}.{uid}"))).collect();
        files.into_iter().filter_map(|(name, mime, data)| self.keep_file(&data, || crate::pst_pim::event_file(&name, &mime, &data))).collect()
    }

    /// A card's picture and files as its lines, those that fit.
    fn card_files(&mut self, files: Vec<File>, photo: Option<(String, Vec<u8>)>, uid: &str) -> Vec<String> {
        let photo = photo.and_then(|(mime, data)| self.keep_file(&data, || crate::pst_pim::card_photo(&mime, &data)));
        let files: Vec<_> = files.into_iter().enumerate().filter_map(|(i, f)| self.file_of(f, &format!("{i}.{uid}"))).collect();
        photo
            .into_iter()
            .chain(files.into_iter().filter_map(|(name, mime, data)| self.keep_file(&data, || crate::pst_pim::card_file(&name, &mime, &data))))
            .collect()
    }

    /// A file's name, type and bytes; an attached message's are its `.eml` (made with
    /// `fallback_id` when it has no id of its own).
    fn file_of(&mut self, f: File, fallback_id: &str) -> Option<(String, String, Vec<u8>)> {
        match f {
            File::Data { name, mime, data, .. } => Some((name, mime, data)),
            File::Message(inner) => {
                let subject = if inner.subject.trim().is_empty() { "message".to_string() } else { inner.subject.trim().to_string() };
                match super::super::rebuild::to_raw(*inner, fallback_id) {
                    Ok(raw) => Some((format!("{subject}.eml"), "message/rfc822".to_string(), raw)),
                    Err(e) => {
                        log::info!("a message attached to a contact or calendar item could not be rebuilt: {e}");
                        self.lose(SkipReason::AttachedMessageUnreadable, 1);
                        None
                    }
                }
            }
        }
    }

    /// The line holding a file of bytes `data` (made by `line`, once it's known to fit in what the
    /// contacts and calendar keep of files); else it's counted as too big.
    fn keep_file(&mut self, data: &[u8], line: impl FnOnce() -> String) -> Option<String> {
        // Counted once, from what's there (an import gone on with has some already).
        let kept = *self.pim_files.get_or_insert_with(|| {
            let cards = self.pim.cards.iter().flat_map(|c| &c.card.extra);
            let events = self.pim.events.iter().flat_map(|e| &e.event.extra);
            cards.chain(events).filter(|l| crate::pst_pim::is_file_line(l)).map(String::len).fold(0, usize::saturating_add)
        });
        let fits = |n: usize| n <= MAX_FILE && kept.saturating_add(n) <= MAX_FILES;
        // Its bytes in base64, before they're written out.
        let line = fits(data.len().div_ceil(3).saturating_mul(4)).then(line).filter(|l| fits(l.len()));
        match &line {
            Some(l) => self.pim_files = Some(kept.saturating_add(l.len())),
            None => {
                log::info!("a file of {} bytes on a contact or calendar item was left out", data.len());
                self.lose(SkipReason::AttachmentTooBig, 1);
            }
        }
        line
    }
}

/// `lines` (an event's files) into the event, before any part of it (its reminder).
fn with_files(e: &mut mailcraft_model::ical::CalEvent, lines: Vec<String>) {
    let at = e.extra.iter().position(|l| l.trim_start().get(..6).is_some_and(|b| b.eq_ignore_ascii_case("BEGIN:"))).unwrap_or(e.extra.len());
    e.extra.splice(at..at, lines);
}
