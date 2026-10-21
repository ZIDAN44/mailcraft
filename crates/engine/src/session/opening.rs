//! The session: opening and selecting mail.

use super::*;

impl Session {
    pub fn open_view(&mut self, view: View) {
        self.view = view;
        self.search = None;
        self.index = None;
        self.selection.clear();
        self.focus = None;
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
        if self.clock.now >= at {
            self.read_due = None;
            if self.reading.as_ref().is_some_and(|r| r.key == key) {
                self.mark_seen(&key);
            }
        }
    }

    /// Select one message and show it in the reading pane (marked read as the options say).
    pub fn open_message(&mut self, key: &MessageKey) {
        self.show_message(key, true);
    }

    /// Select one message and show it; `mark`: as the reading options say (else left as it is).
    pub(super) fn show_message(&mut self, key: &MessageKey, mark: bool) {
        self.leave_reading(Some(key));
        self.selection = vec![key.clone()];
        self.focus = Some(key.clone());
        let thread = self.thread_of(key);
        self.reading = Some(Reading { key: key.clone(), parsed: None, error: None, thread });
        let cached = if key.folder == OUTBOX {
            self.outbox_entry(key).and_then(|(a, i)| self.store.as_ref().and_then(|s| s.outbox_raw(&a, i.id)))
        } else {
            self.store.as_ref().and_then(|s| s.read_body(key))
        };
        match cached {
            Some(raw) => self.show_body(key, &raw),
            None => self.jobs.push(Job::FetchBody { key: key.clone() }),
        }
        let unread = key.folder != OUTBOX && self.summary(key).is_some_and(|m| !m.flags.seen);
        if mark && unread && self.prefs.mark_read_on_open {
            match (self.prefs.read_in_pane, self.prefs.read_after_secs) {
                (ReadInPane::Shown, 0) => self.mark_seen(key),
                (ReadInPane::Shown, secs) => self.read_due = Some((key.clone(), self.clock.now.saturating_add(i64::from(secs.min(3600))))),
                (ReadInPane::Left, _) => self.read_on_leave = Some(key.clone()),
            }
        }
    }

    /// Open a message in its own window (it stays open while the list changes). Already open:
    /// nothing changes. At most [`MAX_WINDOWS`]; the oldest closes first.
    pub fn pop_out(&mut self, key: &MessageKey) -> CmdResult {
        if !self.windows.iter().any(|w| &w.key == key) {
            if self.windows.len() >= MAX_WINDOWS {
                self.windows.remove(0);
            }
            let cached = if key.folder == OUTBOX {
                self.outbox_entry(key).and_then(|(a, i)| self.store.as_ref().and_then(|s| s.outbox_raw(&a, i.id)))
            } else {
                self.store.as_ref().and_then(|s| s.read_body(key))
            };
            let mut w = Reading { key: key.clone(), parsed: None, error: None, thread: Vec::new() };
            match cached {
                Some(raw) => {
                    w.parsed = mime::parse(&raw);
                    w.error = w.parsed.is_none().then(|| "this message could not be read".to_string());
                }
                None => self.jobs.push(Job::FetchBody { key: key.clone() }),
            }
            self.windows.push(w);
            // Opened on its own (a double-click): read, whatever the reading pane does.
            self.mark_seen(key);
        }
        Ok(serde_json::json!({"key": key.to_id(), "windows": self.windows.len()}))
    }

    pub fn close_window(&mut self, key: &MessageKey) {
        self.windows.retain(|w| &w.key != key);
    }

    pub(super) fn show_body(&mut self, key: &MessageKey, raw: &[u8]) {
        for w in self.windows.iter_mut().filter(|w| &w.key == key && w.parsed.is_none()) {
            w.parsed = mime::parse(raw);
            w.error = w.parsed.is_none().then(|| "this message could not be read".to_string());
        }
        // Only the open message is laid out (bodies downloaded ahead aren't, each costing time).
        if let Some(r) = self.reading.as_mut()
            && &r.key == key
        {
            let parsed = mime::parse(raw);
            r.error = if parsed.is_none() { Some("this message could not be read".into()) } else { None };
            r.parsed = parsed;
        }
        self.request_pictures();
    }

    /// A body that came in joins the search index there is (rather than throwing it away).
    pub(super) fn index_body(&mut self, key: &MessageKey, raw: &[u8]) {
        // No index yet: read when one is built (downloading ahead costs nothing extra then).
        if self.index.is_none() {
            return;
        }
        let words: Arc<str> = Arc::from(mime::search_text(raw));
        self.keep_words(key, words.clone());
        if let Some(ix) = self.index.as_mut()
            && let Some(doc) = ix.keys.iter().position(|k| k == key)
            && ix.has_body.get(doc) == Some(&false)
        {
            ix.index.add(u32::try_from(doc).unwrap_or(u32::MAX), &words);
            if let Some(b) = ix.has_body.get_mut(doc) {
                *b = true;
            }
        }
    }
}
