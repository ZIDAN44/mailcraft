//! An account's worker: signing in and connecting (IMAP, JMAP, Graph, EWS, POP), and what a failure means.

use super::*;

/// What Microsoft says when it took an OAuth token but won't open the mailbox over IMAP ("User is
/// authenticated but not connected").
const MICROSOFT_NOT_CONNECTED: &str = "authenticated but not connected";

/// Whether a server's refusal is Microsoft's [`MICROSOFT_NOT_CONNECTED`].
fn microsoft_not_connected(said: &str) -> bool {
    said.to_ascii_lowercase().contains(MICROSOFT_NOT_CONNECTED)
}

/// The connections made again at once with a renewed OAuth token, lately. A refusal after the
/// server took a token gets a fresh one each time ([`Worker::failed`]); a server that takes every
/// token for its folder list and refuses it for what comes next would have a new token asked
/// for, and a connection made, without a pause and without end. So only so many go at once.
#[derive(Default)]
pub(crate) struct Renewals(VecDeque<Instant>);

/// How many connections a renewed token makes again at once within [`RENEWALS_WITHIN`]; each
/// one past that waits until the oldest of them is that long ago.
const RENEWALS_AT_ONCE: usize = 3;
const RENEWALS_WITHIN: Duration = Duration::from_secs(10 * 60);

impl Renewals {
    /// A token was renewed at `now`: when its connection is made (`now`, or later).
    fn connect_at(&mut self, now: Instant) -> Instant {
        while self.0.front().is_some_and(|t| now.saturating_duration_since(*t) >= RENEWALS_WITHIN) {
            self.0.pop_front();
        }
        let at = match self.0.front() {
            Some(oldest) if self.0.len() >= RENEWALS_AT_ONCE => {
                let at = oldest.checked_add(RENEWALS_WITHIN).unwrap_or(now).max(now);
                self.0.pop_front();
                at
            }
            _ => now,
        };
        self.0.push_back(at);
        at
    }
}

/// Renewals go at once a few times, then no faster than [`RENEWALS_AT_ONCE`] in
/// [`RENEWALS_WITHIN`], however long the server goes on refusing; and at once again after a quiet
/// while.
#[test]
fn renewed_tokens_connect_at_once_only_so_often() {
    let start = Instant::now();
    let mut r = Renewals::default();
    let mut now = start;
    let mut made = Vec::new();
    // A server refusing each token a second after it took it, for an hour.
    while now.saturating_duration_since(start) < Duration::from_secs(60 * 60) {
        let at = r.connect_at(now);
        made.push(at.saturating_duration_since(start).as_secs());
        now = at + Duration::from_secs(1);
    }
    assert_eq!(made.get(..4), Some(&[0, 1, 2, 600][..]), "{made:?}");
    assert!(made.len() <= 6 * RENEWALS_AT_ONCE + 1, "{} connections in an hour: {made:?}", made.len());
    assert!(r.0.len() <= RENEWALS_AT_ONCE);
    // Left alone for the while, the next is at once.
    let later = now + RENEWALS_WITHIN + RENEWALS_WITHIN;
    assert_eq!(r.connect_at(later), later);
}

/// Where a test has autodiscovery ask (its fake server), and what it does once the server is
/// found, before it's kept: its way to come between them. Taken once, on this thread.
type InTest = (Option<String>, Option<Box<dyn FnOnce()>>);

#[cfg(test)]
thread_local!(pub(crate) static AUTODISCOVER_IN_TEST: std::cell::RefCell<InTest> = const { std::cell::RefCell::new((None, None)) });

/// Where autodiscovery asks: the usual addresses (`AUTODISCOVER_IN_TEST` in a test).
#[cfg(test)]
fn autodiscover_in_test() -> InTest {
    AUTODISCOVER_IN_TEST.with(|a| std::mem::take(&mut *a.borrow_mut()))
}

#[cfg(not(test))]
fn autodiscover_in_test() -> InTest {
    (None, None)
}

impl Worker {
    /// What to sign in with: the password, or for OAuth accounts a current access token
    /// (refreshed from the stored refresh token when needed).
    pub(crate) fn secret(&mut self) -> Result<String> {
        // Held while refreshing: the account's other connection waits for the new token rather
        // than refreshing with the same old one.
        let mut auth = Auth::held(&self.auth);
        let Some(id) = self.acc.oauth.clone() else { return Ok(auth.secret.clone()) };
        if let Some(t) = &auth.access
            && !t.expired()
        {
            return Ok(t.access.clone());
        }
        let p = crate::oauth::provider_for_account(&self.acc)
            .ok_or_else(|| NetError::Auth(format!("signing in with {id} isn't set up in this copy of MailCraft")))?;
        let t = crate::oauth::refresh(&p, &auth.secret).inspect_err(|e| {
            // The refresh token itself refused: only signing in again helps.
            if matches!(e, NetError::Auth(_)) {
                self.token_retried = true;
            }
        })?;
        // Some providers hand out a new refresh token each time: keep the newest.
        if let Some(r) = t.refresh.clone().filter(|r| *r != auth.secret) {
            auth.secret = r.clone();
            if let Some(c) = &self.creds
                && let Err(e) = c.set(&self.acc, &r)
            {
                self.sink
                    .emit(Event::Notice { account: self.acc.id.clone(), message: format!("the new sign-in couldn't be saved in the keychain: {e}") });
            }
        }
        let access = t.access.clone();
        auth.access = Some(t);
        Ok(access)
    }

    /// Before each job: an OAuth account's Graph or JMAP connection carries the current access
    /// token, refreshed when it ran out by the wall clock (or by the account's other connection).
    pub(crate) fn fresh_token(&mut self) {
        if self.acc.oauth.is_none() || !matches!(self.conn, Some(Conn::Graph(_) | Conn::Jmap(_))) {
            return;
        }
        match self.secret() {
            Ok(t) => self.use_token(&t),
            Err(e) => self.failed(e),
        }
    }

    /// Drop the access token the server refused and fetch a new one with the refresh token.
    pub(crate) fn renew_token(&mut self) -> Result<String> {
        Auth::held(&self.auth).access = None;
        let t = self.secret()?;
        self.use_token(&t);
        Ok(t)
    }

    fn use_token(&mut self, token: &str) {
        match self.conn.as_mut() {
            Some(Conn::Graph(c)) => c.set_token(token),
            Some(Conn::Jmap(c)) => c.set_bearer(token),
            _ => {}
        }
    }

    /// What Microsoft's sign-in granted (its scopes), when known.
    pub(crate) fn granted(&self) -> String {
        Auth::held(&self.auth).access.as_ref().map(|t| t.granted.clone()).unwrap_or_default()
    }

    /// Note a failure; drop the connection when it is unusable.
    pub(crate) fn failed(&mut self, e: NetError) {
        log::warn!("{}: {e}", self.acc.id);
        match e {
            // Microsoft took the token but won't open the mailbox: nothing to retry until changed.
            NetError::Auth(m) | NetError::Bad(m) | NetError::No(m)
                if self.acc.oauth.as_deref() == Some("microsoft") && microsoft_not_connected(&m) =>
            {
                self.conn = None;
                self.auth_failed = true;
                self.status(NetError::MailboxClosed(microsoft_imap_closed(&self.acc.email, self.acc.oauth_user.as_deref())).status());
            }
            // Microsoft's servers refuse passwords from mail apps: asking for it again only loops.
            NetError::Auth(m) if mailcraft_model::microsoft_password_account(&self.acc) => {
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::Error { error: format!("{MICROSOFT_NO_PASSWORDS} ({m})"), kind: FailureKind::MicrosoftNoPasswords });
            }
            // An OAuth token refused (expired early, revoked by a password change elsewhere, or
            // taken before the computer slept): a fresh one from the refresh token, then the
            // connection again, once. Only a refused refresh, or a fresh token refused too
            // before the server took it for mail (its folders listed), means signing in again;
            // a renewal the sign-in service fails is tried again later.
            NetError::Auth(m) if self.acc.oauth.is_some() && !self.token_retried => {
                log::info!("{}: token refused ({m}); refreshing it", self.acc.id);
                self.token_retried = true;
                match self.renew_token() {
                    Ok(_) => {
                        self.conn = None;
                        // At once, unless tokens are being refused one after another.
                        let now = Instant::now();
                        self.next_attempt = self.renewals.connect_at(now);
                        if self.next_attempt > now {
                            let said = format!("the server keeps refusing the sign-in it is given ({m}); trying again in a while");
                            self.status(NetError::Refused(said).status());
                        } else {
                            self.status(SyncStatus::Connecting);
                        }
                    }
                    Err(e) => self.failed(e),
                }
            }
            // Google wants an app password (2-Step Verification on): said plainly, so it can be
            // translated, where Google's English text with a help page was passed on (issue
            // #164). Google's own words are in the log above.
            NetError::Auth(m) if mailcraft_model::google_wants_app_password(&m) => {
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::NeedsPassword { error: mailcraft_model::GOOGLE_APP_PASSWORD_NEEDED.into() });
            }
            NetError::Auth(m) => {
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::NeedsPassword { error: m });
            }
            e if e.is_fatal() => {
                self.conn = None;
                self.back_off();
                self.status(e.status());
            }
            e => self.notice(e.to_string()),
        }
    }

    /// Not connecting again for the wait, which doubles for the next time (five minutes at most).
    fn back_off(&mut self) {
        self.next_attempt = Instant::now() + self.backoff;
        self.backoff = (self.backoff * 2).min(Duration::from_secs(300));
    }

    pub(crate) fn connect(&mut self) -> Result<()> {
        if self.conn.is_some() {
            return Ok(());
        }
        self.open()?;
        // The drafts kept on this computer go to the server on every connection, ahead of
        // anything queued ([`Self::queue_kept_drafts`]).
        self.kept.due = self.main.is_none();
        Ok(())
    }

    fn open(&mut self) -> Result<()> {
        self.status(SyncStatus::Connecting);
        let secret = self.secret()?;
        let user = self.acc.login_name().to_string();
        if let Some(url) = &self.acc.graph {
            self.conn = Some(Conn::Graph(crate::graph::Client::connect(url, &secret)?));
            self.signed_in();
            return Ok(());
        }
        if self.acc.pop.is_some() {
            crate::pop::open(&self.acc, &secret)?.quit()?;
            self.conn = Some(Conn::Pop);
            self.signed_in();
            return Ok(());
        }
        if let Some(url) = &self.acc.ews {
            // `autodiscover`: found the first time, then kept.
            let url = if url == crate::ews::AUTODISCOVER {
                // One that can't be read is found again (and kept anew: it's only the address).
                let kept: String = self.store.load_account_doc(&self.acc.id, "ews.json").unwrap_or_else(|e| {
                    log::warn!("{}: the Exchange server found before couldn't be read: {e}", self.acc.id);
                    String::new()
                });
                if kept.is_empty() {
                    let (base, then) = autodiscover_in_test();
                    let found = crate::ews::autodiscover_as(&self.acc.email, &user, &secret, base.as_deref(), self.acc.user_agent.as_deref())?;
                    if let Some(then) = then {
                        then();
                    }
                    // Not once the account was removed (as autodiscovery went on): its files would
                    // come back.
                    let kept = self.store.while_kept(&self.acc.id, self.removals, || self.store.save_account_doc(&self.acc.id, "ews.json", &found));
                    if let Some(Err(e)) = kept {
                        log::warn!("{}: the Exchange server found isn't kept, and is looked for again next time: {e}", self.acc.id);
                    }
                    found
                } else {
                    kept
                }
            } else {
                url.clone()
            };
            // Its other addresses and its sign-in name: a meeting may name the user by any (#144).
            let also = self.acc.addresses().skip(1).chain([self.acc.username.as_str()]);
            let c = crate::ews::Client::connect_as(&url, &user, &secret, &self.acc.email, self.acc.user_agent.as_deref())?.with_addresses(also);
            c.set_shared(&self.acc.shared_mailboxes);
            self.conn = Some(Conn::Ews(Box::new(c)));
            self.signed_in();
            return Ok(());
        }
        if let Some(url) = &self.acc.jmap {
            self.conn = Some(Conn::Jmap(jmap::Client::connect(url, &user, &secret, self.acc.oauth.is_some())?));
            self.signed_in();
            return Ok(());
        }
        let c = if self.acc.oauth.is_some() {
            self.imap_with_token(&user, &secret)?
        } else {
            let mut c = Client::connect_trusting(&self.acc.imap, crate::stream::Trusted::of(&self.acc))?;
            c.login_as(&user, &secret, self.acc.sign_in)?;
            c
        };
        self.conn = Some(Conn::Imap(c));
        self.signed_in();
        Ok(())
    }

    /// Signed in. The second connection waits its shortest again from here; the main one only
    /// once it has synced the whole account ([`Self::sync_all`]): a failure after signing in
    /// that comes back every time (one folder the server ends the connection on) signed in
    /// again every five seconds for good, the folders after that one never read.
    fn signed_in(&mut self) {
        if self.main.is_some() {
            self.backoff = FIRST_BACKOFF;
        }
    }

    /// An IMAP connection signed in with an OAuth token. Microsoft can take the token and then
    /// refuse the mailbox ("User is authenticated but not connected"): IMAP is switched off for
    /// it (Outlook.com's own setting, or an organisation's), or the browser signed in as another
    /// address. The address it signed in as is tried once; otherwise it's a refusal that says
    /// what to do, not a sign-in to ask for again (that only loops).
    pub(crate) fn imap_with_token(&self, user: &str, token: &str) -> Result<Client<Stream>> {
        let mut c = Client::connect_trusting(&self.acc.imap, crate::stream::Trusted::of(&self.acc))?;
        let m = match c.login_xoauth2(user, token) {
            Ok(()) => return Ok(c),
            Err(NetError::Auth(m) | NetError::Bad(m) | NetError::No(m))
                if self.acc.oauth.as_deref() == Some("microsoft") && microsoft_not_connected(&m) =>
            {
                m
            }
            Err(e) => return Err(e),
        };
        log::warn!("{}: {m}", self.acc.id);
        let other = self.acc.oauth_user.as_deref().filter(|o| !o.eq_ignore_ascii_case(user));
        if let Some(other) = other {
            let mut c = Client::connect_trusting(&self.acc.imap, crate::stream::Trusted::of(&self.acc))?;
            if c.login_xoauth2(other, token).is_ok() {
                return Ok(c);
            }
        }
        Err(NetError::MailboxClosed(microsoft_imap_closed(&self.acc.email, other)))
    }

    /// A connection that couldn't be made is never tried again at once (that looped, showing
    /// "Connecting…" for good): a refusal (Graph's "no mailbox", a setting the server won't take)
    /// waits until asked, anything else for the backoff.
    pub(crate) fn connect_failed(&mut self, e: NetError) {
        match e {
            e @ NetError::Auth(_) => self.failed(e),
            e if e.is_fatal() => self.failed(e),
            // Asked to come back later (a POP mailbox another program holds): tried again by
            // itself, as a server that can't be reached is. Left to wait until asked, the
            // account checked no more.
            e @ NetError::Later(_) => {
                log::warn!("{}: {e}", self.acc.id);
                self.conn = None;
                self.back_off();
                self.status(e.status());
            }
            e => {
                log::warn!("{}: {e}", self.acc.id);
                self.conn = None;
                self.auth_failed = true;
                self.status(e.status());
            }
        }
    }

    pub(crate) fn connect_and_sync(&mut self) {
        if let Err(e) = self.connect() {
            self.connect_failed(e);
            return;
        }
        self.replay();
        self.flush_outbox();
        // Drafts kept while offline go to the server first.
        self.queue_kept_drafts();
        // Every folder read now, unless it looks only when asked (issue #104): then Send / Receive
        // asks for that itself.
        if !self.acc.check_by_hand {
            self.last_full = None;
        }
    }

    pub(crate) fn client(&mut self) -> Result<&mut Client<Stream>> {
        match self.conn.as_mut() {
            Some(Conn::Imap(c)) => Ok(c),
            _ => Err(NetError::Closed("not connected".into())),
        }
    }

    /// Save the JMAP / Graph numbering after it changed; not once the account was removed.
    pub(crate) fn save_jstate(&self) {
        if self.acc.is_api()
            && let Some(Err(e)) = self.store.while_kept(&self.acc.id, self.removals, || self.jstate.save(&self.store, &self.acc.id))
        {
            log::warn!("{}: {e}", self.acc.id);
        }
    }
}

/// The account's main threads (not their second connections) that have ended, for the tests to
/// wait on rather than time: by the store's folder and the account.
#[cfg(test)]
pub(crate) mod ended {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use mailcraft_store::Store;

    static ENDED: Mutex<Vec<(PathBuf, String)>> = Mutex::new(Vec::new());

    /// Noted as a main thread ends, by being dropped.
    pub(crate) struct Ending(pub(crate) PathBuf, pub(crate) String);

    impl Drop for Ending {
        fn drop(&mut self) {
            if let Ok(mut e) = ENDED.lock() {
                e.push((std::mem::take(&mut self.0), std::mem::take(&mut self.1)));
            }
        }
    }

    /// How many of `account`'s main threads over `store` have ended.
    pub(crate) fn count(store: &Store, account: &str) -> usize {
        ENDED.lock().map_or(0, |e| e.iter().filter(|(root, a)| root == store.root() && a == account).count())
    }
}
