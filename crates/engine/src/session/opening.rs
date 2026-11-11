//! The session: opening and selecting mail.

use super::*;

mod files;
pub(crate) use files::OpenedFile;
mod selecting;
mod source;
pub use selecting::Toward;
#[cfg(test)]
pub(crate) use source::source_lines;
pub use source::{SOURCE_SHOWN, SourceView};
mod senders;
pub use senders::SenderMarks;
pub(crate) use senders::Senders;
mod prepare;
pub(crate) use prepare::BodyLane;
pub use prepare::{BodyWork, PreparedBody};

impl Session {
    /// A message file read by the network layer (issue #38), to show as `file.open` shows one.
    pub(crate) fn message_file_read(&mut self, path: &str, raw: Result<Vec<u8>, String>) {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
        self.opened_file = Some(match raw {
            Ok(raw) => self.open_message_file(&name, &raw),
            Err(e) => Err(CmdError::Failed(format!("{name} can't be read as a message: {e}"))),
        });
    }

    /// What drops of message files on folders did, since last asked.
    pub fn take_filed(&mut self) -> Vec<Filed> {
        std::mem::take(&mut self.filed)
    }

    pub fn open_view(&mut self, view: View) {
        self.view = view;
        self.search = None;
        self.drop_search();
        self.stop_server_search();
        self.selection.clear();
        self.focus = None;
        self.anchor = None;
        self.reading = None;
        self.leave_reading(None);
        self.refresh_list();
    }

    /// Mark a message read (when it's there and isn't yet).
    pub(super) fn mark_seen(&mut self, key: &MessageKey) {
        if key.folder != OUTBOX && self.summary(key).is_some_and(|m| !m.flags.seen) {
            self.apply_op(&key.account.clone(), Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true });
        }
    }

    /// The reading pane moves on from what it showed (to `next`, or to nothing): one waiting for
    /// that is marked read now, and one waiting for its time no longer is.
    pub(super) fn leave_reading(&mut self, next: Option<&MessageKey>) {
        if self.read_due.as_ref().is_some_and(|(k, _)| Some(k) != next) {
            self.read_due = None;
        }
        if let Some(prev) = self.read_on_leave.take() {
            if Some(&prev) == next {
                self.read_on_leave = Some(prev);
            } else {
                self.mark_seen(&prev);
            }
        }
    }

    /// A message waiting in the reading pane for its time to be marked read, once it's come.
    pub(crate) fn read_when_due(&mut self) {
        let Some((key, at)) = self.read_due.clone() else { return };
        // Its data file had to itself by a compaction: marked once it's given back.
        if self.clock.now >= at && !self.compaction.exclusive(&key.account) {
            self.read_due = None;
            if self.reading.as_ref().is_some_and(|r| r.key == key) {
                self.mark_seen(&key);
            }
        }
    }

    /// How many times a message was shown in the reading pane (each one read from the cache and
    /// parsed): a held arrow key is one per frame, whatever the key repeat rate.
    pub fn messages_shown(&self) -> u64 {
        self.shown
    }

    /// Select one message and show it in the reading pane (marked read as the options say).
    pub fn open_message(&mut self, key: &MessageKey) {
        self.show_message(key, true);
    }

    /// Select one message and show it; `mark`: as the reading options say (else left as it is).
    pub(super) fn show_message(&mut self, key: &MessageKey, mark: bool) {
        self.shown = self.shown.wrapping_add(1);
        let prev = self.reading.as_ref().map(|r| r.key.clone()).filter(|k| k != key);
        if self.reading.as_ref().is_none_or(|r| &r.key != key) {
            self.make_way_for_reading();
        }
        self.leave_reading(Some(key));
        self.selection = vec![key.clone()];
        self.focus = Some(key.clone());
        self.anchor = Some(key.clone());
        let thread = self.thread_of(key);
        self.reading = Some(Reading { key: key.clone(), parsed: None, error: None, thread, warning: None, security: None, text_first: false });
        if self.body.defer {
            // Read and parsed on another thread: the pane waits for it.
            self.defer_read(key);
        } else {
            match self.body_raw(key) {
                Some(raw) => self.show_body(key, &raw),
                None => self.jobs.push(Job::FetchBody { key: key.clone() }),
            }
        }
        let unread = key.folder != OUTBOX && self.summary(key).is_some_and(|m| !m.flags.seen);
        if mark && unread && self.prefs.mark_read_on_open {
            match (self.prefs.read_in_pane, self.prefs.read_after_secs) {
                (ReadInPane::Shown, 0) => self.mark_seen(key),
                (ReadInPane::Shown, secs) => self.read_due = Some((key.clone(), self.clock.now.saturating_add(i64::from(secs.min(3600))))),
                (ReadInPane::Left, _) => self.read_on_leave = Some(key.clone()),
            }
        }
        // The one read before, kept in the list only while it was read (`refresh_list`), goes.
        if let Some(prev) = prev.filter(|k| self.summary(k).is_some_and(|m| !self.filter_keeps(self.filter, m))) {
            self.drop_from_list(&prev);
        }
    }

    /// Open a message in its own window (it stays open while the list changes). Already open:
    /// nothing changes. At most [`MAX_WINDOWS`]; the oldest closes first.
    pub fn pop_out(&mut self, key: &MessageKey) -> CmdResult {
        // An opened file's window, gone: there's nothing to open again.
        if key.account == OPENED_FILE && !self.windows.iter().any(|w| &w.key == key) {
            return Err(CmdError::Params(format!("`{}` is not open", key.to_id())));
        }
        if !self.windows.iter().any(|w| &w.key == key) {
            if self.windows.len() >= MAX_WINDOWS {
                self.windows.remove(0);
            }
            let cached = self.body_raw(key);
            let mut w = Reading { key: key.clone(), parsed: None, error: None, thread: Vec::new(), warning: None, security: None, text_first: false };
            match cached {
                Some(raw) => {
                    let (shown, security) = self.unwrap(key, &raw);
                    w.parsed = self.parse_for_reading(key, &shown);
                    w.error = w.parsed.is_none().then(|| "this message could not be read".to_string());
                    let from = w.parsed.as_ref().map(|p| (p.from.clone(), p.bulk));
                    w.warning = from.and_then(|(from, bulk)| self.message_warning(key, &from, bulk));
                    w.security = security;
                }
                None => self.jobs.push(Job::FetchBody { key: key.clone() }),
            }
            self.windows.push(w);
            self.forget_closed_files();
            // Opened on its own (a double-click): read, whatever the reading pane does.
            self.mark_seen(key);
        }
        Ok(serde_json::json!({"key": key.to_id(), "windows": self.windows.len()}))
    }

    pub fn close_window(&mut self, key: &MessageKey) {
        self.windows.retain(|w| &w.key != key);
        self.forget_closed_files();
    }

    /// A message as it's read: one without a `Date` gets the time the server took it in, as the
    /// list shows it (replies and forwards quoted it as sent on 1 January 1970).
    fn parse_for_reading(&mut self, key: &MessageKey, raw: &[u8]) -> Option<Arc<mime::Parsed>> {
        self.body.parsed_here = self.body.parsed_here.wrapping_add(1);
        let p = mime::parse(raw)?;
        Some(self.for_reading(key, p))
    }

    /// A parsed message as it's read (see [`Session::parse_for_reading`]).
    fn for_reading(&self, key: &MessageKey, mut p: mime::Parsed) -> Arc<mime::Parsed> {
        if p.date == 0
            && let Some(m) = self.summary(key)
        {
            p.date = m.date;
        }
        if key.account == OPENED_FILE {
            files::show_link_targets(&mut p);
        }
        Arc::new(p)
    }

    pub(super) fn show_body(&mut self, key: &MessageKey, raw: &[u8]) {
        // Signed or encrypted: what's inside is what's shown, the protection said beside it.
        let shown = self.reading.as_ref().is_some_and(|r| &r.key == key) || self.windows.iter().any(|w| &w.key == key && w.parsed.is_none());
        let (raw, security) = if shown { self.unwrap(key, raw) } else { (std::borrow::Cow::Borrowed(raw), None) };
        let text_first = mime::is_text_first(&raw);
        self.show_ready(key, &raw, None, text_first, security);
    }

    /// Show a body (its protection taken off already): `parsed`, when it was parsed already
    /// (on another thread), else it's parsed here.
    fn show_ready(&mut self, key: &MessageKey, raw: &[u8], parsed: Option<Option<Arc<mime::Parsed>>>, text_first: bool, security: Option<Security>) {
        // Shown now: the pane waits for it no longer.
        if self.reading.as_ref().is_some_and(|r| &r.key == key) {
            self.stop_waiting(key);
        }
        let mut windows = std::mem::take(&mut self.windows);
        // Parsed once, however many places show it.
        let mut parsed = parsed;
        for w in windows.iter_mut().filter(|w| &w.key == key && (w.parsed.is_none() || w.text_first)) {
            if parsed.is_none() {
                parsed = Some(self.parse_for_reading(key, raw));
            }
            w.parsed = parsed.clone().flatten();
            w.text_first = text_first;
            w.error = w.parsed.is_none().then(|| "this message could not be read".to_string());
            let from = w.parsed.as_ref().map(|p| (p.from.clone(), p.bulk));
            w.warning = from.and_then(|(from, bulk)| self.message_warning(key, &from, bulk));
            w.security = security.clone();
        }
        self.windows = windows;
        // Only the open message is laid out (bodies downloaded ahead aren't, each costing time).
        if self.reading.as_ref().is_some_and(|r| &r.key == key)
            && let parsed = parsed.unwrap_or_else(|| self.parse_for_reading(key, raw))
            && let Some(r) = self.reading.as_mut()
        {
            r.error = if parsed.is_none() { Some("this message could not be read".into()) } else { None };
            r.parsed = parsed;
            r.security = security;
            r.text_first = text_first;
        }
        let from = self.reading.as_ref().filter(|r| &r.key == key).and_then(|r| r.parsed.as_ref()).map(|p| (p.from.clone(), p.bulk));
        if let Some((from, bulk)) = from {
            let warning = self.message_warning(key, &from, bulk);
            if let Some(r) = self.reading.as_mut() {
                r.warning = warning;
            }
        }
        self.request_pictures();
    }
}
