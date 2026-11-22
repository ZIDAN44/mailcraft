//! The task form: a task's title, its start and due days, status, how far along, priority,
//! reminder, how it repeats, categories and notes, as Outlook's task window has them.

use mailcraft_engine::model::pim::{Task, TaskPriority, TaskStatus};

use super::*;

/// A task being added or edited.
#[derive(Clone, Debug, Default)]
pub struct TaskForm {
    /// `None`: a new task.
    pub uid: Option<String>,
    /// The accounts that can take it: (account id, its address); the one chosen.
    pub accounts: Vec<(String, String)>,
    pub account: usize,
    /// The CalDAV task lists it can go in (issue #201): (account id, list id, name, whether it's
    /// the account's default), those the user may write to; the one chosen, and the one it was in.
    pub lists: Vec<(String, String, String, bool)>,
    pub list: String,
    pub was_in: String,
    /// It's in a task list the user may only read: shown, not changed.
    pub read_only: bool,
    /// Its item on the server holds occurrences changed on their own: shown, not changed.
    pub partial: bool,
    pub title: String,
    pub notes: String,
    /// Local `YYYY-MM-DD`, or empty for none.
    pub start: String,
    pub due: String,
    pub status: TaskStatus,
    pub percent: u8,
    pub priority: TaskPriority,
    /// Its reminder: on, and when (local `YYYY-MM-DD`, and its time as typed, shown on the
    /// clock File › Options sets).
    pub remind: bool,
    pub reminder_date: String,
    pub reminder_time: String,
    /// Times are shown on a 12-hour clock (File › Options).
    pub hours12: bool,
    /// Days are shown and typed in this order (File › Options › Dates, issue #162).
    pub dates: dates::DateOrder,
    pub repeat: Repeat,
    pub categories: Vec<String>,
    pub all_categories: Vec<(String, u8)>,
    pub done: bool,
    pub error: Option<String>,
}

/// A task's day (UTC midnight) as the form shows it.
fn day_text(day: Option<i64>) -> String {
    day.map(|d| dates::day_string(d.div_euclid(86_400))).unwrap_or_default()
}

/// What Status is called.
pub fn status_label(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::NotStarted => "Not started",
        TaskStatus::InProgress => "In progress",
        TaskStatus::Completed => "Completed",
        TaskStatus::Waiting => "Waiting on someone else",
        TaskStatus::Deferred => "Deferred",
    }
}

/// What Priority is called.
fn priority_label(p: TaskPriority) -> &'static str {
    match p {
        TaskPriority::Low => "Low",
        TaskPriority::Normal => "Normal",
        TaskPriority::High => "High",
    }
}

impl TaskForm {
    /// A new task, due on `due` (a day) when given; in the account on screen, or in the task list
    /// shown alone (issue #201), as Outlook puts a new item in the folder it shows.
    pub fn new(s: &Session, due: Option<i64>) -> TaskForm {
        let accounts: Vec<(String, String)> = s.accounts.iter().map(|a| (a.config.id.clone(), a.config.email.clone())).collect();
        let lists = writable_lists(s);
        let shown = s.shown_task_list().and_then(|l| lists.iter().find(|x| x.1 == l));
        let current = match shown {
            Some(l) => l.0.clone(),
            None => s.current_account().map(|a| a.config.id.clone()).unwrap_or_default(),
        };
        TaskForm {
            account: accounts.iter().position(|a| a.0 == current).unwrap_or(0),
            accounts,
            list: shown.map(|l| l.1.clone()).unwrap_or_default(),
            lists,
            due: day_text(due),
            hours12: s.clock.style.hours12,
            dates: s.clock.style.order,
            all_categories: all_categories(s, &[]),
            ..Default::default()
        }
    }

    /// An existing task.
    pub fn open(s: &Session, uid: &str) -> Option<TaskForm> {
        let r = s.find_task(uid)?;
        let t: &Task = &r.task;
        let (reminder_date, reminder_time) = match t.reminder {
            Some(at) => (local_date(at, s), local_time(at, s)),
            None => (String::new(), crate::i18n::time_box(9, 0, s.clock.style)),
        };
        let partial = s.task_partial(uid);
        let read_only = partial || s.pim.get(&r.account).is_some_and(|p| !p.task_list_writable(&r.list));
        Some(TaskForm {
            uid: Some(uid.to_string()),
            accounts: vec![(r.account.clone(), String::new())],
            lists: writable_lists(s),
            list: r.list.clone(),
            was_in: r.list.clone(),
            read_only,
            partial,
            title: t.title.clone(),
            notes: t.notes.clone(),
            start: day_text(t.start),
            due: day_text(t.due),
            status: t.status,
            percent: t.percent,
            priority: t.priority,
            remind: t.reminder.is_some(),
            reminder_date,
            reminder_time,
            hours12: s.clock.style.hours12,
            dates: s.clock.style.order,
            repeat: Repeat::from_rrule(t.rrule.as_deref()),
            categories: t.categories.clone(),
            all_categories: all_categories(s, &t.categories),
            done: t.is_done(),
            ..Default::default()
        })
    }

    pub(crate) fn params(&self) -> Value {
        let reminder = if self.remind {
            let date = if self.reminder_date.trim().is_empty() { self.due.trim() } else { self.reminder_date.trim() };
            json!(format!("{date}T{}", time_param(&self.reminder_time)))
        } else {
            json!("none")
        };
        let mut p = json!({
            "title": self.title.trim(),
            "notes": self.notes,
            "start": self.start.trim(),
            "due": self.due.trim(),
            "status": self.status.code(),
            "percent": self.percent.min(100),
            "priority": self.priority.code(),
            "reminder": reminder,
            "repeat": self.repeat.param(),
            "categories": self.categories,
        });
        match &self.uid {
            Some(u) => p["uid"] = json!(u),
            None => {
                if let Some((a, _)) = self.accounts.get(self.account) {
                    p["account"] = json!(a);
                }
            }
        }
        // The list chosen, when one was (a new task goes to the account's default otherwise).
        if !self.list.is_empty() && self.list != self.was_in {
            p["list"] = json!(self.list);
        }
        p
    }

    /// The task lists of the account chosen that it can go in: (id, name, the default).
    fn account_lists(&self) -> Vec<(String, String, bool)> {
        let Some((account, _)) = self.accounts.get(self.account) else { return Vec::new() };
        self.lists.iter().filter(|l| &l.0 == account).map(|l| (l.1.clone(), l.2.clone(), l.3)).collect()
    }
}

/// Every account's task lists the user may write to: (account, list, name, its default).
fn writable_lists(s: &Session) -> Vec<(String, String, String, bool)> {
    s.task_lists()
        .into_iter()
        .filter(|(_, l)| !l.read_only && !l.foreign)
        .map(|(a, l)| {
            let default = s.pim.get(a).and_then(|p| p.default_task_list()).is_some_and(|d| d.id == l.id);
            (a.to_string(), l.id.clone(), l.name.clone(), default)
        })
        .collect()
}

/// The categories to pick from, as chips: those chosen filled in their colour.
pub(super) fn category_chips(ui: &mut Ui, t: &Tokens, all: &[(String, u8)], chosen: &mut Vec<String>) {
    ui.horizontal_wrapped(|ui| {
        for (name, color) in all {
            let on = chosen.iter().any(|c| c.eq_ignore_ascii_case(name));
            let c = theme::category_color(*color, t.dark);
            let text = RichText::new(format!("● {name}")).color(if on { Color32::WHITE } else { c });
            let r =
                ui.add(egui::Button::new(text).fill(if on { c } else { Color32::TRANSPARENT }).stroke(egui::Stroke::new(1.0, c)).corner_radius(10));
            if r.clicked() {
                if on {
                    chosen.retain(|x| !x.eq_ignore_ascii_case(name));
                } else {
                    chosen.push(name.clone());
                }
            }
        }
    });
}

/// A drop-down of a form, `width` wide, `name`d for a screen reader (its row's name is painted).
fn choice<T: PartialEq + Copy>(ui: &mut Ui, id: &str, name: &str, width: f32, value: &mut T, options: &[T], label: impl Fn(T) -> &'static str) {
    ui.allocate_ui_with_layout(vec2(width, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let r = egui::ComboBox::from_id_salt(id)
            .width(width - 6.0)
            .selected_text(tl!(label(*value)))
            .show_ui(ui, |ui| {
                for o in options {
                    ui.selectable_value(value, *o, tl!(label(*o)));
                }
            })
            .response
            .accessible_name(name);
        crate::drop_down::keys(ui, &r, value, options);
    });
}

pub fn task(ui: &mut Ui, f: &mut TaskForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(540.0);
    form_header(ui, t, Ok(Icon::Task), if f.uid.is_some() { "Task" } else { "New task" });
    title_field(ui, &mut f.title);
    form_row(ui, t, true, Some(Icon::Calendar), "Start date", |ui| {
        date_pick::date_field(ui, t, "task-start", &mut f.start, f.dates, Some(tl!("None")));
    });
    form_row(ui, t, true, Some(Icon::Flag), "Due date", |ui| {
        date_pick::date_field(ui, t, "task-due", &mut f.due, f.dates, Some(tl!("None")));
    });
    let before = f.status;
    form_row(ui, t, true, Some(Icon::Task), "Status", |ui| {
        choice(ui, "task-status", tl!("Status"), 220.0, &mut f.status, &TaskStatus::ALL, status_label);
        ui.add_space(6.0);
        ui.allocate_ui_with_layout(vec2(ui.available_width(), FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::DragValue::new(&mut f.percent).range(0..=100).suffix("%"));
            ui.label(RichText::new(tl!("complete")).color(t.dim));
        });
    });
    // Status and percent go together, as Outlook keeps them.
    if f.status != before {
        f.percent = match f.status {
            TaskStatus::Completed => 100,
            TaskStatus::NotStarted => 0,
            _ if before == TaskStatus::Completed => 0,
            _ => f.percent,
        };
    }
    form_row(ui, t, true, Some(Icon::Star), "Priority", |ui| {
        choice(
            ui,
            "task-priority",
            tl!("Priority"),
            140.0,
            &mut f.priority,
            &[TaskPriority::Low, TaskPriority::Normal, TaskPriority::High],
            priority_label,
        );
    });
    form_row(ui, t, true, Some(Icon::Clock), "Reminder", |ui| {
        ui.allocate_ui_with_layout(vec2(90.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.checkbox(&mut f.remind, tl!("Remind me"));
        });
        if f.remind {
            if f.reminder_date.trim().is_empty() {
                f.reminder_date = f.due.trim().to_string();
            }
            date_pick::date_field(ui, t, "task-reminder", &mut f.reminder_date, f.dates, None);
            time_field(ui, &mut f.reminder_time, 9, f.hours12);
        }
    });
    repeat_row(ui, t, "task-repeat", &mut f.repeat);
    if !f.all_categories.is_empty() {
        form_row(ui, t, true, Some(Icon::Tag), "Categories", |ui| {
            category_chips(ui, t, &f.all_categories, &mut f.categories);
        });
    }
    // Its task list, where the account has several (issue #201); a new one in the account's
    // default until another is picked. Picking one for a task moves it there.
    let lists = f.account_lists();
    // Another account chosen: its own default list.
    if !lists.iter().any(|l| l.0 == f.list) && f.uid.is_none() {
        f.list = lists.iter().find(|l| l.2).map(|l| l.0.clone()).unwrap_or_default();
    }
    if lists.len() > 1 && !f.read_only {
        let shown = lists.iter().find(|l| l.0 == f.list).map(|l| l.1.clone()).unwrap_or_default();
        form_row(ui, t, true, Some(Icon::Task), "Task list", |ui| {
            ui.allocate_ui_with_layout(vec2(260.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let r = egui::ComboBox::from_id_salt("task-list")
                    .width(254.0)
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for (id, name, _) in &lists {
                            ui.selectable_value(&mut f.list, id.clone(), name);
                        }
                    })
                    .response
                    .accessible_name(tl!("Task list"));
                let ids: Vec<String> = lists.iter().map(|l| l.0.clone()).collect();
                crate::drop_down::keys(ui, &r, &mut f.list, &ids);
            });
        });
    }
    if f.read_only {
        ui.add_space(4.0);
        let why = if f.partial {
            tl!("Occurrences of this task were changed on their own elsewhere: change it there.")
        } else {
            tl!("This task is in a task list that can't be changed here.")
        };
        ui.label(RichText::new(why).color(t.dim).small());
    }
    if f.uid.is_none() && f.accounts.len() > 1 {
        let name = f.accounts.get(f.account).map(|a| a.1.clone()).unwrap_or_default();
        form_row(ui, t, true, Some(Icon::Person), "Account", |ui| {
            ui.allocate_ui_with_layout(vec2(260.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let r = egui::ComboBox::from_id_salt("task-account")
                    .width(254.0)
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        for (i, a) in f.accounts.iter().enumerate() {
                            ui.selectable_value(&mut f.account, i, &a.1);
                        }
                    })
                    .response
                    .accessible_name(tl!("Account"));
                let all: Vec<usize> = (0..f.accounts.len()).collect();
                crate::drop_down::keys(ui, &r, &mut f.account, &all);
            });
        });
    }
    egui::ScrollArea::vertical().id_salt("task-notes").max_height(NOTES_MAX_H).show(ui, |ui| {
        area(ui, &mut f.notes, tl!("Notes"), 4);
    });
    if f.repeat != Repeat::Never {
        ui.add_space(4.0);
        ui.label(RichText::new(tl!("Marked complete, it comes back on its next due date.")).color(t.faint).small());
    }
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    footer_rule(ui, t);
    let mut keep = true;
    ui.horizontal(|ui| {
        if let Some(uid) = f.uid.clone().filter(|_| !f.read_only) {
            if ui.button(tl!("Delete")).clicked() {
                acts.push(Action::Submit("task.delete", json!({"uid": uid})));
            }
            let mark = if f.done { "Mark not complete" } else { "Mark complete" };
            if ui.button(tl!(mark)).clicked() {
                acts.push(Action::Submit("task.complete", json!({"uid": uid, "done": !f.done})));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::accent_button(ui, crate::icons::Icon::Task, "Save", t, !f.title.trim().is_empty() && !f.read_only).clicked() {
                f.error = None;
                acts.push(Action::Submit(if f.uid.is_some() { "task.update" } else { "task.create" }, f.params()));
            }
            if ui.button(tl!("Cancel")).clicked() {
                keep = false;
            }
        });
    });
    keep
}
