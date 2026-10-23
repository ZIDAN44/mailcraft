//! An account's worker: syncing folders, searching the server, calendars and contacts, and waiting for new mail.

use super::*;

impl Worker {
    /// Wait for new mail: IDLE on the Inbox when the server supports it, else poll.
    pub(super) fn wait_for_mail(&mut self, rx: &Receiver<Job>) {
        let inbox = self.inbox();
        if matches!(self.conn, Some(Conn::Jmap(_))) && !self.no_push && inbox.is_some() && self.wait_for_push(rx) {
            return;
        }
        let idle = matches!(&self.conn, Some(Conn::Imap(c)) if c.has("IDLE"));
        if !idle || inbox.is_none() {
            let next_soon = self.soon.iter().map(|(at, _)| *at).min();
            let wait = next_soon.map_or(POLL_EVERY, |at| at.saturating_duration_since(Instant::now()).min(POLL_EVERY));
            match rx.recv_timeout(wait) {
                Ok(j) => push_job(&mut self.queue, j),
                Err(RecvTimeoutError::Disconnected) => self.closed = true,
                Err(RecvTimeoutError::Timeout) if next_soon.is_some_and(|at| at <= Instant::now()) => {
                    let now = Instant::now();
                    let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.soon).into_iter().partition(|(at, _)| *at <= now);
                    self.soon = later;
                    let mut folders: Vec<String> = due.into_iter().map(|(_, f)| f).collect();
                    folders.dedup();
                    for f in folders {
                        self.sync_one(&f);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(f) = inbox {
                        self.sync_one(&f);
                    }
                }
            }
            return;
        }
        let Some(inbox) = inbox else { return };
        let result = (|| -> Result<bool> {
            let Some(Conn::Imap(c)) = self.conn.as_mut() else { return Err(NetError::Closed("not connected".into())) };
            // Mail that came after the Inbox was synced and before IDLE starts is never pushed
            // during IDLE: the server already said it (or will only say it to a command). Look
            // now: the Inbox grew since the sync's SELECT, or a command's answer said it did.
            if c.selected() != Some(inbox.as_str()) {
                let synced = c.seen(&inbox).cloned();
                let now = c.select(&inbox, false)?;
                if synced.is_some_and(|s| (s.validity, s.uid_next, s.exists) != (now.validity, now.uid_next, now.exists)) {
                    return Ok(true);
                }
            } else if c.take_news() {
                return Ok(true);
            }
            let queue = &mut self.queue;
            let closed = &mut self.closed;
            c.idle(IDLE_FOR, || {
                loop {
                    match rx.try_recv() {
                        Ok(j) => push_job(queue, j),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            *closed = true;
                            break;
                        }
                    }
                }
                !queue.is_empty() || *closed
            })
        })();
        match result {
            Ok(true) => self.sync_one(&inbox),
            Ok(false) => {}
            Err(e) => self.failed(e),
        }
    }

    /// JMAP push: hold the server's event stream open (as IMAP holds IDLE), checking for jobs
    /// between short reads; a `state` event means something changed. `false` when the server
    /// can't push (the caller polls instead).
    pub(super) fn wait_for_push(&mut self, rx: &Receiver<Job>) -> bool {
        let Some(Conn::Jmap(c)) = &self.conn else { return false };
        let mut es = match c.events() {
            Ok(Some(es)) => es,
            Ok(None) => {
                self.no_push = true;
                return false;
            }
            Err(e @ NetError::Auth(_)) => {
                self.failed(e);
                return true;
            }
            Err(e) => {
                // Push not working (a proxy, an old server): poll from now on.
                log::info!("{}: no JMAP push: {e}", self.acc.id);
                self.no_push = true;
                return false;
            }
        };
        // Mail that came between the last sync and this stream opening sends no event (unless
        // the server replays it, RFC 8620 §7.3): check once now that nothing will be missed.
        if !std::mem::take(&mut self.pushed)
            && let Some(f) = self.inbox()
        {
            self.sync_one(&f);
        }
        let end = Instant::now() + IDLE_FOR;
        let mut changed = false;
        while Instant::now() < end && !changed {
            self.drain(rx);
            if self.closed || !self.queue.is_empty() {
                return true;
            }
            match es.next(Duration::from_millis(500)) {
                Ok(Some(e)) => changed = e.event == "state",
                Ok(None) => {}
                Err(e) => {
                    // Dropped: reopened on the next wait.
                    log::info!("{}: JMAP push ended: {e}", self.acc.id);
                    break;
                }
            }
        }
        if changed && let Some(f) = self.inbox() {
            self.sync_one(&f);
            self.pushed = true;
        }
        true
    }

    pub(super) fn inbox(&self) -> Option<String> {
        self.folders.iter().find(|f| f.role == FolderRole::Inbox).map(|f| f.path.clone())
    }

    /// The folder list, then every folder (Inbox first), staying responsive to opened messages.
    /// Refresh the folder list from the server; `false` when that failed.
    pub(super) fn relist(&mut self) -> bool {
        let id = self.id();
        let listed = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => c.list(&id),
            Some(Conn::Jmap(c)) => jmap::list_folders(c, &mut self.jstate, &id),
            Some(Conn::Graph(c)) => crate::graph::list_folders(c, &mut self.jstate, &id),
            Some(Conn::Ews(c)) => crate::ews::list_folders(c, &mut self.jstate, &id),
            Some(Conn::Pop) => crate::pop::folders(&self.store, &id),
            None => Err(NetError::Closed("not connected".into())),
        };
        let jmap = matches!(self.conn, Some(Conn::Jmap(_) | Conn::Graph(_) | Conn::Ews(_)));
        if jmap {
            self.save_jstate();
        }
        match listed {
            Ok(mut list) => {
                // JMAP lists the counts; IMAP's come from each folder's sync.
                for f in list.iter_mut().filter(|_| !jmap) {
                    if let Some(old) = self.folders.iter().find(|o| o.path == f.path) {
                        f.unread = old.unread;
                        f.total = old.total;
                    }
                }
                sort_folders(&mut list);
                // Folders gone from the server lose their cache.
                for old in &self.folders {
                    if !list.iter().any(|f| f.path == old.path) {
                        let _ = self.store.clear_folder(&id, &old.path);
                    }
                }
                self.folders = list;
                self.publish_folders();
                true
            }
            Err(e) => {
                self.failed(e);
                false
            }
        }
    }

    pub(super) fn sync_all(&mut self, rx: &Receiver<Job>) {
        let began = Instant::now();
        if !self.relist() {
            return;
        }
        let paths: Vec<String> = self.folders.iter().filter(|f| f.selectable).map(|f| f.path.clone()).collect();
        for path in paths {
            self.drain(rx);
            // Body fetches and local edits are served between folders, ahead of the rest.
            let (urgent, rest): (VecDeque<Job>, VecDeque<Job>) = std::mem::take(&mut self.queue).into_iter().partition(between_folders);
            self.queue = rest;
            for j in urgent {
                self.handle(j);
            }
            if self.closed || self.conn.is_none() {
                return;
            }
            self.sync_one(&path);
        }
        // Report Online after mail; CalDAV/EWS calendars and contacts can take far longer and
        // shouldn't hold the status bar on the last folder name.
        if self.conn.is_some() {
            self.report_quota();
            self.prune_jmap_numbers();
            self.last_full = Some(Instant::now());
            // Anything left from a failed attempt gets another try every full sync.
            self.flush_outbox();
            log::info!("{}: synced {} folders in {:.1} s", self.acc.id, self.folders.len(), began.elapsed().as_secs_f32());
            self.status(SyncStatus::Online { at: now() });
        }
        if self.conn.is_some() {
            let began = Instant::now();
            self.sync_dav();
            log::info!("{}: PIM pass took {:?}", self.acc.id, began.elapsed());
        }
    }

    /// Forget the numbers of JMAP messages no longer cached (nor waiting in a change).
    pub(super) fn prune_jmap_numbers(&mut self) {
        if !matches!(self.conn, Some(Conn::Jmap(_) | Conn::Graph(_) | Conn::Ews(_))) {
            return;
        }
        let ops = self.store.load_ops(&self.acc.id).unwrap_or_default();
        let mut keep: std::collections::HashSet<u32> = ops.iter().flat_map(|o| o.keys().iter().map(|k| k.uid).collect::<Vec<_>>()).collect();
        for f in &self.folders {
            if let Ok(st) = self.store.load_folder(&self.acc.id, &f.path) {
                keep.extend(st.messages.iter().map(|m| m.key.uid));
            }
        }
        self.jstate.prune(&keep);
        self.save_jstate();
    }

    pub(super) fn publish_folders(&self) {
        let _ = self.store.save_folders(&self.acc.id, &self.folders);
        self.sink.emit(Event::Folders { account: self.id(), folders: self.folders.clone() });
    }

    /// Search folders on the server (issue #4): IMAP's `SEARCH`, JMAP's `Email/query`, Graph's
    /// `$search` and EWS's `QueryString`, the messages found brought into their folders' lists
    /// and sent as they're found (an IMAP folder a range at a time, the others a folder at a
    /// time). Stops between two when a newer search is asked, or none (the search box cleared).
    pub(super) fn search_server(&mut self, folders: &[String], query: String, search: u64) {
        let id = self.id();
        let (sink, wanted) = (self.sink.clone(), self.search.clone());
        let still = move || wanted.load(Ordering::Relaxed) == search;
        let answer = |keys, error, done| Event::ServerSearched { account: id.clone(), query: query.clone(), keys, error, done };
        if !still() {
            self.sink.emit(answer(Vec::new(), None, true));
            return;
        }
        if self.conn.is_none() {
            self.next_attempt = Instant::now();
            self.connect_and_sync();
        }
        let store = self.store.clone();
        let (imap, filter, words) = (crate::sync::imap_criteria(&query), mailcraft_jmap::search_filter(&query), crate::sync::word_query(&query));
        let asks = match self.conn.as_ref() {
            None => {
                self.sink.emit(answer(Vec::new(), Some("the account isn't connected".into()), true));
                return;
            }
            Some(Conn::Pop) => {
                self.sink.emit(answer(Vec::new(), Some("a POP account keeps all its mail here".into()), true));
                return;
            }
            Some(Conn::Imap(_)) => imap.is_some(),
            Some(Conn::Jmap(_)) => filter.is_some(),
            Some(Conn::Graph(_) | Conn::Ews(_)) => words.is_some(),
        };
        if !asks {
            self.sink.emit(answer(Vec::new(), None, true));
            return;
        }
        let (mut any, mut failed) = (false, None);
        for f in folders.iter().take(200) {
            if !still() {
                break;
            }
            // What a folder finds goes out as it's found.
            let mut sent = |keys: Vec<MessageKey>, new: usize| {
                if new > 0 {
                    sink.emit(Event::FolderChanged { account: id.clone(), folder: f.clone() });
                }
                if !keys.is_empty() {
                    any = true;
                    sink.emit(answer(keys, None, false));
                }
                still()
            };
            let found = match (self.conn.as_mut(), &imap, &filter, &words) {
                (Some(Conn::Imap(c)), Some(q), _, _) => crate::sync::find_on_server(c, &store, &id, f, q, FOUND_MAX, &mut sent).map(|_| ()),
                (Some(Conn::Jmap(c)), _, Some(q), _) => jmap::find_on_server(c, &mut self.jstate, &store, &id, f, q, FOUND_MAX).map(|(keys, new)| {
                    sent(keys, new);
                }),
                (Some(Conn::Graph(c)), _, _, Some(q)) => {
                    crate::graph::find_on_server(c, &mut self.jstate, &store, &id, f, q, FOUND_MAX).map(|(keys, new)| {
                        sent(keys, new);
                    })
                }
                (Some(Conn::Ews(c)), _, _, Some(q)) => {
                    crate::ews::find_on_server(c, &mut self.jstate, &store, &id, f, q, FOUND_MAX).map(|(keys, new)| {
                        sent(keys, new);
                    })
                }
                _ => break,
            };
            if let Err(e) = found {
                log::info!("{id}/{f}: searching the server: {e}");
                failed = Some(e.to_string());
                if matches!(e, NetError::Io(_) | NetError::Closed(_)) {
                    self.conn = None;
                    break;
                }
            }
        }
        if !matches!(self.conn, Some(Conn::Imap(_))) {
            self.save_jstate();
        }
        let error = if any { None } else { failed };
        self.sink.emit(answer(Vec::new(), error, true));
    }

    pub(super) fn sync_one(&mut self, path: &str) {
        self.status(SyncStatus::Syncing { folder: path.to_string() });
        // A folder asked to keep more keeps them (the API servers keep only the newest window).
        let window = self.windows.get(path).copied().unwrap_or(0).max(self.opts.window);
        let (store, opts, id) = (self.store.clone(), SyncOptions { window, ..self.opts }, self.id());
        let result = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => sync_folder(c, &store, &id, path, &opts),
            Some(Conn::Jmap(c)) => {
                let r = jmap::sync_folder(c, &mut self.jstate, &store, &id, path, &opts);
                self.save_jstate();
                r
            }
            Some(Conn::Graph(c)) => {
                let r = crate::graph::sync_folder(c, &mut self.jstate, &store, &id, path, &opts);
                self.save_jstate();
                r
            }
            Some(Conn::Ews(c)) => {
                let r = crate::ews::sync_folder(c, &mut self.jstate, &store, &id, path, &opts);
                self.save_jstate();
                r
            }
            // POP: the Inbox is downloaded; every other folder is here only.
            Some(Conn::Pop) if self.folders.iter().any(|f| f.path == path && f.role == FolderRole::Inbox) => {
                let secret = self.secret();
                (|| {
                    let mut c = crate::pop::open(&self.acc, &secret?)?;
                    let r = crate::pop::fetch_new(&mut c, &store, &id, path, &crate::pop::Keep::of(&self.acc), now())?;
                    let _ = c.quit();
                    Ok(r)
                })()
            }
            Some(Conn::Pop) => Ok(crate::pop::local_counts(&store, &id, path)),
            None => Err(NetError::Closed("not connected".into())),
        };
        match result {
            Ok(mut r) => {
                // The token worked: a later refusal gets a fresh one again.
                self.token_retried = false;
                // The listing predates any queued edits: replay them locally (RFC 4549).
                if self.reapply_waiting(path) {
                    r.changed += 1;
                }
                let first = self.folders.iter().find(|f| f.path == path).is_some_and(|f| f.total == 0 && r.total > 0);
                if let Some(f) = self.folders.iter_mut().find(|f| f.path == path) {
                    let counts_changed = f.unread != r.unread || f.total != r.total;
                    f.unread = r.unread;
                    f.total = r.total;
                    if counts_changed {
                        self.publish_folders();
                    }
                }
                if r.touched() || first {
                    self.sink.emit(Event::FolderChanged { account: id, folder: path.to_string() });
                }
                if self.last_full.is_some() {
                    self.status(SyncStatus::Online { at: now() });
                }
            }
            Err(e) => self.failed(e),
        }
    }

    /// Sync calendars and address books with the account's CalDAV / CardDAV server (password
    /// accounts: OAuth tokens are for mail only).
    pub(super) fn sync_dav(&mut self) {
        // Exchange Online: its calendars and contacts through Graph.
        if let Some(Conn::Graph(c)) = &self.conn {
            let pim: mailcraft_model::pim::Pim = self.store.load_account_doc(&self.acc.id, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
            match crate::graph_pim::sync(c, &pim, now()) {
                Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
                Err(e) => log::info!("{}: calendars and contacts: {e}", self.acc.id),
            }
            return;
        }
        // On-premises Exchange: its calendar and contacts through EWS.
        if let Some(Conn::Ews(c)) = &self.conn {
            let pim: mailcraft_model::pim::Pim = self.store.load_account_doc(&self.acc.id, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
            match crate::ews_pim::sync(c, &pim, now()) {
                Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
                Err(e) => log::info!("{}: calendars and contacts: {e}", self.acc.id),
            }
            return;
        }
        // OAuth tokens are for mail only.
        if self.acc.oauth.is_some() || self.acc.ews.is_some() {
            return;
        }
        let pim: mailcraft_model::pim::Pim = self.store.load_account_doc(&self.acc.id, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        let password = self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).secret.clone();
        let dav = crate::dav::Dav::new(&user, &password);
        let host = self.acc.pop.as_ref().map_or(self.acc.imap.host.as_str(), |p| p.host.as_str());
        match crate::dav::sync(&dav, &self.acc.email, host, &pim, now()) {
            Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
            Err(e) => {
                log::info!("{}: calendars and contacts: {e}", self.acc.id);
                if matches!(e, NetError::Auth(_)) {
                    self.notice(format!("calendars and contacts: {e}"));
                }
            }
        }
    }

    /// Report the space used on the server (when it publishes a quota).
    pub(super) fn report_quota(&mut self) {
        let q = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => c.quota(),
            Some(Conn::Jmap(c)) => c.quota(),
            // Graph doesn't tell a mailbox's size to the mailbox's owner.
            Some(Conn::Graph(_) | Conn::Ews(_) | Conn::Pop) | None => return,
        };
        match q {
            Ok(Some((used, limit))) => self.sink.emit(Event::Quota { account: self.id(), used, limit }),
            Ok(None) => {}
            Err(e) if e.is_fatal() => self.failed(e),
            Err(e) => log::info!("{}: quota: {e}", self.acc.id),
        }
    }

    pub(super) fn fetch(&mut self, key: MessageKey) {
        if let Some(raw) = self.store.read_body(&key) {
            self.sink.emit(Event::Body { key, raw });
            return;
        }
        let store = self.store.clone();
        let r = (|| -> Result<Vec<u8>> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_mut() {
                Some(Conn::Jmap(c)) => jmap::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Graph(c)) => crate::graph::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Ews(c)) => crate::ews::fetch_body(c, &self.jstate, &store, &key),
                // POP mail is downloaded whole: what isn't here isn't anywhere.
                Some(Conn::Pop) => store.read_body(&key).ok_or_else(|| NetError::No("this message isn't on this computer".into())),
                _ => fetch_body(self.client()?, &store, &key),
            }
        })();
        match r {
            Ok(raw) => {
                self.sink.emit(Event::Body { key: key.clone(), raw });
                self.sink.emit(Event::FolderChanged { account: key.account.clone(), folder: key.folder.clone() });
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
