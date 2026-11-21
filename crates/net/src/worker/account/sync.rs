//! An account's worker: syncing folders, calendars and contacts (`calendars.rs`), searching the
//! server (`searching.rs`), and waiting for new mail.

use super::*;

mod calendars;
mod searching;
mod watching;

// Run once, on this thread, as [`Worker::publish_folders`] is about to write the folder list: a
// test's way to come between deciding to write it and writing it.
#[cfg(test)]
thread_local!(pub(crate) static BEFORE_FOLDERS_SAVED: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) });

/// How often a full sync says the counts of the folders it has synced, at most.
const COUNTS_EVERY: Duration = Duration::from_secs(1);
/// A minute of an account's own time between looks for new mail (short in tests, to see some pass).
const MINUTE: Duration = if cfg!(test) { Duration::from_millis(100) } else { Duration::from_secs(60) };
/// Every folder is read at most this often when an account looks at set times, however often it
/// looks at the Inbox: as often as an account left to itself reads them.
const FULL_SYNC_MINUTES: u32 = 15;
/// How long an account that looks only when asked waits to look by itself: never, near enough
/// (a year; a longer wait overflows the clock's sums).
const BY_HAND: Duration = Duration::from_secs(365 * 24 * 3600);
/// How long an IMAP connection stays quiet between looks. A server may drop one after 30 minutes
/// of nothing (RFC 3501 5.4), which would come back as an error on the next thing asked of it.
const QUIET_MINUTES: u32 = 10;

impl Worker {
    /// Wait for new mail: IDLE on the Inbox when the server supports it, told of the other folders
    /// too where it offers NOTIFY; else poll.
    pub(crate) fn wait_for_mail(&mut self, rx: &Receiver<Job>) {
        self.working(false);
        // Whatever was asked to stop has stopped.
        self.cancel.store(false, Ordering::Relaxed);
        // Looking only when asked (issue #104): once it's been quiet a while, signed out until
        // it's asked again, not kept awake for good.
        if self.acc.check_by_hand {
            let wait = MINUTE.saturating_mul(QUIET_MINUTES);
            let Some(got) = self.recv_unless_set_aside(rx, wait) else { return };
            match got {
                Ok(j) => push_job(&mut self.queue, j),
                Err(RecvTimeoutError::Disconnected) => self.closed = true,
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(Conn::Imap(c)) = self.conn.as_mut() {
                        c.logout();
                    }
                    self.conn = None;
                }
            }
            return;
        }
        if let Some(every) = self.check_every() {
            self.wait_for_set_time(rx, every);
            return;
        }
        let inbox = self.inbox();
        if matches!(self.conn, Some(Conn::Jmap(_))) && !self.no_push && inbox.is_some() && self.wait_for_push(rx) {
            return;
        }
        let idle = matches!(&self.conn, Some(Conn::Imap(c)) if c.has("IDLE"));
        if !idle || inbox.is_none() {
            let next_soon = self.soon.iter().map(|(at, _)| *at).min();
            let poll = poll_every(&self.acc.id);
            let wait = next_soon.map_or(poll, |at| at.saturating_duration_since(Instant::now()).min(poll));
            let Some(got) = self.recv_unless_set_aside(rx, wait) else { return };
            match got {
                Ok(j) => push_job(&mut self.queue, j),
                Err(RecvTimeoutError::Disconnected) => self.closed = true,
                Err(RecvTimeoutError::Timeout) if next_soon.is_some_and(|at| at <= Instant::now()) => self.sync_soon_folders(),
                Err(RecvTimeoutError::Timeout) => self.look_for_mail(),
            }
            return;
        }
        let Some(inbox) = inbox else { return };
        let result = (|| -> Result<bool> {
            let Some(Conn::Imap(c)) = self.conn.as_mut() else { return Err(NetError::Closed("not connected".into())) };
            // Told of every folder where the server can (NOTIFY, issue #232): mail its own rules
            // filed elsewhere is read as it comes, not at the next look.
            c.notify()?;
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
            Ok(changed) => {
                if changed {
                    self.sync_one(&inbox);
                }
                let others = match self.conn.as_mut() {
                    Some(Conn::Imap(c)) => c.take_notified(),
                    _ => Vec::new(),
                };
                let known: Vec<String> =
                    others.into_iter().filter(|f| *f != inbox && self.folders.iter().any(|k| k.path == *f && k.selectable)).collect();
                for f in known {
                    self.sync_one(&f);
                }
            }
            Err(e) => self.failed(e),
        }
    }

    /// How often the account was asked to look for new mail (issue #58); `None`: left to itself.
    pub(crate) fn check_every(&self) -> Option<Duration> {
        // Only when asked (issue #104): never by itself.
        if self.acc.check_by_hand {
            return Some(BY_HAND);
        }
        self.acc.check_every.filter(|m| *m > 0).map(|m| MINUTE.saturating_mul(m.min(mailcraft_model::MAX_CHECK_MINUTES)))
    }

    /// How long between reading every folder: an account looking less often than that reads them
    /// when it looks, and nothing in between.
    pub(crate) fn full_sync_every(&self) -> Duration {
        self.check_every().map_or(FULL_SYNC_EVERY, |every| every.max(MINUTE.saturating_mul(FULL_SYNC_MINUTES)))
    }

    /// Folders that were to be read again by now ([`Worker::soon`]).
    fn sync_soon_folders(&mut self) {
        let now = Instant::now();
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.soon).into_iter().partition(|(at, _)| *at <= now);
        self.soon = later;
        let mut folders: Vec<String> = due.into_iter().map(|(_, f)| f).collect();
        folders.dedup();
        for f in folders {
            self.sync_one(&f);
        }
    }

    /// Wait for the account's next set time (issue #58), with no IDLE or push: not being told is
    /// what was asked for. Jobs are done as they come, and don't put the time off: it counts from
    /// when the Inbox was last looked at, or an account in use would never get to it.
    fn wait_for_set_time(&mut self, rx: &Receiver<Job>, every: Duration) {
        let now = Instant::now();
        let due = self.looked.unwrap_or(now).checked_add(every).unwrap_or(now);
        let full = self.last_full.and_then(|t| t.checked_add(self.full_sync_every())).unwrap_or(now);
        let soon = self.soon.iter().map(|(at, _)| *at).min().unwrap_or(due);
        let wait = due.min(full).min(soon).saturating_duration_since(now).min(MINUTE.saturating_mul(QUIET_MINUTES));
        let Some(got) = self.recv_unless_set_aside(rx, wait) else { return };
        match got {
            Ok(j) => {
                // What it was asked to do finds the connection good, or makes one.
                if now.elapsed() >= MINUTE {
                    self.dropped_while_quiet(due);
                }
                push_job(&mut self.queue, j);
            }
            Err(RecvTimeoutError::Disconnected) => self.closed = true,
            Err(RecvTimeoutError::Timeout) => {
                self.sync_soon_folders();
                // Every folder's turn: the Inbox with them, in the loop that called this.
                if self.last_full.is_none_or(|t| t.elapsed() >= self.full_sync_every()) {
                    return;
                }
                if Instant::now() < due {
                    self.dropped_while_quiet(due);
                    return;
                }
                if self.dropped_while_quiet(Instant::now()) {
                    return;
                }
                self.look_for_mail();
            }
        }
    }

    /// Whether the server dropped an IMAP connection left quiet between looks, as servers do, some
    /// within a minute. Found out here it's nothing to report: the connection is let go, to be made
    /// again at `back_at` or by whatever needs it first, instead of failing what's asked of it next.
    pub(crate) fn dropped_while_quiet(&mut self, back_at: Instant) -> bool {
        let Some(Conn::Imap(c)) = self.conn.as_mut() else { return false };
        let Err(e) = c.noop() else { return false };
        log::info!("{}: the server let a quiet connection go: {e}", self.acc.id);
        self.conn = None;
        self.next_attempt = back_at;
        true
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
    pub(crate) fn wait_for_push(&mut self, rx: &Receiver<Job>) -> bool {
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

    pub(crate) fn inbox(&self) -> Option<String> {
        self.folders.iter().find(|f| f.role == FolderRole::Inbox).map(|f| f.path.clone())
    }

    /// Refresh the folder list from the server; `false` when that failed.
    pub(crate) fn relist(&mut self) -> bool {
        self.relist_as(false)
    }

    /// [`Worker::relist`]; `quiet`, the second connection's: the list is its own (not published
    /// or saved), and a failure only logged (the account's status is the main one's to say).
    pub(crate) fn relist_as(&mut self, quiet: bool) -> bool {
        self.relist_with(quiet, true)
    }

    /// [`Worker::relist_as`]; the main connection's list not said (nor written) unless `say`.
    pub(crate) fn relist_with(&mut self, quiet: bool, say: bool) -> bool {
        let id = self.id();
        let listed = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => c.list(&id),
            Some(Conn::Jmap(c)) => jmap::list_folders(c, &mut self.jstate, &id),
            Some(Conn::Graph(c)) => crate::graph::list_folders(c, &mut self.jstate, &id),
            Some(Conn::Ews(c)) => crate::ews::list_folders(c, &mut self.jstate, &id),
            Some(Conn::Pop) => crate::pop::folders(&self.store, &id),
            None => Err(NetError::Closed("not connected".into())),
        };
        let api = matches!(self.conn, Some(Conn::Jmap(_) | Conn::Graph(_) | Conn::Ews(_)));
        if api {
            self.save_jstate();
        }
        match listed {
            Ok(mut list) => {
                // The server took the token for mail (one renewed, say): a refusal from here on
                // is a new one, and gets a fresh token again (found by a test hang: the token a
                // renewal brought, refused before that sync was done, asked to sign in again with
                // the refresh token never tried). POP's list is this computer's.
                if !matches!(self.conn, Some(Conn::Pop)) {
                    self.token_retried = false;
                }
                // JMAP, Graph and Exchange list the counts; IMAP's come from each folder's sync.
                for f in list.iter_mut().filter(|_| !api) {
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
                // Folders gone from the server lose their cache; not one the list on disk still
                // has while changes to the folders are still to be sent, which may make it (found
                // in review: a folder renamed twice offline, listed after the first rename, lost
                // its cache, and its mail was downloaded again). That one's looked at again at the
                // next listing, once they're sent (found in review: one deleted on the server
                // meanwhile kept its cache for good). Changes or a list that can't be read (the
                // disk failing) can't say which those are: every one keeps its cache till the next
                // listing, as does one whose cache couldn't be cleared (found in review: taken for
                // nothing waiting, they lost it all).
                let here = self.kept_while_waiting(&id);
                let mut gone: Vec<String> = self.folders.iter().map(|f| f.path.clone()).chain(std::mem::take(&mut self.gone_meanwhile)).collect();
                gone.retain(|p| !list.iter().any(|f| f.path == *p));
                gone.sort_unstable();
                gone.dedup();
                for path in gone {
                    if here.as_ref().is_none_or(|here| here.iter().any(|f| f.path == path)) {
                        self.gone_meanwhile.push(path);
                    } else if let Err(e) = self.store.clear_folder(&id, &path) {
                        log::warn!("{id}: a folder gone from the server keeps its cache for now: {e}");
                        self.gone_meanwhile.push(path);
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

    /// The folders the list on disk has while changes to the folders wait to be sent (none when
    /// none wait); `None`, said in the log, when either can't be read.
    fn kept_while_waiting(&self, id: &str) -> Option<Vec<Folder>> {
        let read = || -> std::result::Result<Vec<Folder>, mailcraft_store::StoreError> {
            if self.store.load_ops(id)?.iter().any(Op::is_folder_op) { self.store.load_folders(id) } else { Ok(Vec::new()) }
        };
        read().inspect_err(|e| log::warn!("{id}: the folders gone from the server keep their cache for now: {e}")).ok()
    }

    /// The folder list, then every folder (Inbox first), staying responsive to opened messages.
    pub(crate) fn sync_all(&mut self, rx: &Receiver<Job>) {
        let began = Instant::now();
        if !self.relist() {
            return;
        }
        // Exchange: told from now on which folders change, for the looks between full syncs (a
        // change during this one is read again at the next look, which costs little).
        self.watch_from_now();
        let paths: Vec<String> = self.folders.iter().filter(|f| f.selectable).map(|f| f.path.clone()).collect();
        // Counts are said as folders are synced, once a second at most: each time writes them
        // into the account's folder list, and says all of it.
        let mut said = Instant::now();
        // An account whose calendars and contacts were never read has them read once its first
        // folder (the Inbox) is in, not after every folder: a big mailbox's first sync takes
        // hours, and its calendar stayed empty until then (issue #80). Only on the first full
        // sync after connecting: an account with no calendars to find is looked at after its
        // mail, as ever.
        let mut calendars_due = self.last_full.is_none() && !self.calendars_read_before();
        let mut calendars_read = false;
        // An IMAP or Exchange (EWS, issue #231) account's Send / Receive: its folders, one by one
        // (a POP account says its download as it goes, `transfers.rs`).
        let told = matches!(self.conn, Some(Conn::Imap(_) | Conn::Ews(_)));
        let mut going = Transfer { folders_total: u32::try_from(paths.len()).unwrap_or(u32::MAX), ..Transfer::working(TransferKind::Receiving) };
        let mut pace = transfers::Pace::default();
        for path in paths {
            if self.cancel_asked() {
                break;
            }
            if told && pace.due() {
                self.tell_transfer(Transfer { folder: path.clone(), ..going.clone() });
            }
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
            going.folder.clone_from(&path);
            if self.sync_counted(&path) {
                self.counts_unsaid.push(path);
            }
            going.folders_done = going.folders_done.saturating_add(1);
            if std::mem::take(&mut calendars_due) && !self.closed && self.conn.is_some() && !self.in_pim {
                self.say_counts();
                said = Instant::now();
                self.calendars_and_contacts(rx);
                calendars_read = true;
            }
        }
        // What came during the last folder doesn't wait for the calendars and contacts either: a
        // folder renamed here meanwhile has its counts said under its new name now (found in
        // review: the old name's, said first, were lost, and it showed empty until then).
        if !self.closed && self.conn.is_some() {
            self.serve_between_folders(rx);
        }
        self.say_counts();
        if told && !self.closed {
            // Its connection lost on the way: failed, the account's status saying why.
            let lost = self.conn.is_none().then(String::new);
            self.transfer_ended(Transfer { folder: String::new(), ..going }, lost);
        }
        if self.closed {
            return;
        }
        // Report Online after mail; CalDAV/EWS calendars and contacts can take far longer and
        // shouldn't hold the status bar on the last folder name.
        if self.conn.is_some() {
            self.report_quota();
            self.prune_jmap_numbers();
            self.last_full = Some(Instant::now());
            self.looked = self.last_full;
            // All of it read: a connection lost from here on is tried again soonest.
            self.backoff = FIRST_BACKOFF;
            // Anything left from a failed attempt gets another try every full sync.
            self.flush_outbox();
            log::info!("{}: synced {} folders in {:.1} s", self.acc.id, self.folders.len(), began.elapsed().as_secs_f32());
            self.status(SyncStatus::Online { at: now() });
        }
        // Not while they're being read already: this is a Send / Receive between two of their steps.
        if self.conn.is_some() && !calendars_read && !self.in_pim {
            self.calendars_and_contacts(rx);
        }
    }

    /// The account's calendars and contacts, synced with their server now.
    fn calendars_and_contacts(&mut self, rx: &Receiver<Job>) {
        let began = Instant::now();
        self.sync_dav(Some(rx));
        log::info!("{}: PIM pass took {:?}", self.acc.id, began.elapsed());
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

    /// Forget the numbers of JMAP messages no longer cached (nor waiting in a change). Not when
    /// the changes or a folder can't be read (the disk failing): their numbers would be lost, and
    /// the changes never sent (found in review). The next full sync tries again.
    pub(crate) fn prune_jmap_numbers(&mut self) {
        if !matches!(self.conn, Some(Conn::Jmap(_) | Conn::Graph(_) | Conn::Ews(_))) {
            return;
        }
        let read = || -> std::result::Result<std::collections::HashSet<u32>, mailcraft_store::StoreError> {
            let ops = self.store.load_ops(&self.acc.id)?;
            let mut keep: std::collections::HashSet<u32> = ops.iter().flat_map(|o| o.keys().iter().map(|k| k.uid)).collect();
            for f in &self.folders {
                keep.extend(self.store.folder_uids(&self.acc.id, &f.path)?);
            }
            Ok(keep)
        };
        match read() {
            Ok(keep) => {
                self.jstate.prune(&keep);
                self.save_jstate();
            }
            Err(e) => log::warn!("{}: the server's message numbers are all kept for now: {e}", self.acc.id),
        }
    }

    /// The account's folder list read again from disk (a folder made, renamed or deleted here),
    /// the counts a full sync hasn't said yet written into it first: they'd be lost (found in
    /// review). Its counts are taken as they are there, with the window's of mail read here: the
    /// pool said that list as it made the change on disk (a POP account's worker says it next),
    /// and this worker's lists say them from now on, so the next isn't news, and write nothing
    /// over them (found in review: said and written with this worker's counts from before, mail
    /// read here came back unread, in the window and at the next start). A list that can't be
    /// read (the disk failing) leaves this worker's as it was, and `false` (found in review: it
    /// was left with none, and a POP account's wrote that over the one on disk, its mail out of
    /// reach).
    pub(crate) fn reload_folders(&mut self) -> bool {
        self.say_counts();
        match self.store.load_folders(&self.acc.id) {
            Ok(folders) => {
                self.folders = folders;
                true
            }
            Err(e) => {
                log::warn!("{}: the folder list wasn't read again: {e}", self.acc.id);
                false
            }
        }
    }

    /// A folder renamed here (`from` to `to`, with the folders in it): this worker's list and the
    /// counts it hasn't said yet follow it, so they're said under the new name (found in review:
    /// said under the old one, which the list on disk had no more, they were lost, and the
    /// renamed folder showed empty).
    pub(crate) fn renamed(&mut self, from: &str, to: &str) {
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
    pub(crate) fn clear_left_behind(&self, op: &Op) {
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
    pub(crate) fn say_counts(&mut self) {
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
    /// not once it was removed (found in review: its files came back). Written while it's kept
    /// ([`Store::while_kept`]): a removal coming as it's written waits for it, and one just before
    /// is seen (a look first, then the write, let one in between bring the list back).
    pub(crate) fn publish_folders(&self) {
        #[cfg(test)]
        if let Some(before) = BEFORE_FOLDERS_SAVED.with(|b| b.borrow_mut().take()) {
            before();
        }
        if self.store.while_kept(&self.acc.id, self.removals, || self.store.save_folders(&self.acc.id, &self.folders)).is_none() {
            return;
        }
        self.sink.emit(Event::Folders { account: self.id(), folders: self.folders.clone() });
    }

    /// [`Self::sync_one`] for a folder a message was just filed in. A POP account's folders other
    /// than its Inbox are only on this computer, so reading one finds nothing new: said here, or
    /// the list on screen went without it until MailCraft started again (issue #57).
    pub(crate) fn sync_filed(&mut self, path: &str) {
        if matches!(self.conn, Some(Conn::Pop)) {
            self.sink.emit(Event::FolderChanged { account: self.id(), folder: path.to_string() });
        }
        self.sync_one(path);
    }

    pub(crate) fn sync_one(&mut self, path: &str) {
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
            Some(Conn::Pop) if self.folders.iter().any(|f| f.path == path && f.role == FolderRole::Inbox) => self.receive_pop(&store, path, seen),
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

    /// Report the space used on the server (when it publishes a quota).
    pub(crate) fn report_quota(&mut self) {
        let q = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => c.quota(),
            Some(Conn::Jmap(c)) => c.quota(),
            // Exchange says it of the mailbox's folders (issue #143).
            Some(Conn::Ews(c)) => c.quota(),
            // Graph doesn't tell a mailbox's size to the mailbox's owner.
            Some(Conn::Graph(_) | Conn::Pop) | None => return,
        };
        match q {
            Ok(Some((used, limit))) => self.sink.emit(Event::Quota { account: self.id(), used, limit }),
            Ok(None) => {}
            Err(e) if e.is_fatal() => self.failed(e),
            Err(e) => log::info!("{}: quota: {e}", self.acc.id),
        }
    }
}
