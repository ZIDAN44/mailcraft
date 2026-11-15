//! How the pool takes a job: done here (on a thread of its own where it may take a while),
//! answered here for an account whose mail isn't connected, or handed to the account's thread.

use super::*;

mod threads;
#[cfg(test)]
pub(super) use threads::NO_THREADS;
pub(super) use threads::spawn;

/// What a job for an account's mail is told when no thread of the account can take it.
const OFFLINE: &str = "the account is offline";

impl Pool {
    /// Whether the account's thread is waiting (for a job, or out a wait to connect again): what
    /// it has done so far stays done until something wakes it. For the tests to wait on rather
    /// than time.
    #[cfg(test)]
    pub(crate) fn idle(&self, account: &str) -> bool {
        self.workers.get(account).is_some_and(|h| !h.busy.load(Ordering::Relaxed))
    }

    pub fn submit(&mut self, job: Job) {
        self.forget_unstarted();
        self.release_compacted();
        let Some(job) = self.held_for_compaction(job) else { return };
        if let Job::SearchServer { search, .. } = &job {
            self.search.store(*search, Ordering::Relaxed);
        }
        match job {
            Job::FetchUrl { url } => self.fetch_url(url),
            Job::FetchCalendar { account, calendar, url, etag, modified } => self.fetch_calendar(account, calendar, url, etag, modified),
            Job::StopSearch => self.search.store(0, Ordering::Relaxed),
            Job::Index { account, folders } => self.indexer.ask(&self.store, &self.sink, account, folders),
            Job::StopIndex { account } => self.indexer.stop(&account),
            Job::CheckUpdates => self.check_updates(),
            Job::SignIn { account } => self.sign_in(account),
            Job::AllowDirectory { account } => self.allow_directory(account),
            Job::Discover { email } => self.discover(email),
            Job::ImportPst { account, path, resume } => self.import_pst(account, path, resume),
            Job::RereadDataFile { account, path } => self.reread_data_file(account, path),
            Job::InspectDataFile { path } => self.inspect_data_file(path),
            Job::ReadMessageFile { path } => self.read_message_file(path),
            Job::StopImport { account, how } => self.stop_import(&account, how),
            Job::KeepImport { account } => self.keep_import(account),
            Job::DataFileHealth { account } => self.data_file_health(account),
            Job::CompactDataFile { account, mode } => self.compact_data_file(account, mode),
            Job::CancelCompact { account } => self.cancel_compact(&account),
            // The desktop keeps keys in the system's keychain: nothing to keep unlocked.
            Job::KeepUnlocked { .. } => {}
            j @ (Job::ImportIdentity { .. }
            | Job::ForgetIdentity { .. }
            | Job::ForgetOlderIdentity { .. }
            | Job::UseOlderIdentity { .. }
            | Job::Protect { .. }
            | Job::Decrypt { .. }
            | Job::CheckRevocation { .. }
            | Job::FindHeldKeys { .. }
            | Job::UseHeldKey { .. }) => self.smime(j),
            Job::Pgp { account, work } => self.pgp(account, work),
            Job::SealDraft { account, raw, folder, replaces, keep } => {
                let raw = self.seal(&account, raw.0);
                let account = account.id;
                self.submit(if keep { Job::KeepDraft { account, raw, folder, replaces } } else { Job::SaveDraft { account, raw, folder, replaces } });
            }
            Job::ExportPst { account, path, name, follow_up } => self.export_pst(account, path, name, follow_up),
            Job::SetSync { messages, offline } => self.set_sync(messages, offline),
            Job::WorkOffline { on } => self.work_offline(on),
            Job::Start { account, password } if self.offline => self.start_when_online(account, password),
            // Looking only when asked (issue #104): not signed in until something is (one set so
            // while it ran stops looking now).
            Job::Start { account, password } if account.check_by_hand => {
                self.stop(&account.id);
                self.start_when_online(account, password);
            }
            Job::Start { account, password } => self.start(account, password),
            // An empty one: the calendar server signs in as the mail server again.
            Job::DavPassword { account, password } if password.0.is_empty() => self.creds.delete_secret(&account, DAV_SECRET),
            Job::DavPassword { account, password } => {
                if let Err(e) = self.creds.set_secret(&account, DAV_SECRET, &password.0) {
                    self.sink.not_kept(&account.id, "the calendar server's password", &e);
                }
            }
            Job::SmtpPassword { account, password } => {
                if let Err(e) = self.creds.set_smtp(&account, &password.0) {
                    self.sink.not_kept(&account.id, "the outgoing server's password", &e);
                }
            }
            Job::Remove { account } => self.remove_account(&account),
            // Closing with a message written: on disk before the app goes (the worker may be busy
            // or about to stop); filed on the server at the next connection, as offline drafts.
            Job::KeepDraft { account, raw, folder, replaces } => {
                if let Err(e) = keep_draft(&self.store, &account, &folder, replaces, raw, true, &self.order) {
                    log::warn!("{account}: the draft couldn't be kept: {e}");
                }
            }
            // Persisted on the pool's thread for changes, not the account's: it may be minutes
            // into a full sync, and waiting meant the cache could be overwritten by a folder
            // refresh and an exit dropped the job. Nor the caller's, the window's (`edits.rs`).
            // The account's thread is woken after, to upload the queue at its next opportunity.
            // POP accounts keep the old route since their worker does extra local bookkeeping.
            Job::Apply { account, op } if self.records_edits(&account) => {
                let wake = self.workers.get(&account).map(|h| h.tx.clone());
                self.edits.send(&self.store, &self.sink, account, op, wake);
            }
            other => {
                self.start_when_asked(&other);
                if let Some(job) = self.offline_answer(other) {
                    self.route_to_worker(job);
                }
            }
        }
    }

    /// A job for an account's mail that no thread of the account can take (none runs, it works
    /// offline, or it has no mail server), done or answered here. `Some` gives back one its
    /// thread is to take.
    fn offline_answer(&mut self, job: Job) -> Option<Job> {
        let running = self.workers.contains_key(job.account());
        let mail_offline = self.mail_offline(job.account());
        let no_mail_server = self.no_mail_server(job.account());
        match job {
            // Into an account on this computer: with the other changes to its mail. An account
            // with a server that isn't connected can't take them now.
            Job::FileMessages { account, folder, paths } if mail_offline => {
                if self.configs.get(&account).is_some_and(|c| !c.is_local()) {
                    let failed = paths.iter().map(|p| (imports::filing::file_name(p), OFFLINE.to_string())).collect();
                    self.sink.emit(Event::MessagesFiled { account, folder, added: 0, failed });
                } else {
                    self.edits.file(&self.store, &self.sink, account, folder, paths);
                }
            }
            // Onto a server that isn't connected: none goes, and each says so.
            Job::FileCopies { account, folder, mail, moved } if mail_offline => {
                let failed = mail.into_iter().map(|(k, _)| (k, OFFLINE.to_string())).collect();
                self.sink.emit(Event::CopiesFiled { account, folder, done: Vec::new(), failed, moved });
            }
            // Offline or not started: the cache may still have it.
            Job::FetchBody { key } if mail_offline => match self.store.read_body(&key) {
                Some(raw) => self.sink.emit(Event::Body { key, raw }),
                None => self.sink.emit(Event::BodyFailed { key, error: format!("not downloaded yet, and {OFFLINE}") }),
            },
            Job::Send { account, id, .. } if no_mail_server => {
                self.sink.emit(Event::SendFailed {
                    account,
                    id,
                    error: "this account has calendars and contacts only: it has no mail server".into(),
                });
            }
            Job::Send { account, id, raw, from, recipients, sent_folder } if !running => {
                self.queue_offline_send(account, id, raw, from, recipients, sent_folder);
            }
            // Offline: changes wait in the cache for the next sync.
            Job::SyncDav { .. } if !running => {}
            Job::SearchServer { account, query, .. } if mail_offline => {
                self.sink.emit(Event::ServerSearched { account, query, keys: Vec::new(), error: Some(OFFLINE.into()), done: true });
            }
            Job::SearchDirectory { account, query } if mail_offline => {
                self.sink.emit(Event::Directory { account, query, people: Vec::new(), error: Some(OFFLINE.into()), consent: false });
            }
            // Working offline: nothing connects, by choice.
            Job::SyncAll { .. } | Job::SyncFolder { .. } | Job::FlushOutbox { .. } if self.offline && !running => {}
            Job::SaveDraft { account, raw, folder, replaces } if mail_offline => self.save_draft_offline(account, raw, folder, replaces),
            Job::FindIdentities { account } if mail_offline => {
                self.sink.emit(Event::IdentitiesFound { account, found: Vec::new(), error: Some(OFFLINE.into()) });
            }
            Job::GetVacation { account } | Job::SetVacation { account, .. } if mail_offline => {
                self.sink.emit(Event::Vacation { account, vacation: None, error: Some(OFFLINE.into()) });
            }
            job => return Some(job),
        }
        None
    }

    /// A message sent while the account isn't connected: straight into the Outbox.
    fn queue_offline_send(&mut self, account: String, id: u64, raw: Vec<u8>, from: String, recipients: Vec<String>, sent_folder: Option<String>) {
        let item = OutboxItem {
            id,
            from,
            recipients,
            sent_folder,
            queued: now(),
            attempts: 0,
            error: Some("offline".into()),
            not_before: 0,
            refused: false,
        };
        match self.store.outbox_add(&account, &item, &raw) {
            Ok(()) => self.sink.emit(Event::Queued { account, id, error: "offline".into(), kind: Default::default() }),
            Err(e) => self.sink.emit(Event::SendFailed { account, id, error: e.to_string() }),
        }
    }

    /// A draft saved while the account isn't connected: kept on this computer, and filed on the
    /// server at the next connection.
    fn save_draft_offline(&mut self, account: String, raw: Vec<u8>, folder: String, replaces: Option<MessageKey>) {
        let message_id = mailcraft_mime::written_message_id(&raw);
        // Handed out now, as one handed to a worker is: newer than a save of the draft a
        // stopped worker is still filing, which, filed, leaves it kept (found in review:
        // taken for older, it was taken off this computer, and never got to the server).
        DraftOrder::handed(&self.order, &raw);
        let error = keep_draft(&self.store, &account, &folder, replaces, raw, false, &self.order).err().map(|e| e.to_string());
        let kept = error.is_none();
        self.sink.emit(Event::DraftSaved { account: account.clone(), message_id, error });
        // Said after the save's end, so it's the last word on it.
        if kept {
            self.sink
                .emit(Event::Notice { account, message: "the draft is kept on this computer; it goes to Drafts when you're back online".into() });
        }
    }

    /// A job for the account's thread: sending and automatic replies on its second connection,
    /// and a message opened while the main one is busy; the rest on the main one.
    fn route_to_worker(&mut self, job: Job) {
        let id = job.account().to_string();
        let quick = self.configs.get(&id).is_some_and(|c| quick_job(&job, c));
        self.save_on_its_way(&job);
        let Some(h) = self.workers.get(&id) else {
            self.sink.emit(Event::Notice { account: id, message: "the account is not connected".into() });
            return;
        };
        // A message opened while the main connection syncs a folder, or the calendars, doesn't
        // wait for that to end (issue #24).
        let opened = matches!(job, Job::FetchBody { .. }) && h.busy.load(Ordering::Relaxed) && self.configs.get(&id).is_some_and(|c| c.pop.is_none());
        let quick = quick || opened;
        // Sending and automatic replies go on the second connection; if it has stopped, the main
        // one does it (after its sync).
        let job = match h.quick.as_ref().filter(|_| quick) {
            Some(q) => match q.send(job) {
                Ok(()) => return,
                Err(mpsc::SendError(j)) => j,
            },
            None => job,
        };
        if let Err(mpsc::SendError(unsent)) = h.tx.send(job) {
            // Mail that was to be copied there is answered for: none went.
            if let Job::FileCopies { account, folder, mail, moved } = unsent {
                let failed = mail.into_iter().map(|(k, _)| (k, "the account's connection stopped".to_string())).collect();
                self.sink.emit(Event::CopiesFiled { account, folder, done: Vec::new(), failed, moved });
            }
            self.sink.emit(Event::Notice { account: id, message: "the account's connection stopped; restart it from the account menu".into() });
        }
    }

    /// New sync settings, from Options: connected accounts start again with them (a rare change).
    fn set_sync(&mut self, messages: u32, offline: u32) {
        let opts = SyncOptions { window: if messages == 0 { usize::MAX } else { messages as usize }, prefetch: offline as usize, ..self.opts };
        if opts.window != self.opts.window || opts.prefetch != self.opts.prefetch {
            self.opts = opts;
            let running: Vec<AccountConfig> = self.workers.keys().filter_map(|id| self.configs.get(id).cloned()).collect();
            for c in running {
                self.start(c, None);
            }
        }
    }

    /// Working offline turned on (every account stops) or off (every account starts).
    fn work_offline(&mut self, on: bool) {
        self.offline = on;
        if on {
            let ids: Vec<String> = self.workers.keys().cloned().collect();
            for id in ids {
                self.stop(&id);
                self.sink.emit(Event::Status { account: id, status: SyncStatus::Offline });
            }
        } else {
            let configs: Vec<AccountConfig> = self.configs.values().cloned().collect();
            for c in configs {
                self.start(c, None);
            }
        }
    }

    /// An account started while working offline: remembered (and its password kept) for when
    /// that's turned off.
    fn start_when_online(&mut self, account: AccountConfig, password: Option<Secret>) {
        if let Some(p) = &password
            && let Err(e) = self.creds.set(&account, &p.0)
        {
            self.sink.not_kept(&account.id, "the password", &e);
        }
        self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::Offline });
        self.configs.insert(account.id.clone(), account);
    }

    /// An account that looks for mail only when asked (issue #104), not running: started for a
    /// job its server is needed for (Send / Receive, sending, a message not downloaded yet).
    fn start_when_asked(&mut self, job: &Job) {
        let id = job.account();
        if self.offline || self.workers.contains_key(id) {
            return;
        }
        let Some(config) = self.configs.get(id).filter(|c| c.check_by_hand).cloned() else { return };
        let needs_server = match job {
            Job::SyncAll { .. } | Job::SyncFolder { .. } | Job::Send { .. } | Job::FlushOutbox { .. } | Job::MoreMail { .. } => true,
            Job::SearchServer { .. } | Job::SearchDirectory { .. } | Job::GetVacation { .. } | Job::SetVacation { .. } => true,
            Job::FindIdentities { .. } => true,
            Job::FetchBody { key } => self.store.read_body(key).is_none(),
            _ => false,
        };
        if needs_server {
            self.start(config, None);
        }
    }

    /// An account removed: its threads stopped, its index and any import into it dropped, and
    /// everything it kept in the keychain forgotten.
    fn remove_account(&mut self, account: &str) {
        self.forget_saves(account);
        self.stop(account);
        self.indexer.stop(account);
        // An import into it is thrown away, its data file with it.
        self.stop_import(account, mailcraft_model::job::StopHow::Discard);
        if let Some(c) = self.configs.remove(account) {
            self.creds.delete(&c);
            self.creds.delete_smtp(&c);
            // Its certificates' keys and its calendar server's password go with it.
            smime::forget_all(self.creds.as_ref(), &c);
            pgp::forget_all(self.creds.as_ref(), &c);
            self.creds.delete_secret(&c, DAV_SECRET);
            self.creds.delete_secret(&c, DIRECTORY_SECRET);
        }
    }

    /// A calendar-only account (issue #61): its thread syncs calendars and contacts, and has no
    /// mail server for anything else.
    fn no_mail_server(&self, account: &str) -> bool {
        self.configs.get(account).is_some_and(AccountConfig::is_calendar_only)
    }

    /// Whether what's for an account's mail is done here, as for an account that isn't connected:
    /// no thread runs for it, or the one that does has no mail server.
    fn mail_offline(&self, account: &str) -> bool {
        !self.workers.contains_key(account) || self.no_mail_server(account)
    }

    /// Whether this side writes an account's edits to the cache and ops.json: always when no
    /// thread runs for it; for a running one unless it is POP (whose thread does it, with its
    /// own bookkeeping).
    fn records_edits(&self, account: &str) -> bool {
        match self.workers.get(account) {
            None => true,
            Some(_) => matches!(self.configs.get(account), Some(c) if c.pop.is_none()),
        }
    }
}
