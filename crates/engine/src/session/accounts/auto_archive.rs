//! AutoArchive (issue #180), as Outlook's: every so many days (asking first, when set to), each
//! folder's mail older than its age is archived to a data file, or deleted for good, by the
//! folder's own settings or the default ones; a folder can be left alone. It runs as Clean up old
//! items does (`archiving.rs`), one folder group after another.
//!
//! Left out of Outlook's: a message's own "Do not AutoArchive" (no server keeps it, and MailCraft
//! has no per-message setting of its own), and deleting expired items (mail carries no expiry
//! MailCraft keeps).

use std::collections::VecDeque;

use mailcraft_model::dates::{Clock, civil_from_days, days_from_civil, month_len};

use super::archiving::ArchiveTask;
use super::*;

/// How long a day is, in seconds.
const DAY: i64 = 86_400;
/// The most days between two runs that can be set.
const EVERY_MAX: u32 = 60;
/// The most an age can be, in any unit.
const AGE_MAX: u32 = 999;

/// The unit a folder's age is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgeUnit {
    Days,
    Weeks,
    #[default]
    Months,
}

impl AgeUnit {
    pub const ALL: [AgeUnit; 3] = [AgeUnit::Days, AgeUnit::Weeks, AgeUnit::Months];

    /// As the commands take it.
    pub fn code(self) -> &'static str {
        match self {
            AgeUnit::Days => "days",
            AgeUnit::Weeks => "weeks",
            AgeUnit::Months => "months",
        }
    }

    pub fn from_code(code: &str) -> Option<AgeUnit> {
        AgeUnit::ALL.into_iter().find(|u| u.code() == code)
    }
}

/// What's done with a folder's old mail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArchiveRule {
    /// Mail older than this many of `unit` is old.
    pub older_than: u32,
    pub unit: AgeUnit,
    /// Deleted for good, not archived.
    pub delete: bool,
    /// The data file it's archived to.
    pub path: String,
}

impl Default for ArchiveRule {
    /// Mail older than six months, archived (to a file chosen in the settings), as Outlook's.
    fn default() -> ArchiveRule {
        ArchiveRule { older_than: 6, unit: AgeUnit::Months, delete: false, path: String::new() }
    }
}

impl ArchiveRule {
    /// When mail older than this was received: the start of the day that many days, weeks or
    /// months back from today (the same day of the month, or its last).
    pub fn before(&self, clock: &Clock) -> i64 {
        let today = clock.local_day(clock.now);
        let n = i64::from(self.older_than.min(AGE_MAX));
        let day = match self.unit {
            AgeUnit::Days => today.saturating_sub(n),
            AgeUnit::Weeks => today.saturating_sub(n.saturating_mul(7)),
            AgeUnit::Months => {
                let (y, m, d) = civil_from_days(today);
                let month = y.saturating_mul(12).saturating_add(i64::from(m)).saturating_sub(1).saturating_sub(n);
                let (y, m) = (month.div_euclid(12), u32::try_from(month.rem_euclid(12) + 1).unwrap_or(1));
                days_from_civil(y, m, d.min(u32::try_from(month_len(y, m)).unwrap_or(28)))
            }
        };
        clock.midnight(day)
    }
}

/// A folder's own AutoArchive setting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderArchive {
    pub account: String,
    pub folder: String,
    /// Its own; none: never archived.
    #[serde(default)]
    pub rule: Option<ArchiveRule>,
}

/// AutoArchive's settings (File › Options › Data files › AutoArchive settings).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AutoArchive {
    /// Runs every so many days; 0: never (off, as at first: it takes mail out of the mailbox,
    /// and the file to put it in is chosen first).
    pub every_days: u32,
    /// Asks before it runs.
    pub prompt: bool,
    /// The file shown in the folder list once mail's archived to it.
    pub show: bool,
    /// The folders without settings of their own.
    pub rule: ArchiveRule,
    /// Folders with settings of their own.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<FolderArchive>,
    /// When it last ran, or was put off (Unix seconds); counted from when it was turned on.
    pub last_run: i64,
}

impl Default for AutoArchive {
    fn default() -> AutoArchive {
        AutoArchive { every_days: 0, prompt: true, show: true, rule: ArchiveRule::default(), folders: Vec::new(), last_run: 0 }
    }
}

impl AutoArchive {
    /// A folder's own setting: none when it follows the default ones; `Some(None)` when it's
    /// never archived.
    pub fn folder(&self, account: &str, folder: &str) -> Option<Option<&ArchiveRule>> {
        self.folders.iter().find(|f| f.account == account && f.folder == folder).map(|f| f.rule.as_ref())
    }
}

/// AutoArchive running: what's still to archive, and whether it's waiting to be asked or for
/// the answer.
#[derive(Debug, Default)]
pub struct AutoArchiving {
    queue: VecDeque<ArchiveTask>,
    /// It's due and asks first: the window is to ask.
    ask: bool,
    /// Asked (when, Unix seconds): waiting for the answer. One put away unanswered is asked
    /// again a day on.
    asked: Option<i64>,
    /// The archive running is one of its groups.
    started: bool,
}

impl Session {
    /// Whether AutoArchive is due and asks first: true once (the window then asks, and
    /// `autoArchive.run` or `autoArchive.later` answers).
    pub fn take_auto_archive_ask(&mut self) -> bool {
        let ask = std::mem::take(&mut self.auto_archiving.ask);
        if ask {
            self.auto_archiving.asked = Some(self.clock.now);
        }
        ask
    }

    /// Whether AutoArchive takes `account`'s old mail: its mail is kept here, and isn't an
    /// Outlook data file's opened here (whose mail is in a file already).
    pub fn auto_archives(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| !a.config.keeps_online()) && self.data_source(account).is_none()
    }

    /// Whether AutoArchive is archiving, or has more to.
    pub fn auto_archive_running(&self) -> bool {
        !self.auto_archiving.queue.is_empty() || (self.archiving.is_some() && self.auto_archiving.started)
    }

    /// AutoArchive due (each tick): run, or asked about.
    pub(in crate::session) fn auto_archive_due(&mut self) {
        let every = self.prefs.auto_archive.every_days;
        let now = self.clock.now;
        if self.auto_archiving.asked.is_some_and(|at| now.saturating_sub(at) > DAY) {
            self.auto_archiving.asked = None;
        }
        if every == 0
            || self.auto_archiving.ask
            || self.auto_archiving.asked.is_some()
            || !self.auto_archiving.queue.is_empty()
            || self.archiving.is_some()
        {
            return;
        }
        let last = &mut self.prefs.auto_archive.last_run;
        // Counted from when it was turned on (a copy from before has none).
        if *last <= 0 {
            *last = now;
            return;
        }
        if now < last.saturating_add(i64::from(every).saturating_mul(DAY)) {
            return;
        }
        if self.prefs.auto_archive.prompt {
            self.auto_archiving.ask = true;
        } else {
            self.auto_archive_now();
        }
    }

    /// Run AutoArchive now: each folder's old mail by its settings, a group at a time. How many
    /// groups there are.
    pub fn auto_archive_now(&mut self) -> usize {
        self.prefs.auto_archive.last_run = self.clock.now;
        self.auto_archiving.ask = false;
        self.auto_archiving.asked = None;
        let tasks = self.auto_archive_tasks();
        let n = tasks.len();
        self.auto_archiving.queue.extend(tasks);
        if n > 0 && self.archiving.is_none() {
            self.next_auto_archive();
        }
        n
    }

    /// Asked, and not now: due again after as many days.
    pub fn auto_archive_later(&mut self) {
        self.prefs.auto_archive.last_run = self.clock.now;
        self.auto_archiving.ask = false;
        self.auto_archiving.asked = None;
    }

    /// What AutoArchive would archive now: for each account whose mail is kept here (not an
    /// Outlook data file opened here, whose mail is in a file already), each folder's group by
    /// its settings, those never archived left out. A group archived to a file with none chosen
    /// is said, and left out.
    fn auto_archive_tasks(&mut self) -> Vec<ArchiveTask> {
        let settings = self.prefs.auto_archive.clone();
        let mut tasks: Vec<ArchiveTask> = Vec::new();
        let mut unset = false;
        for a in &self.accounts {
            let id = a.config.id.as_str();
            if !self.auto_archives(id) {
                continue;
            }
            for f in a.folders.iter().filter(|f| f.selectable && f.path != OUTBOX) {
                let rule = match settings.folder(id, &f.path) {
                    Some(None) => continue,
                    Some(Some(own)) => own,
                    None => &settings.rule,
                };
                if !rule.delete && rule.path.trim().is_empty() {
                    unset = true;
                    continue;
                }
                let (before, path) = (rule.before(&self.clock), if rule.delete { String::new() } else { rule.path.trim().to_string() });
                match tasks.iter_mut().find(|t| t.account == id && t.before == before && t.delete == rule.delete && t.path == path) {
                    Some(t) => t.folders.push(f.path.clone()),
                    None => tasks.push(ArchiveTask {
                        account: id.to_string(),
                        folders: vec![f.path.clone()],
                        only: true,
                        before,
                        path,
                        delete: rule.delete,
                        show: settings.show,
                    }),
                }
            }
        }
        if unset {
            self.notice("AutoArchive: choose the file to archive to in its settings (File › Options › Data files).".to_string(), true);
        }
        tasks
    }

    /// AutoArchive's next group, when nothing is being archived; one that can't be is said, and
    /// the next tried.
    pub(in crate::session) fn next_auto_archive(&mut self) {
        while self.archiving.is_none() {
            let Some(task) = self.auto_archiving.queue.pop_front() else {
                self.auto_archiving.started = false;
                return;
            };
            self.auto_archiving.started = true;
            let who = self.account(&task.account).map(|a| a.config.email.clone()).unwrap_or_default();
            if let Err(e) = self.start_archive(task) {
                self.notice(format!("AutoArchive: {who}: {e}"), true);
            }
        }
    }

    /// An account gone: AutoArchive has nothing more of it to do.
    pub(in crate::session) fn auto_archive_forget(&mut self, account: &str) {
        self.auto_archiving.queue.retain(|t| t.account != account);
    }

    /// Change AutoArchive's settings (none given: as they are).
    pub(crate) fn set_auto_archive(&mut self, every: Option<u32>, prompt: Option<bool>, show: Option<bool>, rule: RuleChange) -> CmdResult {
        let s = &mut self.prefs.auto_archive;
        if let Some(n) = every {
            let n = n.min(EVERY_MAX);
            // Turned on: counted from now.
            if s.every_days == 0 && n > 0 {
                s.last_run = self.clock.now;
            }
            s.every_days = n;
        }
        if let Some(p) = prompt {
            s.prompt = p;
        }
        if let Some(v) = show {
            s.show = v;
        }
        rule.apply(&mut s.rule)?;
        Ok(serde_json::to_value(&self.prefs.auto_archive).unwrap_or_default())
    }

    /// A folder's own AutoArchive setting: `mode` "off" (never archived), "default" (the default
    /// settings) or "own" (`rule` changed from the default ones, or from its own).
    pub(crate) fn set_folder_archive(&mut self, account: &str, folder: &str, mode: &str, rule: RuleChange) -> CmdResult {
        if self.folder(account, folder).is_none() {
            return Err(CmdError::Params(format!("no folder `{folder}`")));
        }
        let s = &mut self.prefs.auto_archive;
        let was = s.folder(account, folder).flatten().cloned().unwrap_or_else(|| s.rule.clone());
        s.folders.retain(|f| !(f.account == account && f.folder == folder));
        let own = match mode {
            "default" => return Ok(serde_json::json!({ "folder": folder, "archive": "default" })),
            "off" => None,
            "own" => {
                let mut own = was;
                rule.apply(&mut own)?;
                Some(own)
            }
            m => return Err(CmdError::Params(format!("`archive` is \"off\", \"default\" or \"own\", not `{m}`"))),
        };
        s.folders.push(FolderArchive { account: account.to_string(), folder: folder.to_string(), rule: own });
        Ok(serde_json::json!({ "folder": folder, "archive": mode }))
    }

    /// Outlook's "Apply these settings to all folders now": every folder follows the default
    /// settings.
    pub(crate) fn auto_archive_to_all(&mut self) -> CmdResult {
        let n = self.prefs.auto_archive.folders.len();
        self.prefs.auto_archive.folders.clear();
        Ok(serde_json::json!({ "cleared": n }))
    }
}

/// A change to a folder's age and what's done with its old mail, as the commands give it.
#[derive(Debug, Default)]
pub(crate) struct RuleChange {
    pub(crate) older_than: Option<u64>,
    pub(crate) unit: Option<String>,
    pub(crate) delete: Option<bool>,
    pub(crate) path: Option<String>,
}

impl RuleChange {
    fn apply(self, rule: &mut ArchiveRule) -> Result<(), CmdError> {
        if let Some(n) = self.older_than {
            rule.older_than = u32::try_from(n.clamp(1, u64::from(AGE_MAX))).unwrap_or(AGE_MAX);
        }
        if let Some(u) = self.unit {
            rule.unit = AgeUnit::from_code(&u).ok_or_else(|| CmdError::Params(format!("`unit` is \"days\", \"weeks\" or \"months\", not `{u}`")))?;
        }
        if let Some(d) = self.delete {
            rule.delete = d;
        }
        if let Some(p) = self.path {
            let p = p.trim();
            if !p.is_empty() && !p.to_ascii_lowercase().ends_with(".pst") {
                return Err(CmdError::Params("the archive is an Outlook data file: its name ends in .pst".into()));
            }
            rule.path = p.to_string();
        }
        Ok(())
    }
}
