//! The Reminders window: calendar reminders that have come due (Outlook's own window lists them
//! the same way), each to dismiss, snooze or open; and a desktop notification when one comes.

use egui::{RichText, Ui, vec2};
use mailcraft_engine::model::dates;
use mailcraft_engine::{DueReminder, Session};
use serde_json::json;

use crate::theme::Tokens;
use crate::{Action, Dialog, widgets};

/// How often the due list is worked out again.
const EVERY_SECS: i64 = 15;
/// The snoozes offered, in minutes (Outlook's list); negative: minutes before the start.
const SNOOZES: [i64; 10] = [5, 10, 15, 30, 60, 120, 240, 480, 1440, 0];
/// "Before start" choices go in as this minus the minutes (so 0 before start is `BEFORE`).
const BEFORE: i64 = -1;

/// What the window keeps between frames.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// The reminders due, as last worked out, and when.
    pub due: Vec<DueReminder>,
    pub checked: i64,
    /// The one picked in the list.
    pub selected: Option<String>,
    /// The snooze picked (minutes; [`BEFORE`] for "0 minutes before start").
    pub snooze: i64,
    /// Closed with these due: shown again only when another comes.
    pub closed: std::collections::BTreeSet<String>,
    /// Told on the desktop already.
    pub notified: std::collections::BTreeSet<String>,
    /// The calendars' revision the list was worked out at ([`Session::pim_revision`], which every
    /// change to an event or a task, and every reminder dismissed or snoozed, moves on): when it
    /// moves, the list is worked out again on that frame; `None` until it first is.
    pub revision: Option<u64>,
}

/// Work out what's due when it's time; the reminders newly come, to tell on the desktop.
pub fn refresh(st: &mut State, s: &Session, force: bool) -> Vec<DueReminder> {
    let now = s.clock.now;
    if !force && st.revision == Some(s.pim_revision) && now - st.checked < EVERY_SECS && now >= st.checked {
        return Vec::new();
    }
    st.checked = now;
    st.revision = Some(s.pim_revision);
    st.due = s.due_reminders(now);
    let keys: std::collections::BTreeSet<String> = st.due.iter().map(|r| r.key.clone()).collect();
    st.closed.retain(|k| keys.contains(k));
    st.notified.retain(|k| keys.contains(k));
    if st.selected.as_ref().is_none_or(|k| !keys.contains(k)) {
        st.selected = st.due.first().map(|r| r.key.clone());
    }
    let fresh: Vec<DueReminder> = st.due.iter().filter(|r| !st.notified.contains(&r.key)).cloned().collect();
    st.notified.extend(fresh.iter().map(|r| r.key.clone()));
    fresh
}

/// Whether the window is up: something due that wasn't closed away.
pub fn showing(st: &State) -> bool {
    st.due.iter().any(|r| !st.closed.contains(&r.key))
}

/// When an occurrence is, for the list: `Today 09:00`, `Tomorrow`, `Mon 2026-10-26 14:00`.
fn when(r: &DueReminder, s: &Session) -> String {
    let day = s.clock.local_day(r.start);
    let date = crate::tasks::near_day(day, dates::today(&s.clock)).map_or_else(|| crate::i18n::date(&dates::day_text(day, &s.clock)), str::to_string);
    if r.all_day { date } else { format!("{date} {}", crate::calendar::hm(r.start, s)) }
}

/// How far off it is: `Now`, `In 15 minutes`, `Overdue`.
fn due_in(r: &DueReminder, now: i64) -> String {
    let minutes = (r.start - now).div_euclid(60);
    if r.start <= now {
        if r.end.max(r.start) < now { tl!("Overdue").to_string() } else { tl!("Now").to_string() }
    } else if minutes < 60 {
        tf!("In {n} minutes", n = minutes.max(1))
    } else if minutes < 1440 {
        tf!("In {n} hours", n = minutes / 60)
    } else {
        tf!("In {n} days", n = minutes / 1440)
    }
}

fn snooze_label(m: i64) -> String {
    if m == BEFORE { tl!("0 minutes before start").to_string() } else { crate::pim_dialogs::reminder_label(Some(m)) }
}

/// The window, when something is due.
pub fn show(ctx: &egui::Context, st: &mut State, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    if !showing(st) {
        return;
    }
    if st.snooze == 0 {
        st.snooze = 5;
    }
    let now = s.clock.now;
    let mut open = true;
    let title = if st.due.len() == 1 { tl!("1 Reminder").to_string() } else { tf!("{n} Reminders", n = st.due.len()) };
    egui::Window::new(title)
        .id(egui::Id::unique("reminders-window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-24.0, -48.0))
        .show(ctx, |ui: &mut Ui| {
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for r in &st.due {
                    let picked = st.selected.as_deref() == Some(r.key.as_str());
                    let text = format!("{}  ·  {}  ·  {}", r.summary, when(r, s), due_in(r, now));
                    let label = RichText::new(text).color(if r.start <= now { t.danger } else { t.text });
                    if ui.selectable_label(picked, label).on_hover_text(&r.location).clicked() {
                        st.selected = Some(r.key.clone());
                    }
                }
            });
            ui.add_space(6.0);
            let picked = st.selected.clone().and_then(|k| st.due.iter().find(|r| r.key == k).cloned());
            ui.horizontal(|ui| {
                if ui.button(tl!("Dismiss All")).clicked() {
                    acts.push(Action::Run("reminders.dismiss", json!({})));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(r) = &picked {
                        if widgets::accent_button(ui, crate::icons::Icon::Close, "Dismiss", t, true).clicked() {
                            acts.push(Action::Run("reminders.dismiss", json!({ "key": r.key })));
                        }
                        if ui.button(tl!("Open Item")).clicked()
                            && let Some(d) = if r.task { Dialog::open_task(s, &r.uid) } else { Dialog::open_event(s, &r.uid, r.occurrence) }
                        {
                            acts.push(Action::Dialog(d));
                        }
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("Click Snooze to be reminded again in:")).color(t.dim).small());
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("reminder-snooze").width(200.0).selected_text(snooze_label(st.snooze)).show_ui(ui, |ui| {
                    for m in SNOOZES {
                        let m = if m == 0 { BEFORE } else { m };
                        ui.selectable_value(&mut st.snooze, m, snooze_label(m));
                    }
                });
                if let Some(r) = &picked
                    && ui.button(tl!("Snooze")).clicked()
                {
                    let p =
                        if st.snooze == BEFORE { json!({ "key": r.key, "beforeStart": 0 }) } else { json!({ "key": r.key, "minutes": st.snooze }) };
                    acts.push(Action::Run("reminders.snooze", p));
                }
            });
        });
    if !open {
        st.closed.extend(st.due.iter().map(|r| r.key.clone()));
    }
}
