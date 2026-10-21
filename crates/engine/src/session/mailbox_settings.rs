//! The session: mailbox settings on the server (automatic replies).

use super::*;

impl Session {
    /// Delete everything in a folder for good (only Deleted Items and Junk Email).
    pub fn empty_folder(&mut self, account: &str, folder: &str) -> CmdResult {
        let f = self.folder(account, folder).ok_or_else(|| CmdError::Params(format!("no folder `{folder}`")))?;
        if !matches!(f.role, FolderRole::Trash | FolderRole::Junk) {
            // Any other folder: everything goes to Deleted Items, as Outlook's Delete All.
            let trash = self
                .account(account)
                .and_then(|a| a.folder(FolderRole::Trash))
                .map(|t| t.path.clone())
                .ok_or_else(|| CmdError::Params("the account has no Deleted Items folder".into()))?;
            let keys: Vec<MessageKey> = self.folder_state(account, folder).messages.iter().map(|m| m.key.clone()).collect();
            let n = keys.len();
            if n > 0 {
                self.apply_op(account, Op::Move { keys, to: trash });
            }
            return Ok(serde_json::json!({ "moved": n, "to": "trash" }));
        }
        self.apply_op(account, Op::EmptyFolder { path: folder.to_string() });
        Ok(serde_json::json!({ "emptied": folder }))
    }

    /// Ask the server for an account's automatic replies.
    pub fn load_vacation(&mut self, account: &str) -> CmdResult {
        let a = self.account_mut(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if a.is_local() {
            a.vacation = VacationState::Failed("the sample mailbox has no server".into());
        } else {
            a.vacation = VacationState::Loading;
            self.jobs.push(Job::GetVacation { account: account.to_string() });
        }
        Ok(self.vacation_json(account))
    }

    /// Change an account's automatic replies on the server.
    pub fn set_vacation(&mut self, account: &str, v: Vacation) -> CmdResult {
        for d in [&v.from, &v.until].into_iter().flatten() {
            if !mailcraft_model::job::is_day(d) {
                return Err(CmdError::Params(format!("`{d}` is not a date (YYYY-MM-DD)")));
            }
        }
        if let (Some(a), Some(b)) = (&v.from, &v.until)
            && a > b
        {
            return Err(CmdError::Params("the last day is before the first".into()));
        }
        if v.enabled && v.text.trim().is_empty() {
            return Err(CmdError::Params("write the reply first".into()));
        }
        let a = self.account_mut(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if a.is_local() {
            return Err(CmdError::Params("the sample mailbox has no server".into()));
        }
        a.vacation = VacationState::Loading;
        self.jobs.push(Job::SetVacation { account: account.to_string(), vacation: v });
        Ok(self.vacation_json(account))
    }

    pub fn vacation_json(&self, account: &str) -> serde_json::Value {
        match self.account(account).map(|a| &a.vacation) {
            Some(VacationState::Loaded(v)) => serde_json::json!({"vacation": v}),
            Some(VacationState::Loading) => serde_json::json!({"pending": true}),
            Some(VacationState::Failed(e)) => serde_json::json!({"error": e}),
            _ => serde_json::json!({}),
        }
    }

    /// Bytes of mail cached on this computer for an account.
    pub fn cache_size(&self, account: &str) -> u64 {
        self.store.as_ref().map_or(0, |s| s.account_size(account))
    }
}
