//! An account's worker: signing in and connecting (IMAP, JMAP, Graph, EWS, POP), and what a failure means.

use super::*;

impl Worker {
    /// What to sign in with: the password, or for OAuth accounts a current access token
    /// (refreshed from the stored refresh token when needed).
    pub(super) fn secret(&mut self) -> Result<String> {
        // Held while refreshing: the account's other connection waits for the new token rather
        // than refreshing with the same old one.
        let mut auth = self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    pub(super) fn fresh_token(&mut self) {
        if self.acc.oauth.is_none() || !matches!(self.conn, Some(Conn::Graph(_) | Conn::Jmap(_))) {
            return;
        }
        match self.secret() {
            Ok(t) => self.use_token(&t),
            Err(e) => self.failed(e),
        }
    }

    /// Drop the access token the server refused and fetch a new one with the refresh token.
    pub(super) fn renew_token(&mut self) -> Result<String> {
        self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).access = None;
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
    pub(super) fn granted(&self) -> String {
        self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).access.as_ref().map(|t| t.granted.clone()).unwrap_or_default()
    }

    /// Note a failure; drop the connection when it is unusable.
    pub(super) fn failed(&mut self, e: NetError) {
        log::warn!("{}: {e}", self.acc.id);
        match e {
            // Microsoft took the token but won't open the mailbox: nothing to retry until changed.
            NetError::Auth(m) | NetError::Bad(m) | NetError::No(m)
                if self.acc.oauth.as_deref() == Some("microsoft") && m.to_ascii_lowercase().contains("authenticated but not connected") =>
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
                        self.next_attempt = Instant::now();
                        self.status(SyncStatus::Connecting);
                    }
                    Err(e) => self.failed(e),
                }
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

    pub(super) fn connect(&mut self) -> Result<()> {
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
                    let found = crate::ews::autodiscover_as(&self.acc.email, &user, &secret, None, self.acc.user_agent.as_deref())?;
                    if let Err(e) = self.store.save_account_doc(&self.acc.id, "ews.json", &found) {
                        log::warn!("{}: the Exchange server found isn't kept, and is looked for again next time: {e}", self.acc.id);
                    }
                    found
                } else {
                    kept
                }
            } else {
                url.clone()
            };
            let c = crate::ews::Client::connect_as(&url, &user, &secret, &self.acc.email, self.acc.user_agent.as_deref())?;
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
            let mut c = Client::connect_trusting(&self.acc.imap, &self.acc.trusted_certificates)?;
            c.login(&user, &secret)?;
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
            self.backoff = Duration::from_secs(5);
        }
    }

    /// An IMAP connection signed in with an OAuth token. Microsoft can take the token and then
    /// refuse the mailbox ("User is authenticated but not connected"): IMAP is switched off for
    /// it (Outlook.com's own setting, or an organisation's), or the browser signed in as another
    /// address. The address it signed in as is tried once; otherwise it's a refusal that says
    /// what to do, not a sign-in to ask for again (that only loops).
    pub(super) fn imap_with_token(&self, user: &str, token: &str) -> Result<Client<Stream>> {
        let mut c = Client::connect_trusting(&self.acc.imap, &self.acc.trusted_certificates)?;
        let m = match c.login_xoauth2(user, token) {
            Ok(()) => return Ok(c),
            Err(NetError::Auth(m) | NetError::Bad(m) | NetError::No(m))
                if self.acc.oauth.as_deref() == Some("microsoft") && m.to_ascii_lowercase().contains("not connected") =>
            {
                m
            }
            Err(e) => return Err(e),
        };
        log::warn!("{}: {m}", self.acc.id);
        let other = self.acc.oauth_user.as_deref().filter(|o| !o.eq_ignore_ascii_case(user));
        if let Some(other) = other {
            let mut c = Client::connect_trusting(&self.acc.imap, &self.acc.trusted_certificates)?;
            if c.login_xoauth2(other, token).is_ok() {
                return Ok(c);
            }
        }
        Err(NetError::MailboxClosed(microsoft_imap_closed(&self.acc.email, other)))
    }

    /// A connection that couldn't be made is never tried again at once (that looped, showing
    /// "Connecting…" for good): a refusal (Graph's "no mailbox", a setting the server won't take)
    /// waits until asked, anything else for the backoff.
    pub(super) fn connect_failed(&mut self, e: NetError) {
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

    pub(super) fn connect_and_sync(&mut self) {
        if let Err(e) = self.connect() {
            self.connect_failed(e);
            return;
        }
        self.replay();
        self.flush_outbox();
        // Drafts kept while offline go to the server first.
        self.queue_kept_drafts();
        self.last_full = None;
    }

    pub(super) fn client(&mut self) -> Result<&mut Client<Stream>> {
        match self.conn.as_mut() {
            Some(Conn::Imap(c)) => Ok(c),
            _ => Err(NetError::Closed("not connected".into())),
        }
    }

    /// Save the JMAP / Graph numbering after it changed; not once the account was removed.
    pub(super) fn save_jstate(&self) {
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
