//! The session: data files imported into accounts. An import is followed from when it's asked for
//! until the user is done with it: running, stopped (kept, to go on or to keep as it is), or
//! thrown away with its account. Where each account's mail came from is kept in `imports.json`,
//! so an import interrupted, stopped or being removed is still known the next time MailCraft
//! opens. While an account imports, or its import waits on the user, its folders aren't changed
//! (the import writes them), and what comes in isn't new mail.

use mailcraft_model::job::{Blocked, DataFileFacts, DataSource, Eta, ImportPhase, ImportProgress, ImportReport, ImportStage, StopHow};

use super::*;

/// Where each account's mail came from, and how far its import got (beside `rules.json`).
pub(crate) const IMPORTS_DOC: &str = "imports.json";
/// The import's own note of how far it got, kept inside the account's data file.
const CHECKPOINT_DOC: &str = "import.json";
/// Data files looked at, kept with their answer.
const INSPECTED_MAX: usize = 4;
/// Imports finished whose report the window hasn't shown yet: the newest few.
const FINISHED_MAX: usize = 8;
/// `imports.json` read back: at most this many rows, a path at most this long.
const SOURCES_MAX: usize = 1024;
const PATH_MAX: usize = 4096;
/// An account removed while it imports goes once the import says it's thrown away. When that
/// never comes (a reader stuck on a slow drive), it goes anyway after this many events; the
/// engine reads no clock.
pub(crate) const REMOVE_BACKSTOP: u32 = 512;

/// The imports the session follows, where each account's mail came from, and the data files
/// looked at before they're imported.
#[derive(Clone, Debug, Default)]
pub struct Imports {
    /// Imports running or waiting on the user, by account.
    pub running: BTreeMap<String, ImportState>,
    /// `imports.json`: each account a data file was imported into.
    pub sources: BTreeMap<String, DataSource>,
    /// Data files looked at, newest last; `None` while the answer is awaited.
    pub inspected: Vec<(String, Option<Box<DataFileFacts>>)>,
    /// Imports finished, oldest first, whose report the window is still to show.
    finished: Vec<String>,
    /// `imports.json` couldn't be read: never written over.
    unread: bool,
}

/// One account's import as the window shows it.
#[derive(Clone, Debug, Default)]
pub struct ImportState {
    /// The data file's name, and where it is.
    pub file: String,
    pub path: String,
    pub progress: ImportProgress,
    pub eta: Eta,
    /// Being read now; otherwise stopped, waiting for the user to go on or keep it.
    pub live: bool,
    /// Asked to stop, and how.
    pub stopping: Option<StopHow>,
    /// The account is removed once its import has stopped.
    pub removing: bool,
    /// Events applied since the removal was asked for.
    waited: u32,
}

impl Imports {
    /// Whether a data file is being read into `account` now.
    pub fn live(&self, account: &str) -> bool {
        self.running.get(account).is_some_and(|s| s.live)
    }

    /// Whether `account` has an import running or waiting on the user.
    pub fn held(&self, account: &str) -> bool {
        self.running.contains_key(account)
    }

    /// Mark `account`'s import as read now (`on`) or not; whether it was.
    pub(crate) fn set_live(&mut self, account: &str, on: bool) -> bool {
        self.running.get_mut(account).is_some_and(|s| std::mem::replace(&mut s.live, on))
    }
}

/// A path as compared for "the same file": `\` and `/` alike, case ignored.
fn same_path(a: &str, b: &str) -> bool {
    let norm = |p: &str| p.trim().replace('\\', "/").to_lowercase();
    norm(a) == norm(b)
}

/// The file's name at the end of a path.
fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

impl Session {
    /// Whether any data file is being read now.
    pub fn import_running(&self) -> bool {
        self.importing.running.values().any(|s| s.live)
    }

    /// `account`'s import, running or waiting on the user.
    pub fn import_state(&self, account: &str) -> Option<&ImportState> {
        self.importing.running.get(account)
    }

    /// Where `account`'s mail came from, when a data file was imported into it.
    pub fn data_source(&self, account: &str) -> Option<&DataSource> {
        self.importing.sources.get(account)
    }

    /// Every import running or waiting on the user, by account.
    pub fn imports(&self) -> impl Iterator<Item = (&str, &ImportState)> {
        self.importing.running.iter().map(|(a, s)| (a.as_str(), s))
    }

    /// Seconds left of `account`'s import by the session's clock, once that can be told.
    pub fn import_eta_s(&self, account: &str) -> Option<u64> {
        let now = self.ui_ms();
        self.importing.running.get(account).filter(|s| s.live).and_then(|s| s.eta.remaining_s(now))
    }

    /// What was found of the data file at `path`, once it has been looked at.
    pub fn data_file_facts(&self, path: &str) -> Option<&DataFileFacts> {
        self.facts_for(path)
    }

    /// Whether the data file at `path` is being looked at now.
    pub fn inspecting(&self, path: &str) -> bool {
        self.importing.inspected.iter().rev().find(|(p, _)| p == path).is_some_and(|(_, f)| f.is_none())
    }

    /// The account the data file at `path` is open as already (by its record key, once looked at).
    pub fn data_file_open_as(&self, path: &str) -> Option<String> {
        let key = self.facts_for(path).map(|f| f.record_key.clone()).unwrap_or_default();
        self.duplicate_of(path.trim(), &key, None)
    }

    /// An import that has finished whose report the window hasn't shown: each is taken once.
    pub fn take_finished_import(&mut self) -> Option<String> {
        if self.importing.finished.is_empty() { None } else { Some(self.importing.finished.remove(0)) }
    }

    /// The interface's clock (ms) for the time left: the session's.
    fn ui_ms(&self) -> u64 {
        u64::try_from(self.clock.now).unwrap_or(0).saturating_mul(1000)
    }

    /// Why `account`'s folders can't be changed now: its import writes them, or waits to go on.
    /// A failure, not a disabled command: the window says why (it says nothing of a disabled one).
    pub(crate) fn import_holds(&self, account: &str) -> Result<(), CmdError> {
        match self.importing.running.get(account) {
            None => Ok(()),
            Some(s) if s.removing => Err(CmdError::Failed("this account is being removed".into())),
            Some(s) if s.live => Err(CmdError::Failed("wait until the data file is imported".into())),
            Some(_) => Err(CmdError::Failed("continue importing the data file, or keep what's imported, first".into())),
        }
    }

    /// The account already holding the data file at `path` (by its record key, when known), but
    /// not `except`.
    fn duplicate_of(&self, path: &str, record_key: &str, except: Option<&str>) -> Option<String> {
        self.importing
            .sources
            .iter()
            .filter(|(a, _)| Some(a.as_str()) != except && self.account(a).is_some())
            .filter(|(a, _)| self.importing.running.get(*a).is_none_or(|s| !s.removing))
            .find(|(_, s)| (!record_key.is_empty() && s.record_key == record_key) || same_path(&s.path, path))
            .map(|(a, _)| a.clone())
    }

    /// The facts found for the data file at `path`, when it was looked at.
    fn facts_for(&self, path: &str) -> Option<&DataFileFacts> {
        self.importing.inspected.iter().rev().find(|(p, _)| p == path).and_then(|(_, f)| f.as_deref())
    }

    /// `imports.json` read at start. A removal that didn't end is finished (the data file of one
    /// whose account already went, too); an import interrupted (MailCraft closed or stopped while
    /// it ran) goes on by itself, and one the user stopped waits on them (a POP account's
    /// connection with either); a data file's account left with no folders, no record and no
    /// note of its own (from before this was kept) is one to import again or remove.
    pub(super) fn load_imports(&mut self) {
        let Some(store) = self.store.clone() else { return };
        let mut sources: BTreeMap<String, DataSource> = match store.load_doc(IMPORTS_DOC) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("where the data files came from couldn't be read: {e}");
                self.importing.unread = true;
                BTreeMap::new()
            }
        };
        // Read as hostile: bounded, and the reports tidied.
        sources.retain(|a, s| a.len() <= 64 && s.path.len() <= PATH_MAX);
        while sources.len() > SOURCES_MAX {
            sources.pop_last();
        }
        for s in sources.values_mut() {
            if let Some(r) = s.report.as_mut() {
                r.tidy();
            }
        }
        self.importing.sources = sources;
        let removing: Vec<String> = self
            .importing
            .sources
            .iter()
            .filter(|(a, s)| s.stage == ImportStage::Removing && self.account(a).is_some())
            .map(|(a, _)| a.clone())
            .collect();
        for a in removing {
            let _ = self.drop_account(&a, Leaves::Nothing);
        }
        // Gone from the list while its import still held the file: the file goes now, and the
        // record (keeping the id from being given again) once it has.
        let left: Vec<String> = self
            .importing
            .sources
            .iter()
            .filter(|(a, s)| s.stage == ImportStage::Removing && self.account(a).is_none())
            .map(|(a, _)| a.clone())
            .collect();
        for a in left {
            match store.delete_account(&a) {
                Ok(()) => {
                    self.importing.sources.remove(&a);
                    self.save_imports();
                }
                Err(e) => log::warn!("{a}: the data file of an account removed couldn't be removed yet: {e}"),
            }
        }
        let unfinished: Vec<(String, String, bool)> = self
            .importing
            .sources
            .iter()
            .filter(|(a, s)| matches!(s.stage, ImportStage::Running | ImportStage::Stopped) && self.account(a).is_some())
            .map(|(a, s)| (a.clone(), s.path.clone(), s.stage == ImportStage::Running))
            .collect();
        for (a, path, interrupted) in unfinished {
            self.hold_start(&a);
            if interrupted && !path.is_empty() {
                self.queue_import(&a, &path, true);
                continue;
            }
            let progress = ImportProgress { phase: ImportPhase::Paused, ..ImportProgress::default() };
            self.importing.running.insert(a.clone(), ImportState { file: file_name(&path), path, progress, ..ImportState::default() });
        }
        let orphans: Vec<(String, String)> = self
            .accounts
            .iter()
            .filter(|a| a.is_data_file() && a.folders.is_empty())
            .filter(|a| !self.importing.sources.contains_key(&a.config.id) && !self.importing.held(&a.config.id))
            .map(|a| (a.config.id.clone(), a.config.email.clone()))
            .collect();
        for (a, file) in orphans {
            // Its own note says how far it got (it can go on), or it's a file of contacts and
            // calendars only: not one to start again.
            let noted = |doc: &str| !matches!(store.load_account_doc::<Option<serde_json::Value>>(&a, doc), Ok(None));
            if noted(CHECKPOINT_DOC) || noted(mailcraft_model::pim::PIM_DOC) {
                continue;
            }
            let progress = ImportProgress { phase: ImportPhase::Blocked, blocked: Some(Blocked::SourceMissing), ..ImportProgress::default() };
            self.importing.running.insert(a, ImportState { file, progress, ..ImportState::default() });
        }
    }

    pub(super) fn save_imports(&mut self) {
        if self.importing.unread {
            return;
        }
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc(IMPORTS_DOC, &self.importing.sources)
        {
            self.notice(format!("where the data files came from could not be saved: {e}"), true);
        }
    }

    /// An account's connection waits for its data file (they'd write the Inbox at once).
    fn hold_start(&mut self, account: &str) {
        if let Some(i) = self.jobs.iter().position(|j| matches!(j, Job::Start { .. } | Job::SignIn { .. }) && j.account() == account) {
            let start = self.jobs.remove(i);
            self.held.insert(account.to_string(), start);
        }
    }

    /// Read the data file at `path` into `account` (`resume`: going on from where it stopped),
    /// noting where it came from.
    fn queue_import(&mut self, account: &str, path: &str, resume: bool) {
        let mut row = DataSource { path: path.to_string(), stage: ImportStage::Running, ..DataSource::default() };
        // Gone on with: what was known of the file, and its report so far, stay.
        if resume && let Some(was) = self.importing.sources.get(account) {
            (row.size, row.modified, row.record_key) = (was.size, was.modified, was.record_key.clone());
            row.report.clone_from(&was.report);
        }
        if let Some(f) = self.facts_for(path) {
            (row.size, row.modified, row.record_key) = (f.size, f.modified, f.record_key.clone());
        }
        self.importing.sources.insert(account.to_string(), row);
        self.save_imports();
        let state = ImportState { file: file_name(path), path: path.to_string(), live: true, ..ImportState::default() };
        self.importing.running.insert(account.to_string(), state);
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Syncing { folder: String::new() };
        }
        self.jobs.push(Job::ImportPst { account: account.to_string(), path: path.to_string(), resume });
    }

    /// Open an Outlook data file (`.pst`): a new account on this computer, named after the file,
    /// that the import fills. A file already open as another account is refused unless `again`.
    pub fn import_pst(&mut self, path: &str, again: bool) -> CmdResult {
        let path = path.trim();
        if path.is_empty() {
            return Err(CmdError::Params("`path` is required".into()));
        }
        if path.len() > PATH_MAX {
            return Err(CmdError::Params("`path` is too long".into()));
        }
        let key = self.facts_for(path).map(|f| f.record_key.clone()).unwrap_or_default();
        if !again && let Some(a) = self.duplicate_of(path, &key, None) {
            let name = self.account(&a).map(|x| x.config.name.clone()).unwrap_or_default();
            return Err(CmdError::Params(format!("this data file is already open as {name} ({a}); pass again: true to import another copy")));
        }
        let file = file_name(path);
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
        self.queue_import(&id, path, false);
        Ok(serde_json::json!({ "account": id, "importing": true }))
    }

    /// Bring an existing data file's mail into a new account before it first connects (Outlook's
    /// "use an existing data file" for POP accounts).
    pub fn attach_data_file(&mut self, account: &str, path: &str) -> CmdResult {
        let path = path.trim();
        if path.is_empty() {
            return Err(CmdError::Params("`path` is required".into()));
        }
        if path.len() > PATH_MAX {
            return Err(CmdError::Params("`path` is too long".into()));
        }
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if self.importing.held(account) {
            return Err(CmdError::Failed("a data file is being imported into this account already".into()));
        }
        // Connecting waits for the file, so the two don't write the Inbox at once.
        self.hold_start(account);
        self.queue_import(account, path, false);
        Ok(serde_json::json!({ "account": account, "importing": true }))
    }

    /// Look at a data file before importing it: asked once (`again`: once more), answered when
    /// the network side has looked, with the account it's already open as, if any.
    pub fn inspect_data_file(&mut self, path: &str, again: bool) -> CmdResult {
        let path = path.trim();
        if path.is_empty() {
            return Err(CmdError::Params("`path` is required".into()));
        }
        if path.len() > PATH_MAX {
            return Err(CmdError::Params("`path` is too long".into()));
        }
        let known = self.importing.inspected.iter().position(|(p, _)| p == path);
        match known.and_then(|i| self.importing.inspected.get(i)).map(|(_, f)| f.clone()) {
            Some(Some(facts)) if !again => {
                let mut v = serde_json::to_value(&facts).map_err(|e| CmdError::Failed(e.to_string()))?;
                if let Some(a) = self.duplicate_of(path, &facts.record_key, None) {
                    let name = self.account(&a).map(|x| x.config.name.clone()).unwrap_or_default();
                    if let Some(o) = v.as_object_mut() {
                        o.insert("alreadyOpenAs".into(), serde_json::json!({ "account": a, "name": name }));
                    }
                }
                Ok(v)
            }
            Some(None) => Ok(serde_json::json!({ "pending": true })),
            _ => {
                if let Some(i) = known {
                    self.importing.inspected.remove(i);
                }
                self.note_inspected(path, None);
                self.jobs.push(Job::InspectDataFile { path: path.to_string() });
                Ok(serde_json::json!({ "pending": true }))
            }
        }
    }

    /// What the network side found of a data file looked at.
    pub(crate) fn data_file_inspected(&mut self, path: &str, facts: Box<DataFileFacts>) {
        match self.importing.inspected.iter_mut().find(|(p, _)| p == path) {
            Some((_, f)) => *f = Some(facts),
            None => self.note_inspected(path, Some(facts)),
        }
    }

    fn note_inspected(&mut self, path: &str, facts: Option<Box<DataFileFacts>>) {
        self.importing.inspected.push((path.to_string(), facts));
        let excess = self.importing.inspected.len().saturating_sub(INSPECTED_MAX);
        self.importing.inspected.drain(..excess);
    }

    /// How an import is going. Nothing is said: the window shows it. Paused (MailCraft closing),
    /// it's over for this run, and goes on from the record kept the next time MailCraft opens.
    /// Blocked, it waits on the user, then too.
    pub(crate) fn import_progress(&mut self, account: &str, p: &ImportProgress) {
        let now = self.ui_ms();
        // One the network side is reading that wasn't asked for here: followed all the same.
        if !self.importing.held(account)
            && let Some(file) = self.account(account).map(|a| a.config.email.clone())
        {
            self.importing.running.insert(account.to_string(), ImportState { file, live: true, ..ImportState::default() });
        }
        let Some(st) = self.importing.running.get_mut(account) else { return };
        // Being thrown away with its account: nothing more of it counts.
        if st.removing {
            return;
        }
        st.progress = p.clone();
        if matches!(p.phase, ImportPhase::Paused | ImportPhase::Blocked) {
            st.stopping = None;
            self.import_ended(account);
            if let Some(a) = self.account_mut(account) {
                a.status = SyncStatus::Offline;
            }
            if p.phase == ImportPhase::Blocked
                && let Some(row) = self.importing.sources.get_mut(account)
                && row.stage == ImportStage::Running
            {
                row.stage = ImportStage::Stopped;
                self.save_imports();
            }
            return;
        }
        st.eta.update(p, now);
        if st.stopping.is_some() {
            st.progress.phase = ImportPhase::Stopping;
        }
        let folder = p.folder.clone();
        self.import_started(account);
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Syncing { folder };
        }
    }

    /// An import's end (`done`). An error with a `report` of messages that came in (the last
    /// save failed) keeps the account and what came in. Cancelled, what came in is kept and the
    /// import waits to go on or be kept as it is.
    pub(crate) fn imported(
        &mut self,
        account: &str,
        messages: usize,
        skipped: usize,
        done: bool,
        error: Option<String>,
        report: Option<&ImportReport>,
    ) {
        if !done {
            // How far it got, from a worker that says so this way: nothing is said.
            if self.account(account).is_some() {
                self.import_started(account);
            }
            return;
        }
        self.import_ended(account);
        let Some(name) = self.account(account).map(|a| a.config.email.clone()) else { return };
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Offline;
        }
        // Mail, or a file of contacts, calendar or notes only.
        let came_in = report.is_some_and(|r| r.messages.saturating_add(r.contacts).saturating_add(r.events).saturating_add(r.notes) > 0);
        if self.importing.running.get(account).is_some_and(|s| s.removing) {
            // It ended (on its own, or stopped as asked before) as it was being thrown away: the
            // account goes now.
            self.finish_removal(account, Leaves::Nothing);
            return;
        }
        if report.is_some_and(|r| r.stopped == Some(StopHow::Cancel)) && (error.is_none() || came_in) {
            self.import_cancelled(account, &name, messages, report);
            return;
        }
        // An account that was waiting for its data file connects now.
        if let Some(start) = self.held.remove(account) {
            self.jobs.push(start);
            if let Some(e) = &error
                && !came_in
            {
                self.import_done(account, report);
                self.notice(format!("{name}: the data file could not be opened: {e}"), true);
                return;
            }
        }
        match error {
            Some(e) if !came_in => {
                // Nothing came in: the account goes again, and the file it was starting.
                let _ = self.drop_account(account, Leaves::Nothing);
                self.notice(format!("{name} could not be opened: {e}"), true);
            }
            error => {
                self.import_done(account, report);
                self.load_imported_pim(account);
                let more = if skipped > 0 { format!(" ({skipped} could not be read)") } else { String::new() };
                match error {
                    Some(e) => {
                        // The folder list came with the end and is held here: saved again, or
                        // the next start would find none and the mail out of reach.
                        let kept = match (&self.store, self.account(account)) {
                            (Some(store), Some(a)) if !a.folders.is_empty() => {
                                store.save_folders(account, &a.folders).inspect_err(|e| log::warn!("{account}: folders not saved: {e}")).is_ok()
                            }
                            _ => false,
                        };
                        let then = if kept { "" } else { " Import the file again to keep it." };
                        self.notice(format!("{name}: {messages} messages imported{more}, but the last of it could not be saved: {e}.{then}"), true);
                    }
                    None => {
                        self.notice(format!("{name}: {messages} messages imported{more}"), false);
                        // The window shows what it came to.
                        self.importing.finished.retain(|a| a != account);
                        self.importing.finished.push(account.to_string());
                        let excess = self.importing.finished.len().saturating_sub(FINISHED_MAX);
                        self.importing.finished.drain(..excess);
                    }
                }
                if let Some(inbox) = self.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()) {
                    self.open_view(View::Folder { account: account.to_string(), folder: inbox });
                }
            }
        }
    }

    /// The import is over: kept as done, with its report.
    fn import_done(&mut self, account: &str, report: Option<&ImportReport>) {
        self.importing.running.remove(account);
        if let Some(row) = self.importing.sources.get_mut(account) {
            row.stage = ImportStage::Done;
            row.report = report.cloned();
            self.save_imports();
        }
    }

    /// Stopped as asked: what came in stays, the folder it was part-way through too, and the
    /// import waits to go on or to be kept as it is (a POP account's connection with it).
    fn import_cancelled(&mut self, account: &str, name: &str, messages: usize, report: Option<&ImportReport>) {
        if let Some(st) = self.importing.running.get_mut(account) {
            st.stopping = None;
            st.progress.phase = ImportPhase::Paused;
        }
        if let Some(row) = self.importing.sources.get_mut(account) {
            row.stage = ImportStage::Stopped;
            row.report = report.cloned();
            self.save_imports();
        }
        self.load_imported_pim(account);
        self.notice(format!("{name}: importing stopped with {messages} messages imported; it can go on later"), false);
    }

    /// The data file's contacts and calendar, written next to its mail.
    fn load_imported_pim(&mut self, account: &str) {
        if let Some(store) = self.store.clone()
            && let Ok(mut p) = store.load_account_doc::<mailcraft_model::pim::Pim>(account, mailcraft_model::pim::PIM_DOC)
        {
            p.ensure_defaults();
            self.pim.insert(account.to_string(), p);
            self.pim_unread.remove(account);
            self.contacts = None;
            self.pim_revision = self.pim_revision.wrapping_add(1);
        }
    }

    /// An import thrown away has ended: its account, when it was being removed, goes now (its
    /// data file with it). Any other is an earlier account's that had the id, long gone.
    pub(crate) fn import_discarded(&mut self, account: &str, error: Option<String>) {
        if let Some(e) = error {
            log::warn!("{account}: an import thrown away: {e}");
        }
        if self.importing.running.get(account).is_some_and(|s| s.removing) {
            self.finish_removal(account, Leaves::Nothing);
        }
    }

    /// Remove an account whose data file is being read: the import is told to throw it away,
    /// and the account (with the part imported) goes once it has. `None`: not importing.
    pub(super) fn remove_while_importing(&mut self, account: &str) -> Option<CmdResult> {
        let st = self.importing.running.get_mut(account).filter(|s| s.live || s.removing)?;
        if !st.removing {
            st.removing = true;
            st.waited = 0;
            st.stopping = Some(StopHow::Discard);
            st.progress.phase = ImportPhase::Stopping;
            let path = st.path.clone();
            self.importing.sources.entry(account.to_string()).or_insert_with(|| DataSource { path, ..DataSource::default() }).stage =
                ImportStage::Removing;
            self.save_imports();
            self.jobs.push(Job::StopImport { account: account.to_string(), how: StopHow::Discard });
        }
        Some(Ok(serde_json::json!({ "removing": account, "dataFile": null })))
    }

    /// The account being removed goes, with what its import wrote (`leaves`: or, the import
    /// still holding it, once the import has let go).
    fn finish_removal(&mut self, account: &str, leaves: Leaves) {
        self.importing.running.remove(account);
        if self.drop_account(account, leaves).is_err() {
            // Gone already: only its record is left.
            if self.importing.sources.remove(account).is_some() {
                self.save_imports();
            }
        }
    }

    /// Every event applied counts toward the removals waiting on their import: one that has
    /// waited [`REMOVE_BACKSTOP`] events goes from the list without it. Its data file, still the
    /// import's, isn't touched (on Windows it couldn't go, and would be forgotten): the import
    /// removes it when it ends, and its record, kept as being removed, has it removed the next
    /// time MailCraft opens otherwise.
    pub(crate) fn removal_backstop(&mut self) {
        let mut due = Vec::new();
        for (a, s) in self.importing.running.iter_mut().filter(|(_, s)| s.removing) {
            s.waited = s.waited.saturating_add(1);
            if s.waited >= REMOVE_BACKSTOP {
                due.push(a.clone());
            }
        }
        for a in due {
            log::warn!("{a}: its import didn't say it stopped; removed without it");
            self.finish_removal(&a, Leaves::ForImport);
        }
    }

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
        match self.importing.running.get(account) {
            None => Err(CmdError::Params("no import of this account is waiting".into())),
            Some(s) if s.removing => Err(CmdError::Failed("this account is being removed".into())),
            Some(s) if s.live => Err(CmdError::Failed("the data file is being imported".into())),
            Some(s) => Ok(s),
        }
    }

    /// Go on with an import stopped or interrupted, from where it stopped.
    pub fn resume_import(&mut self, account: &str) -> CmdResult {
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

    /// Keep what a stopped import brought in, as it is: it won't go on.
    pub fn keep_imported(&mut self, account: &str) -> CmdResult {
        self.stopped_import(account)?;
        self.importing.running.remove(account);
        if let Some(row) = self.importing.sources.get_mut(account) {
            row.stage = ImportStage::Done;
            self.save_imports();
        }
        if let Some(start) = self.held.remove(account) {
            self.jobs.push(start);
        }
        Ok(serde_json::json!({ "account": account, "kept": true }))
    }

    /// How the imports are going (`account`: that one's, running, waiting or done).
    pub fn import_status(&self, account: Option<&str>) -> CmdResult {
        let now = self.ui_ms();
        let one = |a: &str| {
            let st = self.importing.running.get(a);
            let row = self.importing.sources.get(a);
            serde_json::json!({
                "account": a,
                "file": st.map(|s| s.file.clone()).or_else(|| row.map(|r| file_name(&r.path))),
                "path": st.map(|s| s.path.clone()).or_else(|| row.map(|r| r.path.clone())),
                "live": st.is_some_and(|s| s.live),
                "removing": st.is_some_and(|s| s.removing),
                "stopping": st.and_then(|s| s.stopping),
                "phase": st.map(|s| s.progress.phase),
                "progress": st.map(|s| &s.progress),
                "permille": st.and_then(|s| s.progress.permille()),
                "etaSeconds": st.filter(|s| s.live).and_then(|s| s.eta.remaining_s(now)),
                "stage": row.map(|r| r.stage),
                "report": row.and_then(|r| r.report.as_ref()),
            })
        };
        match account {
            Some(a) if self.importing.running.contains_key(a) || self.importing.sources.contains_key(a) => Ok(one(a)),
            Some(a) => Err(CmdError::Params(format!("no data file was imported into `{a}`"))),
            None => Ok(serde_json::json!({ "imports": self.importing.running.keys().map(|a| one(a)).collect::<Vec<_>>() })),
        }
    }
}
