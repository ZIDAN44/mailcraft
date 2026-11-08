//! An account's worker: syncing folders, searching the server, calendars and contacts, and waiting for new mail.

use super::*;

/// How often a full sync says the counts of the folders it has synced, at most.
const COUNTS_EVERY: Duration = Duration::from_secs(1);

impl Worker {
    /// Wait for new mail: IDLE on the Inbox when the server supports it, else poll.
    pub(super) fn wait_for_mail(&mut self, rx: &Receiver<Job>) {
        self.working(false);
        let inbox = self.inbox();
        if matches!(self.conn, Some(Conn::Jmap(_))) && !self.no_push && inbox.is_some() && self.wait_for_push(rx) {
            return;
        }
        let idle = matches!(&self.conn, Some(Conn::Imap(c)) if c.has("IDLE"));
        if !idle || inbox.is_none() {
            let next_soon = self.soon.iter().map(|(at, _)| *at).min();
            let wait = next_soon.map_or(POLL_EVERY, |at| at.saturating_duration_since(Instant::now()).min(POLL_EVERY));
            let Some(got) = self.recv_unless_set_aside(rx, wait) else { return };
            match got {
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
            let (aside, filing) = (&self.kept.aside, &self.filing);
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
                !queue.is_empty() || *closed || aside.ended(filing)
            })
        })();
        match result {
            Ok(true) => self.sync_one(&inbox),
            Ok(false) => {}
            Err(e) => self.failed(e),
        }
    }

    /// A job, waiting for one up to `wait` ([`Receiver::recv_timeout`]); `None` sooner, once
    /// another worker's filing of a draft kept on this computer and set aside ends, for it to be
    /// filed ([`Self::queue_kept_drafts`]).
    fn recv_unless_set_aside(&self, rx: &Receiver<Job>, wait: Duration) -> Option<std::result::Result<Job, RecvTimeoutError>> {
        if self.kept.aside.is_empty() {
            return Some(rx.recv_timeout(wait));
        }
        let end = Instant::now().checked_add(wait);
        loop {
            let left = end.map_or(wait, |end| end.saturating_duration_since(Instant::now()));
            let step = left.min(changes::LOOK_ASIDE_EVERY);
            match rx.recv_timeout(step) {
                Err(RecvTimeoutError::Timeout) if step < left => {
                    if self.kept.aside.ended(&self.filing) {
                        return None;
                    }
                }
                got => return Some(got),
            }
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
        self.working(false);
        let end = Instant::now() + IDLE_FOR;
        let mut changed = false;
        while Instant::now() < end && !changed {
            self.drain(rx);
            if self.closed || !self.queue.is_empty() || self.kept.aside.ended(&self.filing) {
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
        self.relist_as(false)
    }

    /// [`Worker::relist`]; `quiet`, the second connection's: the list is its own (not published
    /// or saved), and a failure only logged (the account's status is the main one's to say).
    pub(super) fn relist_as(&mut self, quiet: bool) -> bool {
        self.relist_with(quiet, true)
    }

    /// [`Worker::relist_as`]; the main connection's list not said (nor written) unless `say`.
    pub(super) fn relist_with(&mut self, quiet: bool, say: bool) -> bool {
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
                // The second connection keeps its list to itself: the main one says what the
                // account has (its counts kept up), and clears what's gone.
                if quiet {
                    self.folders = list;
                    return true;
                }
                // Folders gone from the server lose their cache.
                for old in &self.folders {
                    if !list.iter().any(|f| f.path == old.path) {
                        let _ = self.store.clear_folder(&id, &old.path);
                    }
                }
                self.folders = list;
                if say {
                    self.publish_folders();
                }
                true
            }
            Err(e) if quiet => {
                log::info!("{}: the folders weren't listed again: {e}", self.acc.id);
                false
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
        // Counts are said as folders are synced, once a second at most: each time writes them
        // into the account's folder list, and says all of it.
        let mut said = Instant::now();
        for path in paths {
            if !self.counts_unsaid.is_empty() && said.elapsed() >= COUNTS_EVERY {
                self.say_counts();
                said = Instant::now();
            }
            self.serve_between_folders(rx);
            if self.closed || self.conn.is_none() {
                break;
            }
            // A folder renamed or deleted here meanwhile, the change made on the server between
            // two folders, is passed over (found in review: read by the name it had, it said "no
            // such mailbox"; renamed, it's synced under its new name after the change).
            if !self.folders.iter().any(|f| f.path == path) {
                continue;
            }
            if self.sync_counted(&path) {
                self.counts_unsaid.push(path);
            }
        }
        // What came during the last folder doesn't wait for the calendars and contacts either: a
        // folder renamed here meanwhile has its counts said under its new name now (found in
        // review: the old name's, said first, were lost, and it showed empty until then).
        if !self.closed && self.conn.is_some() {
            self.serve_between_folders(rx);
        }
        self.say_counts();
        if self.closed {
            return;
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

    /// Between two folders of a full sync: body fetches and local edits served, ahead of the rest,
    /// and a message whose text went first brought whole, unless one was just opened.
    fn serve_between_folders(&mut self, rx: &Receiver<Job>) {
        self.drain(rx);
        let (urgent, rest): (VecDeque<Job>, VecDeque<Job>) = std::mem::take(&mut self.queue).into_iter().partition(between_folders);
        self.queue = rest;
        let opened = !urgent.is_empty();
        for j in urgent {
            self.handle(j);
        }
        if !opened && let Some(key) = self.rest.pop_front() {
            self.fetch_whole(key);
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

    /// The account's folder list read again from disk (a folder made, renamed or deleted here),
    /// the counts a full sync hasn't said yet written into it first: they'd be lost (found in
    /// review). Its counts are taken as they are there, with the window's of mail read here: the
    /// pool said that list as it made the change on disk (a POP account's worker says it next),
    /// and this worker's lists say them from now on, so the next isn't news, and write nothing
    /// over them (found in review: said and written with this worker's counts from before, mail
    /// read here came back unread, in the window and at the next start).
    pub(super) fn reload_folders(&mut self) {
        self.say_counts();
        self.folders = self.store.load_folders(&self.acc.id).unwrap_or_default();
    }

    /// A folder renamed here (`from` to `to`, with the folders in it): this worker's list and the
    /// counts it hasn't said yet follow it, so they're said under the new name (found in review:
    /// said under the old one, which the list on disk had no more, they were lost, and the
    /// renamed folder showed empty).
    pub(super) fn renamed(&mut self, from: &str, to: &str) {
        let delimiter = self.folders.iter().find_map(|f| f.delimiter);
        for f in &mut self.folders {
            if let Some(new) = mailcraft_model::rebase_path(&f.path, from, to, f.delimiter.or(delimiter)) {
                f.path = new;
            }
        }
        for p in &mut self.counts_unsaid {
            if let Some(new) = mailcraft_model::rebase_path(p, from, to, delimiter) {
                *p = new;
            }
        }
    }

    /// A folder renamed or deleted here (`op`, with the folders in it) as this worker read it:
    /// the read, ending after the folder's cache went with it to its new name (or went), wrote it
    /// again under the name it had, which the account's list has no more. That copy goes (found
    /// in review: left for good beside the renamed folder). Called before this worker's list
    /// follows the change; one the list on disk has again (made anew since) stays.
    pub(super) fn clear_left_behind(&self, op: &Op) {
        let (Op::RenameFolder { from: gone, .. } | Op::DeleteFolder { path: gone }) = op else { return };
        let Ok(listed) = self.store.load_folders(&self.acc.id) else { return };
        let delimiter = self.folders.iter().find_map(|f| f.delimiter);
        let within = self.folders.iter().filter(|f| mailcraft_model::is_within(&f.path, gone, f.delimiter.or(delimiter))).map(|f| f.path.as_str());
        let mut paths: Vec<&str> = std::iter::once(gone.as_str()).chain(within).collect();
        paths.sort_unstable();
        paths.dedup();
        for path in paths.into_iter().filter(|p| !listed.iter().any(|f| f.path == *p)) {
            let cleared = self.store.while_kept(&self.acc.id, self.removals, || self.store.clear_folder(&self.acc.id, path));
            if let Some(Err(e)) = cleared {
                log::warn!("{}: the cache {path} had is still there: {e}", self.acc.id);
            }
        }
    }

    /// The folders of `list` this worker has counted take its counts (the server's, as last
    /// synced): the window's own on disk (mail read here) aren't the pool's to report.
    fn counted_here(&self, list: &mut [Folder]) {
        for f in list {
            if let Some(mine) = self.folders.iter().find(|m| m.path == f.path) {
                (f.unread, f.total) = (mine.unread, mine.total);
            }
        }
    }

    /// The counts synced and not said yet ([`Worker::counts_unsaid`]) written into the account's
    /// folder list as it is on disk, and that list said. The rest of it stays as it is there: a
    /// folder made, renamed or deleted here meanwhile, in the list on disk already, isn't undone
    /// by this worker's list from before it (found in review: the old name came back, and a view
    /// of the renamed folder fell back to the Inbox). It's said with this worker's counts, as a
    /// list it publishes is: the session takes a count the pool reports unchanged as no news, and
    /// the window's own count on disk isn't the pool's to report. A list on disk that's gone or
    /// was set aside (damaged) is written whole from this worker's. The counts of a folder the
    /// list on disk hasn't (renamed here, the worker yet to hear of it) stay unsaid, to follow it
    /// to its new name ([`Worker::renamed`]), or go with it from this worker's list. Nothing once
    /// the account was removed.
    pub(super) fn say_counts(&mut self) {
        let paths = std::mem::take(&mut self.counts_unsaid);
        let counts: Vec<(String, u32, u32)> =
            paths.iter().filter_map(|p| self.folders.iter().find(|f| f.path == *p)).map(|f| (f.path.clone(), f.unread, f.total)).collect();
        if counts.is_empty() {
            return;
        }
        let id = self.id();
        let listed = self.store.while_kept(&id, self.removals, || -> Result<Vec<Folder>> {
            self.store.update_folder_counts(&id, &counts)?;
            Ok(self.store.load_folders(&id)?)
        });
        match listed {
            None => {}
            Some(Ok(folders)) if folders.is_empty() => self.publish_folders(),
            Some(Ok(mut folders)) => {
                let (said, unsaid): (Vec<_>, Vec<_>) = counts.into_iter().partition(|(p, ..)| folders.iter().any(|f| f.path == *p));
                for (path, ..) in unsaid {
                    if !self.counts_unsaid.contains(&path) {
                        self.counts_unsaid.push(path);
                    }
                }
                if !said.is_empty() {
                    self.counted_here(&mut folders);
                    self.sink.emit(Event::Folders { account: id, folders });
                }
            }
            Some(Err(e)) => {
                log::warn!("{id}: the folders' counts weren't written into their list: {e}");
                self.publish_folders();
            }
        }
    }

    /// The account's folders written and said as this worker has them (listed from the server);
    /// not once it was removed (found in review: its files came back).
    pub(super) fn publish_folders(&self) {
        if self.removed() {
            return;
        }
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
        if self.sync_counted(path) {
            self.counts_unsaid.push(path.to_string());
            self.say_counts();
        }
    }

    /// [`Self::sync_one`], its count left for the caller to say: whether it's to be said (it
    /// changed, or the folder's mail did).
    fn sync_counted(&mut self, path: &str) -> bool {
        self.working(true);
        self.status(SyncStatus::Syncing { folder: path.to_string() });
        // A folder asked to keep more keeps them (the API servers keep only the newest window).
        let window = self.windows.get(path).copied().unwrap_or(0).max(self.opts.window);
        let (store, opts, id, seen) = (self.store.clone(), SyncOptions { window, ..self.opts }, self.id(), self.removals);
        // Nothing of it written once its account is removed, the sync on its way (found in review:
        // its folder came back with the Inbox in it, over IMAP, JMAP, Graph, Exchange and POP).
        let result = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => sync_folder_as(c, &store, &id, path, &opts, seen),
            Some(Conn::Jmap(c)) => {
                let r = jmap::sync_folder_as(c, &mut self.jstate, &store, &id, path, &opts, seen);
                self.save_jstate();
                r
            }
            Some(Conn::Graph(c)) => {
                let r = crate::graph::sync_folder_as(c, &mut self.jstate, &store, &id, path, &opts, seen);
                self.save_jstate();
                r
            }
            Some(Conn::Ews(c)) => {
                let r = crate::ews::sync_folder_as(c, &mut self.jstate, &store, &id, path, &opts, seen);
                self.save_jstate();
                r
            }
            // POP: the Inbox is downloaded; every other folder is here only.
            Some(Conn::Pop) if self.folders.iter().any(|f| f.path == path && f.role == FolderRole::Inbox) => {
                let secret = self.secret();
                (|| {
                    let mut c = crate::pop::open(&self.acc, &secret?)?;
                    let r = crate::pop::fetch_new_as(&mut c, &store, &id, path, &crate::pop::Keep::of(&self.acc), now(), seen)?;
                    let _ = c.quit();
                    Ok(r)
                })()
            }
            Some(Conn::Pop) => crate::pop::local_counts(&store, &id, path),
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
                let mut counts_changed = false;
                if let Some(f) = self.folders.iter_mut().find(|f| f.path == path) {
                    counts_changed = f.unread != r.unread || f.total != r.total;
                    f.unread = r.unread;
                    f.total = r.total;
                }
                // Changed, then counted: a folder whose mail changed has its count said after it,
                // the same as before or not (the window counted mail read here that another
                // device has marked unread again since).
                let touched = r.touched() || first;
                if touched {
                    self.sink.emit(Event::FolderChanged { account: id, folder: path.to_string() });
                }
                if self.last_full.is_some() {
                    self.status(SyncStatus::Online { at: now() });
                }
                counts_changed || touched
            }
            // Its account removed meanwhile: nothing to say of it.
            Err(e) if self.removed() => {
                log::info!("{}: a folder of an account removed wasn't synced: {e}", self.acc.id);
                false
            }
            Err(e) => {
                self.failed(e);
                false
            }
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
        // Calendars and contacts given by hand (issue #7), with their own sign-in or the mail's.
        let given = self.acc.dav.clone().unwrap_or_default();
        // OAuth tokens are for mail only: such an account's calendars need a sign-in of their own.
        if (self.acc.oauth.is_some() || self.acc.ews.is_some()) && given.username.is_none() {
            return;
        }
        let pim: mailcraft_model::pim::Pim = self.store.load_account_doc(&self.acc.id, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
        let (user, password) = match &given.username {
            Some(u) => (u.clone(), self.creds.as_ref().and_then(|c| c.get_secret(&self.acc, DAV_SECRET)).unwrap_or_default()),
            None => (
                if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() },
                self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).secret.clone(),
            ),
        };
        let dav = crate::dav::Dav::new(&user, &password);
        let host = self.acc.pop.as_ref().map_or(self.acc.imap.host.as_str(), |p| p.host.as_str());
        let at = crate::dav::Where {
            email: &self.acc.email,
            username: &self.acc.username,
            imap_host: host,
            caldav: Some(given.caldav.as_str()).filter(|c| !c.trim().is_empty()),
            carddav: Some(given.carddav.as_str()).filter(|c| !c.trim().is_empty()),
        };
        match crate::dav::sync(&dav, &at, &pim, now(), system_zone()) {
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
}

/// The system's time zone, for calendar times written without one (`UTC` when it can't say).
fn system_zone() -> mailcraft_model::zone::Zone {
    iana_time_zone::get_timezone().ok().and_then(|z| mailcraft_model::zone::Zone::named(&z)).unwrap_or_default()
}
