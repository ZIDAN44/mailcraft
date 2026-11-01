//! The session: the open message's body got ready away from the window's thread. Reading the
//! biggest message from the cache and parsing it took 0.1 s or more, a frame the window couldn't
//! draw; where the window has a thread for it (`MailApp` says so), showing a message only asks
//! for that work ([`Session::take_body_work`]), the reading pane waits, and the work's result is
//! shown when it comes back ([`Session::finish_body`]). The CLI, `--cmd` and the tests read and
//! parse at once, as before. A command other than moving about the list finishes the work first
//! ([`Session::finish_pending_body`]), so what it acts on is the message as shown.

use super::*;

/// What the reading pane waits for: the work handed out (or still to be), latest only.
#[derive(Default)]
pub(crate) struct BodyLane {
    /// Bodies are got ready away from the window's thread.
    pub(crate) defer: bool,
    /// Work not handed out yet.
    work: Option<BodyWork>,
    /// The work the reading pane waits for (a copy of it, done here if a command can't wait).
    waiting: Option<BodyWork>,
    /// Messages parsed to be shown on the session's own thread ([`Session::bodies_parsed_here`]).
    pub(crate) parsed_here: u64,
}

/// A message's body to get ready to read: read from the cache (or as it arrived), then parsed,
/// unless it's signed or encrypted (taking that off needs the session's keys). Runs on any
/// thread.
#[derive(Clone)]
pub struct BodyWork {
    key: MessageKey,
    store: Option<Store>,
    /// An Outbox message: its account and Outbox id.
    outbox: Option<(String, u64)>,
    /// The body as it arrived: parsed only.
    raw: Option<Arc<[u8]>>,
    /// A test's parse that panics.
    #[cfg(test)]
    pub(crate) breaks: bool,
}

/// What [`BodyWork::run`] made of a body.
pub struct PreparedBody {
    key: MessageKey,
    /// `None`: not in the cache (it's downloaded).
    raw: Option<Arc<[u8]>>,
    /// `None` with `raw`: it couldn't be read (or wasn't parsed, being protected).
    parsed: Option<Parsed>,
    protected: bool,
    text_first: bool,
    /// The work failed (its parse panicked).
    broken: bool,
}

impl BodyWork {
    pub fn key(&self) -> &MessageKey {
        &self.key
    }

    /// Read the body (or take the one given), and parse it when it isn't protected. A panic
    /// (hostile mail the parser trips on) comes back as a body that couldn't be read.
    pub fn run(self) -> PreparedBody {
        let key = self.key.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || self.prepare())) {
            Ok(p) => p,
            Err(_) => {
                log::warn!("a message couldn't be read (the parser failed)");
                PreparedBody { key, raw: None, parsed: None, protected: false, text_first: false, broken: true }
            }
        }
    }

    fn prepare(self) -> PreparedBody {
        let raw = match (self.raw, &self.store, &self.outbox) {
            (Some(raw), _, _) => Some(raw),
            (None, Some(store), Some((account, id))) => store.outbox_raw(account, *id).map(Arc::from),
            (None, Some(store), None) => store.read_body(&self.key).map(Arc::from),
            (None, None, _) => None,
        };
        let Some(raw) = raw else {
            return PreparedBody { key: self.key, raw: None, parsed: None, protected: false, text_first: false, broken: false };
        };
        #[cfg(test)]
        if self.breaks {
            std::panic::resume_unwind(Box::new("a message that breaks the parser"));
        }
        let protected = mime::smime::is_protected(&raw);
        let parsed = if protected { None } else { mime::parse(&raw) };
        let text_first = mime::is_text_first(&raw);
        PreparedBody { key: self.key, raw: Some(raw), parsed, protected, text_first, broken: false }
    }
}

impl PreparedBody {
    pub fn key(&self) -> &MessageKey {
        &self.key
    }

    /// The message's source, when it was read (what its pictures are taken out of).
    pub fn raw(&self) -> Option<&Arc<[u8]>> {
        self.raw.as_ref()
    }
}

impl Session {
    /// Get bodies ready away from the window's thread from now on (`on`: it has a thread for
    /// them), or at once again.
    pub fn defer_bodies(&mut self, on: bool) {
        self.body.defer = on;
        if !on {
            self.finish_pending_body();
        }
    }

    /// How many messages were parsed to be shown on the session's own thread (the window's,
    /// where bodies are deferred: only signed or encrypted mail and messages' own windows).
    pub fn bodies_parsed_here(&self) -> u64 {
        self.body.parsed_here
    }

    /// Whether the reading pane waits for `key`'s body to be got ready (drawn as a spinner).
    pub fn body_waiting(&self, key: &MessageKey) -> bool {
        self.body.waiting.as_ref().is_some_and(|w| w.key == *key)
    }

    /// The work the reading pane waits for, to run on another thread and give back to
    /// [`Session::finish_body`]. Asked once a frame: only the latest is handed out.
    pub fn take_body_work(&mut self) -> Option<BodyWork> {
        self.body.work.take()
    }

    /// The reading pane waits for `key`'s body, read from the cache: what was waiting before
    /// isn't any more.
    pub(super) fn defer_read(&mut self, key: &MessageKey) {
        let (store, outbox) = if key.folder == OUTBOX {
            match self.outbox_entry(key) {
                Some((account, item)) => (self.store.clone(), Some((account, item.id))),
                None => (None, None),
            }
        } else {
            (self.store.clone(), None)
        };
        self.wait_for(BodyWork {
            key: key.clone(),
            store,
            outbox,
            raw: None,
            #[cfg(test)]
            breaks: false,
        });
    }

    /// A body that arrived for the message in the reading pane, to be parsed away from the
    /// window's thread; `false` when it's shown at once (bodies aren't deferred, or a window
    /// waits for it too, which shows it from the same parse).
    pub(crate) fn defer_arrived(&mut self, key: &MessageKey, raw: &[u8]) -> bool {
        let pane = self.reading.as_ref().is_some_and(|r| r.key == *key && (r.parsed.is_none() || r.text_first));
        let window = self.windows.iter().any(|w| w.key == *key && (w.parsed.is_none() || w.text_first));
        if !self.body.defer || !pane || window {
            return false;
        }
        self.wait_for(BodyWork {
            key: key.clone(),
            store: None,
            outbox: None,
            raw: Some(Arc::from(raw)),
            #[cfg(test)]
            breaks: false,
        });
        true
    }

    fn wait_for(&mut self, work: BodyWork) {
        self.body.waiting = Some(work.clone());
        self.body.work = Some(work);
    }

    /// Nothing waits for a body any more (`key`'s was shown here).
    pub(super) fn stop_waiting(&mut self, key: &MessageKey) {
        if self.body_waiting(key) {
            self.body.waiting = None;
            self.body.work = None;
        }
    }

    /// A body got ready on another thread: shown, when the reading pane still waits for it
    /// (one it moved on from is let go).
    pub fn finish_body(&mut self, p: PreparedBody) {
        if !self.body_waiting(&p.key) {
            return;
        }
        self.body.waiting = None;
        self.body.work = None;
        if !self.reading.as_ref().is_some_and(|r| r.key == p.key) {
            return;
        }
        let key = p.key;
        if p.broken {
            if let Some(r) = self.reading.as_mut() {
                r.error = Some("this message could not be read".into());
            }
            return;
        }
        let Some(raw) = p.raw else {
            self.jobs.push(Job::FetchBody { key });
            return;
        };
        if p.protected {
            // Its protection is taken off here, with the session's keys (rare).
            self.show_body(&key, &raw);
            return;
        }
        let parsed = p.parsed.map(|parsed| self.for_reading(&key, parsed));
        self.show_ready(&key, &raw, Some(parsed), p.text_first, None);
    }

    /// The body the reading pane waits for, got ready here and shown: a command acts on the
    /// message as shown.
    pub fn finish_pending_body(&mut self) {
        let Some(work) = self.body.waiting.clone() else { return };
        self.body.work = None;
        if self.reading.as_ref().is_some_and(|r| r.key == work.key) {
            self.finish_body(work.run());
        } else {
            // The pane moved on from it (another folder): nothing to read.
            self.body.waiting = None;
        }
    }
}
