//! Snooze › Choose a date…: the messages selected come back at a time of the user's own (issue
//! #274: only the menu's four times were offered).

use mailcraft_engine::model::organize::SnoozeUntil;

use super::pattern::form_day;
use super::*;

/// The messages selected being snoozed to a day and time typed or picked.
#[derive(Clone, Debug)]
pub struct SnoozeForm {
    pub subject: String,
    /// Local `YYYY-MM-DD`, as the date box keeps it.
    pub date: String,
    /// The time as typed.
    pub time: String,
    pub hours12: bool,
    /// The order dates are shown and typed in (File › Options › Dates).
    pub dates: dates::DateOrder,
}

impl SnoozeForm {
    /// The form for the messages selected, at the menu's Tomorrow to begin with.
    pub fn for_selection(s: &Session) -> Option<SnoozeForm> {
        let n = s.selected_iter().count();
        let first = s.selected_iter().next()?;
        let at = SnoozeUntil::Tomorrow.at(&s.clock);
        Some(SnoozeForm {
            subject: if n > 1 { tf!("{n} messages selected", n = n) } else { first.subject.clone() },
            date: local_date(at, s),
            time: local_time(at, s),
            hours12: s.clock.style.hours12,
            dates: s.clock.style.order,
        })
    }

    /// The time as `mail.snooze` takes it, and whether it's still to come; `None` while what's
    /// typed isn't a date and time.
    pub fn at(&self, s: &Session) -> Option<(String, bool)> {
        let day = form_day(&self.date)?;
        let (h, m) = crate::i18n::read_time(&self.time)?;
        let local = day.saturating_mul(86_400).saturating_add(i64::from(h) * 3600 + i64::from(m) * 60);
        Some((format!("{}T{h:02}:{m:02}", dates::day_string(day)), s.clock.utc(local) > s.clock.now))
    }
}

pub fn snooze(ui: &mut Ui, f: &mut SnoozeForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(420.0);
    form_header(ui, t, Ok(Icon::Clock), "Snooze");
    let subject = if f.subject.trim().is_empty() { tl!("(no subject)") } else { f.subject.as_str() };
    ui.add(egui::Label::new(RichText::new(subject).color(t.dim)).truncate());
    ui.add_space(6.0);
    form_row(ui, t, true, Some(Icon::Clock), "Until", |ui| {
        date_pick::date_field(ui, t, "snooze-date", &mut f.date, f.dates, None);
        time_field(ui, &mut f.time, 8, f.hours12);
    });
    let at = f.at(s);
    if at.as_ref().is_some_and(|(_, ahead)| !ahead) {
        ui.label(RichText::new(tl!("Pick a time in the future.")).color(t.danger).small());
    }
    footer_rule(ui, t);
    let mut keep = true;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let ready = at.as_ref().filter(|(_, ahead)| *ahead);
        if widgets::accent_button(ui, Icon::Clock, "Snooze", t, ready.is_some()).clicked()
            && let Some((when, _)) = ready
        {
            acts.push(Action::Run("mail.snooze", json!({"at": when})));
            keep = false;
        }
        if ui.button(tl!("Cancel")).clicked() {
            keep = false;
        }
    });
    keep
}
