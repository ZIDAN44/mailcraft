//! A backup of what MailCraft keeps in the browser (issue #83): saved to a file, and brought
//! back from one.

use super::*;

/// The backup about to be saved.
#[derive(Clone, Debug, Default)]
pub struct BackupForm {
    /// Locks the file, when given; typed twice.
    pub password: String,
    pub again: String,
    /// With the mail downloaded to this browser.
    pub mail: bool,
    pub error: Option<String>,
}

/// A backup file picked to be brought in.
#[derive(Clone, Debug, Default)]
pub struct RestoreForm {
    pub name: String,
    /// The file. (Shared: a dialog is copied as it's drawn.)
    pub data: std::sync::Arc<Vec<u8>>,
    pub password: String,
    pub error: Option<String>,
}

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::Backup(f) => {
            ui.set_width(440.0);
            title(ui, t, "Back up");
            ui.add(
                egui::Label::new(
                    RichText::new(tl!(
                        "Your accounts and their settings, rules, signatures, calendars and contacts go into one file. Passwords are never in it."
                    ))
                    .color(t.dim),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            ui.checkbox(&mut f.mail, tl!("Include the mail downloaded to this browser"));
            ui.add_space(8.0);
            ui.label(RichText::new(tl!("Password for the file (optional)")).color(t.text));
            ui.add(egui::TextEdit::singleline(&mut f.password).password(true).desired_width(f32::INFINITY));
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("The password again")).color(t.text));
            ui.add(egui::TextEdit::singleline(&mut f.again).password(true).desired_width(f32::INFINITY));
            ui.add_space(4.0);
            let about = if f.password.is_empty() {
                tl!("Without a password, anyone who has the file can read what's in it.")
            } else {
                tl!("The file can't be opened without this password: MailCraft can't recover it.")
            };
            ui.add(egui::Label::new(RichText::new(about).color(t.faint).small()).wrap());
            if let Some(e) = &f.error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let (ok, cancel) = buttons(ui, t, "Save backup", true);
            if ok {
                if f.password == f.again {
                    f.error = None;
                    acts.push(Action::Backup { password: f.password.clone(), mail: f.mail });
                } else {
                    f.error = Some(tl!("The two passwords aren't the same.").to_string());
                }
            }
            !cancel
        }
        Dialog::Restore(f) => {
            ui.set_width(440.0);
            title(ui, t, "Restore from backup");
            ui.add(egui::Label::new(RichText::new(f.name.as_str()).color(t.text)).wrap());
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    RichText::new(tl!(
                        "This takes the place of everything MailCraft keeps in this browser now: its accounts, their settings and their mail. Each account's password is asked for again."
                    ))
                    .color(t.dim),
                )
                .wrap(),
            );
            let locked = mailcraft_engine::backup::locked(&f.data) == Some(true);
            let mut enter = false;
            if locked {
                ui.add_space(8.0);
                ui.label(RichText::new(tl!("The backup's password")).color(t.text));
                let r = ui.add(egui::TextEdit::singleline(&mut f.password).password(true).desired_width(f32::INFINITY));
                enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.memory(|m| m.focused().is_none()) {
                    r.request_focus();
                }
            }
            if let Some(e) = &f.error {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(e.as_str()).color(t.danger).small()).wrap());
            }
            let (ok, cancel) = buttons(ui, t, "Restore", !locked || !f.password.is_empty());
            if ok || (enter && !f.password.is_empty()) {
                f.error = None;
                acts.push(Action::Restore { data: f.data.clone(), password: f.password.clone() });
            }
            !cancel
        }
        _ => true,
    }
}
