//! The session: adding, changing and removing accounts.

use super::*;

mod importing;
#[cfg(test)]
pub(crate) use importing::REMOVE_BACKSTOP;
pub use importing::{Changes, ImportState, Imports};

/// A local account with more messages than this keeps the counts it saved when it opens, rather
/// than reading every folder to count them.
const RECOUNT_MAX: u64 = 50_000;

/// What a removed account leaves on this computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Leaves {
    /// Its data file, closed where it is (an account the user removes).
    DataFile,
    /// Nothing: the file goes too (one that never held anyone's mail, or starting again).
    Nothing,
    /// Its data file and the record of it, while an import still holds the file: the import
    /// removes the file when it ends, or the next start does.
    ForImport,
}

impl Session {
    /// The accounts the cache keeps, with their folders. Remote ones get a `Start` job (password
    /// from the keychain); local ones are counted again, unless big; what's already cached isn't new.
    pub(super) fn load_accounts(&mut self) {
        if let Some(store) = &self.store {
            let accounts = store.load_accounts().unwrap_or_else(|e| {
                log::warn!("accounts: {e}");
                Vec::new()
            });
            for mut config in accounts {
                config.migrate_signature();
                // Online only: held in memory from the start (anything left on disk goes).
                if config.keeps_online()
                    && let Err(e) = store.set_online_only(&config.id, true)
                {
                    log::warn!("{}: couldn't be kept online only: {e}", config.id);
                }
                let mut folders = store.load_folders(&config.id).unwrap_or_default();
                // A folder belongs to the account it's kept with (a data file brought in from
                // another account once kept that one's name).
                for f in &mut folders {
                    if f.account != config.id {
                        f.account.clone_from(&config.id);
                    }
                }
                sort_folders(&mut folders);
                self.accounts.push(AccountState { config, folders, status: SyncStatus::Offline, quota: None, vacation: VacationState::Unknown });
            }
        }
        let mut local = Vec::new();
        for a in &self.accounts {
            // A calendar-only account's mail is local, and its calendars have a server.
            if a.config.has_server() {
                self.jobs.push(Job::Start { account: a.config.clone(), password: None });
            }
            if a.is_local() {
                local.push(a.config.id.clone());
            }
        }
        // Imports interrupted, stopped or being removed when MailCraft last closed.
        self.load_imports();
        let local: Vec<String> = local.into_iter().filter(|id| self.account(id).is_some()).collect();
        // Counted again on opening, unless big: a big data file's counts were kept as it came in
        // and with every change since (counting meant reading its every folder's list).
        for id in local {
            let kept: u64 = self.account(&id).map(|a| a.folders.iter().map(|f| u64::from(f.total)).sum()).unwrap_or(0);
            if kept <= RECOUNT_MAX {
                self.recount_local(&id, None);
            }
        }
        // What's already cached isn't new.
        let inboxes: Vec<(String, String)> =
            self.accounts.iter().filter_map(|a| a.folder(FolderRole::Inbox).map(|f| (a.config.id.clone(), f.path.clone()))).collect();
        for (a, f) in inboxes {
            self.note_new_mail(&a, &f);
        }
    }

    /// Add an account (the password goes to the keychain through the `Start` job).
    pub fn add_account(&mut self, mut config: AccountConfig, password: Option<String>) -> Result<String, CmdError> {
        config.email = config.email.trim().to_string();
        config.migrate_signature();
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
        self.move_to_sending_account();
        if config.oauth.is_some() {
            self.jobs.push(Job::SignIn { account: config });
        } else if config.has_server() {
            self.jobs.push(Job::Start { account: config, password: password.map(Secret) });
        }
        if self.accounts.len() == 1 || self.view == View::Empty {
            self.view = View::Folder { account: id.clone(), folder: "INBOX".into() };
            self.refresh_list();
        }
        Ok(id)
    }

    /// A calendar-only account (issue #61): no mail server, its calendars and contacts on the
    /// CalDAV and CardDAV server at `server` (its own address, the user's home, or one calendar,
    /// as [`Session::set_dav`] takes them), signed in to as `username` with the account's password.
    pub fn add_calendar_account(
        &mut self,
        email: &str,
        name: &str,
        username: &str,
        server: &str,
        password: Option<String>,
    ) -> Result<String, CmdError> {
        let server = dav_address(server)?;
        if server.is_empty() {
            return Err(CmdError::Params("the calendar server's address is required".into()));
        }
        let dav = mailcraft_model::DavSettings { caldav: server.clone(), carddav: server, username: None };
        let config = AccountConfig {
            name: name.to_string(),
            email: email.to_string(),
            username: username.to_string(),
            dav: Some(dav),
            ..AccountConfig::default()
        };
        let id = self.add_account(config, password)?;
        // What it brings is calendars and contacts: those are shown.
        self.module = crate::Module::Calendar;
        Ok(id)
    }

    pub(super) fn new_account_id(&self) -> String {
        let mut n = self.accounts.len() + 1;
        // Nor one a data file's record still names (an account whose removal didn't finish).
        while self.accounts.iter().any(|a| a.config.id == format!("a{n}")) || self.importing.sources.contains_key(&format!("a{n}")) {
            n += 1;
        }
        format!("a{n}")
    }

    /// Write an account's mail to an Outlook data file at `path`.
    pub fn export_pst(&mut self, account: &str, path: &str) -> CmdResult {
        let path = path.trim();
        if path.is_empty() {
            return Err(CmdError::Params("`path` is required".into()));
        }
        let a = self.account(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        let name = if a.config.name.trim().is_empty() { a.config.email.clone() } else { a.config.name.clone() };
        // When flagged messages are due, and when completed ones were done, which only this
        // computer keeps, go into the file too.
        let follow_up = mailcraft_model::organize::FollowUpDays { due: self.prefs.follow_up.clone(), done: self.prefs.follow_up_done.clone() };
        self.jobs.push(Job::ExportPst { account: account.to_string(), path: path.to_string(), name, follow_up });
        Ok(serde_json::json!({ "account": account, "exporting": true }))
    }

    /// How an export is going; `left`: messages not on this computer, ones the data file already
    /// had, and occurrences of series Outlook can't keep the rule of, each written on its own.
    pub(super) fn exported(&mut self, account: &str, messages: usize, left: (usize, usize, usize, usize), done: bool, error: Option<String>) {
        let (skipped, already, occurrences, replaced) = left;
        let name = self.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
        match (done, error) {
            (false, _) => self.notice(format!("{name}: {messages} messages exported so far…"), false),
            (true, Some(e)) => self.notice(format!("{name} could not be exported: {e}"), true),
            (true, None) => {
                let mut more = Vec::new();
                if skipped > 0 {
                    more.push(format!("{skipped} not on this computer were left out"));
                }
                if already > 0 {
                    more.push(format!("{already} the data file already had"));
                }
                if occurrences > 0 {
                    more.push(format!("{occurrences} occurrences of series whose rule Outlook can't keep went in one by one"));
                }
                if replaced > 0 {
                    more.push(format!(
                        "{replaced} calendar items, contacts, notes or tasks changed since the last export replaced their earlier copies"
                    ));
                }
                let more = if more.is_empty() { String::new() } else { format!(" ({})", more.join("; ")) };
                self.notice(format!("{name}: {messages} messages exported{more}"), false);
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
    /// One whose data file is being imported goes once the import has stopped, with what it
    /// brought in (a part of a file that's still there is worth nothing).
    pub fn remove_account(&mut self, id: &str) -> CmdResult {
        if let Some(removing) = self.remove_while_importing(id) {
            return removing;
        }
        // The import's part-way folder is being written into its data file: a moment.
        if self.importing.keeping.contains_key(id) {
            return Err(CmdError::Failed("what was imported is being kept; remove the account once that's done".into()));
        }
        self.drop_account(id, Leaves::DataFile)
    }

    /// An account gone, leaving what `leaves` says.
    pub(super) fn drop_account(&mut self, id: &str, leaves: Leaves) -> CmdResult {
        let Some(email) = self.account(id).map(|a| a.config.email.clone()) else {
            return Err(CmdError::Params(format!("no account `{id}`")));
        };
        self.accounts.retain(|a| a.config.id != id);
        self.forget_transfers(id);
        self.cache.retain(|(a, _), _| a != id);
        // Its messages held for their drafts go with it: kept as MailCraft closes, they'd be
        // written into a removed account's folder, and sent up by a later one of the same id.
        self.saving.retain(|c| c.account != id);
        self.forget_echoes(id);
        self.search_folders_changed(id);
        self.importing.running.remove(id);
        self.importing.keeping.remove(id);
        if leaves != Leaves::ForImport && self.importing.sources.remove(id).is_some() {
            self.save_imports();
        }
        self.held.remove(id);
        self.pim.remove(id);
        // An account added later under the same id has its own calendar and contacts to save.
        self.pim_unread.remove(id);
        let mut kept = None;
        if let Some(store) = &self.store {
            let gone = match leaves {
                Leaves::DataFile => store.remove_account(id).map(|f| kept = f),
                Leaves::Nothing => store.delete_account(id),
                Leaves::ForImport => Ok(()),
            };
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
        self.drop_account(id, Leaves::Nothing)?;
        let config = AccountConfig {
            name: c.name,
            email: c.email,
            username: String::new(),
            // Its signatures, all of them with their look and pictures, go with it.
            signatures: c.signatures,
            signature_new: c.signature_new,
            signature_reply: c.signature_reply,
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
        let sent = self.server_query(&q);
        self.server_search = Some(ServerSearch { id, query: q.clone(), sent: sent.clone(), pending: by_account.len(), ..Default::default() });
        for (account, folders) in by_account {
            self.jobs.push(Job::SearchServer { account, folders, query: sent.clone(), search: id });
        }
        Ok(serde_json::json!({ "query": q, "searching": true }))
    }

    /// The search box's query for a server: `category:Work` asks for the Work category's keyword
    /// (`$label2`), and a name not in the list the keyword Thunderbird makes of it
    /// (`category:"To do list"` → `to=20do=20list`, issue #28); the server knows no names.
    pub fn server_query(&self, q: &str) -> String {
        mailcraft_model::search::Query::with_keywords(q, |v| match self.category(v) {
            Some(c) => Some(c.keyword.clone()),
            None if !mailcraft_model::is_keyword(v) => Some(mailcraft_model::keyword_for(v)),
            None => None,
        })
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

    /// A connection was refused, and `error` says a mail server on this computer showed a
    /// certificate no authority vouches for (issue #77): the window is to ask about it, once for
    /// each certificate.
    pub(crate) fn note_untrusted_certificate(&mut self, account: &str, error: &str) {
        let Some((host, fingerprint)) = mailcraft_model::untrusted_local_certificate(error) else { return };
        if self.asked_certificate.len() < 64 && self.asked_certificate.insert((account.to_string(), fingerprint.clone())) {
            self.ask_certificate.push((account.to_string(), host, fingerprint));
        }
    }

    /// Trust the certificate with this fingerprint for an account's servers on this computer,
    /// and connect again. The newest [`mailcraft_model::MAX_TRUSTED_CERTIFICATES`] are kept.
    pub fn trust_certificate(&mut self, account: &str, fingerprint: &str) -> CmdResult {
        let fingerprint = fingerprint.trim().to_ascii_lowercase();
        if !mailcraft_model::is_certificate_fingerprint(&fingerprint) {
            return Err(CmdError::Params("`fingerprint` is a certificate's SHA-256 in hex: 64 characters".into()));
        }
        let mut config = self.account(account).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if !config.has_server() {
            return Err(CmdError::Params("that account has no server".into()));
        }
        config.trusted_certificates.retain(|t| !t.eq_ignore_ascii_case(&fingerprint));
        config.trusted_certificates.push(fingerprint.clone());
        let over = config.trusted_certificates.len().saturating_sub(mailcraft_model::MAX_TRUSTED_CERTIFICATES);
        config.trusted_certificates.drain(..over);
        self.update_account(config, None)?;
        Ok(serde_json::json!({ "account": account, "trusted": fingerprint }))
    }

    pub fn update_account(&mut self, mut config: AccountConfig, password: Option<String>) -> CmdResult {
        config.migrate_signature();
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
            || a.config.check_every != config.check_every
            || a.config.dav != config.dav
            || a.config.user_agent != config.user_agent
            || a.config.trusted_certificates != config.trusted_certificates
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
        // A calendar-only account signing in as someone else: the other user's calendars and
        // contacts move out of the way (those changed here kept on this computer), as when the
        // server changes.
        let other_user = config.is_calendar_only() && a.config.username != config.username;
        a.config = config.clone();
        let server = config.has_server();
        self.save_accounts();
        if other_user {
            self.pim_mut(&id).leave_server();
            self.save_pim(&id);
        }
        if reconnect && server {
            self.jobs.push(Job::Start { account: config, password: password.map(Secret) });
        }
        Ok(serde_json::json!({ "updated": id }))
    }

    /// Where an account's calendars and contacts are, given by hand (issue #7): CalDAV and CardDAV
    /// addresses (empty: found from the account's address), and a user name of their own with its
    /// password (an app password), or `None` to sign in as the mail server does. A new user name
    /// needs its password; one already kept keeps it when none is given. They're looked for again
    /// at once.
    pub fn set_dav(&mut self, account: &str, caldav: &str, carddav: &str, username: Option<&str>, password: Option<&str>) -> CmdResult {
        let mut config = self.account(account).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if (config.is_local() && !config.is_calendar_only()) || config.graph.is_some() || config.ews.is_some() {
            return Err(CmdError::Params("this account's calendars and contacts come with it: there's no address to give".into()));
        }
        let caldav = dav_address(caldav)?;
        let carddav = dav_address(carddav)?;
        // Without one, a calendar-only account would be left with nothing but mail kept here.
        if config.is_calendar_only() && caldav.is_empty() && carddav.is_empty() {
            return Err(CmdError::Params("a calendar-only account needs its server's address".into()));
        }
        let username = username.map(str::trim).filter(|u| !u.is_empty()).map(|u| u.chars().take(320).collect::<String>());
        let password = password.filter(|p| !p.is_empty());
        // A password kept is the user name's it was kept for: another user name needs its own.
        let before = config.dav.clone().unwrap_or_default();
        if username.is_some() && password.is_none() && before.username != username {
            return Err(CmdError::Params("the calendar server's password is required".into()));
        }
        let dav = mailcraft_model::DavSettings { caldav, carddav, username };
        let changed = config.dav.clone().unwrap_or_default() != dav || password.is_some();
        config.dav = (dav != mailcraft_model::DavSettings::default()).then_some(dav);
        let own = config.dav.as_ref().and_then(|d| d.username.as_ref()).is_some();
        if let (Some(pw), true) = (password, own) {
            self.jobs.push(Job::DavPassword { account: config.clone(), password: Secret(pw.to_string()) });
        } else if before.username.is_some() && !own {
            // Back to the mail server's sign-in: the calendar server's own password goes.
            self.jobs.push(Job::DavPassword { account: config.clone(), password: Secret(String::new()) });
        }
        let password_only = self.account(account).is_some_and(|a| a.config.dav == config.dav) && changed;
        self.update_account(config.clone(), None)?;
        if changed {
            let p = self.pim_mut(account);
            if password_only {
                // The same server: looked for again with the new password.
                p.caldav_home = None;
                p.carddav_home = None;
                p.dav_checked = None;
            } else {
                // Another server, or another user on it: the old one's items move out of the way
                // (those changed here kept on this computer), and the homes are looked for again.
                p.leave_server();
            }
            self.save_pim(account);
            // A new password alone doesn't reconnect: the next sync of calendars takes it.
            if password_only {
                self.jobs.push(Job::SyncDav { account: account.to_string() });
            }
        }
        Ok(serde_json::json!({ "account": account, "dav": config.dav }))
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
            for a in self.accounts.iter_mut().filter(|a| a.config.has_server()) {
                a.status = SyncStatus::Offline;
            }
        }
    }

    pub fn sync_all(&mut self) -> usize {
        let ids: Vec<String> = self.accounts.iter().filter(|a| a.config.has_server()).map(|a| a.config.id.clone()).collect();
        for id in &ids {
            self.jobs.push(Job::SyncAll { account: id.clone() });
        }
        ids.len()
    }
}

/// A calendar or contacts address as typed: empty, or https (`posteo.de:8443` reads as
/// `https://posteo.de:8443`); plain http only to this computer, so no password crosses the network
/// in the clear.
fn dav_address(given: &str) -> Result<String, CmdError> {
    let g = given.trim();
    if g.is_empty() {
        return Ok(String::new());
    }
    let bad = || CmdError::Params(format!("`{g}` isn't an https:// address"));
    if g.len() > 2048 || g.contains(char::is_whitespace) {
        return Err(bad());
    }
    let lower = g.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("http://") {
        let host = r.split(['/', '?', '#']).next().unwrap_or_default();
        let host = host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(h, _)| h);
        if !matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
            return Err(CmdError::Params(format!("`{g}` isn't https: the password would cross the network unprotected")));
        }
        r
    } else if lower.contains("://") {
        return Err(bad());
    } else {
        return Ok(format!("https://{g}"));
    };
    if rest.split(['/', '?', '#']).next().is_none_or(str::is_empty) {
        return Err(bad());
    }
    Ok(g.to_string())
}
