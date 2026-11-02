//! The session: mailbox settings on the server (automatic replies), and the account's data file
//! on this computer: how it's doing, and made smaller (`mailcraft_model::job::plan`).

use mailcraft_model::job::{CompactMode, CompactPhase, CompactReport, DataFileHealth, Work};

use super::*;

impl Session {
    /// Delete everything in a folder for good (only Deleted Items and Junk Email).
    pub fn empty_folder(&mut self, account: &str, folder: &str) -> CmdResult {
        let f = self.folder(account, folder).ok_or_else(|| CmdError::Params(format!("no folder `{folder}`")))?;
        let role = f.role;
        self.import_holds(account)?;
        if !matches!(role, FolderRole::Trash | FolderRole::Junk) {
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

/// How long MailCraft is left alone before a data file is looked at to be compacted.
pub const IDLE_AFTER_S: u64 = 600;
/// A data file is looked at again for the idle policy at most this often (one whose compressing
/// again was stopped part way goes on at the next idle, whatever this says).
const CHECK_EVERY_S: i64 = 24 * 3600;
/// Events held for an account while its data file is had to itself, at most: past it, they're
/// applied as they come (the window may wait for the file a moment).
const DEFERRED_MAX: usize = 4096;
/// Commands that never touch an account's data file, run while one is had to itself.
const UNTOUCHED: [&str; 6] = ["dataFile.", "view.", "app.", "options.", "file.", "folderPane."];

/// Data files made smaller: how each is doing, the compactions running and what the last came
/// to, and what waits for one that has its file to itself.
#[derive(Debug, Default)]
pub struct Compaction {
    /// By account: how its data file was doing when last measured.
    pub health: HashMap<String, DataFileHealth>,
    /// By account: the compaction running, as it last said.
    pub running: HashMap<String, Compacting>,
    /// By account: what its last compaction came to.
    pub last: HashMap<String, CompactReport>,
    /// When each account's data file was last measured for the idle policy (Unix seconds).
    checked: HashMap<String, i64>,
    /// Measured for the idle policy: compacted when it needs it.
    idle_asked: HashSet<String>,
    /// Events for accounts whose data file is had to itself, applied once it's given back.
    deferred: Vec<Event>,
}

/// A compaction running ([`Compaction::running`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compacting {
    pub mode: CompactMode,
    pub phase: CompactPhase,
    pub work: Work,
    /// About how long the part that has the file to itself takes.
    pub exclusive_s: Option<u64>,
}

impl Compaction {
    /// Whether `account`'s data file is had to itself now: nothing of it can be read or written.
    pub fn exclusive(&self, account: &str) -> bool {
        self.running.get(account).is_some_and(|c| c.phase == CompactPhase::Exclusive)
    }
}

/// The account an event is about, when it's one account's.
fn event_account(e: &Event) -> Option<&str> {
    match e {
        Event::Body { key, .. } | Event::BodyFailed { key, .. } | Event::Decrypted { key, .. } => Some(&key.account),
        Event::Folders { account, .. }
        | Event::FolderChanged { account, .. }
        | Event::Sent { account, .. }
        | Event::SendFailed { account, .. }
        | Event::Queued { account, .. }
        | Event::OpFailed { account, .. }
        | Event::DavSynced { account, .. }
        | Event::ImportProgress { account, .. }
        | Event::Imported { account, .. }
        | Event::Indexed { account, .. }
        | Event::AttachmentsKnown { account, .. }
        | Event::Exported { account, .. }
        | Event::ServerSearched { account, .. }
        | Event::CalendarFetched { account, .. } => Some(account),
        _ => None,
    }
}

impl Session {
    /// The account's data file measured ([`Job::DataFileHealth`]); what's known of it so far.
    pub fn data_file_health(&mut self, account: &str) -> CmdResult {
        self.account(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        self.jobs.push(Job::DataFileHealth { account: account.to_string() });
        Ok(self.compaction_json(account))
    }

    /// Compact the account's data file now (File › Info › Compact now), as long as that takes.
    pub fn compact_data_file(&mut self, account: &str) -> CmdResult {
        self.account(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if !self.store.as_ref().is_some_and(|s| s.data_file(account).is_some()) {
            return Err(CmdError::Failed("this account isn't kept in a data file on this computer".into()));
        }
        if self.compaction.running.contains_key(account) {
            return Err(CmdError::Failed("its data file is being compacted already".into()));
        }
        self.import_holds(account)?;
        self.start_compacting(account, CompactMode::Manual);
        Ok(self.compaction_json(account))
    }

    /// Stop compacting the account's data file, where that can be stopped.
    pub fn cancel_compact(&mut self, account: &str) -> CmdResult {
        if self.compaction.running.contains_key(account) {
            self.jobs.push(Job::CancelCompact { account: account.to_string() });
        }
        Ok(self.compaction_json(account))
    }

    fn start_compacting(&mut self, account: &str, mode: CompactMode) {
        let running = Compacting { mode, phase: CompactPhase::Measuring, work: Work::default(), exclusive_s: None };
        self.compaction.running.insert(account.to_string(), running);
        self.jobs.push(Job::CompactDataFile { account: account.to_string(), mode });
    }

    /// The idle policy: MailCraft left alone for `idle_s` seconds (the window says, from the time
    /// of the last thing the user did). Each account kept in a data file that isn't syncing,
    /// importing or compacting, with no message being written, is measured, at most once a day
    /// (one whose compressing again was stopped part way, at each idle); one that needs it is then
    /// compacted ([`Event::DataFileHealth`]). Returns the accounts measured.
    pub fn data_files_idle(&mut self, idle_s: u64) -> Vec<String> {
        let due = self.idle_due(idle_s);
        for a in &due {
            self.compaction.checked.insert(a.clone(), self.clock.now);
            self.compaction.idle_asked.insert(a.clone());
            self.jobs.push(Job::DataFileHealth { account: a.clone() });
        }
        due
    }

    /// The accounts the idle policy measures now ([`Session::data_files_idle`]).
    pub(crate) fn idle_due(&self, idle_s: u64) -> Vec<String> {
        if idle_s < IDLE_AFTER_S || self.compose.is_some() || !self.composes.is_empty() {
            return Vec::new();
        }
        let Some(store) = self.store.as_ref() else { return Vec::new() };
        self.accounts
            .iter()
            .filter(|a| !matches!(a.status, SyncStatus::Syncing { .. } | SyncStatus::Connecting))
            .map(|a| a.config.id.as_str())
            .filter(|a| store.data_file(a).is_some() && self.import_holds(a).is_ok() && !self.compaction.running.contains_key(*a))
            .filter(|a| {
                let since = self.compaction.checked.get(*a).map_or(i64::MAX, |t| self.clock.now.saturating_sub(*t));
                let stopped = self.compaction.health.get(*a).is_some_and(|h| h.recompress_stopped);
                since >= CHECK_EVERY_S || (stopped && since >= IDLE_AFTER_S as i64)
            })
            .map(str::to_string)
            .collect()
    }

    /// What's known of compacting the account's data file, for the window and agents.
    pub fn compaction_json(&self, account: &str) -> serde_json::Value {
        let c = &self.compaction;
        let running =
            c.running.get(account).map(|r| serde_json::json!({"mode": r.mode, "phase": r.phase, "work": r.work, "exclusiveS": r.exclusive_s}));
        serde_json::json!({
            "account": account,
            "health": c.health.get(account),
            "running": running,
            "last": c.last.get(account),
        })
    }

    /// Why a command can't run now because a data file it may touch is had to itself by a
    /// compaction: anything acting on that account (named, open, chosen, or on screen).
    pub(crate) fn compaction_blocks(&self, id: &str, params: &serde_json::Value) -> Option<String> {
        if !self.compaction.running.values().any(|c| c.phase == CompactPhase::Exclusive) || UNTOUCHED.iter().any(|p| id.starts_with(p)) {
            return None;
        }
        let named = params.get("account").and_then(serde_json::Value::as_str);
        let keyed = params.get("key").and_then(serde_json::Value::as_str).and_then(MessageKey::from_id).map(|k| k.account);
        let open = self.reading.as_ref().map(|r| r.key.account.as_str());
        let chosen = self.selection.iter().chain(self.focus.as_ref()).map(|k| k.account.as_str());
        let shown = self.current_account().map(|a| a.config.id.as_str());
        let touched = named.into_iter().chain(keyed.as_deref()).chain(open).chain(chosen).chain(shown).any(|a| self.compaction.exclusive(a));
        touched.then(|| "this data file is being compacted; it's ready in a moment".to_string())
    }

    /// Held, when it's for an account whose data file is had to itself (its status and notices
    /// aside): applied once it's given back.
    pub(crate) fn defer_for_compaction(&mut self, e: Event) -> Option<Event> {
        let held = event_account(&e).is_some_and(|a| self.compaction.exclusive(a));
        if !held || self.compaction.deferred.len() >= DEFERRED_MAX {
            return Some(e);
        }
        self.compaction.deferred.push(e);
        None
    }

    /// The account's data file measured: kept, and compacted when the idle policy asked and it
    /// needs it.
    pub(crate) fn health_measured(&mut self, account: &str, health: Option<DataFileHealth>, error: Option<String>) {
        let asked = self.compaction.idle_asked.remove(account);
        if let Some(e) = error {
            log::warn!("{account}: its data file couldn't be measured: {e}");
        }
        let Some(h) = health else {
            self.compaction.health.remove(account);
            return;
        };
        let wanted = h.needs_compacting() || h.recompress_stopped;
        self.compaction.health.insert(account.to_string(), h);
        if asked && wanted && !self.compaction.running.contains_key(account) && self.import_holds(account).is_ok() {
            self.start_compacting(account, CompactMode::Idle);
        }
    }

    pub(crate) fn compacting(&mut self, account: &str, phase: CompactPhase, work: Work, exclusive_s: Option<u64>) {
        if let Some(c) = self.compaction.running.get_mut(account) {
            (c.phase, c.work, c.exclusive_s) = (phase, work, exclusive_s);
        }
    }

    /// A compaction ended: what it came to kept (and said, when it was asked for), its file
    /// measured again, and the events held for it applied.
    pub(crate) fn compacted(&mut self, account: &str, report: CompactReport) {
        self.compaction.running.remove(account);
        if report.mode == CompactMode::Manual {
            let who = self.account(account).map(|a| a.config.email.clone()).unwrap_or_else(|| account.to_string());
            let mb = |b: u64| b >> 20;
            match (&report.error, report.reimport, report.cancelled) {
                (Some(e), _, _) => self.notice(format!("{who}: the data file wasn't compacted: {e}"), true),
                (None, true, _) => self.notice(
                    format!("{who}: the data file is too big to compact here, and there's no room for a copy; importing it again makes it as small"),
                    true,
                ),
                (None, false, true) => self.notice(format!("{who}: compacting was stopped"), false),
                (None, false, false) if report.after < report.before => {
                    self.notice(format!("{who}: compacted from {} MB to {} MB", mb(report.before), mb(report.after)), false)
                }
                (None, false, false) => self.notice(format!("{who}: the data file has nothing to give back"), false),
            }
        }
        if report.compacted || report.copied || report.recompressed > 0 {
            self.jobs.push(Job::DataFileHealth { account: account.to_string() });
        }
        self.compaction.last.insert(account.to_string(), report);
        // Applied now, in the order they came (another account's still had to itself are held
        // again).
        for e in std::mem::take(&mut self.compaction.deferred) {
            self.apply(e);
        }
    }
}
