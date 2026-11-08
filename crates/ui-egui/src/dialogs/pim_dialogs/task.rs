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
    /// `""` (never), `daily`, `weekly`, `monthly`, `yearly`, or a rule kept as it was.
    pub repeat: String,
    pub categories: Vec<String>,
    pub all_categories: Vec<(String, u8)>,
    pub done: bool,
    pub error: Option<String>,
}

/// A task's day (UTC midnight) as the form shows it.
fn day_text(day: Option<i64>) -> String {
    day.map(|d| {
        let (y, m, dd) = dates::civil_from_days(d.div_euclid(86_400));
        format!("{y:04}-{m:02}-{dd:02}")
    })
    .unwrap_or_default()
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

fn repeat_of(rule: Option<&str>) -> String {
    match rule {
        None => String::new(),
        Some(r) => match r.to_ascii_uppercase().as_str() {
            "FREQ=DAILY" => "daily".into(),
            "FREQ=WEEKLY" => "weekly".into(),
            "FREQ=MONTHLY" => "monthly".into(),
            "FREQ=YEARLY" => "yearly".into(),
            _ => r.to_string(),
        },
    }
}

impl TaskForm {
    /// A new task, due on `due` (a day) when given; in the account on screen.
    pub fn new(s: &Session, due: Option<i64>) -> TaskForm {
        let accounts: Vec<(String, String)> = s.accounts.iter().map(|a| (a.config.id.clone(), a.config.email.clone())).collect();
        let current = s.current_account().map(|a| a.config.id.clone()).unwrap_or_default();
        TaskForm {
            account: accounts.iter().position(|a| a.0 == current).unwrap_or(0),
            accounts,
            due: day_text(due),
            hours12: s.clock.style.hours12,
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
        Some(TaskForm {
            uid: Some(uid.to_string()),
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
            repeat: repeat_of(t.rrule.as_deref()),
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
            "repeat": if self.repeat.is_empty() { "none" } else { self.repeat.as_str() },
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
        p
    }
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

/// A drop-down of a form, `width` wide.
fn choice<T: PartialEq + Copy>(ui: &mut Ui, id: &str, width: f32, value: &mut T, options: &[T], label: impl Fn(T) -> &'static str) {
    ui.allocate_ui_with_layout(vec2(width, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        egui::ComboBox::from_id_salt(id).width(width - 6.0).selected_text(tl!(label(*value))).show_ui(ui, |ui| {
            for o in options {
                ui.selectable_value(value, *o, tl!(label(*o)));
            }
        });
    });
}

pub fn task(ui: &mut Ui, f: &mut TaskForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(540.0);
    form_header(ui, t, Ok(Icon::Task), if f.uid.is_some() { "Task" } else { "New task" });
    let w = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(&mut f.title)
            .hint_text(tl!("Add a title"))
            .font(theme::bold(16.0))
            .desired_width(w)
            .min_size(vec2(w, 36.0))
            .margin(Margin::symmetric(10, 8))
            .align(egui::Align2::LEFT_CENTER),
    );
    ui.add_space(12.0);
    form_row(ui, t, true, Some(Icon::Calendar), "Start date", |ui| {
        field(ui, &mut f.start, tl!("None"), 120.0);
    });
    form_row(ui, t, true, Some(Icon::Flag), "Due date", |ui| {
        field(ui, &mut f.due, tl!("None"), 120.0);
    });
    let before = f.status;
    form_row(ui, t, true, Some(Icon::Task), "Status", |ui| {
        choice(ui, "task-status", 220.0, &mut f.status, &TaskStatus::ALL, status_label);
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
        choice(ui, "task-priority", 140.0, &mut f.priority, &[TaskPriority::Low, TaskPriority::Normal, TaskPriority::High], priority_label);
    });
    form_row(ui, t, true, Some(Icon::Clock), "Reminder", |ui| {
        ui.allocate_ui_with_layout(vec2(90.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.checkbox(&mut f.remind, tl!("Remind me"));
        });
        if f.remind {
            if f.reminder_date.trim().is_empty() {
                f.reminder_date = f.due.trim().to_string();
            }
            field(ui, &mut f.reminder_date, "YYYY-MM-DD", 120.0);
            time_field(ui, &mut f.reminder_time, 9, f.hours12);
        }
    });
    let custom = !matches!(f.repeat.as_str(), "" | "daily" | "weekly" | "monthly" | "yearly");
    let label = |r: &str| -> String {
        match r {
            "" => tl!("Never").to_string(),
            "daily" => tl!("Every day").to_string(),
            "weekly" => tl!("Every week").to_string(),
            "monthly" => tl!("Every month").to_string(),
            "yearly" => tl!("Every year").to_string(),
            _ => tl!("Custom").to_string(),
        }
    };
    form_row(ui, t, true, Some(Icon::Sync), "Repeat", |ui| {
        ui.allocate_ui_with_layout(vec2(200.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            egui::ComboBox::from_id_salt("task-repeat").width(194.0).selected_text(label(&f.repeat)).show_ui(ui, |ui| {
                for r in ["", "daily", "weekly", "monthly", "yearly"] {
                    ui.selectable_value(&mut f.repeat, r.to_string(), label(r));
                }
                if custom {
                    let keep = f.repeat.clone();
                    ui.selectable_value(&mut f.repeat, keep, tl!("Custom"));
                }
            });
        });
    });
    if !f.all_categories.is_empty() {
        form_row(ui, t, true, Some(Icon::Tag), "Categories", |ui| {
            category_chips(ui, t, &f.all_categories, &mut f.categories);
        });
    }
    if f.uid.is_none() && f.accounts.len() > 1 {
        let name = f.accounts.get(f.account).map(|a| a.1.clone()).unwrap_or_default();
        form_row(ui, t, true, Some(Icon::Person), "Account", |ui| {
            ui.allocate_ui_with_layout(vec2(260.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt("task-account").width(254.0).selected_text(name).show_ui(ui, |ui| {
                    for (i, a) in f.accounts.iter().enumerate() {
                        ui.selectable_value(&mut f.account, i, &a.1);
                    }
                });
            });
        });
    }
    egui::ScrollArea::vertical().id_salt("task-notes").max_height(NOTES_MAX_H).show(ui, |ui| {
        area(ui, &mut f.notes, tl!("Notes"), 4);
    });
    if !f.repeat.is_empty() {
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
        if let Some(uid) = f.uid.clone() {
            if ui.button(tl!("Delete")).clicked() {
                acts.push(Action::Submit("task.delete", json!({"uid": uid})));
            }
            let mark = if f.done { "Mark not complete" } else { "Mark complete" };
            if ui.button(tl!(mark)).clicked() {
                acts.push(Action::Submit("task.complete", json!({"uid": uid, "done": !f.done})));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::accent_button(ui, crate::icons::Icon::Task, "Save", t, !f.title.trim().is_empty()).clicked() {
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
