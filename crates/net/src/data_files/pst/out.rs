//! What an import has written so far: folders, their lists, message sources and contacts.

use super::item::Item;
use super::rebuild::{summary_of, to_raw};
use super::*;

/// Message sources written to the cache together (one transaction): at most this many…
const BATCH_MESSAGES: usize = 2000;
/// …or this many bytes.
const BATCH_BYTES: usize = 16 << 20;

/// A folder being read.
pub(super) struct Open {
    uid_next: u32,
    /// Messages whose sources may not be written yet (they go to the list once they are).
    waiting: Vec<mailcraft_model::Summary>,
    list: mailcraft_store::ListWriter,
}

/// What has been written so far.
pub(super) struct Out<'a> {
    pub(super) store: &'a Store,
    pub(super) account: &'a str,
    pub(super) folders: Vec<Folder>,
    /// The folders being read: their next number, and their list going out a page at a time
    /// (a folder of hundreds of thousands isn't held whole; written ones keep only their counts).
    pub(super) states: HashMap<String, Open>,
    pub(super) imported: usize,
    /// Items left out, in all and by reason.
    pub(super) skipped: usize,
    pub(super) skips: BTreeMap<SkipReason, usize>,
    pub(super) charset_unsupported: usize,
    pub(super) progress: &'a mut dyn FnMut(usize),
    /// Contacts and calendar items read so far.
    pub(super) pim: Pim,
    /// Sources read but not yet written, and their size.
    pub(super) pending: Vec<(MessageKey, Vec<u8>)>,
    pub(super) pending_bytes: usize,
}

impl Out<'_> {
    /// Count `n` items left out for `reason`.
    pub(super) fn skip(&mut self, reason: SkipReason, n: usize) {
        self.skipped = self.skipped.saturating_add(n);
        let c = self.skips.entry(reason).or_default();
        *c = c.saturating_add(n);
    }

    /// A folder for `name` under `parent`, with a role when it's a usual top-level one.
    pub(super) fn add_folder(&mut self, parent: Option<&str>, name: &str) -> String {
        let name = name.trim().replace('/', "∕");
        let name = if name.is_empty() { "Folder".to_string() } else { name };
        let mut role = match parent {
            None => FolderRole::from_name(&name, None),
            Some(_) => FolderRole::Other,
        };
        if role != FolderRole::Other && self.folders.iter().any(|f| f.role == role) {
            role = FolderRole::Other;
        }
        let base = match (role, parent) {
            (FolderRole::Inbox, _) => "INBOX".to_string(),
            (_, Some(p)) => format!("{p}/{}", utf7::encode(&name)),
            (_, None) => utf7::encode(&name),
        };
        let mut path = base.clone();
        let mut n = 2;
        while self.folders.iter().any(|f| f.path.eq_ignore_ascii_case(&path)) {
            path = format!("{base} ({n})");
            n += 1;
        }
        self.folders.push(Folder {
            account: self.account.to_string(),
            path: path.clone(),
            delimiter: Some('/'),
            role,
            selectable: true,
            ..Default::default()
        });
        self.states.insert(path.clone(), Open { uid_next: 1, waiting: Vec::new(), list: self.store.list_writer(self.account, &path) });
        path
    }

    /// A contact, calendar item or note (anything else in their folders is left out).
    pub(super) fn add_pim(&mut self, it: Item) {
        let Some(x) = &it.extra else { return };
        let n = self.pim.events.len() + self.pim.cards.len() + self.pim.notes.len();
        let uid = format!("pst-{}-{n}@mailcraft.invalid", self.account);
        if it.class.to_ascii_lowercase().starts_with("ipm.stickynote") {
            // A note's text is its body (its subject is the first line of it).
            let text = if it.body.trim().is_empty() { it.subject.clone() } else { it.body.replace("\r\n", "\n") };
            if text.trim().is_empty() {
                self.skip(SkipReason::ItemNotUsable, 1);
                return;
            }
            let note =
                Note { uid, text: text.trim_end().to_string(), color: x.note_color.unwrap_or(NOTE_YELLOW), modified: x.modified.unwrap_or(it.date) };
            self.pim.notes.push(StoredNote { note, ..Default::default() });
        } else if it.class.to_ascii_lowercase().starts_with("ipm.task") {
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
        } else if it.class.to_ascii_lowercase().starts_with("ipm.contact") {
            let card = crate::pst_pim::card_of(x, &it.body, uid);
            self.pim.cards.push(StoredCard { book: LOCAL.into(), card, ..Default::default() });
        } else {
            let people: Vec<EmailAddress> = it.to.iter().chain(&it.cc).cloned().collect();
            match crate::pst_pim::event_of(x, &it.subject, &it.body, &it.from, &people, uid) {
                Some(event) => self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event, ..Default::default() }),
                None => self.skip(SkipReason::ItemNotUsable, 1),
            }
        }
    }

    pub(super) fn add_message(&mut self, path: &str, it: Item) {
        // Mail only: meeting requests and receipts are mail too; appointments, contacts and
        // tasks in a mail folder aren't.
        let class = it.class.to_ascii_lowercase();
        if !(class.is_empty() || class.starts_with("ipm.note") || class.starts_with("ipm.schedule") || class.starts_with("report.")) {
            return;
        }
        let Some(st) = self.states.get_mut(path) else { return };
        let uid = st.uid_next;
        let fallback = format!("pst.{}.{}.{uid}@mailcraft.invalid", self.account, self.imported);
        let raw = match to_raw(&it, &fallback) {
            Ok(r) => r,
            Err(e) => {
                log::info!("a message could not be rebuilt: {e}");
                self.skip(SkipReason::MessageNotRebuilt, 1);
                return;
            }
        };
        if it.charset_unsupported {
            self.charset_unsupported += 1;
        }
        let key = MessageKey { account: self.account.to_string(), folder: path.to_string(), validity: 1, uid };
        st.uid_next = uid.saturating_add(1);
        let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        st.waiting.push(summary_of(&raw, &it, key.clone(), size));
        let page_full = st.waiting.len() >= mailcraft_store::PAGE_MESSAGES;
        self.imported += 1;
        self.pending_bytes += raw.len();
        self.pending.push((key, raw));
        if page_full || self.pending.len() >= BATCH_MESSAGES || self.pending_bytes >= BATCH_BYTES {
            self.flush();
        }
        // A page's worth whose sources are written: into the folder's list.
        if page_full {
            self.list_out(path);
        }
        if self.imported.is_multiple_of(200) {
            (self.progress)(self.imported);
        }
    }

    /// Write the sources read so far, together. A batch that can't be written leaves its
    /// messages out of their folders' lists (counted as skipped), as one failed write did.
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        self.pending_bytes = 0;
        if let Err(e) = self.store.write_bodies(&batch) {
            log::warn!("import: {e}");
            for (key, _) in &batch {
                if let Some(st) = self.states.get_mut(&key.folder) {
                    st.waiting.retain(|m| m.key.uid != key.uid);
                }
            }
            self.imported = self.imported.saturating_sub(batch.len());
            self.skip(SkipReason::NotWritten, batch.len());
        }
    }

    /// Hand a folder's messages whose sources are written to its list (full pages go out).
    fn list_out(&mut self, path: &str) {
        let Some(st) = self.states.get_mut(path) else { return };
        let ready = std::mem::take(&mut st.waiting);
        if let Err(e) = st.list.push(self.store, ready) {
            log::warn!("import: {e}");
        }
    }

    /// Write a finished folder's list and counts, then let go of it (only the counts are needed
    /// from here).
    pub(super) fn save_folder(&mut self, path: &str) {
        self.flush();
        self.list_out(path);
        let Some(st) = self.states.remove(path) else { return };
        let head = FolderState { validity: 1, uid_next: st.uid_next, ..Default::default() };
        let (unread, total) = match st.list.finish(self.store, &head) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("import: {e}");
                (0, 0)
            }
        };
        if let Some(f) = self.folders.iter_mut().find(|f| f.path == path) {
            f.unread = unread;
            f.total = total;
        }
    }

    pub(super) fn finish(mut self) -> Result<Imported, String> {
        self.flush();
        // An Inbox to open, even when the file had none.
        if !self.folders.iter().any(|f| f.role == FolderRole::Inbox) {
            let path = self.add_folder(None, "Inbox");
            self.save_folder(&path);
        }
        // Folders left part-way (the file ended early) still get their list.
        let unsaved: Vec<String> = self.states.keys().cloned().collect();
        for p in unsaved {
            self.save_folder(&p);
        }
        self.store.save_folders(self.account, &self.folders).map_err(|e| e.to_string())?;
        let (contacts, events, notes) = (self.pim.cards.len(), self.pim.events.len(), self.pim.notes.len());
        if contacts + events + notes > 0 {
            self.pim.ensure_defaults();
            self.store.save_account_doc(self.account, PIM_DOC, &self.pim).map_err(|e| e.to_string())?;
        }
        if self.charset_unsupported > 0 {
            log::info!("{} messages are in a code page MailCraft can't read (ISO-2022-KR): only their ASCII was kept", self.charset_unsupported);
        }
        Ok(Imported {
            folders: self.folders,
            messages: self.imported,
            contacts,
            events,
            notes,
            skipped: self.skipped,
            skips: self.skips,
            charset_unsupported: self.charset_unsupported,
        })
    }
}
