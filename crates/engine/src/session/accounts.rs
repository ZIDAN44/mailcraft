//! The session: adding, changing and removing accounts.

use super::*;

impl Session {
    /// Add an account (the password goes to the keychain through the `Start` job).
    pub fn add_account(&mut self, mut config: AccountConfig, password: Option<String>) -> Result<String, CmdError> {
        config.email = config.email.trim().to_string();
        if !addr::is_valid_email(&config.email) {
            return Err(CmdError::Params(format!("`{}` is not an email address", config.email)));
        }
        if self.accounts.iter().any(|a| a.config.email.eq_ignore_ascii_case(&config.email)) {
            return Err(CmdError::Params(format!("{} is already added", config.email)));
        }
        if config.username.trim().is_empty() {
            config.username = config.email.clone();
        }
        config.id = self.new_account_id();
        config.color = u8::try_from(self.accounts.len() % usize::from(ACCOUNT_COLORS)).unwrap_or(0);
        let local = config.is_local();
        let incoming = config.pop.as_ref().unwrap_or(&config.imap);
        if !local
            && !config.is_api()
            && (incoming.host.trim().is_empty() || incoming.port == 0 || config.smtp.host.trim().is_empty() || config.smtp.port == 0)
        {
            return Err(CmdError::Params("incoming and outgoing server names and ports are required".into()));
        }
        let id = config.id.clone();
        if config.online_only && !config.can_be_online_only() {
            return Err(CmdError::Params("a POP account keeps its mail on this computer: it can't be online only".into()));
        }
        if let Some(store) = &self.store
            && config.keeps_online()
        {
            store.set_online_only(&id, true).map_err(|e| CmdError::Failed(e.to_string()))?;
        }
        self.accounts.push(AccountState {
            config: config.clone(),
            folders: Vec::new(),
            status: SyncStatus::Connecting,
            quota: None,
            vacation: VacationState::Unknown,
        });
        self.save_accounts();
        if config.oauth.is_some() {
            self.jobs.push(Job::SignIn { account: config });
        } else if !local {
            self.jobs.push(Job::Start { account: config, password: password.map(Secret) });
        }
        if self.accounts.len() == 1 || self.view == View::Empty {
            self.view = View::Folder { account: id.clone(), folder: "INBOX".into() };
            self.refresh_list();
        }
        Ok(id)
    }

    pub(super) fn new_account_id(&self) -> String {
        let mut n = self.accounts.len() + 1;
        while self.accounts.iter().any(|a| a.config.id == format!("a{n}")) {
            n += 1;
        }
        format!("a{n}")
    }

    /// Open an Outlook data file (`.pst`): a new account on this computer, named after the file,
    /// that the import fills.
    pub fn import_pst(&mut self, path: &str) -> CmdResult {
        let path = path.trim();
        if path.is_empty() {
            return Err(CmdError::Params("`path` is required".into()));
        }
        let file = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
        let stem = file.rsplit_once('.').map_or(file.as_str(), |(s, _)| s).trim().to_string();
        let id = self.new_account_id();
        let config = AccountConfig {
            id: id.clone(),
            name: if stem.is_empty() { file.clone() } else { stem },
            // Shown where an account's address would be; never an address, so never "me".
            email: file,
            color: u8::try_from(self.accounts.len() % usize::from(ACCOUNT_COLORS)).unwrap_or(0),
            ..Default::default()
        };
        self.accounts.push(AccountState {
            config,
            folders: Vec::new(),
            status: SyncStatus::Syncing { folder: String::new() },
            quota: None,
            vacation: VacationState::Unknown,
        });
        self.save_accounts();
        self.jobs.push(Job::ImportPst { account: id.clone(), path: path.to_string() });
        Ok(serde_json::json!({ "account": id, "importing": true }))
    }

    /// Write an account's mail to an Outlook data file at `path`.
    pub fn export_pst(&mut self, account: &str, path: &str) -> CmdResult {
        let path = path.trim();
        if path.is_empty() {
            return Err(CmdError::Params("`path` is required".into()));
        }
        let a = self.account(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        let name = if a.config.name.trim().is_empty() { a.config.email.clone() } else { a.config.name.clone() };
        self.jobs.push(Job::ExportPst { account: account.to_string(), path: path.to_string(), name });
        Ok(serde_json::json!({ "account": account, "exporting": true }))
    }

    pub(super) fn exported(&mut self, account: &str, messages: usize, skipped: usize, done: bool, error: Option<String>) {
        let name = self.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
        match (done, error) {
            (false, _) => self.notice(format!("{name}: {messages} messages exported so far…"), false),
            (true, Some(e)) => self.notice(format!("{name} could not be exported: {e}"), true),
            (true, None) => {
                let more = if skipped > 0 { format!(" ({skipped} not on this computer were left out)") } else { String::new() };
                self.notice(format!("{name}: {messages} messages exported{more}"), false);
            }
        }
    }

    /// Bring an existing data file's mail into a new account before it first connects (Outlook's
    /// "use an existing data file" for POP accounts).
    pub fn attach_data_file(&mut self, account: &str, path: &str) -> CmdResult {
        let path = path.trim();
        if path.is_empty() {
            return Err(CmdError::Params("`path` is required".into()));
        }
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        // Connecting waits for the file, so the two don't write the Inbox at once.
        if let Some(i) = self.jobs.iter().position(|j| matches!(j, Job::Start { .. } | Job::SignIn { .. }) && j.account() == account) {
            let start = self.jobs.remove(i);
            self.held.insert(account.to_string(), start);
        }
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Syncing { folder: String::new() };
        }
        self.jobs.push(Job::ImportPst { account: account.to_string(), path: path.to_string() });
        Ok(serde_json::json!({ "account": account, "importing": true }))
    }

    pub(super) fn imported(&mut self, account: &str, messages: usize, skipped: usize, done: bool, error: Option<String>) {
        let Some(name) = self.account(account).map(|a| a.config.email.clone()) else { return };
        if !done {
            self.notice(format!("{name}: {messages} messages imported so far…"), false);
            return;
        }
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Offline;
        }
        // An account that was waiting for its data file connects now.
        if let Some(start) = self.held.remove(account) {
            self.jobs.push(start);
            if let Some(e) = &error {
                self.notice(format!("{name}: the data file could not be opened: {e}"), true);
                return;
            }
        }
        match error {
            Some(e) => {
                // Nothing came in: the account goes again, and the file it was starting.
                let _ = self.drop_account(account, false);
                self.notice(format!("{name} could not be opened: {e}"), true);
            }
            None => {
                // The file's contacts and calendar came in too.
                if let Some(store) = self.store.clone()
                    && let Ok(mut p) = store.load_account_doc::<mailcraft_model::pim::Pim>(account, mailcraft_model::pim::PIM_DOC)
                {
                    p.ensure_defaults();
                    self.pim.insert(account.to_string(), p);
                    self.contacts = None;
                    self.pim_revision = self.pim_revision.wrapping_add(1);
                }
                let more = if skipped > 0 { format!(" ({skipped} could not be read)") } else { String::new() };
                self.notice(format!("{name}: {messages} messages imported{more}"), false);
                if let Some(inbox) = self.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()) {
                    self.open_view(View::Folder { account: account.to_string(), folder: inbox });
                }
            }
        }
    }

    /// Move an account up (`by` < 0) or down the folder pane, as Outlook moves a mailbox.
    pub fn move_account(&mut self, id: &str, by: i64) -> CmdResult {
        if self.account(id).is_none() {
            return Err(CmdError::Params(format!("no account `{id}`")));
        }
        let to = self.move_pane_section(id, by)?;
        Ok(serde_json::json!({ "account": id, "position": to }))
    }

    /// The folder pane's mailboxes in order (issue #1): each account, and each shared mailbox an
    /// account opens as a mailbox of its own (`<account>|<address>`), placed where the user moved
    /// them; new ones after the mailbox they came with.
    pub fn pane_sections(&self) -> Vec<PaneSection> {
        let mut natural = Vec::new();
        for a in &self.accounts {
            natural.push(PaneSection { key: a.config.id.clone(), account: a.config.id.clone(), mailbox: None });
            for top in a.folders.iter().filter(|f| f.depth() == 0) {
                if let Some(address) = &top.mailbox {
                    let key = format!("{}|{}", a.config.id, address.to_ascii_lowercase());
                    if !natural.iter().any(|s: &PaneSection| s.key == key) {
                        natural.push(PaneSection { key, account: a.config.id.clone(), mailbox: Some(top.path.clone()) });
                    }
                }
            }
        }
        let mut out: Vec<PaneSection> = self.prefs.pane_order.iter().filter_map(|k| natural.iter().find(|s| &s.key == k).cloned()).collect();
        for (i, s) in natural.iter().enumerate() {
            if out.iter().any(|o| o.key == s.key) {
                continue;
            }
            let after = natural.get(..i).unwrap_or_default().iter().rev().find_map(|p| out.iter().position(|o| o.key == p.key));
            out.insert(after.map_or(0, |at| at + 1), s.clone());
        }
        out
    }

    /// Move a mailbox in the folder pane `by` places (an account's own order follows, as the
    /// accounts' order is theirs everywhere else). Returns where it is now.
    pub fn move_pane_section(&mut self, key: &str, by: i64) -> Result<usize, CmdError> {
        let mut order: Vec<String> = self.pane_sections().into_iter().map(|s| s.key).collect();
        let from = order.iter().position(|k| k == key).ok_or_else(|| CmdError::Params(format!("no mailbox `{key}` in the folder pane")))?;
        let last = order.len().saturating_sub(1);
        let to = if by < 0 { from.saturating_sub(by.unsigned_abs() as usize) } else { from.saturating_add(by as usize).min(last) };
        if to != from {
            let k = order.remove(from);
            order.insert(to, k);
            let ids: Vec<String> = order.iter().filter(|k| !k.contains('|')).cloned().collect();
            let mut accounts = std::mem::take(&mut self.accounts);
            accounts.sort_by_key(|a| ids.iter().position(|id| *id == a.config.id).unwrap_or(usize::MAX));
            self.accounts = accounts;
            self.prefs.pane_order = order;
            self.save_accounts();
            self.revision = self.revision.wrapping_add(1);
        }
        Ok(to)
    }

    /// Remove an account as Outlook does: gone from here, its data file kept (closed) where it
    /// is, so it can be opened again from File › Open & Export; nothing changes on the server.
    pub fn remove_account(&mut self, id: &str) -> CmdResult {
        self.drop_account(id, true)
    }

    /// An account gone; `keep` keeps its data file (an account the user removes), otherwise the
    /// file goes too (one that never held anyone's mail, or an account starting again).
    pub(super) fn drop_account(&mut self, id: &str, keep: bool) -> CmdResult {
        let Some(email) = self.account(id).map(|a| a.config.email.clone()) else {
            return Err(CmdError::Params(format!("no account `{id}`")));
        };
        self.accounts.retain(|a| a.config.id != id);
        self.cache.retain(|(a, _), _| a != id);
        self.pim.remove(id);
        let mut kept = None;
        if let Some(store) = &self.store {
            let gone = if keep { store.remove_account(id).map(|f| kept = f) } else { store.delete_account(id) };
            if let Err(e) = gone {
                self.notice(e.to_string(), true);
            }
        }
        if let Some(f) = &kept {
            self.notice(format!("{email} removed. Its data file stays at {}", f.display()), false);
        }
        self.save_accounts();
        self.jobs.push(Job::Remove { account: id.to_string() });
        if self.compose.as_ref().is_some_and(|c| c.account == id) {
            self.compose = None;
        }
        self.composes.retain(|c| c.account != id);
        let view = self.default_view();
        self.open_view(view);
        Ok(serde_json::json!({ "removed": id, "dataFile": kept.map(|f| f.display().to_string()) }))
    }

    /// An account that signs in to Microsoft with a password (which Microsoft refuses) switched to
    /// Sign in with Microsoft, keeping its mail: the browser opens, and the token replaces the
    /// password in the keychain.
    pub fn use_microsoft_sign_in(&mut self, id: &str) -> CmdResult {
        let a = self.account_mut(id).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
        if !mailcraft_model::microsoft_password_account(&a.config) {
            return Err(CmdError::Params("this account doesn't sign in to Microsoft with a password".into()));
        }
        a.config.oauth = Some("microsoft".into());
        // Outlook.com's own servers, where the address is one of Microsoft's.
        if a.config.imap.host.trim().is_empty() || mailcraft_model::microsoft_consumer(&a.config.email) && a.config.pop.is_none() {
            let guess = AccountConfig::guess(&a.config.email);
            a.config.imap = guess.imap;
            a.config.smtp = guess.smtp;
        }
        a.status = SyncStatus::Connecting;
        let config = a.config.clone();
        self.save_accounts();
        self.jobs.push(Job::SignIn { account: config });
        Ok(serde_json::json!({ "account": id, "signingIn": true }))
    }

    /// A Microsoft account that signs in with Microsoft moved from IMAP and SMTP to Microsoft
    /// Graph (for a mailbox Microsoft won't open over IMAP). Graph names folders its own way, so
    /// it starts again as a new account in the same place: its mail comes down from the server,
    /// and what was only on this computer for it (unsent mail in the Outbox) goes.
    pub fn use_graph(&mut self, id: &str) -> CmdResult {
        let a = self.account(id).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
        let c = a.config.clone();
        let microsoft = c.oauth.as_deref() == Some("microsoft") || mailcraft_model::microsoft_password_account(&c);
        if c.graph.is_some() || c.jmap.is_some() || c.ews.is_some() || c.pop.is_some() || !microsoft {
            return Err(CmdError::Params("only a Microsoft account connected over IMAP can move to Microsoft Graph".into()));
        }
        if !self.oauth_available.iter().any(|p| p == "microsoft") {
            return Err(CmdError::Params("signing in with Microsoft isn't set up in this copy of MailCraft".into()));
        }
        let at = self.accounts.iter().position(|x| x.config.id == id).unwrap_or(0);
        // Starting again from the server: its old file isn't kept beside the new one.
        self.drop_account(id, false)?;
        let config = AccountConfig {
            name: c.name,
            email: c.email,
            username: String::new(),
            signature: c.signature,
            oauth: Some("microsoft".into()),
            graph: Some(mailcraft_model::GRAPH_URL.to_string()),
            ..Default::default()
        };
        let new = self.add_account(config, None)?;
        if let Some(from) = self.accounts.iter().position(|x| x.config.id == new) {
            let mut moved = self.accounts.remove(from);
            moved.config.color = c.color;
            self.accounts.insert(at.min(self.accounts.len()), moved);
            self.save_accounts();
        }
        self.open_view(View::Folder { account: new.clone(), folder: "INBOX".into() });
        Ok(serde_json::json!({ "account": new, "graph": true }))
    }

    /// What closing MailCraft would lose (issue #4): online-only accounts' mail still waiting to
    /// be sent, and their changes not yet on the server, held in memory only.
    pub fn lost_on_close(&self) -> (usize, usize) {
        let online: Vec<&str> = self.accounts.iter().filter(|a| a.config.keeps_online()).map(|a| a.config.id.as_str()).collect();
        if online.is_empty() {
            return (0, 0);
        }
        let unsent = self.outbox.iter().filter(|(a, _)| online.contains(&a.as_str())).count();
        let changes = online.iter().map(|a| self.store.as_ref().and_then(|s| s.load_ops(a).ok()).map_or(0, |o| o.len())).sum();
        (unsent, changes)
    }

    /// How many more of the folder on view its server has than are kept here (issue #4), with
    /// the folder: shown at the end of its list, to show more.
    pub fn more_on_server(&self) -> Option<(String, String, u32)> {
        let View::Folder { account, folder } = &self.view else { return None };
        if self.search.is_some() {
            return None;
        }
        let a = self.account(account)?;
        if a.is_local() || a.config.pop.is_some() {
            return None;
        }
        let total = a.folders.iter().find(|f| &f.path == folder)?.total;
        // The folder on view is loaded (its list is on screen).
        let kept = self.cache.get(&(account.clone(), folder.clone())).map_or(0, |st| st.messages.len());
        let kept = u32::try_from(kept).unwrap_or(u32::MAX);
        (total > kept).then(|| (account.clone(), folder.clone(), total - kept))
    }

    /// Keep `more` older messages of a folder than it keeps now, fetched from its server.
    pub fn show_more(&mut self, account: &str, folder: &str, more: u32) -> CmdResult {
        let a = self.account(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if a.is_local() || a.config.pop.is_some() {
            return Err(CmdError::Disabled("this account keeps all its mail here".into()));
        }
        self.jobs.push(Job::MoreMail { account: account.to_string(), folder: folder.to_string(), more });
        self.showing_more = Some((account.to_string(), folder.to_string()));
        Ok(serde_json::json!({ "account": account, "folder": folder, "more": more }))
    }

    /// Ask the servers for the search box's query (issue #4: mail past what's kept here): the
    /// folder on view, or every folder of every account with a server.
    pub fn search_server(&mut self) -> CmdResult {
        let (q, scope) = self.search.clone().ok_or_else(|| CmdError::Disabled("search for something first".into()))?;
        let mut by_account: Vec<(String, Vec<String>)> = Vec::new();
        self.remember_search(&q);
        for (a, f) in self.scope_folders(scope) {
            if self.account(&a).is_none_or(|x| x.is_local() || x.config.pop.is_some()) {
                continue;
            }
            match by_account.iter_mut().find(|(x, _)| *x == a) {
                Some((_, v)) => v.push(f),
                None => by_account.push((a, vec![f])),
            }
        }
        if by_account.is_empty() {
            return Err(CmdError::Disabled("no account here has its mail on a server".into()));
        }
        self.searches = self.searches.wrapping_add(1).max(1);
        let id = self.searches;
        self.server_search = Some(ServerSearch { id, query: q.clone(), pending: by_account.len(), ..Default::default() });
        for (account, folders) in by_account {
            self.jobs.push(Job::SearchServer { account, folders, query: q.clone(), search: id });
        }
        Ok(serde_json::json!({ "query": q, "searching": true }))
    }

    /// Ask for MailCraft's releases (File › About › Check for updates).
    pub(crate) fn check_updates(&mut self) {
        self.update = UpdateCheck::Checking;
        self.jobs.push(Job::CheckUpdates);
    }

    /// Keep an account's outgoing server's own password (its `smtp_username`'s), in the keychain.
    pub(crate) fn keep_smtp_password(&mut self, account: &str, password: String) {
        if let Some(a) = self.account(account) {
            self.jobs.push(Job::SmtpPassword { account: a.config.clone(), password: Secret(password) });
        }
    }

    pub fn update_account(&mut self, config: AccountConfig, password: Option<String>) -> CmdResult {
        let id = config.id.clone();
        let Some(a) = self.account_mut(&id) else { return Err(CmdError::Params(format!("no account `{id}`"))) };
        let reconnect = a.config.imap != config.imap
            || a.config.smtp != config.smtp
            || a.config.jmap != config.jmap
            || a.config.graph != config.graph
            || a.config.ews != config.ews
            || a.config.pop != config.pop
            || a.config.pop_delete != config.pop_delete
            || a.config.pop_keep_days != config.pop_keep_days
            || a.config.pop_delete_with_trash != config.pop_delete_with_trash
            || a.config.username != config.username
            || a.config.smtp_username != config.smtp_username
            || a.config.shared_mailboxes != config.shared_mailboxes
            || a.config.online_only != config.online_only
            || password.is_some();
        if config.online_only && !config.can_be_online_only() {
            return Err(CmdError::Params("a POP account keeps its mail on this computer: it can't be online only".into()));
        }
        // Online only, or kept here again: what's kept moves between memory and the disk first.
        if a.config.online_only != config.online_only
            && let Some(store) = self.store.clone()
        {
            store.set_online_only(&id, config.keeps_online()).map_err(|e| CmdError::Failed(e.to_string()))?;
        }
        let Some(a) = self.account_mut(&id) else { return Err(CmdError::Params(format!("no account `{id}`"))) };
        a.config = config.clone();
        let local = a.is_local();
        self.save_accounts();
        if reconnect && !local {
            self.jobs.push(Job::Start { account: config, password: password.map(Secret) });
        }
        Ok(serde_json::json!({ "updated": id }))
    }

    pub(super) fn save_accounts(&mut self) {
        let configs: Vec<AccountConfig> = self.accounts.iter().map(|a| a.config.clone()).collect();
        if let Some(store) = &self.store
            && let Err(e) = store.save_accounts(&configs)
        {
            self.notice(format!("account settings could not be saved: {e}"), true);
        }
    }

    /// Ask every account to sync now.
    /// Work offline, or connect again: every account's connection closes (changes, sent mail and
    /// drafts wait on this computer) or opens. Ahead of anything queued, so what follows sees it.
    #[cfg(test)]
    pub(crate) fn jobs_push_for_test(&mut self, j: Job) {
        self.jobs.push(j);
    }

    /// Tell the network side how much mail to keep (ahead of anything queued, so accounts
    /// start with it).
    pub fn apply_sync_prefs(&mut self) {
        self.jobs.retain(|j| !matches!(j, Job::SetSync { .. }));
        self.jobs.insert(0, Job::SetSync { messages: self.prefs.keep_messages, offline: self.prefs.keep_offline });
    }

    pub fn set_work_offline(&mut self, on: bool) {
        self.prefs.work_offline = on;
        self.jobs.retain(|j| !matches!(j, Job::WorkOffline { .. }));
        self.jobs.insert(0, Job::WorkOffline { on });
        if on {
            for a in self.accounts.iter_mut().filter(|a| !a.is_local()) {
                a.status = SyncStatus::Offline;
            }
        }
    }

    pub fn sync_all(&mut self) -> usize {
        let ids: Vec<String> = self.accounts.iter().filter(|a| !a.is_local()).map(|a| a.config.id.clone()).collect();
        for id in &ids {
            self.jobs.push(Job::SyncAll { account: id.clone() });
        }
        ids.len()
    }
}
