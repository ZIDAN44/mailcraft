//! Links to the people invited to a calendar event.

use super::*;

/// Keep each attendee's email address with the name and reply status shown in the form.
pub(super) fn from_event(items: &[mailcraft_engine::model::ical::Attendee]) -> Vec<(String, String, String)> {
    items.iter().map(|a| (if a.name.is_empty() { a.email.clone() } else { a.name.clone() }, a.email.clone(), a.partstat.clone())).collect()
}
/// Show who is invited and start a new message to someone who has an email address.
pub(super) fn show(ui: &mut Ui, t: &Tokens, attendees: &[(String, String, String)], acts: &mut Vec<Action>) {
    if ui.ctx().input(|input| input.has_touch_screen()) {
        ui.vertical(|ui| {
            ui.label(RichText::new(tl!("Attendees:")).color(t.dim));
            for (name, email, status) in attendees {
                let shown = format!("{name} ({})", tl!(partstat_label(status)));
                if let Some(url) = crate::mailto::link(email) {
                    let response = wide_link(ui, t, &shown, email);
                    if response.clicked() {
                        acts.push(Action::OpenUrl(url));
                    }
                    copy_email_menu(response, email);
                } else {
                    ui.label(RichText::new(shown).color(t.dim));
                }
            }
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tl!("Attendees:")).color(t.dim));
            for (name, email, status) in attendees {
                let shown = format!("{name} ({})", tl!(partstat_label(status)));
                if let Some(url) = crate::mailto::link(email) {
                    let response = ui.link(shown.as_str()).on_hover_text(email);
                    if response.clicked() {
                        acts.push(Action::OpenUrl(url));
                    }
                    copy_email_menu(response, email);
                } else {
                    ui.label(RichText::new(shown).color(t.dim));
                }
            }
        });
    }
}
/// Show and link an event's organizer when their email address is present.
pub(super) fn organizer(ui: &mut Ui, t: &Tokens, name: Option<&str>, email: Option<&str>, acts: &mut Vec<Action>) {
    let Some(name) = name else { return };
    ui.add_space(4.0);
    if ui.ctx().input(|input| input.has_touch_screen()) {
        ui.vertical(|ui| {
            ui.label(RichText::new(tl!("Organizer:")).color(t.dim).small());
            if let Some(email) = email
                && let Some(url) = crate::mailto::link(email)
            {
                let response = wide_link(ui, t, name, email);
                if response.clicked() {
                    acts.push(Action::OpenUrl(url));
                }
                copy_email_menu(response, email);
            } else {
                ui.label(RichText::new(name).color(t.dim).small());
            }
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tl!("Organizer:")).color(t.dim).small());
            if let Some(email) = email
                && let Some(url) = crate::mailto::link(email)
            {
                let response = ui.link(name).on_hover_text(email);
                if response.clicked() {
                    acts.push(Action::OpenUrl(url));
                }
                copy_email_menu(response, email);
            } else {
                ui.label(RichText::new(name).color(t.dim).small());
            }
        });
    }
}

fn copy_email_menu(response: egui::Response, email: &str) {
    response.context_menu(|ui| {
        if ui.button(tl!("Copy address")).clicked() {
            ui.ctx().copy_text(email.to_owned());
            ui.close();
        }
    });
}

fn wide_link(ui: &mut Ui, t: &Tokens, label: &str, email: &str) -> egui::Response {
    ui.add_sized(vec2(ui.available_width(), 44.0), egui::Button::new(RichText::new(label).color(t.accent).underline()).frame(false))
        .on_hover_text(email)
}
