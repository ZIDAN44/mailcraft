//! File › Open & Export, where MailCraft keeps everything in the browser (issue #83): a backup
//! of it saved to a file, and one brought back.

use super::*;

pub(super) fn backup(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    if !s.backs_up() {
        return;
    }
    card(
        ui,
        t,
        tl!("Backup"),
        Some(tl!(
            "MailCraft keeps your accounts, settings and mail in this browser only. Clearing the browser's data for this site removes them: a backup file brings them back, here or in another browser."
        )),
        |ui| {
            setting_row(
                ui,
                t,
                true,
                name(t, tl!("Back up")),
                Some(note(t, tl!("Save what's kept here to a file, with a password if you like."))),
                |ui| {
                    if ui.button(tl!("Back up…")).clicked() {
                        acts.push(Action::Dialog(Dialog::Backup(Default::default())));
                    }
                },
            );
            setting_row(
                ui,
                t,
                false,
                name(t, tl!("Restore")),
                Some(note(t, tl!("Bring a backup file in. It takes the place of what's kept here now."))),
                |ui| {
                    if ui.button(tl!("Restore…")).clicked() {
                        acts.push(Action::PickBackup);
                    }
                },
            );
        },
    );
}
