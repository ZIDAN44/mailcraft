//! Propose New Time (issue #238), from a meeting invitation: the time the user would rather have,
//! sent to the organizer, with an answer of Tentative or Decline beside it if wanted, as
//! Outlook's.

use mailcraft_engine::model::ical::CalEvent;

use super::pattern::form_day;
use super::*;

/// A new time for a meeting the user is invited to.
#[derive(Clone, Debug)]
pub struct ProposeForm {
    pub uid: String,
    pub summary: String,
    /// Local `YYYY-MM-DD` days, as the date boxes keep them, and the times as typed.
    pub date: String,
    pub start: String,
    pub end_date: String,
    pub end: String,
    /// The answer sent with it: none, `tentative` or `declined`.
    pub answer: Option<&'static str>,
    pub hours12: bool,
    pub dates: dates::DateOrder,
}

impl ProposeForm {
    /// The form for `e`, at its own time to begin with.
    pub fn new(s: &Session, e: &CalEvent) -> ProposeForm {
        ProposeForm {
            uid: e.uid.clone(),
            summary: e.summary.clone(),
            date: local_date(e.start, s),
            start: local_time(e.start, s),
            end_date: local_date(e.end, s),
            end: local_time(e.end, s),
            answer: Some("tentative"),
            hours12: s.clock.style.hours12,
            dates: s.clock.style.order,
        }
    }

    /// `invite.propose`'s params, when what's typed reads.
    fn params(&self) -> Option<Value> {
        let (day, end_day) = (form_day(&self.date)?, form_day(&self.end_date)?);
        crate::i18n::read_time(&self.start)?;
        crate::i18n::read_time(&self.end)?;
        let at = |d: i64, t: &str| format!("{}T{}", dates::day_string(d), time_param(t));
        let mut p = json!({"uid": self.uid, "start": at(day, &self.start), "end": at(end_day, &self.end)});
        if let Some(a) = self.answer {
            p["answer"] = json!(a);
        }
        Some(p)
    }
}

pub fn propose(ui: &mut Ui, f: &mut ProposeForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(440.0);
    form_header(ui, t, Ok(Icon::Clock), "Propose new time");
    ui.add(egui::Label::new(RichText::new(&f.summary).color(t.dim)).truncate());
    ui.add_space(6.0);
    form_row(ui, t, true, Some(Icon::Clock), "Start", |ui| {
        date_pick::date_field(ui, t, "propose-start", &mut f.date, f.dates, None);
        time_field(ui, &mut f.start, 9, f.hours12);
    });
    form_row(ui, t, true, None, "End", |ui| {
        date_pick::date_field(ui, t, "propose-end", &mut f.end_date, f.dates, None);
        time_field(ui, &mut f.end, 10, f.hours12);
    });
    form_row(ui, t, true, Some(Icon::Calendar), "Your answer", |ui| {
        for (label, code) in [("Tentative", Some("tentative")), ("Decline", Some("declined")), ("Don't answer", None)] {
            ui.radio_value(&mut f.answer, code, tl!(label));
        }
    });
    let run = f.params();
    footer_rule(ui, t);
    let mut keep = true;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::accent_button(ui, Icon::Send, "Send proposal", t, run.is_some()).clicked()
            && let Some(p) = run
        {
            acts.push(Action::Submit("invite.propose", p));
            keep = true;
        }
        if ui.button(tl!("Cancel")).clicked() {
            keep = false;
        }
    });
    keep
}
