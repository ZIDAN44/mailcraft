//! Follow Up › Custom… and Add Reminder…: the day a flagged message is due, typed or picked from
//! a month, and when it reminds (gap 9); for a flagged message of the To-Do Bar (issue #160) or
//! the messages selected.

use mailcraft_engine::model::{MessageKey, Summary};

use super::pattern::form_day;
use super::*;

/// What the form flags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FollowUpTarget {
    /// A flagged message of the To-Do Bar, as commands name it.
    Todo(String),
    /// The messages selected, this many.
    Selection(usize),
}

/// Messages being given a day of their own, and a reminder.
#[derive(Clone, Debug)]
pub struct FollowUpForm {
    pub target: FollowUpTarget,
    pub subject: String,
    /// Local `YYYY-MM-DD`, as the date box keeps it.
    pub due: String,
    /// Its reminder: on, and when (local `YYYY-MM-DD`, and its time as typed).
    pub remind: bool,
    pub reminder_date: String,
    pub reminder_time: String,
    pub hours12: bool,
    /// The order dates are shown and typed in (File › Options › Dates).
    pub dates: dates::DateOrder,
}

impl FollowUpForm {
    /// The form for a flagged message of the To-Do Bar, on the day it's due (today, when none).
    pub fn new(s: &Session, key: &MessageKey) -> Option<FollowUpForm> {
        let m = s.todo_message(key)?;
        Some(FollowUpForm::of(s, m, FollowUpTarget::Todo(key.to_id()), false))
    }

    /// The form for the messages selected, as the first is flagged: Custom…, or (`remind`) Add
    /// Reminder…, its reminder on.
    pub fn for_selection(s: &Session, remind: bool) -> Option<FollowUpForm> {
        let n = s.selected_iter().count();
        let first = s.selected_iter().next()?;
        let mut f = FollowUpForm::of(s, first, FollowUpTarget::Selection(n), remind);
        if n > 1 {
            f.subject = tf!("{n} messages selected", n = n);
        }
        Some(f)
    }

    fn of(s: &Session, m: &Summary, target: FollowUpTarget, remind: bool) -> FollowUpForm {
        let today = dates::today(&s.clock);
        let due = s.follow_up_due(m);
        let reminder = s.message_reminder(m);
        // None yet: nine in the morning of its day (today, when it has none), or the next full
        // hour once that's passed.
        let at = reminder.unwrap_or_else(|| {
            let nine = s.clock.utc(due.unwrap_or(today).saturating_mul(86_400).saturating_add(9 * 3600));
            if nine > s.clock.now { nine } else { (s.clock.now.div_euclid(3600) + 1) * 3600 }
        });
        FollowUpForm {
            target,
            subject: m.subject.clone(),
            due: dates::day_string(due.unwrap_or(today)),
            remind: remind || reminder.is_some(),
            reminder_date: local_date(at, s),
            reminder_time: local_time(at, s),
            hours12: s.clock.style.hours12,
            dates: s.clock.style.order,
        }
    }

    /// The command that flags them so, and its params, when what's typed reads.
    fn command(&self) -> Option<(&'static str, Value)> {
        let day = form_day(&self.due)?;
        let reminder = if self.remind {
            let date = form_day(&self.reminder_date)?;
            crate::i18n::read_time(&self.reminder_time)?;
            json!(format!("{}T{}", dates::day_string(date), time_param(&self.reminder_time)))
        } else {
            json!("none")
        };
        let mut p = json!({"due": dates::day_string(day), "reminder": reminder});
        Some(match &self.target {
            FollowUpTarget::Todo(key) => {
                p["key"] = json!(key);
                ("todo.flag", p)
            }
            FollowUpTarget::Selection(_) => ("mail.flag", p),
        })
    }
}

pub fn follow_up(ui: &mut Ui, f: &mut FollowUpForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(420.0);
    form_header(ui, t, Ok(Icon::Flag), "Custom");
    let subject = if f.subject.trim().is_empty() { tl!("(no subject)") } else { f.subject.as_str() };
    ui.add(egui::Label::new(RichText::new(subject).color(t.dim)).truncate());
    ui.add_space(6.0);
    form_row(ui, t, true, Some(Icon::Flag), "Due date", |ui| {
        date_pick::date_field(ui, t, "follow-up-due", &mut f.due, f.dates, None);
    });
    form_row(ui, t, true, Some(Icon::Clock), "Reminder", |ui| {
        ui.allocate_ui_with_layout(vec2(90.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.checkbox(&mut f.remind, tl!("Remind me"));
        });
        if f.remind {
            date_pick::date_field(ui, t, "follow-up-reminder", &mut f.reminder_date, f.dates, None);
            time_field(ui, &mut f.reminder_time, 9, f.hours12);
        }
    });
    let run = f.command();
    footer_rule(ui, t);
    let mut keep = true;
    // A row of its own height (issue #220): laid out from the end in all that's left, the
    // buttons were centred in the screen's height, and the window grew to hold it.
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::accent_button(ui, Icon::Flag, "OK", t, run.is_some()).clicked()
            && let Some((id, p)) = run
        {
            acts.push(Action::Run(id, p));
            keep = false;
        }
        if ui.button(tl!("Cancel")).clicked() {
            keep = false;
        }
    });
    keep
}
