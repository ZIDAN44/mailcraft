//! The session: what the network side reports, applied.

use super::*;

impl Session {
    /// Apply an event from the network layer.
    pub fn apply(&mut self, e: Event) {
        if !self.importing.running.is_empty() {
            self.removal_backstop();
        }
        // An account's data file had to itself by a compaction: what it sends waits for it.
        let Some(e) = self.defer_for_compaction(e) else { return };
        match e {
            e @ (Event::Identity { .. }
            | Event::Protected { .. }
            | Event::Decrypted { .. }
            | Event::Revocation { .. }
            | Event::HeldKeys { .. }
            | Event::PgpKeys { .. }
            | Event::PgpDecrypted { .. }) => self.apply_smime(e),
            Event::Status { account, status } => {
                // Show more ended (with nothing new, or failing): its footer can be asked again.
                if !matches!(status, SyncStatus::Syncing { .. } | SyncStatus::Connecting)
                    && self.showing_more.as_ref().is_some_and(|(a, _)| *a == account)
                {
                    self.showing_more = None;
                }
                // Refusals that need something done say so once, as they come.
                if let SyncStatus::Error { error } = &status
                    && (error.starts_with(mailcraft_model::MICROSOFT_IMAP_CLOSED) || error.starts_with(mailcraft_model::MICROSOFT_NO_PASSWORDS))
                    && self.account(&account).is_some_and(|a| a.status != status)
                {
                    let who = self.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
                    self.notice(format!("{who}: {error}"), true);
                }
                if let SyncStatus::NeedsPassword { error } = &status {
                    let (who, browser) = self.account(&account).map(|a| (a.config.email.clone(), a.config.oauth.is_some())).unwrap_or_default();
                    let why = if error.is_empty() { String::new() } else { format!(": {error}") };
                    // An account that signs in in the browser has no password to ask for.
                    let what = if browser { "needs you to sign in again" } else { "needs its password" };
                    self.notice(format!("{who} {what}{why}"), true);
                    if !browser && self.asked_password.insert(account.clone()) {
                        self.ask_password.push(account.clone());
                    }
                }
                // Its mail synced in full: an Inbox empty until now (so never said to have
                // changed) is marked, so its first message counts as arriving, not as the backlog.
                if matches!(status, SyncStatus::Online { .. }) {
                    self.mark_inbox(&account);
                }
                if let Some(a) = self.account_mut(&account) {
                    a.status = status;
                }
            }
            Event::Folders { account, mut folders } => {
                // They're this account's, whatever name they were kept under.
                for f in &mut folders {
                    if f.account != account {
                        f.account.clone_from(&account);
                    }
                }
                sort_folders(&mut folders);
                // The pool counts a folder as it syncs it, and lists them all again now and then
                // (Drafts made, a draft's folder synced) with the others' counts from their last
                // sync, from before mail read here since reached the server: a count it reports
                // unchanged is no news, and the one counted here stands, until the folder is
                // synced (`FolderChanged`).
                let here: HashMap<String, (u32, u32)> =
                    self.account(&account).map(|a| a.folders.iter().map(|f| (f.path.clone(), (f.unread, f.total))).collect()).unwrap_or_default();
                for f in &mut folders {
                    let now = (f.unread, f.total);
                    let key = (account.clone(), f.path.clone());
                    let synced = self.counts_stale.remove(&key);
                    let before = self.reported_counts.insert(key, now);
                    if before == Some(now)
                        && !synced
                        && let Some(&(unread, total)) = here.get(&f.path)
                    {
                        (f.unread, f.total) = (unread, total);
                    }
                }
                let first = self.account(&account).is_some_and(|a| a.folders.is_empty());
                if let Some(a) = self.account_mut(&account) {
                    a.folders = folders;
                }
                // A view on a folder that no longer exists falls back to the Inbox.
                let missing = matches!(&self.view, View::Folder { account: va, folder } if *va == account && self.folder(va, folder).is_none());
                if missing {
                    let inbox =
                        self.account(&account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()).unwrap_or_else(|| "INBOX".into());
                    self.open_view(View::Folder { account: account.clone(), folder: inbox });
                } else if first {
                    self.refresh_list();
                }
                self.mark_junk(&account);
                // Its drafts say which messages have a reply begun (#52).
                self.read_drafts(&account);
                self.search_folders_changed(&account);
                self.want_index(&account, None);
            }
            Event::FolderChanged { account, folder } => match self.echo_of(&account, &folder) {
                // The echo of a flag or category changed here: the folder in memory has it already.
                mailboxes::Echo::Kept => {}
                // A move or delete made here, on disk now: read again, its count no news.
                mailboxes::Echo::Made => self.folder_changed(&account, &folder),
                // Synced, or changed on disk otherwise: the next count reported for it is news,
                // whatever it is (a mark undone on another device comes back this way). The count
                // last reported is kept, for a change the server refuses.
                mailboxes::Echo::None => {
                    self.counts_stale.insert((account.clone(), folder.clone()));
                    self.folder_changed(&account, &folder);
                }
            },
            Event::Directory { account, query, people, error, consent } => self.apply_directory(&account, &query, &people, error, consent),
            Event::CalendarFetched { account, calendar, ics, etag, modified, unchanged, error } => {
                let read = match (ics, error) {
                    (_, Some(e)) => pim::web::WebRead::Failed(e),
                    (Some(ics), None) => pim::web::WebRead::Read { ics, etag, modified },
                    (None, None) if unchanged => pim::web::WebRead::Unchanged,
                    (None, None) => pim::web::WebRead::Failed("nothing came back".into()),
                };
                self.apply_web_calendar(&account, &calendar, read);
            }
            Event::DirectoryAllowed { account, error } => self.directory_allowed(&account, error),
            Event::ServerSearched { account, query, keys, error, done } => {
                if let Some(ss) = self.server_search.as_mut().filter(|ss| ss.sent == query) {
                    // What's found so far shows at once; the account is done at its last answer.
                    if done {
                        ss.pending = ss.pending.saturating_sub(1);
                    }
                    ss.found.extend(keys);
                    if let Some(e) = error {
                        let who = self.accounts.iter().find(|a| a.config.id == account).map(|a| a.config.email.clone()).unwrap_or(account);
                        if let Some(ss) = self.server_search.as_mut() {
                            ss.errors.push(format!("{who}: {e}"));
                        }
                    }
                    self.refresh_list();
                }
            }
            Event::DavSynced { account, sync } => self.apply_dav(&account, &sync),
            Event::ImportProgress { account, progress } => self.import_progress(&account, &progress),
            Event::ImportPlanned { account, folders } => self.import_planned(&account, folders),
            Event::ImportKept { account, error } => self.import_kept(&account, error),
            // A discarded import is a removed account's (only removing one discards it): the
            // account being removed goes now; any other end of one is an account's long gone.
            Event::Imported { account, error, report: Some(r), .. } if r.stopped == Some(mailcraft_model::job::StopHow::Discard) => {
                self.import_discarded(&account, error)
            }
            Event::Imported { account, messages, skipped, done, error, report } => {
                self.imported(&account, messages, skipped, done, error, report.as_deref())
            }
            Event::DataFileInspected { path, facts } => self.data_file_inspected(&path, facts),
            Event::MessagesFiled { account, folder, added, failed } => {
                if self.filed.len() < 100 {
                    self.filed.push(Filed { account, folder, added, failed });
                }
            }
            Event::MessageFileRead { path, raw } => self.message_file_read(&path, raw),
            Event::DataFileHealth { account, health, error } => self.health_measured(&account, health.map(|h| *h), error),
            Event::Compacting { account, phase, work, exclusive_s } => self.compacting(&account, phase, work, exclusive_s),
            Event::Compacted { account, report } => self.compacted(&account, *report),
            Event::Indexed { account, work, generation, failed, finished } => {
                self.indexed(&account, IndexPass { work, generation, failed, finished })
            }
            Event::AttachmentsKnown { account, folder, with, without } => self.files_known(&account, &folder, &with, &without),
            Event::Exported { account, messages, skipped, already, occurrences, replaced, done, error } => {
                self.exported(&account, messages, (skipped, already, occurrences, replaced), done, error)
            }
            Event::Quota { account, used, limit } => {
                if let Some(a) = self.account_mut(&account) {
                    a.quota = Some((used, limit));
                }
            }
            Event::Vacation { account, vacation, error } => {
                let mut failed = None;
                if let Some(a) = self.account_mut(&account) {
                    a.vacation = match (vacation, error) {
                        (Some(v), None) => VacationState::Loaded(v),
                        (Some(v), Some(e)) => {
                            // Read back after a change that failed: show what the server has.
                            failed = Some(e);
                            VacationState::Loaded(v)
                        }
                        (None, e) => VacationState::Failed(e.unwrap_or_else(|| "this server doesn't offer automatic replies".into())),
                    };
                }
                if let Some(e) = failed {
                    self.notice(format!("automatic replies weren't changed: {e}"), true);
                }
            }
            // Its text first (issue #24): shown, and nothing else until the whole message comes.
            Event::Body { key, raw } if mime::is_text_first(&raw) => self.show_body(&key, &raw),
            Event::Body { key, raw } => {
                self.source_arrived(&key, &raw);
                // The open message's: parsed on another thread, where the window has one.
                if !self.defer_arrived(&key, &raw) {
                    self.show_body(&key, &raw);
                }
                self.invitation_body(&key);
                // Downloaded now, or read from the cache: a pass finds which. A local account's
                // (a data file's) is always read from it, nothing new to index.
                if !self.account(&key.account).is_none_or(AccountState::is_local) {
                    self.want_index(&key.account, Some(&key.folder));
                }
            }
            Event::Filled { summary } => self.filled(*summary),
            Event::BodyFailed { key, error } => {
                self.source_failed(&key, &error);
                for w in self.windows.iter_mut().filter(|w| w.key == key && w.parsed.is_none()) {
                    w.error = Some(error.clone());
                }
                if let Some(r) = self.reading.as_mut()
                    && r.key == key
                {
                    r.error = Some(error);
                }
            }
            Event::Sent { id, .. } => {
                if let Some(c) = self.compose.clone().filter(|c| c.id == id) {
                    self.finish_sent(&c, true);
                } else {
                    self.notice("Message sent", false);
                }
                // Delivered from the Outbox: set the flag it owed.
                if let Some((key, flag)) = self.pending_marks.remove(&id) {
                    self.save_pending_marks();
                    self.apply_op(&key.account.clone(), Op::SetFlag { keys: vec![key], flag, on: true });
                }
                self.after_outbox_change();
            }
            Event::Queued { id, error, .. } => {
                if let Some(c) = self.compose.clone().filter(|c| c.id == id) {
                    // It's safe in the Outbox: as far as the user is concerned, the message is done
                    // (a Replied/Forwarded flag stays owed until delivery).
                    self.finish_sent(&c, false);
                    self.notices.pop();
                    self.notice("Saved in the Outbox: it will be sent when you're back online", false);
                } else {
                    log::info!("outbox {id}: still waiting ({error})");
                }
                self.after_outbox_change();
            }
            Event::SendFailed { id, error, .. } => {
                if let Some(c) = self.compose.as_mut().filter(|c| c.id == id) {
                    c.sending = false;
                    c.error = Some(error.clone());
                }
                self.notice(format!("Not sent: {error}"), true);
                self.after_outbox_change();
            }
            Event::OpFailed { account, op, error } => {
                // What it counted here (a message read) didn't happen: the folders it names count
                // as the server last said, which the sync that follows may not say again (its
                // count unchanged, it reports none).
                let folders: BTreeSet<String> = op.keys().iter().filter(|k| k.account == account).map(|k| k.folder.clone()).collect();
                for f in folders {
                    let reported = self.reported_counts.get(&(account.clone(), f.clone())).copied();
                    if let (Some((unread, total)), Some(a)) = (reported, self.account_mut(&account))
                        && let Some(folder) = a.folders.iter_mut().find(|x| x.path == f)
                    {
                        (folder.unread, folder.total) = (unread, total);
                    }
                }
                self.notice(format!("The server refused a change: {error}"), true);
            }
            Event::OpenUrl { url } => {
                if self.open_urls.len() < 8 {
                    self.open_urls.push(url);
                }
            }
            Event::SignedIn { account, user } => {
                // Who the browser signed in as, kept when it isn't the account's address: tried
                // when the mailbox won't open under that, and named when it still won't.
                let user = user.map(|u| u.trim().to_string()).filter(|u| !u.is_empty() && u.len() < 320);
                if let Some(a) = self.account_mut(&account) {
                    let other = user.filter(|u| !u.eq_ignore_ascii_case(&a.config.email));
                    let changed = a.config.oauth_user != other;
                    a.config.oauth_user = other.clone();
                    let config = a.config.clone();
                    let who = config.email.clone();
                    if changed {
                        self.save_accounts();
                    }
                    self.jobs.push(Job::Start { account: config, password: None });
                    match other {
                        Some(o) => self.notice(format!("{who} is signed in (as {o})"), false),
                        None => self.notice(format!("{who} is signed in"), false),
                    }
                }
            }
            Event::Discovered { email, config, source } => {
                self.discovered.insert(email.trim().to_ascii_lowercase(), Some((config.map(|c| *c), source)));
            }
            Event::Releases { releases, error } => {
                self.update = match error {
                    Some(e) => UpdateCheck::Failed(e),
                    None => match newer_release(env!("CARGO_PKG_VERSION"), self.build_sha.as_deref(), &releases, self.updates_stable_only) {
                        Some(r) => UpdateCheck::Available(r.clone()),
                        None => UpdateCheck::UpToDate(releases.first().map(|r| r.tag.clone()).unwrap_or_default()),
                    },
                };
            }
            Event::Fetched { url, data, error } => {
                let p = match (data, error) {
                    (Some(d), _) => Picture::Ready(Arc::new(d)),
                    (None, e) => Picture::Failed(e.unwrap_or_default()),
                };
                self.pictures.insert(url, p);
            }
            Event::DraftSaved { account, message_id, error } => self.draft_saved(&account, &message_id, error),
            Event::Notice { account, message } => {
                // A change that failed among them: what the account says changed next is read.
                self.forget_echoes(&account);
                let who = self.account(&account).map(|a| a.config.email.clone()).unwrap_or(account);
                self.notice(format!("{who}: {message}"), true);
            }
        }
    }
}
