//! The session: imports stopped, by the user or by MailCraft closing, and what becomes of them:
//! gone on with from where they stopped, started again from the beginning, or kept as they are.

use mailcraft_model::job::StopHow;

use super::*;

impl Session {
    /// Stop the imports into `accounts`, as `how` says (kept to go on, or paused).
    pub fn stop_import(&mut self, accounts: &[String], how: StopHow) -> CmdResult {
        if how == StopHow::Discard {
            return Err(CmdError::Params("an import is thrown away by removing its account".into()));
        }
        let mut stopping = Vec::new();
        for a in accounts {
            let Some(st) = self.importing.running.get_mut(a).filter(|s| s.live && !s.removing) else { continue };
            st.stopping = Some(how);
            st.progress.phase = ImportPhase::Stopping;
            self.jobs.push(Job::StopImport { account: a.clone(), how });
            stopping.push(a.clone());
        }
        if stopping.is_empty() {
            return Err(CmdError::Disabled("nothing is being imported".into()));
        }
        Ok(serde_json::json!({ "stopping": stopping, "how": how }))
    }

    /// The import waiting on the user, unless it's being read or the account removed.
    fn stopped_import(&self, account: &str) -> Result<&ImportState, CmdError> {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if self.importing.keeping.contains_key(account) {
            return Err(CmdError::Failed("what was imported is being kept".into()));
        }
        match self.importing.running.get(account) {
            None => Err(CmdError::Params("no import of this account is waiting".into())),
            Some(s) if s.removing => Err(CmdError::Failed("this account is being removed".into())),
            Some(s) if s.live => Err(CmdError::Failed("the data file is being imported".into())),
            Some(s) => Ok(s),
        }
    }

    /// Go on with an import stopped or interrupted, from where it stopped.
    pub fn resume_import(&mut self, account: &str) -> CmdResult {
        self.compaction_holds_import(account)?;
        let st = self.stopped_import(account)?;
        let path = self.importing.sources.get(account).map(|s| s.path.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| st.path.clone());
        if path.is_empty() {
            return Err(CmdError::Params("where its data file is isn't known: import it again, giving `path`".into()));
        }
        self.queue_import(account, &path, true);
        Ok(serde_json::json!({ "account": account, "importing": true }))
    }

    /// Start an import stopped, interrupted or lost again from the beginning: what came in goes
    /// (the account stays), and the data file at `path` (else the one it came from) is read.
    pub fn restart_import(&mut self, account: &str, path: Option<&str>) -> CmdResult {
        self.compaction_holds_import(account)?;
        let st = self.stopped_import(account)?;
        let path = match path.map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => p.to_string(),
            None => self.importing.sources.get(account).map(|s| s.path.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| st.path.clone()),
        };
        if path.is_empty() {
            return Err(CmdError::Params("`path` is required".into()));
        }
        if path.len() > PATH_MAX {
            return Err(CmdError::Params("`path` is too long".into()));
        }
        if let Some(store) = &self.store {
            store.delete_account(account).map_err(|e| CmdError::Failed(e.to_string()))?;
        }
        // An Outlook file's copy is made beside it again.
        if self.account(account).is_some_and(AccountState::is_data_file) {
            self.place_copy(account, &path);
        }
        if let Some(a) = self.account_mut(account) {
            a.folders.clear();
        }
        self.cache.retain(|(a, _), _| a != account);
        self.seen_uid.retain(|(a, _), _| a != account);
        self.search_folders_changed(account);
        self.pim.remove(account);
        self.pim_unread.remove(account);
        self.contacts = None;
        self.queue_import(account, &path, false);
        if matches!(&self.view, View::Folder { account: va, .. } if va == account) {
            self.refresh_list();
        }
        Ok(serde_json::json!({ "account": account, "importing": true }))
    }

    /// Keep what a stopped import brought in, as it is: it won't go on. The folder a pause left
    /// part-way through, kept out of the list until then, comes in with what it has: the network
    /// side finishes it from the import's note, sends the folders and says it's done
    /// ([`Self::import_kept`]). Only then is the import over and the account (a POP one, which
    /// writes the Inbox) connects.
    pub fn keep_imported(&mut self, account: &str) -> CmdResult {
        self.compaction_holds_import(account)?;
        self.stopped_import(account)?;
        if let Some(st) = self.importing.running.remove(account) {
            self.importing.keeping.insert(account.to_string(), st);
        }
        self.jobs.push(Job::KeepImport { account: account.to_string() });
        Ok(serde_json::json!({ "account": account, "kept": true }))
    }

    /// What a stopped import brought in is kept as it is: the import is over, the contacts and
    /// calendar read so far are the account's, and its connection, held till now, goes ahead.
    /// Not all of it (`error`): it waits again to go on or be kept, and the account stays
    /// unconnected (a Keep tried again would write over what a POP account fetched meanwhile).
    pub(crate) fn import_kept(&mut self, account: &str, error: Option<String>) {
        let Some(st) = self.importing.keeping.remove(account) else { return };
        if self.account(account).is_none() {
            return;
        }
        if let Some(e) = error {
            let name = self.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
            self.importing.running.insert(account.to_string(), st);
            self.notice(format!("{name}: what was imported couldn't all be kept: {e}"), true);
            return;
        }
        if let Some(row) = self.importing.sources.get_mut(account) {
            row.stage = ImportStage::Done;
            self.save_imports();
        }
        self.load_imported_pim(account);
        if let Some(start) = self.held.remove(account) {
            self.jobs.push(start);
        }
    }
}
