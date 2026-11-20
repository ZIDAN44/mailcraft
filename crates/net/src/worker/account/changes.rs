//! An account's worker: the jobs it's handed, mailbox settings, and changes made offline replayed.

use super::*;

mod drafts;
mod filing;

pub(crate) use drafts::{DraftOrder, KeptDrafts, Order, Stopped, keep_draft};
#[cfg(test)]
pub(crate) use drafts::{KEPT_DRAFTS, PENDING_DRAFTS, PendingDraft, kept_raws, sweep_kept};
pub(crate) use filing::{Filed, Filing, LOOK_ASIDE_EVERY, SetAside};

impl Worker {
    pub(crate) fn handle(&mut self, job: Job) {
        match job {
            Job::SyncAll { .. } => {
                // Send / Receive: the drafts kept here go too, those that failed tried again now.
                self.kept.retry_now();
                if self.conn.is_none() {
                    self.auth_failed = false;
                    self.token_retried = false;
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                self.last_full = None;
            }
            Job::SyncFolder { folder, .. } => {
                if self.conn.is_none() {
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                if self.conn.is_some() {
                    self.sync_one(&folder);
                }
            }
            Job::MoreMail { folder, more, .. } => {
                let kept = self.store.load_folder(&self.acc.id, &folder).map(|f| f.messages.len()).unwrap_or(0);
                let now = self.windows.get(&folder).copied().unwrap_or(0).max(self.opts.window).max(kept);
                self.windows.insert(folder.clone(), now.saturating_add(more as usize).min(1_000_000));
                let windows = &self.windows;
                if let Some(Err(e)) =
                    self.store.while_kept(&self.acc.id, self.removals, || self.store.save_account_doc(&self.acc.id, WINDOWS, windows))
                {
                    log::warn!("{}: {WINDOWS}: {e}", self.acc.id);
                }
                if self.conn.is_none() {
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                if self.conn.is_some() {
                    self.sync_one(&folder);
                }
            }
            Job::SearchServer { folders, query, search, .. } => self.search_server(&folders, query, search),
            Job::FetchBody { key } => self.fetch(key),
            // Already persisted by the pool; just push the queue.
            Job::Apply { op, .. } if self.acc.pop.is_none() => {
                self.clear_left_behind(&op);
                if let Op::RenameFolder { from, to } = &op {
                    self.renamed(from, to);
                }
                if op.is_folder_op() {
                    self.reload_folders();
                }
                if self.conn.is_some() {
                    self.replay();
                }
            }
            Job::Apply { op, .. } => {
                // Made while the account is kept ([`Store::while_kept`]): one removed meanwhile has
                // nothing of it written again (its files would come back).
                let made = self.store.while_kept(&self.acc.id, self.removals, || {
                    // POP: a move is carried out here (there's no server copy to come back); what's
                    // deleted for good is noted, for removing the server's copy when the account says.
                    let gone: Vec<mailcraft_model::MessageKey> = match &op {
                        Op::Expunge { keys } => keys.clone(),
                        Op::EmptyFolder { path } => {
                            self.store.load_folder(&self.acc.id, path).map(|f| f.messages.iter().map(|m| m.key.clone()).collect()).unwrap_or_default()
                        }
                        _ => Vec::new(),
                    };
                    let noted = crate::pop::deleted_for_good(&self.store, &self.acc.id, &gone);
                    let applied = match &op {
                        Op::Move { keys, to } => crate::pop::move_locally(&self.store, &self.acc.id, keys, to),
                        _ => self.store.apply_op(&self.acc.id, &op).map_err(NetError::from),
                    };
                    (noted, applied)
                });
                let Some((noted, applied)) = made else { return };
                if let Err(e) = noted {
                    self.notice(e.to_string());
                }
                match applied {
                    Ok(changed) => {
                        if let Op::RenameFolder { from, to } = &op {
                            self.renamed(from, to);
                        }
                        if op.is_folder_op() && self.reload_folders() {
                            self.publish_folders();
                        }
                        for folder in changed {
                            self.sink.emit(Event::FolderChanged { account: self.id(), folder });
                        }
                    }
                    Err(e) => self.notice(e.to_string()),
                }
                if let Some(Err(e)) = self.store.while_kept(&self.acc.id, self.removals, || self.store.add_op(&self.acc.id, op)) {
                    self.notice(e.to_string());
                }
                if self.conn.is_some() {
                    self.replay();
                }
            }
            Job::Send { id, raw, from, recipients, sent_folder, .. } => self.send(id, &raw, &from, &recipients, sent_folder),
            // The pool keeps these itself; one reaching a worker is kept the same way.
            Job::KeepDraft { raw, folder, replaces, .. } => {
                if let Err(e) = keep_draft(&self.store, &self.acc.id, &folder, replaces, raw, true, &self.order) {
                    log::warn!("{}: the draft couldn't be kept: {e}", self.acc.id);
                }
            }
            Job::SaveDraft { raw, folder, replaces, .. } => self.save_draft(raw, folder, replaces),
            Job::FileMessages { folder, paths, .. } => self.file_messages(folder, paths),
            Job::FileCopies { folder, mail, moved, .. } => self.file_copies(folder, mail, moved),
            Job::FlushOutbox { .. } => self.flush_outbox(),
            // The pool raises the account's flag itself, whatever its thread is doing.
            Job::CancelTransfers { .. } => {}
            Job::SyncDav { .. } => self.sync_dav(None),
            Job::SearchDirectory { query, .. } => self.search_directory(query),
            Job::GetServerRules { .. } => self.server_rules(None),
            Job::SetServerRules { ops, drop_outlook_list, .. } => self.server_rules(Some((ops, drop_outlook_list))),
            Job::FreeBusy { people, from, to, .. } => self.free_busy(people, from, to),
            // Only a JMAP server keeps the addresses its mail is sent from where a mail
            // program can ask for them.
            Job::FindIdentities { .. } => {
                if let Err(e) = self.connected_for_job() {
                    self.sink.emit(Event::IdentitiesFound { account: self.id(), found: Vec::new(), error: Some(e) });
                    return;
                }
                let found = match self.conn.as_ref() {
                    Some(Conn::Jmap(c)) => jmap::identities(c).map_err(|e| e.to_string()),
                    _ => Err("this account's server doesn't say which addresses it sends from".to_string()),
                };
                let (found, error) = match found {
                    Ok(found) => (found, None),
                    Err(e) => (Vec::new(), Some(e)),
                };
                self.sink.emit(Event::IdentitiesFound { account: self.id(), found, error });
            }
            Job::GetVacation { .. } => {
                let r = self.vacation(None);
                self.report_vacation(r, None);
            }
            Job::SetVacation { vacation, .. } => {
                let r = self.vacation(Some(&vacation));
                match r {
                    Ok(v) => self.report_vacation(Ok(v), None),
                    // Show what the server still has, with why the change failed.
                    Err(e) => {
                        let back = self.vacation(None);
                        self.report_vacation(back, Some(e.to_string()));
                    }
                }
            }
            Job::FetchUrl { .. }
            | Job::FetchCalendar { .. }
            | Job::Discover { .. }
            | Job::SignIn { .. }
            | Job::AllowDirectory { .. }
            | Job::ImportPst { .. }
            | Job::ReadMessageFile { .. }
            | Job::RereadDataFile { .. }
            | Job::InspectDataFile { .. }
            | Job::StopImport { .. }
            | Job::KeepImport { .. }
            | Job::Index { .. }
            | Job::StopIndex { .. }
            | Job::ExportPst { .. }
            | Job::ArchivePst { .. }
            | Job::DataFileHealth { .. }
            | Job::CompactDataFile { .. }
            | Job::CancelCompact { .. }
            | Job::ImportIdentity { .. }
            | Job::ForgetIdentity { .. }
            | Job::ForgetOlderIdentity { .. }
            | Job::KeepUnlocked { .. }
            | Job::UseOlderIdentity { .. }
            | Job::Protect { .. }
            | Job::Decrypt { .. }
            | Job::SealDraft { .. }
            | Job::CheckRevocation { .. }
            | Job::FindHeldKeys { .. }
            | Job::UseHeldKey { .. }
            | Job::Pgp { .. }
            | Job::CheckUpdates
            | Job::StopSearch => {}
            Job::Start { .. }
            | Job::SmtpPassword { .. }
            | Job::DavPassword { .. }
            | Job::Remove { .. }
            | Job::WorkOffline { .. }
            | Job::SetSync { .. } => {}
        }
    }

    /// Connected for a job that needs the server, with one try when it isn't. A refused sign-in
    /// isn't tried again (the password known to be wrong was sent with each such job), and a
    /// try that fails shows as the account's status (left at "Connecting…" otherwise).
    pub(crate) fn connected_for_job(&mut self) -> std::result::Result<(), String> {
        if self.conn.is_some() {
            return Ok(());
        }
        if self.auth_failed {
            return Err("the account isn't signed in".into());
        }
        // Between tries at a server that couldn't be reached: not one more now (each failed
        // one puts the next further off, long after the network is back).
        if Instant::now() < self.next_attempt {
            return Err("the server couldn't be reached; it's tried again shortly".into());
        }
        self.connect().map_err(|e| {
            let said = e.to_string();
            if self.main.is_none() {
                self.connect_failed(e);
            }
            said
        })
    }

    /// Read (and first change, when `set`) the automatic replies: JMAP's own, or a Sieve script
    /// through ManageSieve on the IMAP host.
    pub(crate) fn vacation(&mut self, set: Option<&mailcraft_model::job::Vacation>) -> Result<mailcraft_model::job::Vacation> {
        // Not connected: one try, and a failure shows as the account's status (it said
        // "Connecting…" for good when the vacation page was opened on an account that can't).
        // (The second connection asks ManageSieve straight away: it needs no IMAP session.)
        if self.conn.is_none() && (self.main.is_none() || self.acc.is_api()) {
            if self.auth_failed {
                return Err(NetError::Refused("it shows once the account is connected".into()));
            }
            if let Err(e) = self.connect() {
                if self.main.is_some() {
                    return Err(e);
                }
                self.connect_failed(e);
                return Err(NetError::Refused("it shows once the account is connected".into()));
            }
        }
        if let Some(r) = self.conn.as_ref().and_then(|c| c.vacation(set)) {
            return r;
        }
        // IMAP: a Sieve script, through ManageSieve on the IMAP host.
        let secret = self.secret()?;
        let user = self.acc.login_name().to_string();
        let mut s =
            crate::sieve::Client::connect_trusting(&self.acc.imap, self.sieve_port, crate::stream::Trusted::of(&self.acc)).map_err(|e| match e {
                NetError::Io(_) => NetError::No("this server doesn't offer automatic replies (no ManageSieve)".into()),
                e => e,
            })?;
        if self.acc.oauth.is_some() {
            s.login_xoauth2(&user, &secret)?;
        } else {
            s.login(&user, &secret)?;
        }
        if let Some(v) = set {
            s.set_vacation(v)?;
        }
        let v = s.vacation();
        s.logout();
        v
    }

    pub(crate) fn report_vacation(&self, r: Result<mailcraft_model::job::Vacation>, failed: Option<String>) {
        let (vacation, error) = match r {
            Ok(v) => (Some(v), failed),
            // What the server said, without "the server refused:" before it: the page says
            // "Not available:" already, and a server without automatic replies refused nothing.
            Err(NetError::No(said)) if !said.trim().is_empty() => (None, Some(failed.unwrap_or(said))),
            Err(e) => (None, Some(failed.unwrap_or_else(|| e.to_string()))),
        };
        self.sink.emit(Event::Vacation { account: self.id(), vacation, error });
    }

    /// Re-apply queued ops that touch `path` to its freshly saved cache. True if the cache
    /// changed as a result. Not once the account was removed.
    pub(crate) fn reapply_waiting(&mut self, path: &str) -> bool {
        let mut any = false;
        let waiting = self.store.load_ops(&self.acc.id).unwrap_or_else(|e| {
            log::warn!("the changes waiting couldn't be read, so none is put back over {path}: {e}");
            Vec::new()
        });
        for op in waiting {
            let touches = match &op {
                o if o.whole_folder().is_some() => o.whole_folder() == Some(path),
                o if o.is_folder_op() => false,
                o => o.keys().iter().any(|k| k.folder == path),
            };
            if touches
                && self
                    .store
                    .while_kept(&self.acc.id, self.removals, || self.store.apply_op(&self.acc.id, &op))
                    .is_some_and(|r| r.is_ok_and(|c| !c.is_empty()))
            {
                any = true;
            }
        }
        any
    }

    /// Replay offline changes on the server, oldest first.
    pub(crate) fn replay(&mut self) {
        let mut destinations: Vec<String> = Vec::new();
        let mut relist = false;
        let trash = self.folders.iter().find(|f| f.role == FolderRole::Trash).map(|f| f.path.clone());
        // A change refused once is tried again after the folders are listed anew (see below).
        let mut retried = false;
        // Read once, then kept in step with ops.json as each change comes off it (one queued
        // meanwhile, by the window, is read in then): not read whole again for every change.
        let mut waiting = self.store.waiting_ops(&self.acc.id).unwrap_or_default();
        // Cap the loop in case removal keeps failing.
        for _ in 0..100_000 {
            let Some(op) = waiting.first().cloned() else { break };
            let r = match self.conn.as_mut() {
                Some(c) => c.apply_remote(&self.jstate, &op, trash.as_deref()),
                None => Err(NetError::Closed("not connected".into())),
            };
            match r {
                Ok(()) => {
                    retried = false;
                    match &op {
                        Op::Move { to, .. } => destinations.push(to.clone()),
                        Op::RenameFolder { to, .. } => {
                            relist = true;
                            destinations.push(to.clone());
                        }
                        Op::CreateFolder { .. } | Op::DeleteFolder { .. } => relist = true,
                        Op::EmptyFolder { path } | Op::MarkFolderRead { path } => destinations.push(path.clone()),
                        Op::SetFlag { .. } | Op::SetFollowUp { .. } | Op::SetReminder { .. } | Op::SetKeyword { .. } | Op::Expunge { .. } => {}
                    }
                    if let Err(e) = self.store.op_made(&self.acc.id, &mut waiting) {
                        self.notice(e.to_string());
                        break;
                    }
                    // The next queued edit may name this folder (or its new name): refresh the
                    // folder ids now rather than after the pass. With another change to the
                    // folders still to make, the server's list isn't said (nor written) until
                    // it's made too: the folder list on disk has it already (found in review: a
                    // folder made after another was renamed was shown gone between the two).
                    if op.is_folder_op() && relist {
                        let more = waiting.iter().any(Op::is_folder_op);
                        let listed = self.relist_with(false, !more);
                        relist = more || !listed;
                    }
                }
                Err(e) if e.is_fatal() => {
                    self.failed(e);
                    return;
                }
                // A folder the change names may be one the last listing missed (one just renamed
                // or made, after a listing that failed): listed again, the change is tried once
                // more before it counts as refused, rather than dropped for want of its id.
                Err(_) if !retried && op_names_folder(&op) && self.relist() => retried = true,
                Err(e) => {
                    retried = false;
                    // The server refused: drop the change and resync so the cache matches the
                    // server again (forcing a full flag check).
                    if let Err(e) = self.store.op_made(&self.acc.id, &mut waiting) {
                        self.notice(e.to_string());
                        break;
                    }
                    let mut folders: Vec<String> = op.keys().iter().map(|k| k.folder.clone()).collect();
                    folders.sort();
                    folders.dedup();
                    for f in &folders {
                        // Not once the account was removed: its files would come back.
                        let marked = self.store.while_kept(&self.acc.id, self.removals, || {
                            let mut st = self.store.load_folder(&self.acc.id, f).ok()?;
                            st.highest_modseq = 0;
                            st.uid_next = 0;
                            Some(self.store.save_folder(&self.acc.id, f, &st))
                        });
                        if let Some(Some(Err(e))) = marked {
                            log::warn!("{}: {f} isn't marked to be read in full again: {e}", self.acc.id);
                        }
                    }
                    relist |= op.is_folder_op();
                    self.sink.emit(Event::OpFailed { account: self.id(), op, error: e.to_string() });
                    destinations.extend(folders);
                }
            }
        }
        if relist {
            self.relist();
        }
        destinations.sort();
        destinations.dedup();
        // Only the folders the server has: one renamed again after (found in review: the folder
        // in between, gone, said "no such mailbox").
        for d in destinations {
            if self.conn.is_some() && self.folders.iter().any(|f| f.path == d) {
                self.sync_one(&d);
            }
        }
    }
}

/// Whether a change names a folder by its path (one a listing may not know yet).
fn op_names_folder(op: &Op) -> bool {
    op.is_folder_op() || matches!(op, Op::Move { .. } | Op::EmptyFolder { .. } | Op::MarkFolderRead { .. })
}
