//! Dialogs: previews, calendars and people.

use super::*;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::ConfirmDeleteContact { uid, name } => {
            ui.set_width(380.0);
            if name.trim().is_empty() {
                title(ui, t, "Delete this contact?");
            } else {
                title(ui, t, &tf!("Delete “{name}”?", name = name));
            }
            ui.label(RichText::new(tl!("This contact will be deleted, along with everything kept about them. This can't be undone.")).color(t.dim));
            let (ok, cancel) = buttons(ui, t, "Delete", true);
            if ok {
                acts.push(Action::Run("contact.delete", json!({"uid": uid})));
                return false;
            }
            !cancel
        }
        Dialog::PicturePreview { key, index, name } => {
            let w = (ui.ctx().content_rect().width() * 0.75).clamp(360.0, 1200.0);
            let h = (ui.ctx().content_rect().height() * 0.7).max(240.0);
            ui.set_width(w);
            title(ui, t, name);
            match crate::reading::big_picture(s, ui.ctx(), key, *index) {
                Some(tex) => {
                    let sz = tex.size_vec2();
                    let scale = (w / sz.x).min(h / sz.y).min(1.0);
                    ui.vertical_centered(|ui| ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale)));
                }
                None => {
                    ui.label(RichText::new(tl!("This picture can't be shown.")).color(t.dim));
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("Open")).clicked() {
                    acts.push(Action::InWindow(key.clone(), Box::new(Action::OpenAttachment(*index))));
                }
                close_row(ui)
            })
            .inner
        }
        Dialog::Preview { title: name, header, text } => {
            let w = (ui.ctx().content_rect().width() * 0.7).clamp(360.0, 900.0);
            ui.set_width(w);
            title(ui, t, name);
            for (k, v) in header.iter() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(k.as_str()).color(t.dim).small());
                    ui.label(RichText::new(v.as_str()).small());
                });
            }
            ui.add_space(6.0);
            let h = (ui.ctx().content_rect().height() * 0.6).max(200.0);
            egui::ScrollArea::both().max_height(h).auto_shrink([false, true]).show(ui, |ui| {
                let mut shown = text.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut shown).font(egui::FontId::monospace(13.0)).desired_width(f32::INFINITY).frame(egui::Frame::NONE),
                );
            });
            ui.add_space(6.0);
            close_row(ui)
        }
        Dialog::OpenSharedCalendar { email, error } => {
            ui.set_width(420.0);
            title(ui, t, "Open a shared calendar");
            ui.add(
                egui::Label::new(
                    RichText::new(tl!("Whose calendar? It opens read-only beside yours, when they've shared it with you in Exchange.")).color(t.dim),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(email).hint_text("name@example.com").desired_width(f32::INFINITY));
            // Before the focus request below, which would hide the Enter that ended input.
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            if let Some(e) = error {
                ui.add_space(4.0);
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let ready = email.contains('@');
            let (ok, cancel) = buttons(ui, t, "Open", ready);
            if (ok || enter) && ready {
                acts.push(Action::Submit("calendar.openShared", json!({"email": email.trim()})));
            }
            !cancel
        }
        Dialog::Event(f) => crate::pim_dialogs::event(ui, f, t, acts),
        Dialog::Contact(f) => crate::pim_dialogs::contact(ui, f, t, acts),
        // Every other kind is shown by its own group (`body`).
        _ => true,
    }
}
