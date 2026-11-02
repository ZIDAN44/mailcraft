//! Tasks (Outlook's Tasks): each account's, kept with its calendar and contacts (`pim.json`) on
//! this computer only (owner's decision: no CalDAV or Microsoft To Do sync). A task with a due
//! day shows in the calendar that day, drawn from the task ([`Session::task_items`]): it isn't
//! copied into the events. Marking a repeating task done keeps a done copy and moves the task to
//! its next due day, as Outlook does.

use mailcraft_model::pim::{StoredTask, Task, task_day};
use mailcraft_model::search::{fold_all, folded_has_phrase};

use super::reminders::REMINDER_KEPT_AFTER_END;
use super::*;

/// Most tasks an account keeps, and most characters in a title and in the notes.
const MAX_TASKS: usize = 10_000;
const MAX_TITLE: usize = 1_000;
const MAX_NOTES: usize = 100_000;
/// How far ahead a repeating task's next due day is looked for.
const NEXT_WITHIN: i64 = 10 * 366 * 86_400;
/// The calendar a task is drawn in (it's no calendar's).
pub const TASKS_CALENDAR: &str = "tasks";

/// A task with the account it's in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRef {
    pub account: String,
    pub task: Task,
}

/// How tasks are listed (Outlook's Arrange by for tasks). Done ones go last whichever it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TasksOrder {
    /// The soonest due first; those with no due day after them.
    #[default]
    Due,
    /// The most important first, then by due day.
    Priority,
    /// A to Z.
    Title,
    /// The last changed first.
    Modified,
}

impl TasksOrder {
    pub const ALL: [TasksOrder; 4] = [TasksOrder::Due, TasksOrder::Priority, TasksOrder::Title, TasksOrder::Modified];

    pub fn code(self) -> &'static str {
        match self {
            TasksOrder::Due => "due",
            TasksOrder::Priority => "priority",
            TasksOrder::Title => "title",
            TasksOrder::Modified => "modified",
        }
    }

    pub fn from_code(s: &str) -> Option<TasksOrder> {
        TasksOrder::ALL.into_iter().find(|o| o.code().eq_ignore_ascii_case(s.trim()))
    }
}

/// Whether a task's words (title, notes, categories) hold every word of a search, as words'
/// starts, ignoring case and accents.
fn task_matches(t: &Task, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let notes: String = t.notes.chars().take(64 * 1024).collect();
    let text = fold_all(&format!("{} {} {}", t.title, notes, t.categories.join(" ")));
    words.iter().all(|w| folded_has_phrase(&text, std::slice::from_ref(w)))
}

fn check_task(t: &Task) -> Result<(), CmdError> {
    if t.title.trim().is_empty() {
        return Err(CmdError::Params("give the task a title".into()));
    }
    if let (Some(s), Some(d)) = (t.start, t.due)
        && d < s
    {
        return Err(CmdError::Params("the task is due before it starts".into()));
    }
    Ok(())
}

/// Its text held to what's kept, its days to whole days, its status and percent in step.
fn tidy(t: &mut Task, now: i64) {
    t.title = t.title.chars().take(MAX_TITLE).collect();
    t.notes = t.notes.chars().take(MAX_NOTES).collect();
    t.start = t.start.map(task_day);
    t.due = t.due.map(task_day);
    t.settle(now);
}

impl Session {
    /// Every task matching a search (in its title, notes and categories), in the order chosen;
    /// done ones last, and left out when the list hides them.
    pub fn tasks(&self, query: &str) -> Vec<TaskRef> {
        let words: Vec<String> = fold_all(query).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
        let hide_done = self.prefs.tasks_hide_done;
        let mut out: Vec<TaskRef> = self
            .pim
            .iter()
            .flat_map(|(a, p)| {
                p.tasks
                    .iter()
                    .filter(|t| !(hide_done && t.task.is_done()) && task_matches(&t.task, &words))
                    .map(|t| TaskRef { account: a.clone(), task: t.task.clone() })
            })
            .collect();
        let title = |r: &TaskRef| r.task.title.trim().to_lowercase();
        // No due day sorts after every due day.
        let due = |r: &TaskRef| r.task.due.unwrap_or(i64::MAX);
        let rank = |r: &TaskRef| std::cmp::Reverse(r.task.priority);
        match self.prefs.tasks_order {
            TasksOrder::Due => out.sort_by_cached_key(|r| (r.task.is_done(), due(r), rank(r), title(r), r.task.uid.clone())),
            TasksOrder::Priority => out.sort_by_cached_key(|r| (r.task.is_done(), rank(r), due(r), title(r), r.task.uid.clone())),
            TasksOrder::Title => out.sort_by_cached_key(|r| (r.task.is_done(), title(r), r.task.uid.clone())),
            TasksOrder::Modified => out.sort_by_cached_key(|r| (r.task.is_done(), std::cmp::Reverse(r.task.modified), r.task.uid.clone())),
        }
        out
    }

    /// The task with a uid, and its account.
    pub fn find_task(&self, uid: &str) -> Option<TaskRef> {
        self.pim.iter().find_map(|(a, p)| p.tasks.iter().find(|t| t.task.uid == uid).map(|t| TaskRef { account: a.clone(), task: t.task.clone() }))
    }

    fn task_account(&self, uid: &str) -> Result<String, CmdError> {
        self.find_task(uid).map(|r| r.account).ok_or_else(|| CmdError::Params(format!("no task `{uid}`")))
    }

    /// Add a task; returns its uid (a new one when it has none, or its own is taken).
    pub fn create_task(&mut self, account: Option<&str>, mut t: Task) -> CmdResult {
        let account = self.pim_account(account)?;
        check_task(&t)?;
        if t.uid.trim().is_empty() || self.find_task(&t.uid).is_some() {
            t.uid = self.new_uid();
        }
        let now = self.clock.now;
        tidy(&mut t, now);
        t.modified = now;
        let uid = t.uid.clone();
        let p = self.pim_mut(&account);
        if p.tasks.len() >= MAX_TASKS {
            return Err(CmdError::Params(format!("an account keeps at most {MAX_TASKS} tasks")));
        }
        p.tasks.push(StoredTask { task: t });
        self.after_pim_change(&account, false);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    /// Change a task.
    pub fn update_task(&mut self, uid: &str, change: impl FnOnce(&mut Task)) -> CmdResult {
        let account = self.task_account(uid)?;
        let now = self.clock.now;
        let p = self.pim_mut(&account);
        let Some(stored) = p.tasks.iter_mut().find(|t| t.task.uid == uid) else { return Err(CmdError::Params(format!("no task `{uid}`"))) };
        let before = stored.task.clone();
        let mut t = before.clone();
        change(&mut t);
        t.uid = before.uid.clone();
        check_task(&t)?;
        tidy(&mut t, now);
        if t == before {
            return Ok(serde_json::json!({"uid": uid, "changed": false}));
        }
        t.modified = now;
        stored.task = t;
        self.after_pim_change(&account, false);
        Ok(serde_json::json!({"uid": uid}))
    }

    /// Mark a task done or not. A repeating one marked done stays, moved to its next due day,
    /// and a done copy of this time is kept beside it (its uid in the answer's `done`).
    pub fn complete_task(&mut self, uid: &str, done: bool) -> CmdResult {
        let Some(r) = self.find_task(uid) else { return Err(CmdError::Params(format!("no task `{uid}`"))) };
        let now = self.clock.now;
        let next = match (&r.task.rrule, r.task.due.or(r.task.start)) {
            (Some(rule), Some(from)) if done && !r.task.is_done() => next_due(rule, from),
            _ => None,
        };
        let Some(next) = next else { return self.update_task(uid, |t| t.set_done(done, now)) };
        // This time, done.
        let mut copy = r.task.clone();
        copy.uid = String::new();
        copy.rrule = None;
        copy.reminder = None;
        copy.set_done(true, now);
        let copied = self.create_task(Some(&r.account), copy)?;
        // The task, from its next time on.
        let shift = next.saturating_sub(r.task.due.or(r.task.start).unwrap_or(next));
        self.update_task(uid, |t| {
            t.start = t.start.map(|s| s.saturating_add(shift));
            t.due = t.due.map(|d| d.saturating_add(shift));
            t.reminder = t.reminder.map(|d| d.saturating_add(shift));
            t.set_done(false, now);
            t.status = mailcraft_model::pim::TaskStatus::NotStarted;
            t.percent = 0;
        })?;
        Ok(serde_json::json!({"uid": uid, "done": copied.get("uid"), "due": next}))
    }

    pub fn delete_task(&mut self, uid: &str) -> CmdResult {
        let account = self.task_account(uid)?;
        self.pim_mut(&account).tasks.retain(|t| t.task.uid != uid);
        self.after_pim_change(&account, false);
        Ok(serde_json::json!({"deleted": uid}))
    }

    /// The tasks' reminders due at `now`: those not done whose reminder time has passed, that
    /// weren't dismissed and aren't snoozed past `now`, unless the day the task was due (or the
    /// reminder itself, when it has none) ended a day ago, as an event's reminder goes.
    pub fn task_reminders(&self, now: i64) -> Vec<DueReminder> {
        let mut out = Vec::new();
        for (account, p) in &self.pim {
            if self.account(account).is_none() {
                continue;
            }
            for t in p.tasks.iter().map(|t| &t.task).filter(|t| !t.is_done()) {
                let Some(at) = t.reminder else { continue };
                let key = format!("{account}|{}|{at}", t.uid);
                let due = match self.prefs.reminders.get(&key) {
                    Some(0) => continue,
                    Some(until) => *until,
                    None => at,
                };
                let (start, end) = match t.due {
                    Some(d) => (self.clock.utc(d), self.clock.utc(d).saturating_add(86_400)),
                    None => (at, at),
                };
                if due > now || end < now.saturating_sub(REMINDER_KEPT_AFTER_END) {
                    continue;
                }
                out.push(DueReminder {
                    key,
                    account: account.clone(),
                    uid: t.uid.clone(),
                    occurrence: at,
                    summary: t.title.clone(),
                    location: String::new(),
                    start,
                    end,
                    all_day: t.due.is_some(),
                    due,
                    task: true,
                });
            }
        }
        out
    }

    /// Tasks due in `[from, to)` as the calendar draws them: on their due day, all day, each
    /// in its account's colour (or its first category's), done ones as completed. They aren't
    /// events: `task` is set, and the uid is the task's.
    pub fn task_items(&self, from: i64, to: i64) -> Vec<AgendaItem> {
        let mut out = Vec::new();
        for (account, p) in &self.pim {
            let Some(acc) = self.account(account) else { continue };
            for t in p.tasks.iter().map(|t| &t.task) {
                let Some(due) = t.due else { continue };
                let start = self.clock.utc(due);
                if start >= to || start.saturating_add(86_400) <= from {
                    continue;
                }
                out.push(AgendaItem {
                    account: account.clone(),
                    calendar: TASKS_CALENDAR.into(),
                    uid: t.uid.clone(),
                    occurrence: due,
                    recurring: t.rrule.is_some(),
                    summary: t.title.clone(),
                    location: String::new(),
                    start,
                    end: start.saturating_add(86_400),
                    all_day: true,
                    color: ACCOUNT_COLOR.saturating_add(acc.config.color),
                    category: t.categories.first().map(|c| self.category_color_of(c)),
                    status: if t.is_done() { "COMPLETED".into() } else { String::new() },
                    needs_reply: false,
                    privacy: ical::Privacy::Public,
                    show_as: ical::ShowAs::Busy,
                    task: true,
                });
            }
        }
        // Each day's done ones after the rest.
        out.sort_by(|a, b| (a.start, !a.status.is_empty(), &a.summary).cmp(&(b.start, !b.status.is_empty(), &b.summary)));
        out
    }
}

/// The day a repeating task is next due after `from` (a day), by its rule.
fn next_due(rule: &str, from: i64) -> Option<i64> {
    let e = CalEvent {
        uid: "next".into(),
        start: from,
        end: from.saturating_add(86_400),
        all_day: true,
        rrule: Some(rule.to_string()),
        ..Default::default()
    };
    let after = from.saturating_add(86_400);
    ical::occurrences(&[e], after, after.saturating_add(NEXT_WITHIN)).into_iter().map(|o| o.start).find(|s| *s >= after)
}
