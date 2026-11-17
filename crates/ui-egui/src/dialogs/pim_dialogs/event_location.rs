//! Open a calendar event's saved location in the system map app.

use super::*;

pub(super) fn show(ui: &mut Ui, form: &mut EventForm, t: &Tokens, can_open_map: bool, acts: &mut Vec<Action>) {
    form_row(ui, t, true, Some(Icon::Location), "Location", |ui| {
        if form.read_only && !form.location.trim().is_empty() {
            let response = if can_open_map { ui.link(&form.location) } else { ui.label(&form.location) };
            if can_open_map && response.clicked() {
                acts.push(Action::OpenMap(form.location.clone()));
            }
            response.context_menu(|ui| {
                if ui.button(tl!("Copy address")).clicked() {
                    ui.ctx().copy_text(form.location.clone());
                    ui.close();
                }
            });
        } else {
            let show_map_button = !form.read_only && can_open_map && !form.location.trim().is_empty();
            let field_width = if show_map_button { (ui.available_width() - 68.0).max(100.0) } else { f32::INFINITY };
            ui.add_enabled_ui(!form.read_only, |ui| {
                field(ui, &mut form.location, tl!("Add a place"), field_width);
            });
            if show_map_button && ui.button(tl!("Open")).clicked() {
                acts.push(Action::OpenMap(form.location.clone()));
            }
        }
    });
}
