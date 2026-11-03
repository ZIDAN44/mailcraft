//! The session: an Outlook data file opened as a mailbox (File › Open & Export) keeps MailCraft's
//! copy beside it (`archive.pst` → `archive.mailcraft`), and MailCraft never writes into the file
//! itself: everything changed in MailCraft is in the copy. Opened again once its account was
//! removed, the file goes on with that copy. Changed since the copy read it, the user says
//! whether to read what's new into the copy ([`Changes::Read`]: added, nothing of the copy
//! changed otherwise) or to open the copy as it was read ([`Changes::Keep`]). A folder that can't
//! be written has the copy go with the other data files, and the user is told where.

use std::path::Path;

use mailcraft_model::job::{Blocked, DataSource, ImportPhase, ImportProgress, ImportReport, ImportStage};
use mailcraft_store::Found;

use super::*;

/// What becomes of an Outlook data file changed since MailCraft's copy of it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Changes {
    /// Read the file again into the copy: what it has new is added.
    Read,
    /// Open the copy as it was read.
    Keep,
}

impl Changes {
    /// `changes` as the command takes it.
    pub fn parse(changes: Option<&str>) -> Result<Option<Changes>, CmdError> {
        match changes {
            None => Ok(None),
            Some("read") => Ok(Some(Changes::Read)),
            Some("keep") => Ok(Some(Changes::Keep)),
            Some(c) => Err(CmdError::Params(format!("`changes` is \"read\" or \"keep\", not `{c}`"))),
        }
    }
}

/// Whether `path` names an Outlook data file (its copy goes beside it), not MailCraft's own.
pub(super) fn is_outlook(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".pst") || lower.ends_with(".ost")
}

/// Why a copy with no note of what came in from its file isn't read into again.
pub(super) const NO_NOTE: &str =
    "MailCraft has no note of what came in from this file before, so its changes can't be told from what was deleted in MailCraft and aren't read in";

/// Whether an id read from a file can be an account's: as MailCraft gives them.
fn usable_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Session {
    /// The copy of the Outlook file at `path` from an earlier opening, beside it and no account's
    /// now, when there is one.
    pub(super) fn copy_found(&self, path: &str) -> Option<Found> {
        if !is_outlook(path) {
            return None;
        }
        self.store.as_ref().and_then(|s| s.copy_beside(Path::new(path))).and_then(|b| b.found)
    }

    /// `account`'s data file is MailCraft's copy beside the Outlook file at `path` (a new one when
    /// `fresh`, another copy of a file open already); one that can't go there goes with the other
    /// data files, and the user is told where. The copy found there, when there is one.
    pub(super) fn place_copy(&mut self, account: &str, path: &str, fresh: bool) -> Option<Found> {
        if !is_outlook(path) {
            return None;
        }
        let place = |s: &mailcraft_store::Store| {
            if fresh { s.place_new_beside(account, Path::new(path)) } else { s.place_beside(account, Path::new(path)) }
        };
        let beside = self.store.as_ref().and_then(place)?;
        if beside.elsewhere {
            let file = file_name(path);
            let folder = beside.path.display().to_string();
            self.notice(format!("{file}'s folder can't be written: MailCraft's copy of it goes in {folder}"), false);
        }
        beside.found
    }

    /// The id the copy found beside a file is opened as: the account's it was, when that's free.
    pub(super) fn id_for_copy(&self, found: &Found) -> String {
        let free = |id: &str| self.account(id).is_none() && !self.importing.sources.contains_key(id);
        if usable_id(&found.account) && free(&found.account) { found.account.clone() } else { self.new_account_id() }
    }

    /// Open the copy found beside the Outlook file at `path` as the new account `id` (made by the
    /// caller, its data file that copy): its mail as it was left, an import it had stopped
    /// waiting on the user, one cut off going on. The file changed since, as `changes` says.
    pub(super) fn open_copy(&mut self, id: &str, path: &str, found: &Found, changes: Option<Changes>) -> CmdResult {
        let Some(store) = self.store.clone() else { return Err(CmdError::Failed("nothing keeps the copy".into())) };
        if found.account != id
            && let Err(e) = store.claim_folders(id)
        {
            // Its mail can't be this account's: it goes again, and the copy stays as it was.
            let _ = self.drop_account(id, Leaves::DataFile);
            return Err(CmdError::Failed(format!("MailCraft's copy of this file couldn't be opened: {e}")));
        }
        let mut folders = store.load_folders(id).unwrap_or_default();
        sort_folders(&mut folders);
        if let Some(a) = self.account_mut(id) {
            a.folders = folders;
            a.status = SyncStatus::Offline;
        }
        let stage = match found.state.as_str() {
            "done" => ImportStage::Done,
            "running" => ImportStage::Running,
            _ => ImportStage::Stopped,
        };
        let record_key = self.facts_for(path).map(|f| f.record_key.clone()).unwrap_or_default();
        let row = DataSource { path: path.to_string(), size: found.size, modified: found.modified, record_key, stage, report: None };
        self.importing.sources.insert(id.to_string(), row);
        self.save_imports();
        let importing = match stage {
            // Cut off: it goes on (a file changed since waits on the user, as at start).
            ImportStage::Running => {
                self.queue_import(id, path, true);
                true
            }
            ImportStage::Stopped | ImportStage::Removing => {
                let progress = ImportProgress { phase: ImportPhase::Paused, ..ImportProgress::default() };
                self.importing
                    .running
                    .insert(id.to_string(), ImportState { file: file_name(path), path: path.to_string(), progress, ..ImportState::default() });
                false
            }
            ImportStage::Done => {
                self.load_imported_pim(id);
                self.recount_local(id, None);
                if found.changed && changes == Some(Changes::Read) {
                    self.queue_reread(id, path);
                } else if found.changed {
                    let file = file_name(path);
                    self.notice(format!("{file} has changed since MailCraft read it: its copy is opened as it was read"), false);
                }
                false
            }
        };
        if let Some(inbox) = self.account(id).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()) {
            self.open_view(View::Folder { account: id.to_string(), folder: inbox });
        }
        let copy = store.data_file(id).map(|f| f.display().to_string());
        Ok(serde_json::json!({ "account": id, "importing": importing, "reopened": true, "dataFile": copy }))
    }

    /// Whether what's new in `account`'s data file can be read into it: its copy notes what came
    /// in from the file before (one opened before MailCraft noted it doesn't, and what's new in
    /// the file couldn't be told from what was deleted here).
    pub fn can_read_changes(&self, account: &str) -> bool {
        self.store.as_ref().is_some_and(|s| s.has_seen(account))
    }

    /// Read the Outlook file at `path` again into `account`, the account keeping its copy: what
    /// it has new is added. Not while the account imports, is compacted, or isn't one opened
    /// from a data file, nor when its copy has no note of what came in before.
    pub(super) fn reread(&mut self, account: &str, path: &str) -> CmdResult {
        if !self.account(account).is_some_and(AccountState::is_data_file) {
            return Err(CmdError::Params(format!("`{account}` isn't a data file opened here")));
        }
        if !self.can_read_changes(account) {
            return Err(CmdError::Failed(format!("{NO_NOTE}: import another copy of it to see them")));
        }
        self.compaction_holds_import(account)?;
        self.import_holds(account)?;
        self.queue_reread(account, path);
        Ok(serde_json::json!({ "account": account, "importing": true, "rereading": true }))
    }

    fn queue_reread(&mut self, account: &str, path: &str) {
        let state = ImportState { file: file_name(path), path: path.to_string(), live: true, rereading: true, ..ImportState::default() };
        self.importing.running.insert(account.to_string(), state);
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Syncing { folder: String::new() };
        }
        self.jobs.push(Job::RereadDataFile { account: account.to_string(), path: path.to_string() });
    }

    /// A read of the file again has ended: what it added is said (and shown, as an import's
    /// report), or why nothing was. The account stays, whatever became of it, unless it was
    /// being removed: then it goes now, its copy kept where it is.
    pub(super) fn reread_ended(&mut self, account: &str, messages: usize, error: Option<String>, report: Option<&ImportReport>) {
        let Some(st) = self.importing.running.remove(account) else { return };
        self.import_ended(account);
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Offline;
        }
        if st.removing {
            let _ = self.drop_account(account, Leaves::DataFile);
            return;
        }
        let name = self.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
        match error {
            Some(e) => self.notice(format!("{name} could not be read again: {e}"), true),
            None if report.is_some_and(|r| r.stopped.is_some()) => {
                self.notice(format!("{name}: reading it again stopped; nothing of it was added"), false);
            }
            None => {
                // The file as read now is the one the copy has.
                if let Some(f) = self.facts_for(&st.path).map(|f| (f.size, f.modified))
                    && let Some(row) = self.importing.sources.get_mut(account)
                {
                    (row.size, row.modified) = f;
                    self.save_imports();
                }
                self.load_imported_pim(account);
                self.notice(format!("{name}: {messages} new messages read in"), false);
            }
        }
    }

    /// A read of the file again that stopped as MailCraft closed, or for want of room: nothing
    /// of it was added, and it's over (asked for again, it reads the file from the start).
    pub(super) fn reread_stopped(&mut self, account: &str, progress: &ImportProgress) {
        let Some(st) = self.importing.running.remove(account) else { return };
        self.import_ended(account);
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Offline;
        }
        if st.removing {
            let _ = self.drop_account(account, Leaves::DataFile);
        } else if progress.phase == ImportPhase::Blocked {
            let name = self.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
            let why = match progress.blocked {
                Some(Blocked::DiskFull { .. }) => "there isn't room to read it",
                _ => "it can't be read now",
            };
            self.notice(format!("{name} could not be read again: {why}"), true);
        }
    }
}
