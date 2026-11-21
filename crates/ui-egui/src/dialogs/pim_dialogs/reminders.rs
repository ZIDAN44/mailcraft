//! An event's reminders past its first (issue #224): Google's calendar gives an event several
//! ("an hour before and 2 minutes before"), and each reminded here once only the first was. Each
//! has a list of its own, beside a button that takes it away, and Add reminder gives one more;
//! not on a calendar that keeps one (Exchange's, as Outlook's).

use mailcraft_engine::model::ical::REMINDERS_MAX;

use super::*;

/// The accounts whose calendars keep one reminder an event (Exchange, through its web services or
/// Microsoft Graph): more would be lost at its next sync.
pub(super) fn one_reminder(s: &Session) -> Vec<String> {
    s.accounts.iter().filter(|a| a.config.ews.is_some() || a.config.graph.is_some()).map(|a| a.config.id.clone()).collect()
}

/// The rows of the event's reminders past its first, and Add reminder under them once it has a
/// first, up to the most an event keeps.
pub(super) fn more_rows(ui: &mut Ui, t: &Tokens, f: &mut EventForm) {
    let mut gone = None;
    for i in 0..f.more_reminders.len() {
        let Some(mut chosen) = f.more_reminders.get(i).copied().map(Some) else { continue };
        form_row(ui, t, true, None, "", |ui| {
            ui.allocate_ui_with_layout(vec2(240.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let mut choices: Vec<Option<i64>> = REMINDER_CHOICES.iter().map(|m| Some(*m)).collect();
                // One set elsewhere (another app, a server) stays offered as it is.
                if !choices.contains(&chosen) {
                    choices.push(chosen);
                }
                let r = egui::ComboBox::from_id_salt(("event-reminder-more", i))
                    .width(194.0)
                    .selected_text(reminder_label(chosen))
                    .show_ui(ui, |ui| {
                        for c in choices.iter().copied() {
                            ui.selectable_value(&mut chosen, c, reminder_label(c));
                        }
                    })
                    .response
                    .accessible_name(tl!("Reminder"));
                crate::drop_down::keys(ui, &r, &mut chosen, &choices);
                if widgets::tiny_icon_button(ui, Icon::Close, "Remove reminder", t).clicked() {
                    gone = Some(i);
                }
            });
        });
        if let (Some(m), Some(c)) = (f.more_reminders.get_mut(i), chosen) {
            *m = c;
        }
    }
    if let Some(i) = gone.filter(|i| *i < f.more_reminders.len()) {
        f.more_reminders.remove(i);
    }
    let account = f.calendars.get(f.calendar).map(|c| c.0.as_str());
    let keeps_one = account.is_some_and(|a| f.one_reminder.iter().any(|o| o == a));
    let count = usize::from(f.reminder.is_some()).saturating_add(f.more_reminders.len());
    if f.reminder.is_none() || keeps_one || count >= REMINDERS_MAX || f.read_only {
        return;
    }
    form_row(ui, t, true, None, "", |ui| {
        if ui.link(RichText::new(tl!("Add reminder")).color(t.accent).small()).clicked() {
            // The longest of Outlook's list not taken yet, after the first's: 15 minutes, then 5…
            let taken = |m: &i64| f.reminder == Some(*m) || f.more_reminders.contains(m);
            let next = [DEFAULT_REMINDER, 5, 30, 60, 1440].into_iter().chain(REMINDER_CHOICES).find(|m| !taken(m)).unwrap_or(0);
            f.more_reminders.push(next);
        }
    });
}
