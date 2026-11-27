//! The welcome screen: no accounts yet.

use super::*;

/// First run: no accounts yet.
pub fn welcome(ui: &mut Ui, t: &Tokens, jmap_only: bool, acts: &mut Vec<Action>) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.18).max(20.0));
        egui::Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::same(32)).show(ui, |ui| {
            ui.set_max_width(460.0);
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                ui.painter().rect_filled(r, 14.0, t.accent);
                icons::paint(ui.painter(), icons::centered(r, 38.0), Icon::Unread, t.on_accent);
                ui.add_space(12.0);
                ui.label(RichText::new(tl!("Welcome to MailCraft")).font(theme::semibold(crate::sizes::TITLE)).color(t.text));
                ui.add_space(4.0);
                let about = if jmap_only {
                    "Your mail in the browser. Add a JMAP account (such as Fastmail): MailCraft talks to your mail server directly, and your password stays in this tab only."
                } else {
                    "Your mail, on your computer. Add any IMAP account: messages are kept on this device so they're fast to search and readable offline. Your password stays in the system keychain."
                };
                ui.label(RichText::new(tl!(about)).color(t.dim));
                ui.add_space(18.0);
                if accent_button(ui, Icon::Plus, "Add an account", t, true).clicked() {
                    acts.push(Action::Dialog(Dialog::add_account()));
                }
                // Or begin from what Outlook kept: there are no data files in the browser. There,
                // from a backup of what another browser kept, or this one before its data for
                // the site was cleared (issue #83).
                ui.add_space(6.0);
                if jmap_only {
                    if ui.button(tl!("Restore from a backup…")).clicked() {
                        acts.push(Action::PickBackup);
                    }
                } else if ui.button(tl!("Open an Outlook data file…")).clicked() {
                    acts.push(Action::OpenDataFile);
                }
            });
        });
    });
}
