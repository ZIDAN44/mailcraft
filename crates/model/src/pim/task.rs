//! Tasks, as Outlook's Tasks keeps them (MS-OXOTASK): a title and notes, the days it starts and
//! is due, how far along it is, and when it was done. Kept on this computer; for an on-premises
//! Exchange account in its mailbox's Tasks folder too (issue #161), as Outlook keeps them; and on
//! a CalDAV server in its task lists (issue #201: Nextcloud, SOGo, Stalwart, Radicale), each a
//! calendar collection that takes `VTODO`s. No Microsoft To Do sync. They go in and out of
//! calendar files as `VTODO`s (RFC 5545 §3.6.2).

use serde::{Deserialize, Serialize};

/// Where a task is, as Outlook's Status has it (PidLidTaskStatus's values, in its order).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    #[default]
    NotStarted,
    InProgress,
    Completed,
    /// Waiting on someone else.
    Waiting,
    Deferred,
}

impl TaskStatus {
    pub const ALL: [TaskStatus; 5] =
        [TaskStatus::NotStarted, TaskStatus::InProgress, TaskStatus::Completed, TaskStatus::Waiting, TaskStatus::Deferred];

    pub fn code(self) -> &'static str {
        match self {
            TaskStatus::NotStarted => "notStarted",
            TaskStatus::InProgress => "inProgress",
            TaskStatus::Completed => "completed",
            TaskStatus::Waiting => "waiting",
            TaskStatus::Deferred => "deferred",
        }
    }

    pub fn from_code(s: &str) -> Option<TaskStatus> {
        TaskStatus::ALL.into_iter().find(|t| t.code().eq_ignore_ascii_case(s.trim()))
    }

    /// Outlook's PidLidTaskStatus number (0 to 4).
    pub fn from_number(n: i64) -> Option<TaskStatus> {
        usize::try_from(n).ok().and_then(|i| TaskStatus::ALL.get(i).copied())
    }
}

/// How much a task matters (Outlook's Priority: PidTagImportance's low, normal, high).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskPriority {
    Low,
    #[default]
    Normal,
    High,
}

impl TaskPriority {
    pub fn code(self) -> &'static str {
        match self {
            TaskPriority::Low => "low",
            TaskPriority::Normal => "normal",
            TaskPriority::High => "high",
        }
    }

    pub fn from_code(s: &str) -> Option<TaskPriority> {
        [TaskPriority::Low, TaskPriority::Normal, TaskPriority::High].into_iter().find(|p| p.code().eq_ignore_ascii_case(s.trim()))
    }

    /// RFC 5545's `PRIORITY` (1 highest to 9 lowest; 0 or none: not said).
    pub fn from_ical(n: i64) -> TaskPriority {
        match n {
            1..=4 => TaskPriority::High,
            6..=9 => TaskPriority::Low,
            _ => TaskPriority::Normal,
        }
    }

    /// As `PRIORITY` writes it (`None`: normal, which needs no line).
    pub fn to_ical(self) -> Option<u8> {
        match self {
            TaskPriority::High => Some(1),
            TaskPriority::Normal => None,
            TaskPriority::Low => Some(9),
        }
    }
}

/// The most characters of a task's notes kept, however it came (made here, from Outlook or a
/// calendar file).
pub const TASK_NOTES_KEPT: usize = 100_000;

/// A task. Its start and due days are dates, kept as UTC midnight of the day (as all-day events
/// keep theirs); when it was done and its reminder are times.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub uid: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<i64>,
    /// When it was marked complete (`None` while it isn't).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<i64>,
    #[serde(default)]
    pub status: TaskStatus,
    /// How far along it is, 0 to 100.
    #[serde(default)]
    pub percent: u8,
    #[serde(default)]
    pub priority: TaskPriority,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// When it reminds (Unix seconds), as Outlook's task reminders are a day and a time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder: Option<i64>,
    /// How it repeats (an RFC 5545 `RRULE`, as written: `FREQ=WEEKLY;BYDAY=MO`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrule: Option<String>,
    /// When it was last changed (Unix seconds).
    #[serde(default)]
    pub modified: i64,
    /// Properties MailCraft doesn't read, kept as written (unfolded content lines): its files
    /// (`ATTACH`) among them, as an event keeps its own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<String>,
    /// What the to-do it was read from said that MailCraft keeps less of (issue #201): written
    /// back as it was while what MailCraft read of it is unchanged, so a task changed here
    /// doesn't lose its times of day, its other alarms or a status Outlook has no word for on the
    /// server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Box<TaskSource>>,
}

/// What a to-do said beyond what a task keeps ([`Task::source`]), each with what MailCraft read
/// of it, to tell whether it changed since.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSource {
    /// `DTSTART` and `DUE` with a time of day: the day kept, and the time it was (UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<(i64, i64)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<(i64, i64)>,
    /// Its `STATUS` line as written, and the status read from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<(TaskStatus, String)>,
    /// Its alarms as written (unfolded lines, `BEGIN:VALARM` to `END:VALARM`), and the reminder
    /// read from them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarms: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder: Option<i64>,
}

impl Task {
    /// Whether it's done.
    pub fn is_done(&self) -> bool {
        self.status == TaskStatus::Completed
    }

    /// Mark it done (at `at`) or not: its status, how far along and when it was done go together,
    /// as Outlook keeps them (a task taken back up is not started, at 0%).
    pub fn set_done(&mut self, done: bool, at: i64) {
        if done {
            self.status = TaskStatus::Completed;
            self.percent = 100;
            // A time not known (0) isn't one.
            self.completed = self.completed.or(Some(at).filter(|t| *t > 0));
        } else if self.is_done() {
            self.status = TaskStatus::NotStarted;
            self.percent = 0;
            self.completed = None;
        }
    }

    /// Keep its status and how far along it is in step: 100% is complete, and complete is 100%;
    /// a task begun is in progress.
    pub fn settle(&mut self, at: i64) {
        self.percent = self.percent.min(100);
        if self.status == TaskStatus::Completed || self.percent == 100 {
            self.set_done(true, at);
            return;
        }
        self.completed = None;
        if self.status == TaskStatus::NotStarted && self.percent > 0 {
            self.status = TaskStatus::InProgress;
        }
    }

    /// The day it's due (days since the epoch), when it has one: the same day in every time
    /// zone, as it's kept at UTC midnight.
    pub fn due_day(&self) -> Option<i64> {
        self.due.map(|d| d.div_euclid(86_400))
    }

    /// Its due day has passed (by `today`, the viewer's local day as days since the epoch) and
    /// it isn't done.
    pub fn overdue(&self, today: i64) -> bool {
        !self.is_done() && self.due_day().is_some_and(|d| d < today)
    }
}

/// A task with what's needed to keep it: where it is on the server (an Exchange account's
/// Tasks folder: its item and change key; a CalDAV task list: its list, its item's address and
/// etag; empty for one kept here only), and whether it changed here since, to send.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTask {
    pub task: Task,
    /// The CalDAV task list it's in ([`TaskList::id`]); empty for one kept here, or in an
    /// Exchange account's Tasks folder.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub list: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
    /// Its item on the server holds more than this task: occurrences of it changed on their own
    /// (`RECURRENCE-ID`), which MailCraft doesn't keep. Shown, not changed here, so they aren't
    /// lost when it's written back.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
}

/// A task list on a CalDAV server (issue #201): a calendar collection that takes to-dos
/// (`VTODO` in its `supported-calendar-component-set`). One that takes events too (SOGo's,
/// Stalwart's, Radicale's calendars) is also one of the account's calendars; Nextcloud's are
/// lists of tasks alone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskList {
    /// The collection's address on the server.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    /// The collection's version at the last sync (`getctag` / sync token).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ctag: String,
    /// It takes to-dos alone, not events: a list made for tasks (Nextcloud's Tasks), where new
    /// tasks go before a calendar that takes both.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tasks_only: bool,
    /// Someone else's, listed in the user's home (as a calendar can be).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub foreign: bool,
}

/// The day (UTC midnight) a time stored for a date falls on: the nearest midnight, as Outlook
/// stores a task's dates at the local midnight of their day (so within half a day of it).
pub fn task_day(t: i64) -> i64 {
    t.saturating_add(43_200).div_euclid(86_400).saturating_mul(86_400)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Done and not done carry status, percent and the time it was done together; a task begun
    /// is in progress; 100% is done.
    #[test]
    fn status_percent_and_completion_go_together() {
        let mut t = Task { uid: "t".into(), title: "Plan".into(), percent: 40, ..Default::default() };
        t.settle(100);
        assert_eq!((t.status, t.completed), (TaskStatus::InProgress, None));
        t.set_done(true, 200);
        assert_eq!((t.status, t.percent, t.completed), (TaskStatus::Completed, 100, Some(200)));
        t.set_done(true, 300);
        assert_eq!(t.completed, Some(200), "done once: the first time stays");
        t.set_done(false, 400);
        assert_eq!((t.status, t.percent, t.completed), (TaskStatus::NotStarted, 0, None));
        t.percent = 250;
        t.settle(500);
        assert_eq!((t.status, t.percent, t.completed), (TaskStatus::Completed, 100, Some(500)));
        let waiting = Task { status: TaskStatus::Waiting, percent: 10, completed: Some(1), ..Default::default() };
        let mut w = waiting.clone();
        w.settle(1);
        assert_eq!((w.status, w.completed), (TaskStatus::Waiting, None));
    }

    #[test]
    fn codes_numbers_and_days() {
        for s in TaskStatus::ALL {
            assert_eq!(TaskStatus::from_code(s.code()), Some(s));
        }
        assert_eq!(TaskStatus::from_number(3), Some(TaskStatus::Waiting));
        assert_eq!(TaskStatus::from_number(5), None);
        assert_eq!(TaskStatus::from_number(-1), None);
        assert_eq!(TaskPriority::from_ical(1), TaskPriority::High);
        assert_eq!(TaskPriority::from_ical(0), TaskPriority::Normal);
        assert_eq!(TaskPriority::from_ical(9), TaskPriority::Low);
        assert_eq!(task_day(86_400 * 10 - 3600), 86_400 * 10, "local midnight east of UTC");
        assert_eq!(task_day(86_400 * 10 + 5 * 3600), 86_400 * 10, "west of UTC");
        assert_eq!(task_day(i64::MAX), task_day(i64::MAX));
        let t = Task { due: Some(86_400 * 10), ..Default::default() };
        assert!(t.overdue(11) && !t.overdue(10));
        // Old documents (no tasks) still load.
        let p: crate::pim::Pim = serde_json::from_str(r#"{"events":[]}"#).unwrap();
        assert!(p.tasks.is_empty());
        let s: Task = serde_json::from_str(r#"{"uid":"a"}"#).unwrap();
        assert_eq!((s.status, s.priority, s.percent), (TaskStatus::NotStarted, TaskPriority::Normal, 0));
        // Tasks kept before task lists: in none, and accounts with no lists (issue #201).
        let st: StoredTask = serde_json::from_str(r#"{"task":{"uid":"a"},"href":"task:1"}"#).unwrap();
        assert_eq!((st.list.as_str(), st.href.as_str()), ("", "task:1"));
        assert!(p.task_lists.is_empty());
    }

    /// New tasks go to a list made for tasks before a calendar that takes them too, never to a
    /// read-only list or someone else's.
    #[test]
    fn the_list_new_tasks_go_to() {
        let list = |id: &str, tasks_only: bool, read_only: bool, foreign: bool| TaskList {
            id: id.into(),
            name: id.into(),
            tasks_only,
            read_only,
            foreign,
            ..Default::default()
        };
        let mut p = crate::pim::Pim::default();
        assert_eq!(p.default_task_list(), None);
        p.task_lists = vec![list("theirs", true, false, true), list("cal", false, false, false), list("ro", true, true, false)];
        assert_eq!(p.default_task_list().map(|l| l.id.as_str()), Some("cal"));
        p.task_lists.push(list("tasks", true, false, false));
        assert_eq!(p.default_task_list().map(|l| l.id.as_str()), Some("tasks"));
        p.task_lists = vec![list("ro", true, true, false)];
        assert_eq!(p.default_task_list(), None);
    }
}
