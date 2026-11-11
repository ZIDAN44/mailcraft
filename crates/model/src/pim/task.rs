//! Tasks, as Outlook's Tasks keeps them (MS-OXOTASK): a title and notes, the days it starts and
//! is due, how far along it is, and when it was done. Synced with CalDAV servers that publish a
//! `VTODO` task list, and read and written in calendar files as `VTODO`s (RFC 5545 §3.6.2).

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

    /// Its due day has passed (by `today`, the viewer's local day as days since the epoch) and
    /// it isn't done.
    pub fn overdue(&self, today: i64) -> bool {
        !self.is_done() && self.due.is_some_and(|d| d.div_euclid(86_400) < today)
    }
}

/// A task with where it lives: `href` is its item on the server (a CalDAV `VTODO` resource),
/// empty for a task kept on this computer only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTask {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    pub task: Task,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
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
    }
}
