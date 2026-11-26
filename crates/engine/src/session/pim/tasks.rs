//! Tasks (Outlook's Tasks): each account's, kept with its calendar and contacts (`pim.json`),
//! and on its server where that keeps them: an on-premises Exchange account's Tasks folder
//! (issue #161), a CalDAV server's task lists (issue #201). No Microsoft To Do sync. A task with
//! a due day shows in the calendar that day, drawn from the task ([`Session::task_items`]): it
//! isn't copied into the events. Marking a repeating task done keeps a done copy and moves the
//! task to its next due day, as Outlook does.

use mailcraft_model::pim::{StoredTask, TASK_NOTES_KEPT, Task, task_day};
use mailcraft_model::search::{fold_all, folded_has_phrase};

use super::reminders::{REMINDER_KEPT_AFTER_END, ReminderKey};
use super::*;

/// Most tasks an account keeps, and most characters in a title and in the notes.
const MAX_TASKS: usize = 10_000;
const MAX_TITLE: usize = 1_000;
const MAX_NOTES: usize = TASK_NOTES_KEPT;
/// How far ahead a repeating task's next due day is looked for.
const NEXT_WITHIN: i64 = 10 * 366 * 86_400;
/// The calendar a task is drawn in (it's no calendar's).
pub const TASKS_CALENDAR: &str = "tasks";

/// A task with the account it's in, and its CalDAV task list (empty: none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRef {
    pub account: String,
    pub list: String,
    pub task: Task,
}

/// A task in the list, with the account it's in: borrowed, as the list is drawn every frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListedTask<'a> {
    pub account: &'a str,
    /// Its CalDAV task list (empty: none).
    pub list: &'a str,
    pub task: &'a Task,
    /// Its place among its account's tasks (`Pim::tasks`), to find it again without a search.
    pub at: usize,
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
    let has_all = |text: &str| words.iter().all(|w| folded_has_phrase(text, std::slice::from_ref(w)));
    // Its title and categories first: the notes, longer, only when those don't hold every word.
    let short = fold_all(&format!("{} {}", t.title, t.categories.join(" ")));
    if has_all(&short) {
        return true;
    }
    let notes: String = t.notes.chars().take(64 * 1024).collect();
    has_all(&format!("{short} {}", fold_all(&notes)))
}

/// A task as given: a title, unless `before` (the task as it was) had none either, as one
/// brought in from Outlook or a calendar file may not; due no earlier than it starts.
fn check_task(t: &Task, before: Option<&Task>) -> Result<(), CmdError> {
    if t.title.trim().is_empty() && before.is_none_or(|b| !b.title.trim().is_empty()) {
        return Err(CmdError::Params("give the task a title".into()));
    }
    if let (Some(s), Some(d)) = (t.start, t.due)
        && d < s
    {
        return Err(CmdError::Params("the task is due before it starts".into()));
    }
    Ok(())
}

/// A rule with `COUNT` counted down by the time just done (`None`: that was its last), so a
/// series that ends after so many times ends; any other rule as it is.
fn count_down(rule: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for part in rule.split(';') {
        match part.split_once('=') {
            Some((k, n)) if k.trim().eq_ignore_ascii_case("COUNT") => {
                let left = n.trim().parse::<u64>().unwrap_or(0).saturating_sub(1);
                if left == 0 {
                    return None;
                }
                parts.push(format!("COUNT={left}"));
            }
            _ => parts.push(part.to_string()),
        }
    }
    Some(parts.join(";"))
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
    pub fn tasks(&self, query: &str) -> Vec<ListedTask<'_>> {
        let mut out = self.tasks_matching(query);
        let title = |r: &ListedTask<'_>| r.task.title.trim().to_lowercase();
        // No due day sorts after every due day.
        let due = |r: &ListedTask<'_>| r.task.due.unwrap_or(i64::MAX);
        let rank = |r: &ListedTask<'_>| std::cmp::Reverse(r.task.priority);
        // The uid borrowed (it was copied for every task, every frame).
        match self.prefs.tasks_order {
            TasksOrder::Due => out.sort_by_cached_key(|r| (r.task.is_done(), due(r), rank(r), title(r), r.task.uid.as_str())),
            TasksOrder::Priority => out.sort_by_cached_key(|r| (r.task.is_done(), rank(r), due(r), title(r), r.task.uid.as_str())),
            TasksOrder::Title => out.sort_by_cached_key(|r| (r.task.is_done(), title(r), r.task.uid.as_str())),
            TasksOrder::Modified => out.sort_by_key(|r| (r.task.is_done(), std::cmp::Reverse(r.task.modified), r.task.uid.as_str())),
        }
        out
    }

    /// How many tasks [`Session::tasks`] lists, and how many of them are overdue `today`: the
    /// side pane's counts, each frame, without putting them in order.
    pub fn task_counts(&self, query: &str, today: i64) -> (usize, usize) {
        let list = self.tasks_matching(query);
        (list.len(), list.iter().filter(|r| r.task.overdue(today)).count())
    }

    /// The task list shown alone (issue #201): the one chosen, while an account has it.
    pub fn shown_task_list(&self) -> Option<&str> {
        let l = self.prefs.tasks_list.as_deref()?;
        self.pim.values().any(|p| p.task_lists.iter().any(|x| x.id == l)).then_some(l)
    }

    /// The tasks a search finds, done ones left out when the list hides them, in no order; of
    /// the task list shown alone, when one is.
    fn tasks_matching(&self, query: &str) -> Vec<ListedTask<'_>> {
        let words: Vec<String> = fold_all(query).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
        let hide_done = self.prefs.tasks_hide_done;
        let only = self.shown_task_list();
        self.pim
            .iter()
            .flat_map(|(a, p)| {
                p.tasks
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| only.is_none_or(|l| t.list == l))
                    .filter(|(_, t)| !(hide_done && t.task.is_done()) && task_matches(&t.task, &words))
                    .map(|(at, t)| ListedTask { account: a, list: &t.list, task: &t.task, at })
            })
            .collect()
    }

    /// The task with a uid, and its account.
    pub fn find_task(&self, uid: &str) -> Option<TaskRef> {
        self.pim.iter().find_map(|(a, p)| {
            p.tasks.iter().find(|t| t.task.uid == uid).map(|t| TaskRef { account: a.clone(), list: t.list.clone(), task: t.task.clone() })
        })
    }

    /// Tasks go to the server only where it keeps them: an on-premises Exchange account's Tasks
    /// folder (issue #161), as its notes go to its Notes folder.
    fn tasks_sync(&self, account: &str) -> bool {
        self.notes_sync(account)
    }

    /// The accounts a task list can be made on (issue #294): those whose calendars are on a
    /// CalDAV server, found.
    pub fn task_list_accounts(&self) -> Vec<&str> {
        self.accounts
            .iter()
            .filter(|a| a.config.has_server() && a.config.graph.is_none() && a.config.ews.is_none())
            .filter(|a| self.pim.get(&a.config.id).is_some_and(|p| p.caldav_home.as_deref().is_some_and(|h| !h.is_empty())))
            .map(|a| a.config.id.as_str())
            .collect()
    }

    /// Ask for a task list called `name` on `account`'s CalDAV server (or the one on screen's,
    /// else the first that has one): made at its next sync (issue #294: a server with none kept
    /// MailCraft's tasks on this computer).
    pub fn new_task_list(&mut self, account: Option<&str>, name: &str) -> CmdResult {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
            return Err(CmdError::Params("a task list's name: up to 100 characters, on one line".into()));
        }
        let able = self.task_list_accounts();
        let current = self.current_account().map(|a| a.config.id.clone());
        let account = match account {
            Some(a) if able.contains(&a) => a.to_string(),
            Some(a) => return Err(CmdError::Disabled(format!("`{a}` has no calendar server to make a task list on"))),
            None => match current.filter(|c| able.contains(&c.as_str())).or_else(|| able.first().map(|a| a.to_string())) {
                Some(a) => a,
                None => return Err(CmdError::Disabled("no account has a calendar server to make a task list on".into())),
            },
        };
        let p = self.pim_mut(&account);
        let taken = p
            .task_lists
            .iter()
            .map(|l| l.name.as_str())
            .chain(p.new_task_lists.iter().map(String::as_str))
            .any(|n| n.trim().eq_ignore_ascii_case(name));
        if taken {
            return Err(CmdError::Params(format!("there's a task list called “{name}” already")));
        }
        p.new_task_lists.push(name.to_string());
        self.after_pim_change(&account, true);
        Ok(serde_json::json!({"account": account, "name": name}))
    }

    /// Each account's CalDAV task lists (issue #201), in the order the server gave them.
    pub fn task_lists(&self) -> Vec<(&str, &mailcraft_model::pim::TaskList)> {
        self.accounts
            .iter()
            .filter_map(|a| self.pim.get(&a.config.id).map(|p| (a.config.id.as_str(), p)))
            .flat_map(|(a, p)| p.task_lists.iter().map(move |l| (a, l)))
            .collect()
    }

    /// The task list of `account` a task goes to: the one named (one of its own it may write
    /// to), else the account's default; empty where its server keeps no task lists.
    fn list_for(&self, account: &str, list: Option<&str>) -> Result<String, CmdError> {
        let Some(p) = self.pim.get(account) else { return Ok(String::new()) };
        match list.map(str::trim).filter(|l| !l.is_empty()) {
            Some(l) => match p.task_lists.iter().find(|x| x.id == l) {
                Some(x) if x.read_only => Err(CmdError::Disabled(format!("the task list `{}` can't be changed here", x.name))),
                Some(x) => Ok(x.id.clone()),
                None => Err(CmdError::Params(format!("no task list `{l}` in this account"))),
            },
            None => Ok(p.default_task_list().map(|l| l.id.clone()).unwrap_or_default()),
        }
    }

    /// A task in a list the user may only read can't be changed here, nor one whose item on the
    /// server holds occurrences changed on their own (they'd be lost).
    fn task_writable(&self, account: &str, list: &str) -> Result<(), CmdError> {
        if self.pim.get(account).is_some_and(|p| !p.task_list_writable(list)) {
            return Err(CmdError::Disabled("this task is in a task list that can't be changed here".into()));
        }
        Ok(())
    }

    /// Whether a task's item on the server holds more than it ([`StoredTask::partial`]).
    pub fn task_partial(&self, uid: &str) -> bool {
        self.pim.values().flat_map(|p| &p.tasks).any(|t| t.task.uid == uid && t.partial)
    }

    fn task_whole(&self, uid: &str) -> Result<(), CmdError> {
        if self.task_partial(uid) {
            return Err(CmdError::Disabled("this task has occurrences changed on their own elsewhere: change it there".into()));
        }
        Ok(())
    }

    /// Move a task to another of its account's task lists (issue #201): stored in that one,
    /// deleted from the one it was in.
    pub fn move_task(&mut self, uid: &str, list: &str) -> CmdResult {
        let Some(r) = self.find_task(uid) else { return Err(CmdError::Params(format!("no task `{uid}`"))) };
        self.task_writable(&r.account, &r.list)?;
        self.task_whole(uid)?;
        let to = self.list_for(&r.account, Some(list))?;
        if to == r.list {
            return Ok(serde_json::json!({"uid": uid, "changed": false}));
        }
        let p = self.pim_mut(&r.account);
        let Some(stored) = p.tasks.iter_mut().find(|t| t.task.uid == uid) else { return Err(CmdError::Params(format!("no task `{uid}`"))) };
        let gone = (!stored.href.is_empty()).then(|| mailcraft_model::pim::Tombstone {
            href: stored.href.clone(),
            etag: stored.etag.clone(),
            ..Default::default()
        });
        stored.list = to.clone();
        stored.href.clear();
        stored.etag.clear();
        stored.dirty = true;
        p.deleted.extend(gone);
        self.after_pim_change(&r.account, true);
        Ok(serde_json::json!({"uid": uid, "list": to}))
    }

    fn task_account(&self, uid: &str) -> Result<String, CmdError> {
        self.find_task(uid).map(|r| r.account).ok_or_else(|| CmdError::Params(format!("no task `{uid}`")))
    }

    /// Add a task; returns its uid (a new one when it has none, or its own is taken).
    pub fn create_task(&mut self, account: Option<&str>, t: Task) -> CmdResult {
        self.create_task_in(account, None, t)
    }

    /// Add a task to a task list of its account's server (`None`: the account's default).
    pub fn create_task_in(&mut self, account: Option<&str>, list: Option<&str>, t: Task) -> CmdResult {
        check_task(&t, None)?;
        self.add_task(account, list, t, false)
    }

    /// Add a to-do from a calendar file: one with the uid of a task already in the account
    /// takes its place (as an event from a file does), so opening the file again doesn't add
    /// its to-dos twice. It keeps its title (even none) and when it was changed, and starts no
    /// later than it's due.
    pub fn open_task(&mut self, account: Option<&str>, mut t: Task) -> CmdResult {
        if let (Some(s), Some(d)) = (t.start, t.due)
            && s > d
        {
            t.start = Some(d);
        }
        self.add_task(account, None, t, true)
    }

    /// Add a task as it is, in `list` (`None`: the account's default, or the list of the task
    /// it replaces). `from_file`: it replaces the account's task with its uid and keeps its own
    /// changed time (else a taken uid gets a new one, and the task is changed now).
    fn add_task(&mut self, account: Option<&str>, list: Option<&str>, mut t: Task, from_file: bool) -> CmdResult {
        let account = self.account_or_shown(account)?;
        let taken = self.find_task(&t.uid).map(|r| r.account);
        let replaces = from_file && !t.uid.trim().is_empty() && taken.as_deref() == Some(account.as_str());
        if t.uid.trim().is_empty() || (taken.is_some() && !replaces) {
            t.uid = self.new_uid();
        }
        let now = self.clock.now;
        tidy(&mut t, now);
        if !from_file || t.modified <= 0 {
            t.modified = now;
        }
        let uid = t.uid.clone();
        let replaced = replaces.then(|| self.find_task(&uid)).flatten();
        if let Some(r) = &replaced {
            self.task_writable(&account, &r.list)?;
            self.task_whole(&uid)?;
        }
        let list = match (list, &replaced) {
            (None, Some(r)) => r.list.clone(),
            _ => self.list_for(&account, list)?,
        };
        let remote = self.tasks_sync(&account) || !list.is_empty();
        let p = self.pim_mut(&account);
        // One it takes the place of keeps its item on the server: changed there, not made twice.
        let mut kept = StoredTask::default();
        if replaces && let Some(at) = p.tasks.iter().position(|x| x.task.uid == uid) {
            kept = p.tasks.remove(at);
            p.tasks.retain(|x| x.task.uid != uid);
        }
        if p.tasks.len() >= MAX_TASKS {
            return Err(CmdError::Params(format!("an account keeps at most {MAX_TASKS} tasks")));
        }
        p.tasks.push(StoredTask { task: t, list, href: kept.href, etag: kept.etag, dirty: remote, partial: false });
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    /// Change a task.
    pub fn update_task(&mut self, uid: &str, change: impl FnOnce(&mut Task)) -> CmdResult {
        let account = self.task_account(uid)?;
        let list = self.find_task(uid).map(|r| r.list).unwrap_or_default();
        self.task_writable(&account, &list)?;
        self.task_whole(uid)?;
        let now = self.clock.now;
        let remote = self.tasks_sync(&account) || !list.is_empty();
        let p = self.pim_mut(&account);
        let Some(stored) = p.tasks.iter_mut().find(|t| t.task.uid == uid) else { return Err(CmdError::Params(format!("no task `{uid}`"))) };
        let before = stored.task.clone();
        let mut t = before.clone();
        change(&mut t);
        t.uid = before.uid.clone();
        check_task(&t, Some(&before))?;
        tidy(&mut t, now);
        if t == before {
            return Ok(serde_json::json!({"uid": uid, "changed": false}));
        }
        t.modified = now;
        stored.task = t;
        stored.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid}))
    }

    /// Mark a task done or not. A repeating one marked done stays, moved to its next due day,
    /// and a done copy of this time is kept beside it (its uid in the answer's `done`); one
    /// whose rule ends after so many times (`COUNT`) counts down, and is done after its last.
    pub fn complete_task(&mut self, uid: &str, done: bool) -> CmdResult {
        let Some(r) = self.find_task(uid) else { return Err(CmdError::Params(format!("no task `{uid}`"))) };
        let now = self.clock.now;
        let next = match (&r.task.rrule, r.task.due.or(r.task.start)) {
            (Some(rule), Some(from)) if done && !r.task.is_done() => next_due(rule, from).zip(count_down(rule)),
            _ => None,
        };
        let Some((next, rule)) = next else { return self.update_task(uid, |t| t.set_done(done, now)) };
        // This time, done.
        let mut copy = r.task.clone();
        copy.uid = String::new();
        copy.rrule = None;
        copy.reminder = None;
        copy.set_done(true, now);
        self.task_writable(&r.account, &r.list)?;
        self.task_whole(uid)?;
        let copied = self.add_task(Some(&r.account), Some(&r.list).filter(|l| !l.is_empty()).map(String::as_str), copy, false)?;
        // The task, from its next time on.
        let shift = next.saturating_sub(r.task.due.or(r.task.start).unwrap_or(next));
        self.update_task(uid, |t| {
            t.rrule = Some(rule);
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
        let list = self.find_task(uid).map(|r| r.list).unwrap_or_default();
        self.task_writable(&account, &list)?;
        self.task_whole(uid)?;
        let p = self.pim_mut(&account);
        // This one (the first with the uid, as `find_task` finds): a copy of it in another task
        // list, a colleague's say, is another item, left as it is. One on the server is deleted
        // there too.
        let Some(at) = p.tasks.iter().position(|t| t.task.uid == uid) else { return Err(CmdError::Params(format!("no task `{uid}`"))) };
        let gone = p.tasks.remove(at);
        let remote = !gone.href.is_empty();
        if remote {
            p.deleted.push(mailcraft_model::pim::Tombstone { href: gone.href, etag: gone.etag, ..Default::default() });
        }
        self.after_pim_change(&account, remote);
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
                let key = ReminderKey { account, uid: &t.uid, at }.to_string();
                let due = match self.prefs.reminders.get(&key) {
                    // Dismissed (an event's are kept negative, with when: a task's is for good).
                    Some(v) if *v <= 0 => continue,
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
                    message: None,
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
                    busy_only: false,
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
