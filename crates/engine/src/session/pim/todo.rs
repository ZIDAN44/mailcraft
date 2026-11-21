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

mod flag_reminders;

/// Flagged messages kept of one folder, at most (the newest).
const FLAGGED_PER_FOLDER: usize = 500;
/// The bar's width, at least and at most.
pub const TODO_BAR_WIDTH: std::ops::RangeInclusive<u16> = 200..=480;
/// The appointments' share of the height under the month when the tasks are under them, in
/// percent, at least and at most (issue #196): each part keeps a few rows.
pub const TODO_BAR_SPLIT: std::ops::RangeInclusive<u8> = 15..=85;

/// The To-Do Bar's settings: its parts, each shown or not, its width, and where the line between
/// the appointments and the tasks is.
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
    /// The appointments' share of the room under the month, in percent ([`TODO_BAR_SPLIT`]),
    /// when the tasks are under them; the tasks have the rest (issue #196).
    pub split: u8,
}

impl Default for TodoBar {
    fn default() -> Self {
        TodoBar { calendar: false, appointments: false, tasks: false, width: 260, split: 50 }
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
        let week_end = today.saturating_sub(i64::from(mailcraft_model::dates::weekday_index(today))).saturating_add(7);
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
        self.todo_follow_up_op(op);
        let gone: &[MessageKey] = match op {
            Op::SetFlag { keys, flag: Flag::Flagged, on: false } | Op::Move { keys, .. } | Op::Expunge { keys } => keys,
            Op::SetFlag { keys, flag: Flag::Flagged, on: true } => {
                // The list walked once, and each folder's kept list once (select all and Flag
                // in a list of 20,000 looked each key up in all of it: 470 ms); the newest kept of
                // each folder, however the list is arranged, as many as reading the folder keeps.
                let flagged: HashSet<&MessageKey> = keys.iter().collect();
                let mut new: BTreeMap<(String, String), Vec<SharedSummary>> = BTreeMap::new();
                for m in self.list.iter().filter(|m| flagged.contains(&m.key)) {
                    let mut m = (**m).clone();
                    m.flags.flagged = true;
                    new.entry((m.key.account.clone(), m.key.folder.clone())).or_default().push(SharedSummary::new(m));
                }
                for (folder, mut add) in new {
                    let kept = self.todo.read.entry(folder).or_default();
                    let there: HashSet<&MessageKey> = kept.iter().map(|m| &m.key).collect();
                    add.retain(|m| !there.contains(&m.key));
                    add.append(kept);
                    add.sort_by_key(|m| std::cmp::Reverse(m.when()));
                    add.truncate(FLAGGED_PER_FOLDER);
                    *kept = add;
                }
                &[]
            }
            _ => &[],
        };
        let gone: HashSet<&MessageKey> = gone.iter().collect();
        let folders: BTreeSet<(&str, &str)> = gone.iter().map(|k| (k.account.as_str(), k.folder.as_str())).collect();
        for (account, folder) in folders {
            if let Some(kept) = self.todo.read.get_mut(&(account.to_string(), folder.to_string())) {
                kept.retain(|m| !gone.contains(&m.key));
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
        // A folder's flagged mail with a reminder in it, read or gone (gap 9): the Reminders
        // window works its list out again at once, not at its next look.
        let reminds = |s: &Session, list: &[SharedSummary]| list.iter().any(|m| s.message_reminder(m).is_some());
        let had = self.todo.read.get(&next).is_some_and(|l| reminds(self, l));
        let known = self.account(&next.0).is_some_and(|a| a.folders.iter().any(|f| f.path == next.1));
        let flagged: Vec<SharedSummary> = match self.store.as_ref().filter(|_| known).map(|s| s.load_folder(&next.0, &next.1)) {
            Some(Ok(st)) => st.messages.into_iter().filter(|m| m.flags.flagged).take(FLAGGED_PER_FOLDER).collect(),
            // A folder with nothing kept, or gone: nothing flagged in it.
            _ => Vec::new(),
        };
        if had || reminds(self, &flagged) {
            self.pim_revision = self.pim_revision.wrapping_add(1);
        }
        if flagged.is_empty() {
            self.todo.read.remove(&next);
        } else {
            self.todo.read.insert(next, flagged);
        }
        !self.todo.stale.is_empty()
    }

    /// The flagged message of the task list with this key.
    pub fn todo_message(&self, key: &MessageKey) -> Option<&Summary> {
        self.todo_shared(key).map(|m| &**m)
    }

    /// [`Session::todo_message`], shared.
    pub(crate) fn todo_shared(&self, key: &MessageKey) -> Option<&SharedSummary> {
        self.todo.read.get(&(key.account.clone(), key.folder.clone()))?.iter().find(|m| &m.key == key)
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
                // Without its `RE:` / `FW:`, as Outlook's task list shows a reply or forward
                // flagged (issue #163); its `[tags]` kept. One that's all prefix stays as it is.
                let bare = mailcraft_model::thread::without_prefixes(&m.subject);
                let title = if bare.trim().is_empty() { m.subject.clone() } else { bare };
                out.push(TodoItem { what: TodoWhat::Mail { key: m.key.clone() }, title, due, group: TodoGroup::of(due, today) });
            }
        }
        out.sort_by_cached_key(|i| (i.group, i.due, i.title.to_lowercase()));
        out
    }

    /// The To-Do Bar's appointments overlapping `[from, to)`, by start: the shown calendars' as
    /// [`Session::agenda`] has them, of the user's own calendars only (issue #205). A colleague's
    /// calendar opened here, or listed with the user's as someone else's, and their free/busy
    /// time stay in the Calendar: a colleague's holiday was listed under Today as if it were the
    /// user's own. Outlook's bar shows the user's calendar too.
    pub fn todo_appointments(&self, from: i64, to: i64) -> Vec<AgendaItem> {
        let mut out = self.agenda_where(from, to, |account, cal| {
            !cal.foreign && mailcraft_model::pim::shared_calendar_owner(&cal.id).is_none() && self.calendar_shown(account, &cal.id)
        });
        out.sort_by(|a, b| (b.all_day, a.start, &a.summary).cmp(&(a.all_day, b.start, &b.summary)));
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
