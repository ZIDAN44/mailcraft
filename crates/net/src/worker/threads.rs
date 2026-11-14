//! What the pool runs on threads of its own, so it never waits behind a sync: browser sign-in,
//! looking up an address's servers, asking a JMAP server how it's signed in to, the release
//! check, a message's pictures, calendars read from their links, and Outlook data files written
//! (`imports.rs` reads them).

use super::*;

impl Pool {
    /// The releases published since this one (the update check).
    pub(super) fn check_updates(&mut self) {
        let sink = self.sink.clone();
        let spawned = std::thread::Builder::new().name("mail-updates".into()).spawn(move || {
            let (releases, error) = match crate::updates::releases(&crate::updates::Feed::current()) {
                Ok(r) => (r, None),
                Err(e) => (Vec::new(), Some(e.to_string())),
            };
            sink.emit(Event::Releases { releases, error });
        });
        if let Err(e) = spawned {
            self.sink.emit(Event::Releases { releases: Vec::new(), error: Some(e.to_string()) });
        }
    }

    /// An address's servers, as its provider publishes them (autoconfiguration).
    pub(super) fn discover(&mut self, email: String) {
        let sink = self.sink.clone();
        let spawned = std::thread::Builder::new().name("mail-discover".into()).spawn(move || {
            let found = crate::autoconfig::discover(&email);
            let (config, source) = match found {
                Some(f) => (Some(Box::new(f.config)), f.source),
                None => (None, String::new()),
            };
            sink.emit(Event::Discovered { email, config, source });
        });
        if let Err(e) = spawned {
            log::warn!("autoconfiguration: {e}");
        }
    }

    /// How a JMAP server is signed in to: with a password, or through the browser with the
    /// single sign-on its address names.
    pub(super) fn probe_jmap(&mut self, url: String) {
        let sink = self.sink.clone();
        let spawned = std::thread::Builder::new().name("mail-probe".into()).spawn(move || {
            let probe = crate::oauth::oidc::probe(&url);
            sink.emit(Event::JmapProbed { url, probe });
        });
        if let Err(e) = spawned {
            log::warn!("asking the JMAP server how it's signed in to: {e}");
        }
    }

    /// Browser sign-in for an OAuth account, on its own thread: the page opens through the UI
    /// (`Event::OpenUrl`), the refresh token goes to the keychain, then the account starts. A
    /// single sign-on's service is asked where its pages are first.
    pub(super) fn sign_in(&mut self, account: AccountConfig) {
        let (sink, creds) = (self.sink.clone(), self.creds.clone());
        let spawned = std::thread::Builder::new().name("mail-sign-in".into()).spawn(move || {
            let p = match crate::oauth::sign_in_provider(&account) {
                Ok(p) => p,
                // Without a client id, signing in again can't help: the server settings can.
                Err(NetError::Auth(error)) if error == mailcraft_model::NO_CLIENT_ID => {
                    return sink.emit(Event::Status { account: account.id, status: SyncStatus::Error { error } });
                }
                Err(e) => {
                    let error = match e {
                        NetError::Auth(m) => m,
                        e => e.to_string(),
                    };
                    return sink.emit(Event::Status { account: account.id, status: SyncStatus::NeedsPassword { error } });
                }
            };
            let mut open = |url: &str| -> std::result::Result<(), String> {
                sink.emit(Event::OpenUrl { url: url.to_string() });
                Ok(())
            };
            match crate::oauth::authorize(&p, &account.email, &mut open, Duration::from_secs(5 * 60)) {
                Ok(t) => match t.refresh {
                    Some(r) => match credentials::keep_token(creds.as_ref(), &account, &r) {
                        Ok(()) => sink.emit(Event::SignedIn { account: account.id, user: t.user }),
                        Err(e) => sink.emit(Event::Status {
                            account: account.id,
                            status: SyncStatus::NeedsPassword { error: format!("the sign-in couldn't be kept in the keychain: {e}") },
                        }),
                    },
                    None => sink.emit(Event::Status {
                        account: account.id,
                        status: SyncStatus::NeedsPassword { error: format!("{} gave no lasting sign-in; try again", p.name) },
                    }),
                },
                Err(e) => sink.emit(Event::Status { account: account.id, status: SyncStatus::NeedsPassword { error: e.to_string() } }),
            }
        });
        if let Err(e) = spawned {
            log::warn!("sign-in: {e}");
        }
    }

    /// Issue #9, online: ask the user in the browser to let an Exchange Online account search its
    /// organisation's directory, a sign-in for those permissions alone; its refresh token is kept
    /// apart from the mail's, which stays as it was whatever the answer.
    pub(super) fn allow_directory(&mut self, account: AccountConfig) {
        let Some(p) = crate::oauth::directory_provider(&account) else {
            let error = "only an Exchange Online account signed in with Microsoft asks for this".to_string();
            return self.sink.emit(Event::DirectoryAllowed { account: account.id, error: Some(error) });
        };
        let (sink, creds) = (self.sink.clone(), self.creds.clone());
        let spawned = std::thread::Builder::new().name("mail-directory-consent".into()).spawn(move || {
            let mut open = |url: &str| -> std::result::Result<(), String> {
                sink.emit(Event::OpenUrl { url: url.to_string() });
                Ok(())
            };
            let error = match crate::oauth::authorize(&p, &account.email, &mut open, Duration::from_secs(5 * 60)) {
                Ok(t) => match t.refresh {
                    Some(r) => creds
                        .set_secret(&account, DIRECTORY_SECRET, &r)
                        .err()
                        .map(|e| format!("the permission couldn't be kept in the keychain: {e}")),
                    None => Some(format!("{} gave no lasting permission; try again", p.name)),
                },
                Err(e) => Some(e.to_string()),
            };
            sink.emit(Event::DirectoryAllowed { account: account.id, error });
        });
        if let Err(e) = spawned {
            log::warn!("directory consent: {e}");
        }
    }

    pub(super) fn fetch_url(&mut self, url: String) {
        if self.fetcher.is_none() {
            let (tx, rx) = mpsc::channel::<String>();
            let sink = self.sink.clone();
            let spawned = std::thread::Builder::new().name("mail-pictures".into()).spawn(move || {
                for url in rx {
                    // A message's picture: from the internet only, never this computer's network.
                    let r = crate::http::get_public(&url, MAX_PICTURE);
                    let e = match r {
                        Ok(resp) if resp.ok() => Event::Fetched { url, data: Some(resp.body), error: None },
                        Ok(resp) => Event::Fetched { url, data: None, error: Some(format!("HTTP {}", resp.status)) },
                        Err(e) => Event::Fetched { url, data: None, error: Some(e.to_string()) },
                    };
                    sink.emit(e);
                }
            });
            match spawned {
                Ok(_) => self.fetcher = Some(tx),
                Err(e) => return self.sink.emit(Event::Fetched { url, data: None, error: Some(e.to_string()) }),
            }
        }
        if let Some(tx) = &self.fetcher {
            let _ = tx.send(url);
        }
    }

    /// Read a calendar subscribed to by its link (issue #7) on a thread of its own: the link the
    /// user gave (their network's too: a calendar server at home), HTTPS never left for HTTP.
    pub(super) fn fetch_calendar(&mut self, account: String, calendar: String, url: String, etag: String, modified: String) {
        let sink = self.sink.clone();
        let (acc, cal) = (account.clone(), calendar.clone());
        let spawned = std::thread::Builder::new().name("mail-calendar".into()).spawn(move || {
            let e = match crate::web_calendar::fetch(&url, &etag, &modified) {
                Ok(crate::web_calendar::Fetched::Unchanged) => {
                    Event::CalendarFetched { account, calendar, ics: None, etag, modified, unchanged: true, error: None }
                }
                Ok(crate::web_calendar::Fetched::Calendar { ics, etag, modified }) => {
                    Event::CalendarFetched { account, calendar, ics: Some(ics), etag, modified, unchanged: false, error: None }
                }
                Err(e) => Event::CalendarFetched {
                    account,
                    calendar,
                    ics: None,
                    etag: String::new(),
                    modified: String::new(),
                    unchanged: false,
                    error: Some(e.to_string()),
                },
            };
            sink.emit(e);
        });
        if let Err(e) = spawned {
            self.sink.emit(Event::CalendarFetched {
                account: acc,
                calendar: cal,
                ics: None,
                etag: String::new(),
                modified: String::new(),
                unchanged: false,
                error: Some(e.to_string()),
            });
        }
    }

    /// Export to a data file on a thread of its own.
    pub(super) fn export_pst(&mut self, account: String, path: String, name: String, follow_up: mailcraft_model::organize::FollowUpDays) {
        // With the changes made just before it.
        self.edits.flush();
        let sink = self.sink.clone();
        let store = self.store.clone();
        let spawned = std::thread::Builder::new().name("mail-export".into()).spawn(move || {
            let mut progress = |messages| {
                sink.emit(Event::Exported {
                    account: account.clone(),
                    messages,
                    skipped: 0,
                    already: 0,
                    occurrences: 0,
                    replaced: 0,
                    done: false,
                    error: None,
                })
            };
            let event = match crate::pst_export::export(&store, &account, &name, std::path::Path::new(&path), &follow_up, &mut progress) {
                Ok(done) => Event::Exported {
                    account,
                    messages: done.messages,
                    skipped: done.skipped,
                    already: done.already,
                    occurrences: done.occurrences,
                    replaced: done.pim_replaced,
                    done: true,
                    error: None,
                },
                Err(e) => Event::Exported { account, messages: 0, skipped: 0, already: 0, occurrences: 0, replaced: 0, done: true, error: Some(e) },
            };
            sink.emit(event);
        });
        if let Err(e) = spawned {
            self.sink.emit(Event::Notice { account: String::new(), message: format!("export: {e}") });
        }
    }
}
