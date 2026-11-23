//! Options' Calendar card (issue #185): the work time, the days worked and the hours between,
//! which the work week shows and the calendar shades outside of.

use mailcraft_engine::model::dates;

use super::*;

/// 1970-01-05, a Monday: the weekdays' names are those of the days from it.
const A_MONDAY: i64 = 4;
/// The latest end of a work day offered.
const LAST_END: u16 = 23 * 60 + 30;

pub(super) fn calendar(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let w = s.work_time();
    card(ui, t, tl!("Calendar"), None, |ui| {
        setting_by(s, "calendar.workTime", ui, t, true, tl!("Work days"), Some(tl!("The work week shows these days")), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let from_end = from_end(ui);
                let order: Vec<u32> = if from_end { (0..7).rev().collect() } else { (0..7).collect() };
                for d in order {
                    let on = w.works_on(d);
                    let name = tl!(dates::weekday_short(A_MONDAY + i64::from(d)));
                    let r = ui.selectable_label(on, name).on_hover_text(tl!(dates::weekday_long(A_MONDAY + i64::from(d))));
                    // The last day worked stays: a week has one at least.
                    if r.clicked() && !(on && w.days.count_ones() == 1) {
                        let days: Vec<u32> = (0..7).filter(|x| if *x == d { !on } else { w.works_on(*x) }).collect();
                        acts.push(Action::Run("calendar.workTime", json!({"days": days})));
                    }
                }
            });
        });
        setting_by(s, "calendar.workTime", ui, t, false, tl!("Work hours"), Some(tl!("The calendar shades the hours outside them")), |ui| {
            ui.horizontal(|ui| {
                let start = |ui: &mut Ui| time_choice(s, ui, "work-start", tl!("Start time"), w.start, 0..=w.end.saturating_sub(30));
                // The last end in the list is 23:30, so midnight isn't read as the day's start.
                let end = |ui: &mut Ui| time_choice(s, ui, "work-end", tl!("End time"), w.end, (w.start + 30)..=LAST_END.max(w.end));
                let (start, end) = if from_end(ui) {
                    let e = end(ui);
                    ui.label("–");
                    (start(ui), e)
                } else {
                    let a = start(ui);
                    ui.label("–");
                    (a, end(ui))
                };
                if let Some(m) = start {
                    acts.push(Action::Run("calendar.workTime", json!({"start": hhmm(m)})));
                }
                if let Some(m) = end {
                    acts.push(Action::Run("calendar.workTime", json!({"end": hhmm(m)})));
                }
            });
        });
    });
}

/// Whether a row is laid out from its end: its parts go in the other way round, to read in
/// order.
fn from_end(ui: &Ui) -> bool {
    ui.layout().prefer_right_to_left() != crate::i18n::current().rtl()
}

/// `08:30`, as the command takes a time.
fn hhmm(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// A time of day picked by the half hour within `range` (minutes past midnight): the one
/// picked, when it's another.
fn time_choice(s: &Session, ui: &mut Ui, id: &str, name: &str, now: u16, range: std::ops::RangeInclusive<u16>) -> Option<u16> {
    let shown = |m: u16| crate::i18n::time_box(u32::from(m / 60 % 24), u32::from(m % 60), s.clock.style);
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt(id).selected_text(shown(now)).width(96.0).show_ui(ui, |ui| {
        for m in range.step_by(30) {
            if ui.selectable_label(m == now, shown(m)).clicked() && m != now {
                picked = Some(m);
            }
        }
    });
    widgets::named(ui, &r.response, name, true, None);
    picked
}
