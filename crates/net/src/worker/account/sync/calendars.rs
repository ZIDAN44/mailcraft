//! An account's worker: calendars and contacts with their server (CalDAV and CardDAV, Graph,
//! EWS), and the whole of a calendar-only account's work (issue #61).

use super::*;

/// How often a calendar-only account looks at its calendars, unless asked to look at set times.
const CALENDAR_MINUTES: u32 = 15;
/// The longest a calendar-only account waits to try again after its server couldn't be reached.
const MOST_BACKOFF: Duration = Duration::from_secs(300);
/// Where a Google account's access token may go for its calendars and contacts: Google's CalDAV
/// and CardDAV servers.
const GOOGLE_DAV: &[&str] = &["apidata.googleusercontent.com", "www.googleapis.com"];

impl Worker {
    /// Sync calendars and address books with the account's CalDAV / CardDAV server (password
    /// accounts, and Google's signed in with OAuth). An Exchange account's are read a step at a
    /// time, its mail looked at between ([`Worker::between_pim_steps`]; jobs too, from `rx`).
    pub(in crate::worker) fn sync_dav(&mut self, rx: Option<&Receiver<Job>>) {
        // Exchange Online: its calendars and contacts through Graph.
        if matches!(self.conn, Some(Conn::Graph(_))) {
            let pim = self.pim_taken_in();
            let Some(Conn::Graph(c)) = &self.conn else { return };
            let read = crate::graph_pim::sync(c, &pim, now());
            self.exchange_read(read, &pim);
            return;
        }
        // On-premises Exchange: its calendar and contacts through EWS.
        if matches!(self.conn, Some(Conn::Ews(_))) {
            let pim = self.pim_taken_in();
            let mut pass = crate::ews_pim::Pass::new(now());
            self.in_pim = true;
            let read = loop {
                let step = match &self.conn {
                    Some(Conn::Ews(c)) => pass.step(c, &pim),
                    _ => Err(NetError::Closed("the connection was lost".into())),
                };
                match step {
                    Ok(Some(done)) => break Ok(done),
                    Ok(None) => self.between_pim_steps(rx),
                    Err(e) => break Err(e),
                }
                if self.closed {
                    break Err(NetError::Closed("the account was closed".into()));
                }
            };
            self.in_pim = false;
            self.exchange_read(read, &pim);
            return;
        }
        // Google's calendars take its access token (issue #262); another OAuth account's token is
        // for mail only, so its calendars, as an EWS account's, need a sign-in of their own.
        let own = self.acc.dav.as_ref().is_some_and(|d| d.username.is_some());
        let google = self.acc.oauth.as_deref() == Some("google");
        if !google && (self.acc.oauth.is_some() || self.acc.ews.is_some()) && !own {
            return;
        }
        if let Err(e) = self.dav_pass() {
            log::info!("{}: calendars and contacts: {e}", self.acc.id);
            if matches!(e, NetError::Auth(_)) {
                self.notice(format!("calendars and contacts: {e}"));
            }
        }
    }

    /// The calendars of people just opened (issue #208), read on their own: an Exchange (EWS)
    /// account's at once, between the steps of a pass under way too; another account's with the
    /// next whole sync, as before.
    pub(in crate::worker) fn read_shared(&mut self, people: Vec<String>) {
        let Some(Conn::Ews(c)) = &self.conn else {
            let account = self.id();
            push_job(&mut self.queue, Job::SyncDav { account });
            return;
        };
        let pim: mailcraft_model::pim::Pim = self.store.load_account_doc(&self.acc.id, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
        let sync = crate::ews_pim::read_people(c, &pim, &people, now());
        self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) });
    }

    /// An Exchange account's calendars and contacts, read or not. What the server wouldn't give
    /// is said with why, once and not at every sync (issue #80: the calendar stayed empty and
    /// nothing said so); a server that couldn't be reached is the mail's to say.
    fn exchange_read(&mut self, read: Result<mailcraft_model::pim::DavSync>, pim: &mailcraft_model::pim::Pim) {
        match read {
            Ok(mut sync) => {
                self.stamp(&mut sync, pim);
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

    /// A client for the account's calendar server, signed in as its settings say: as the mail
    /// server is, with the calendar server's own user name and password, or with an OAuth Bearer token.
    pub(in crate::worker) fn dav_client(&mut self) -> Result<crate::dav::Dav> {
        // Google: the mail's access token (refreshed as for mail), to Google's servers alone.
        if self.acc.oauth.as_deref() == Some("google") && self.acc.dav.as_ref().and_then(|d| d.username.as_ref()).is_none() {
            let token = self.secret()?;
            return Ok(crate::dav::Dav::bearer(&token, GOOGLE_DAV));
        }
        // Password auth: the DAV username's password, or the mail password.
        let given = self.acc.dav.clone().unwrap_or_default();
        let (user, password) = match &given.username {
            Some(u) => (u.clone(), self.creds.as_ref().and_then(|c| c.get_secret(&self.acc, DAV_SECRET)).unwrap_or_default()),
            None => (self.acc.login_name().to_string(), Auth::held(&self.auth).secret.clone()),
        };
        Ok(crate::dav::Dav::new(&user, &password).with_digest(self.dav_digest.take()))
    }

    /// One sync with the CalDAV and CardDAV server: the addresses given by hand (issue #7), or
    /// those found from the account's address; signed in with their own user name, the mail's password,
    /// or an OAuth Bearer token (Google).
    fn dav_pass(&mut self) -> Result<()> {
        let given = self.acc.dav.clone().unwrap_or_default();
        let pim = self.pim_taken_in();
        let dav = self.dav_client()?;
        let host = self.acc.pop.as_ref().map_or(self.acc.imap.host.as_str(), |p| p.host.as_str());
        // Google OAuth: use its CalDAV and CardDAV endpoints when no address is given.
        let google = self.acc.oauth.as_deref() == Some("google") && given.username.is_none();
        let caldav = if google && given.caldav.trim().is_empty() {
            let email_enc = crate::http::pct(&self.acc.email);
            Some(format!("https://apidata.googleusercontent.com/caldav/v2/{email_enc}/events"))
        } else {
            Some(given.caldav.clone()).filter(|c| !c.trim().is_empty())
        };
        let carddav = if google && given.carddav.trim().is_empty() {
            let email_enc = crate::http::pct(&self.acc.email);
            Some(format!("https://www.googleapis.com/carddav/v1/principals/{email_enc}/lists/default/"))
        } else {
            Some(given.carddav.clone()).filter(|c| !c.trim().is_empty())
        };
        let at = crate::dav::Where {
            email: &self.acc.email,
            username: &self.acc.username,
            others: &self.acc.identities,
            imap_host: host,
            caldav: caldav.as_deref(),
            carddav: carddav.as_deref(),
            mails: !self.acc.is_calendar_only(),
        };
        let sync = crate::dav::sync(&dav, &at, &pim, now(), system_zone());
        self.dav_digest = dav.digest();
        let mut sync = sync?;
        self.stamp(&mut sync, &pim);
        self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) });
        Ok(())
    }

    /// The account's calendars, contacts and tasks as the engine keeps them, once it has taken in
    /// the last sync sent: one begun before read what changed here as it was before that sync,
    /// and sent it again (on a real Stalwart, a task changed twice: its second change was refused
    /// as "changed on the server meanwhile", the server's copy kept). Waited for a while at most,
    /// as an engine that doesn't take it in (closing) mustn't stop the syncs.
    fn pim_taken_in(&self) -> mailcraft_model::pim::Pim {
        let load = || -> mailcraft_model::pim::Pim { self.store.load_account_doc(&self.acc.id, mailcraft_model::pim::PIM_DOC).unwrap_or_default() };
        let mut pim = load();
        while waiting_for_engine(pim.dav_pass, self.dav_sent, Instant::now()) && !self.closed {
            std::thread::sleep(Duration::from_millis(100));
            pim = load();
        }
        pim
    }

    /// Number this sync, after the last taken in and the last sent, and note it sent: when, if it
    /// stored or deleted anything (one that only read has nothing to send again, and the next
    /// needn't wait for it: a mail account's syncs share its thread).
    fn stamp(&mut self, sync: &mut mailcraft_model::pim::DavSync, pim: &mailcraft_model::pim::Pim) {
        sync.pass = pim.dav_pass.max(self.dav_sent.0).saturating_add(1);
        let sent = !(sync.pushed_events.is_empty()
            && sync.pushed_tasks.is_empty()
            && sync.pushed_cards.is_empty()
            && sync.pushed_notes.is_empty()
            && sync.deleted.is_empty());
        self.dav_sent = (sync.pass, sent.then(Instant::now));
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
                // Colleagues' free/busy times, from its calendar server (issue #242).
                Ok(Job::FreeBusy { people, from, to, .. }) => self.free_busy(people, from, to),
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
                self.backoff = FIRST_BACKOFF;
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
                self.status(e.status());
                now_at.checked_add(wait)
            }
        }
    }
}

/// The system's time zone, for calendar times written without one (`UTC` when it can't say).
fn system_zone() -> mailcraft_model::zone::Zone {
    iana_time_zone::get_timezone().ok().and_then(|z| mailcraft_model::zone::Zone::named(&z)).unwrap_or_default()
}

/// How long a sync waits at most for the engine to take in the one before (it does within a
/// frame of the window).
const WAIT_FOR_ENGINE: Duration = Duration::from_secs(10);

/// Whether a sync waits: the engine has taken in up to `taken_in`, and `sent` is the last sent
/// (its number, and when it was sent, if it sent any changes), not long ago.
fn waiting_for_engine(taken_in: u64, sent: (u64, Option<Instant>), now: Instant) -> bool {
    sent.1.is_some_and(|at| taken_in < sent.0 && now.saturating_duration_since(at) < WAIT_FOR_ENGINE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sync waits for the engine to take in the one sent before, for a while at most; the first
    /// doesn't wait, nor one after the last was taken in (or a later one, saved by another run).
    #[test]
    fn a_sync_waits_for_the_one_before_to_be_taken_in() {
        let now = Instant::now();
        assert!(!waiting_for_engine(0, (0, None), now), "none sent yet");
        assert!(!waiting_for_engine(4, (5, None), now), "the last sent changed nothing");
        assert!(waiting_for_engine(4, (5, Some(now)), now), "the last sent not taken in");
        assert!(!waiting_for_engine(5, (5, Some(now)), now), "taken in");
        assert!(!waiting_for_engine(9, (5, Some(now)), now), "a later one taken in");
        assert!(!waiting_for_engine(4, (5, Some(now)), now + WAIT_FOR_ENGINE), "not taken in after a while: it goes on");
    }
}
