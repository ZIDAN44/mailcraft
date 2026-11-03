//! An account's worker: opening a message (issue #24). Its text first where the server can give
//! it apart from its files, then the whole of it; and, while the main connection is busy with a
//! sync, on the second connection, so opening a message never waits behind a folder or the
//! calendars.

use super::*;

impl Worker {
    /// Whether the main connection is at work (a sync, a job) rather than waiting for one: a
    /// message opened meanwhile goes to the second connection ([`Pool`] reads it).
    pub(super) fn working(&self, busy: bool) {
        self.busy.store(busy, Ordering::Relaxed);
    }

    /// A message opened: from here, or else from the server; a big message's text first, the
    /// whole of it later ([`Worker::fetch_whole`]).
    pub(super) fn fetch(&mut self, key: MessageKey) {
        // Given back by the second connection: the whole of it, here.
        let whole = self.given_back.lock().is_ok_and(|mut g| g.remove(&key));
        self.fetch_as(key, whole);
    }

    /// On the second connection, a message opened while the main one was busy: its text first
    /// when that's quicker, then the whole of it, both here. `true` when the whole is still to
    /// come (after any other message opened meanwhile).
    pub(super) fn fetch_opened(&mut self, key: MessageKey) -> bool {
        if let Some(raw) = self.store.read_body(&key) {
            self.sink.emit(Event::Body { key, raw });
            return false;
        }
        if self.acc.is_api() {
            self.jstate = jmap::State::load(&self.store, &self.acc.id);
        }
        let store = self.store.clone();
        let text = (|| -> Result<Option<Vec<u8>>> {
            if self.conn.is_none() {
                self.connect()?;
            }
            self.text_first(&store, &key)
        })();
        match text {
            Ok(Some(raw)) => {
                log::info!("{}: message {}: its text first on the second connection ({} bytes)", self.acc.id, key.uid, raw.len());
                self.sink.emit(Event::Body { key, raw });
                return true;
            }
            Ok(None) => {}
            Err(e) => log::info!("{}: the text first: {e}", self.acc.id),
        }
        self.fetch_for_main(key);
        false
    }

    /// A message's text first, when the server gives it apart from its files and the message is
    /// big enough for that to be quicker: `None` otherwise.
    fn text_first(&mut self, store: &Store, key: &MessageKey) -> Result<Option<Vec<u8>>> {
        match self.conn.as_mut() {
            Some(Conn::Ews(c)) => crate::ews::fetch_text(c, &self.jstate, store, key),
            Some(Conn::Graph(c)) => crate::graph::fetch_text(c, &self.jstate, store, key),
            Some(Conn::Imap(c)) => crate::sync::fetch_text(c, store, key),
            _ => Ok(None),
        }
    }

    /// On the second connection: the whole of a message, its text maybe given first (issue
    /// #24), with the message ids the main one keeps; one it can't get goes back there.
    pub(super) fn fetch_for_main(&mut self, key: MessageKey) {
        if let Some(raw) = self.store.read_body(&key) {
            self.sink.emit(Event::Body { key, raw });
            return;
        }
        if self.acc.is_api() {
            self.jstate = jmap::State::load(&self.store, &self.acc.id);
        }
        let (store, began) = (self.store.clone(), Instant::now());
        let r = (|| -> Result<(Vec<u8>, Option<mailcraft_model::Summary>)> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_mut() {
                Some(Conn::Ews(c)) => crate::ews::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Graph(c)) => crate::graph::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Jmap(c)) => jmap::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Imap(c)) => fetch_body(c, &store, &key),
                Some(Conn::Pop) | None => Err(NetError::No("not a connection that downloads one message".into())),
            }
        })();
        match r {
            Ok((raw, filled)) => {
                log::info!(
                    "{}: message {} downloaded on the second connection ({} bytes) in {:.1} s",
                    self.acc.id,
                    key.uid,
                    raw.len(),
                    began.elapsed().as_secs_f32()
                );
                self.sink.emit(Event::Body { key, raw });
                if let Some(summary) = filled {
                    self.sink.emit(Event::Filled { summary: Box::new(summary) });
                }
            }
            Err(e) => {
                log::info!("{}: message {}: left to the main connection: {e}", self.acc.id, key.uid);
                if matches!(e, NetError::Closed(_) | NetError::Io(_)) {
                    self.conn = None;
                }
                if let Some(m) = &self.main {
                    if let Ok(mut g) = self.given_back.lock() {
                        g.insert(key.clone());
                    }
                    let _ = m.send(Job::FetchBody { key });
                }
            }
        }
    }

    /// The whole of a message whose text went first.
    pub(super) fn fetch_whole(&mut self, key: MessageKey) {
        self.fetch_as(key, true);
    }

    fn fetch_as(&mut self, key: MessageKey, whole: bool) {
        if let Some(raw) = self.store.read_body(&key) {
            self.sink.emit(Event::Body { key, raw });
            return;
        }
        let store = self.store.clone();
        let began = Instant::now();
        let r = (|| -> Result<Option<(Vec<u8>, Option<mailcraft_model::Summary>)>> {
            if self.conn.is_none() {
                self.connect()?;
            }
            // A big message's text first, to read while the rest comes (issue #24); the rest on
            // the second connection, or here once nothing else waits.
            if !whole {
                match self.text_first(&store, &key) {
                    Ok(Some(raw)) => {
                        log::info!("{}: message {}: its text first ({} bytes), then the rest", self.acc.id, key.uid, raw.len());
                        self.sink.emit(Event::Body { key: key.clone(), raw });
                        return Ok(None);
                    }
                    Ok(None) => {}
                    Err(e) if e.is_fatal() || matches!(e, NetError::Auth(_) | NetError::Closed(_) | NetError::Io(_)) => return Err(e),
                    Err(e) => log::info!("{}: the text first: {e}", self.acc.id),
                }
            }
            let raw = match self.conn.as_mut() {
                Some(Conn::Jmap(c)) => jmap::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Graph(c)) => crate::graph::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Ews(c)) => crate::ews::fetch_body(c, &self.jstate, &store, &key),
                // POP mail is downloaded whole: what isn't here isn't anywhere.
                Some(Conn::Pop) => {
                    store.read_body(&key).map(|raw| (raw, None)).ok_or_else(|| NetError::No("this message isn't on this computer".into()))
                }
                _ => fetch_body(self.client()?, &store, &key),
            }?;
            Ok(Some(raw))
        })();
        match r {
            // The rest on the second connection, or here once nothing else waits.
            Ok(None) => {
                if let Ok(mut w) = self.whole_only.lock() {
                    w.insert(key.clone());
                }
                let handed = self.quick.lock().is_ok_and(|q| q.as_ref().is_some_and(|q| q.send(Job::FetchBody { key: key.clone() }).is_ok()));
                if !handed {
                    if let Ok(mut w) = self.whole_only.lock() {
                        w.remove(&key);
                    }
                    if !self.rest.contains(&key) {
                        self.rest.push_back(key);
                    }
                }
            }
            // Its summary filled from it, said as it is now: the window takes it into the folder
            // it holds (it read all of the folder again for it, 24 ms on 10,941 messages).
            Ok(Some((raw, filled))) => {
                log::info!("{}: message {} downloaded ({} bytes) in {:.1} s", self.acc.id, key.uid, raw.len(), began.elapsed().as_secs_f32());
                self.sink.emit(Event::Body { key, raw });
                if let Some(summary) = filled {
                    self.sink.emit(Event::Filled { summary: Box::new(summary) });
                }
            }
            Err(e) => {
                self.sink.emit(Event::BodyFailed { key, error: e.to_string() });
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.failed(e);
                }
            }
        }
    }
}
