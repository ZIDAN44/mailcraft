//! Follow Up › Custom… on the To-Do Bar's flagged mail (issue #160): the day a message is due,
//! typed or picked from a month.

use mailcraft_engine::model::MessageKey;

use super::pattern::form_day;
use super::*;

/// A flagged message being given a day of its own.
#[derive(Clone, Debug)]
pub struct FollowUpForm {
    /// The message, as commands name it.
    pub key: String,
    pub subject: String,
    /// Local `YYYY-MM-DD`, as the date box keeps it.
    pub due: String,
    /// The order dates are shown and typed in (File › Options › Dates).
    pub dates: dates::DateOrder,
}

impl FollowUpForm {
    /// The form for a flagged message of the To-Do Bar, on the day it's due (today, when none).
    pub fn new(s: &Session, key: &MessageKey) -> Option<FollowUpForm> {
        let m = s.todo_message(key)?;
        let due = s.follow_up_due(m).unwrap_or_else(|| dates::today(&s.clock));
        Some(FollowUpForm { key: key.to_id(), subject: m.subject.clone(), due: dates::day_string(due), dates: s.clock.style.order })
    }
}

pub fn follow_up(ui: &mut Ui, f: &mut FollowUpForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(380.0);
    form_header(ui, t, Ok(Icon::Flag), "Custom");
    let subject = if f.subject.trim().is_empty() { tl!("(no subject)") } else { f.subject.as_str() };
    ui.add(egui::Label::new(RichText::new(subject).color(t.dim)).truncate());
    ui.add_space(6.0);
    form_row(ui, t, true, Some(Icon::Flag), "Due date", |ui| {
        date_pick::date_field(ui, t, "follow-up-due", &mut f.due, f.dates, None);
    });
    let day = form_day(&f.due);
    footer_rule(ui, t);
    let mut keep = true;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::accent_button(ui, Icon::Flag, "OK", t, day.is_some()).clicked()
            && let Some(d) = day
        {
            acts.push(Action::Run("todo.flag", json!({"key": f.key, "due": dates::day_string(d)})));
            keep = false;
        }
        if ui.button(tl!("Cancel")).clicked() {
            keep = false;
        }
    });
    keep
}
