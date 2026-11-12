//! An account's worker: calendars and contacts with their server (CalDAV and CardDAV, Graph,
//! EWS), and the whole of a calendar-only account's work (issue #61).

use super::*;

/// How often a calendar-only account looks at its calendars, unless asked to look at set times.
const CALENDAR_MINUTES: u32 = 15;
/// The longest a calendar-only account waits to try again after its server couldn't be reached.
const MOST_BACKOFF: Duration = Duration::from_secs(300);

impl Worker {
    /// Sync calendars and address books with the account's CalDAV / CardDAV server (password
    /// accounts: OAuth tokens are for mail only).
    pub(in crate::worker) fn sync_dav(&mut self) {
        // Exchange Online: its calendars and contacts through Graph.
        if let Some(Conn::Graph(c)) = &self.conn {
            let pim: mailcraft_model::pim::Pim = self.store.load_account_doc(&self.acc.id, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
            let read = crate::graph_pim::sync(c, &pim, now());
            self.exchange_read(read);
            return;
        }
        // On-premises Exchange: its calendar and contacts through EWS.
        if let Some(Conn::Ews(c)) = &self.conn {
            let pim: mailcraft_model::pim::Pim = self.store.load_account_doc(&self.acc.id, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
            let read = crate::ews_pim::sync(c, &pim, now());
            self.exchange_read(read);
            return;
        }
        // OAuth tokens are for mail only: such an account's calendars need a sign-in of their own.
        let own = self.acc.dav.as_ref().is_some_and(|d| d.username.is_some());
        if (self.acc.oauth.is_some() || self.acc.ews.is_some()) && !own {
            return;
        }
        if let Err(e) = self.dav_pass() {
            log::info!("{}: calendars and contacts: {e}", self.acc.id);
            if matches!(e, NetError::Auth(_)) {
                self.notice(format!("calendars and contacts: {e}"));
            }
        }
    }

    /// An Exchange account's calendars and contacts, read or not. What the server wouldn't give
    /// is said with why, once and not at every sync (issue #80: the calendar stayed empty and
    /// nothing said so); a server that couldn't be reached is the mail's to say.
    fn exchange_read(&mut self, read: Result<mailcraft_model::pim::DavSync>) {
        match read {
            Ok(mut sync) => {
                let said = (!sync.errors.is_empty()).then(|| sync.errors.join("; "));
                if said.is_some() && said == self.dav_said {
                    sync.errors.clear();
                }
                self.dav_said = said;
                self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) });
            }
            Err(e) => {
                log::warn!("{}: calendars and contacts: {e}", self.acc.id);
                let why = e.to_string();
                if !matches!(e, NetError::Io(_) | NetError::Closed(_)) && self.dav_said.as_deref() != Some(why.as_str()) {
                    self.notice(format!("calendars and contacts: {why}"));
                    self.dav_said = Some(why);
                }
            }
        }
    }

    /// Whether the account's calendars and contacts have ever been read from its server.
    pub(in crate::worker) fn calendars_read_before(&self) -> bool {
        let pim: mailcraft_model::pim::Pim = self.store.load_account_doc(&self.acc.id, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
        pim.dav_checked.is_some()
    }

    /// One sync with the CalDAV and CardDAV server: the addresses given by hand (issue #7), or
    /// those found from the account's address; signed in with their own user name, or the mail's.
    fn dav_pass(&mut self) -> Result<()> {
        let given = self.acc.dav.clone().unwrap_or_default();
        let pim: mailcraft_model::pim::Pim = self.store.load_account_doc(&self.acc.id, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
        let (user, password) = match &given.username {
            Some(u) => (u.clone(), self.creds.as_ref().and_then(|c| c.get_secret(&self.acc, DAV_SECRET)).unwrap_or_default()),
            None => (
                if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() },
                self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).secret.clone(),
            ),
        };
        let dav = crate::dav::Dav::new(&user, &password).with_digest(self.dav_digest.take());
        let host = self.acc.pop.as_ref().map_or(self.acc.imap.host.as_str(), |p| p.host.as_str());
        let at = crate::dav::Where {
            email: &self.acc.email,
            username: &self.acc.username,
            imap_host: host,
            caldav: Some(given.caldav.as_str()).filter(|c| !c.trim().is_empty()),
            carddav: Some(given.carddav.as_str()).filter(|c| !c.trim().is_empty()),
            mails: !self.acc.is_calendar_only(),
        };
        let sync = crate::dav::sync(&dav, &at, &pim, now(), system_zone());
        self.dav_digest = dav.digest();
        let sync = sync?;
        self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) });
        Ok(())
    }

    /// A calendar-only account's thread (issue #61): no mail server, so no connection is held.
    /// Its calendars and contacts are synced as it starts, every quarter of an hour (or at its
    /// set times), at Send / Receive, and after a change made here. A refused sign-in waits for
    /// Send / Receive or a new password; a server that can't be reached is tried again later.
    pub(in crate::worker) fn run_calendars(mut self, rx: Receiver<Job>) {
        self.status(SyncStatus::Connecting);
        if !self.settle_password() {
            return;
        }
        let mut due = Some(Instant::now());
        while !self.closed {
            let got = match due {
                Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match got {
                Ok(Job::SyncAll { .. }) => {
                    self.auth_failed = false;
                    due = Some(Instant::now());
                }
                // A change made here: sent at once, unless the sign-in is refused (it would be
                // again, for every change).
                Ok(Job::SyncDav { .. }) if !self.auth_failed => due = Some(Instant::now()),
                // Its mail is on this computer only: the pool keeps that itself.
                Ok(other) => log::debug!("{}: {other:?} isn't for an account with no mail server", self.acc.id),
                Err(RecvTimeoutError::Timeout) => due = self.calendars_pass(),
                Err(RecvTimeoutError::Disconnected) => self.closed = true,
            }
        }
    }

    /// [`Worker::run_calendars`]' sync, and when the next one is due (`None`: when asked).
    fn calendars_pass(&mut self) -> Option<Instant> {
        self.working(true);
        let done = self.dav_pass();
        self.working(false);
        let now_at = Instant::now();
        match done {
            Ok(()) => {
                self.backoff = Duration::from_secs(5);
                self.status(SyncStatus::Online { at: now() });
                let every = self.check_every().unwrap_or(MINUTE.saturating_mul(CALENDAR_MINUTES));
                now_at.checked_add(every)
            }
            Err(NetError::Auth(error)) => {
                self.auth_failed = true;
                self.status(SyncStatus::NeedsPassword { error });
                None
            }
            Err(e) => {
                let wait = self.backoff;
                self.backoff = self.backoff.saturating_mul(2).min(MOST_BACKOFF);
                self.status(SyncStatus::Error { error: e.to_string() });
                now_at.checked_add(wait)
            }
        }
    }
}

/// The system's time zone, for calendar times written without one (`UTC` when it can't say).
fn system_zone() -> mailcraft_model::zone::Zone {
    iana_time_zone::get_timezone().ok().and_then(|z| mailcraft_model::zone::Zone::named(&z)).unwrap_or_default()
}
