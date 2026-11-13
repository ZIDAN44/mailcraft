//! The To-Do Bar beside the mail (Outlook's View › To-Do Bar): which of its parts show, and
//! what its task list holds: the tasks still to do and the mail flagged in any folder, by when
//! each is due.
//!
//! Flagged mail is in every folder's list, and nothing lists it across them: the folders are
//! read from the cache one at a time ([`Session::todo_step`], a folder a frame while the bar
//! shows), and a folder is read again when its mail changes.

use std::collections::{BTreeMap, BTreeSet};

use mailcraft_model::{Flag, FolderRole, MessageKey, Op, SharedSummary};
use serde::{Deserialize, Serialize};

use super::*;

/// Flagged messages kept of one folder, at most (the newest).
const FLAGGED_PER_FOLDER: usize = 500;
/// The bar's width, at least and at most.
pub const TODO_BAR_WIDTH: std::ops::RangeInclusive<u16> = 200..=480;

/// The To-Do Bar's settings: its parts, each shown or not, and its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TodoBar {
    /// The date navigator: a month, the days with something on them in bold.
    pub calendar: bool,
    /// What's coming up in the calendars.
    pub appointments: bool,
    /// Tasks and flagged mail, by when they're due.
    pub tasks: bool,
    pub width: u16,
}

impl Default for TodoBar {
    fn default() -> Self {
        TodoBar { calendar: false, appointments: false, tasks: false, width: 260 }
    }
}

impl TodoBar {
    /// Whether any of it shows.
    pub fn shown(&self) -> bool {
        self.calendar || self.appointments || self.tasks
    }
}

/// Where the task list puts something, by its due day (Outlook's groups).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TodoGroup {
    Overdue,
    Today,
    Tomorrow,
    ThisWeek,
    NextWeek,
    Later,
    NoDate,
}

impl TodoGroup {
    /// The group of something due on `day` (a local day), seen on `today`. Weeks begin on
    /// Monday.
    pub fn of(day: Option<i64>, today: i64) -> TodoGroup {
        let Some(day) = day else { return TodoGroup::NoDate };
        let week_end = today.saturating_sub((today.rem_euclid(7) + 3) % 7).saturating_add(7);
        match day.saturating_sub(today) {
            i64::MIN..=-1 => TodoGroup::Overdue,
            0 => TodoGroup::Today,
            1 => TodoGroup::Tomorrow,
            _ if day < week_end => TodoGroup::ThisWeek,
            _ if day < week_end.saturating_add(7) => TodoGroup::NextWeek,
            _ => TodoGroup::Later,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TodoGroup::Overdue => "Overdue",
            TodoGroup::Today => "Today",
            TodoGroup::Tomorrow => "Tomorrow",
            TodoGroup::ThisWeek => "This week",
            TodoGroup::NextWeek => "Next week",
            TodoGroup::Later => "Later",
            TodoGroup::NoDate => "No date",
        }
    }
}

/// What a line of the task list is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodoWhat {
    Task { uid: String },
    Mail { key: MessageKey },
}

/// A line of the To-Do Bar's task list: a task to do, or a flagged message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodoItem {
    pub what: TodoWhat,
    pub title: String,
    /// The local day it's due, if it has one.
    pub due: Option<i64>,
    pub group: TodoGroup,
}

/// The flagged mail of every folder, as read from the cache.
#[derive(Default)]
pub struct TodoMail {
    read: BTreeMap<(String, String), Vec<SharedSummary>>,
    /// Folders to read, or to read again.
    stale: BTreeSet<(String, String)>,
    /// Every folder has been asked for once.
    begun: bool,
}

impl Session {
    /// A folder's mail changed: its flagged mail is read again.
    pub(crate) fn todo_folder_changed(&mut self, account: &str, folder: &str) {
        if self.todo.begun {
            self.todo.stale.insert((account.to_string(), folder.to_string()));
        }
    }

    /// A change made here to messages: what it takes off the task list goes at once, and what it
    /// puts there comes when it's already in the list on screen. A local account's folders are
    /// read again (they're changed on disk by now); a server's, when the server answers.
    pub(crate) fn todo_op(&mut self, account: &str, op: &Op) {
        if !self.todo.begun {
            return;
        }
        let gone: &[MessageKey] = match op {
            Op::SetFlag { keys, flag: Flag::Flagged, on: false } | Op::Move { keys, .. } | Op::Expunge { keys } => keys,
            Op::SetFlag { keys, flag: Flag::Flagged, on: true } => {
                for k in keys {
                    if let Some(m) = self.list.iter().find(|m| &m.key == k).cloned() {
                        let kept = self.todo.read.entry((k.account.clone(), k.folder.clone())).or_default();
                        if !kept.iter().any(|x| &x.key == k) {
                            let mut m = (*m).clone();
                            m.flags.flagged = true;
                            kept.insert(0, SharedSummary::new(m));
                        }
                    }
                }
                &[]
            }
            _ => &[],
        };
        for k in gone {
            if let Some(kept) = self.todo.read.get_mut(&(k.account.clone(), k.folder.clone())) {
                kept.retain(|m| &m.key != k);
            }
        }
        if self.account(account).is_none_or(AccountState::is_local) {
            let folders: BTreeSet<String> =
                op.keys().iter().map(|k| k.folder.clone()).chain(if let Op::Move { to, .. } = op { Some(to.clone()) } else { None }).collect();
            for f in folders {
                self.todo_folder_changed(account, &f);
            }
        }
    }

    /// Read one folder's flagged mail from the cache; whether more folders wait. The first call
    /// asks for every folder of every account but the ones mail is thrown away or written in.
    pub fn todo_step(&mut self) -> bool {
        if !self.todo.begun {
            self.todo.begun = true;
            for a in &self.accounts {
                for f in a.folders.iter().filter(|f| f.selectable && !matches!(f.look(), FolderRole::Trash | FolderRole::Junk | FolderRole::Drafts)) {
                    self.todo.stale.insert((a.config.id.clone(), f.path.clone()));
                }
            }
        }
        let Some(next) = self.todo.stale.pop_first() else { return false };
        let known = self.account(&next.0).is_some_and(|a| a.folders.iter().any(|f| f.path == next.1));
        let flagged: Vec<SharedSummary> = match self.store.as_ref().filter(|_| known).map(|s| s.load_folder(&next.0, &next.1)) {
            Some(Ok(st)) => st.messages.into_iter().filter(|m| m.flags.flagged).take(FLAGGED_PER_FOLDER).collect(),
            // A folder with nothing kept, or gone: nothing flagged in it.
            _ => Vec::new(),
        };
        if flagged.is_empty() {
            self.todo.read.remove(&next);
        } else {
            self.todo.read.insert(next, flagged);
        }
        !self.todo.stale.is_empty()
    }

    /// The flagged message of the task list with this key.
    pub fn todo_message(&self, key: &MessageKey) -> Option<&Summary> {
        self.todo.read.get(&(key.account.clone(), key.folder.clone()))?.iter().find(|m| &m.key == key).map(|m| &**m)
    }

    /// The To-Do Bar's task list: tasks not done and flagged mail, the soonest due first (those
    /// with no day last), each with the group it goes under.
    pub fn todo_items(&self) -> Vec<TodoItem> {
        let today = mailcraft_model::dates::today(&self.clock);
        let mut out: Vec<TodoItem> = Vec::new();
        for (account, p) in &self.pim {
            if self.account(account).is_none() {
                continue;
            }
            for t in p.tasks.iter().map(|t| &t.task).filter(|t| !t.is_done()) {
                let due = t.due.map(|d| d.div_euclid(86_400));
                out.push(TodoItem { what: TodoWhat::Task { uid: t.uid.clone() }, title: t.title.clone(), due, group: TodoGroup::of(due, today) });
            }
        }
        for ((account, _), list) in &self.todo.read {
            if self.account(account).is_none() {
                continue;
            }
            for m in list {
                let due = self.follow_up_due(m);
                out.push(TodoItem { what: TodoWhat::Mail { key: m.key.clone() }, title: m.subject.clone(), due, group: TodoGroup::of(due, today) });
            }
        }
        out.sort_by_cached_key(|i| (i.group, i.due, i.title.to_lowercase()));
        out
    }

    /// Mark a flagged message of the task list complete ([`Session::complete_messages`]).
    pub fn todo_complete(&mut self, key: &MessageKey) -> CmdResult {
        let m = self.todo_message(key).cloned().ok_or_else(|| CmdError::Params(format!("`{}` isn't flagged", key.to_id())))?;
        self.complete_messages(vec![m])
    }

    /// Show or hide a part of the To-Do Bar (`on: None` turns it the other way), or all of it.
    pub fn todo_bar_part(&mut self, part: Option<&str>, on: Option<bool>) -> Result<(), CmdError> {
        let bar = &mut self.prefs.todo_bar;
        match part {
            Some("calendar") => bar.calendar = on.unwrap_or(!bar.calendar),
            Some("appointments") => bar.appointments = on.unwrap_or(!bar.appointments),
            Some("tasks") => bar.tasks = on.unwrap_or(!bar.tasks),
            Some(other) => return Err(CmdError::Params(format!("unknown part `{other}`"))),
            None => {
                let on = on.unwrap_or(!bar.shown());
                (bar.calendar, bar.appointments, bar.tasks) = (on, on, on);
            }
        }
        Ok(())
    }
}
