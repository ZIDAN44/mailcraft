//! The forms' label column: wide enough for their longest label in the language they're drawn in.

use egui::{Color32, Ui};

use crate::theme;

/// Width of the forms' label column, at the least: it grows to fit a longer label.
pub(super) const LABEL_W: f32 = 104.0;
/// The widest the label column grows; a label longer still is cut off rather than run into its
/// field.
pub(crate) const LABEL_W_MAX: f32 = 220.0;
/// Room for the icon before a label, in the forms that have them.
pub(crate) const ICON_W: f32 = 24.0;
/// The labels of the forms with icons before them (event, task, follow-up, snooze, a meeting's
/// new time, a series' pattern), and of the contact form, which has none: every label
/// [`form_row`](super::form_row) is given, so the column fits them all (a test holds the lists
/// to the forms).
pub(crate) const ICON_FORM_LABELS: [&str; 23] = [
    "Starts",
    "Ends",
    "Start",
    "End",
    "Start date",
    "Due date",
    "Reminder",
    "Repeat",
    "Repeats every",
    "On these days",
    "Which day",
    "Series ends",
    "Show as",
    "Location",
    "Categories",
    "Task list",
    "Invite",
    "Calendar",
    "Status",
    "Priority",
    "Account",
    "Your answer",
    "Until",
];
pub(crate) const CONTACT_FORM_LABELS: [&str; 18] = [
    "Name",
    "Company",
    "Job title",
    "Email",
    "Email 2",
    "Email 3",
    "Business phone",
    "Home phone",
    "Mobile phone",
    "Business fax",
    "Other phone",
    "Address",
    "Street",
    "Postal code",
    "State/Province",
    "Country/Region",
    "Birthday",
    "Notes",
];

/// The forms' label column: as wide as their widest label in this language, so a form's fields
/// line up and none of its labels runs into its field (#198: many are longer in other languages
/// than in English), from [`LABEL_W`] to [`LABEL_W_MAX`]. Measured once a frame.
pub(crate) fn label_column(ui: &Ui, icons_used: bool) -> f32 {
    let pass = ui.ctx().cumulative_pass_nr();
    let id = egui::Id::unique("form-labels").with((icons_used, crate::i18n::current().code()));
    if let Some((at, w)) = ui.ctx().data(|d| d.get_temp::<(u64, f32)>(id))
        && at == pass
    {
        return w;
    }
    let (labels, indent): (&[&str], f32) = if icons_used { (&ICON_FORM_LABELS, ICON_W) } else { (&CONTACT_FORM_LABELS, 0.0) };
    let widest = labels
        .iter()
        .map(|l| ui.painter().layout_no_wrap(tl!(l).to_string(), theme::regular(13.0), Color32::PLACEHOLDER).size().x)
        .fold(0.0, f32::max);
    let w = (indent + widest + 12.0).clamp(LABEL_W, LABEL_W_MAX);
    ui.ctx().data_mut(|d| d.insert_temp(id, (pass, w)));
    w
}
