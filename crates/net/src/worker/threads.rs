//! What the pool runs on threads of its own, so it never waits behind a sync: browser sign-in,
//! a message's pictures, and Outlook data files looked at, read or written.

use mailcraft_model::job::{DataFileFacts, ImportPhase, ImportProgress, ImportReport, Work};
use mailcraft_model::pim::{PIM_DOC, Pim};

use super::*;

impl Pool {
    /// Browser sign-in for an OAuth account, on its own thread: the page opens through the UI
    /// (`Event::OpenUrl`), the refresh token goes to the keychain, then the account starts.
    pub(super) fn sign_in(&mut self, account: AccountConfig) {
        let Some(p) = crate::oauth::provider_for_account(&account) else {
            let error = format!("signing in with {} isn't set up in this copy of MailCraft", account.oauth.as_deref().unwrap_or("that provider"));
            return self.sink.emit(Event::Status { account: account.id, status: SyncStatus::NeedsPassword { error } });
        };
        let (sink, creds) = (self.sink.clone(), self.creds.clone());
        let spawned = std::thread::Builder::new().name("mail-sign-in".into()).spawn(move || {
            let mut open = |url: &str| -> std::result::Result<(), String> {
                sink.emit(Event::OpenUrl { url: url.to_string() });
                Ok(())
            };
            match crate::oauth::authorize(&p, &account.email, &mut open, Duration::from_secs(5 * 60)) {
                Ok(t) => match t.refresh {
                    Some(r) => match creds.set(&account, &r) {
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
    pub(super) fn export_pst(&mut self, account: String, path: String, name: String) {
        let sink = self.sink.clone();
        let store = self.store.clone();
        let spawned = std::thread::Builder::new().name("mail-export".into()).spawn(move || {
            let mut progress =
                |messages| sink.emit(Event::Exported { account: account.clone(), messages, skipped: 0, already: 0, done: false, error: None });
            let event = match crate::pst_export::export(&store, &account, &name, std::path::Path::new(&path), &mut progress) {
                Ok(done) => {
                    Event::Exported { account, messages: done.messages, skipped: done.skipped, already: done.already, done: true, error: None }
                }
                Err(e) => Event::Exported { account, messages: 0, skipped: 0, already: 0, done: true, error: Some(e) },
            };
            sink.emit(event);
        });
        if let Err(e) = spawned {
            self.sink.emit(Event::Notice { account: String::new(), message: format!("export: {e}") });
        }
    }

    /// Import a data file on a thread of its own: an Outlook one (`.pst`), or MailCraft's own
    /// (`.mailcraft`, copied in as it is).
    pub(super) fn import_pst(&mut self, account: String, path: String) {
        let sink = self.sink.clone();
        let store = self.store.clone();
        let id = account.clone();
        let spawned = std::thread::Builder::new().name("mail-import".into()).spawn(move || {
            let began = Instant::now();
            // The file the account's mail was copied into ("" for a cache that isn't kept in one).
            let data_file = |store: &Store| store.data_file(&account).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            if path.to_ascii_lowercase().ends_with(".mailcraft") {
                match store.import_data_file(&account, std::path::Path::new(&path)) {
                    Ok(messages) => {
                        let folders = store.load_folders(&account).unwrap_or_default();
                        let paths: Vec<String> = folders.iter().map(|f| f.path.clone()).collect();
                        // Its contacts, calendar and notes came with it.
                        let pim = store.load_account_doc::<Pim>(&account, PIM_DOC).unwrap_or_default();
                        let report = ImportReport {
                            file: path,
                            data_file: data_file(&store),
                            messages: count(messages),
                            folders: u32::try_from(paths.len()).unwrap_or(u32::MAX),
                            contacts: count(pim.cards.len()),
                            events: count(pim.events.len()),
                            notes: count(pim.notes.len()),
                            runs: 1,
                            elapsed_ms: ms(began),
                            ..Default::default()
                        };
                        sink.emit(Event::Folders { account: account.clone(), folders });
                        for folder in paths {
                            sink.emit(Event::FolderChanged { account: account.clone(), folder });
                        }
                        sink.emit(Event::Imported { account, messages, skipped: 0, done: true, error: None, report: Some(Box::new(report)) });
                    }
                    Err(e) => sink.emit(Event::Imported { account, messages: 0, skipped: 0, done: true, error: Some(e.to_string()), report: None }),
                }
                return;
            }
            // No totals yet: the bar has none to go by.
            let mut progress = |t: crate::pst::Progress| {
                let run_ms = ms(began);
                let progress = ImportProgress {
                    phase: ImportPhase::Importing,
                    // Items read, which only go up: a batch that can't be written is still read.
                    work: Work { done: count(t.read), ..Work::default() },
                    imported: count(t.imported),
                    skipped: count(t.skipped),
                    elapsed_ms: run_ms,
                    run_ms,
                    ..ImportProgress::default()
                };
                sink.emit(Event::ImportProgress { account: account.clone(), progress });
            };
            let result = crate::pst::import(std::path::Path::new(&path), &store, &account, &mut progress);
            match result {
                Ok(done) => {
                    let mut report = ImportReport {
                        file: path,
                        data_file: data_file(&store),
                        messages: count(done.messages),
                        folders: u32::try_from(done.folders.len()).unwrap_or(u32::MAX),
                        contacts: count(done.contacts),
                        events: count(done.events),
                        notes: count(done.notes),
                        elapsed_ms: ms(began),
                        runs: 1,
                        ..ImportReport::default()
                    };
                    for (reason, n) in &done.skips {
                        report.skip(*reason, count(*n));
                    }
                    let paths: Vec<String> = done.folders.iter().map(|f| f.path.clone()).collect();
                    sink.emit(Event::Folders { account: account.clone(), folders: done.folders });
                    for folder in paths {
                        sink.emit(Event::FolderChanged { account: account.clone(), folder });
                    }
                    // A last save that failed is said with what came in (the account stays).
                    let (messages, skipped, error) = (done.messages, done.skipped, done.last_save);
                    sink.emit(Event::Imported { account, messages, skipped, done: true, error, report: Some(Box::new(report)) });
                }
                Err(e) => sink.emit(Event::Imported { account, messages: 0, skipped: 0, done: true, error: Some(e), report: None }),
            }
        });
        // No thread, no import: it ends here, as a failed one (the account isn't left importing).
        if let Err(e) = spawned {
            self.sink.emit(Event::Imported { account: id, messages: 0, skipped: 0, done: true, error: Some(format!("import: {e}")), report: None });
        }
    }

    /// Look at a data file before it's imported, on a thread of its own (it may be on a slow
    /// drive): what kind it is, how big, when it last changed. What it holds comes from a walk
    /// over its folders, which the import does on its own for now.
    pub(super) fn inspect_data_file(&mut self, path: String) {
        let (sink, asked) = (self.sink.clone(), path.clone());
        let spawned = std::thread::Builder::new().name("mail-inspect".into()).spawn(move || {
            let facts = facts_of(&path);
            sink.emit(Event::DataFileInspected { path, facts: Box::new(facts) });
        });
        if let Err(e) = spawned {
            let facts = DataFileFacts { path: asked.clone(), error: Some(format!("inspect: {e}")), ..DataFileFacts::default() };
            self.sink.emit(Event::DataFileInspected { path: asked, facts: Box::new(facts) });
        }
    }
}

/// What the file system says of a data file (nothing read from inside it).
fn facts_of(path: &str) -> DataFileFacts {
    let lower = path.to_ascii_lowercase();
    let kind = [".pst", ".ost", ".mailcraft"].iter().find(|k| lower.ends_with(*k)).map_or("", |k| k.trim_start_matches('.'));
    let mut facts = DataFileFacts { path: path.to_string(), kind: kind.to_string(), ..DataFileFacts::default() };
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => {
            facts.size = m.len();
            facts.modified =
                m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        }
        Ok(_) => facts.error = Some("not a file".into()),
        Err(e) => facts.error = Some(e.to_string()),
    }
    facts
}

/// A count as the model keeps it.
fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Milliseconds since `began`.
fn ms(began: Instant) -> u64 {
    u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX)
}
