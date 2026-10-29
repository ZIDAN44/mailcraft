//! Dialogs: what closing MailCraft would leave behind (messages being written, what online-only
//! accounts hold in memory, an update about to be installed or still downloading).

use super::*;
use crate::UpdateState;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, _s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::LostOnClose { unsent, changes, sending } => {
            ui.set_width(420.0);
            title(ui, t, "Close MailCraft?");
            if *sending > 0 {
                ui.add(
                    egui::Label::new(
                        RichText::new(tf!(
                            "Messages still being sent: {n}. They may not go if MailCraft closes now: wait a moment, or close anyway.",
                            n = sending
                        ))
                        .color(t.dim),
                    )
                    .wrap(),
                );
            }
            let mut lost = Vec::new();
            if *unsent > 0 {
                lost.push(tf!("Messages waiting to be sent: {n}", n = unsent));
            }
            if *changes > 0 {
                lost.push(tf!("Changes not yet on the server: {n}", n = changes));
            }
            if !lost.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(tf!(
                            "These are held in memory only, as their accounts are online only, and are lost if MailCraft closes now: {what}. Connect and send them first (Send / Receive), or close anyway.",
                            what = lost.join("; ")
                        ))
                        .color(t.dim),
                    )
                    .wrap(),
                );
            }
            let (ok, cancel) = buttons(ui, t, "Close anyway", true);
            if ok {
                acts.push(Action::CloseAnyway);
                return false;
            }
            !cancel
        }
        Dialog::SaveOnClose(n) => {
            ui.set_width(400.0);
            title(ui, t, if *n == 1 { "Save the message you're writing?" } else { "Save the messages you're writing?" });
            let about = if *n == 1 {
                tl!("It's kept in Drafts if you save it.").to_string()
            } else {
                tf!("{n} messages are being written. They're kept in Drafts if you save them.", n = n)
            };
            ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
            ui.add_space(10.0);
            let mut keep = true;
            button_row(ui, |ui| {
                if ui.button(tl!("Cancel")).clicked() {
                    keep = false;
                }
                if ui.button(tl!("Don't save")).clicked() {
                    acts.push(Action::CloseAnyway);
                    keep = false;
                }
                if widgets::accent_button(ui, crate::icons::Icon::Drafts, "Save", t, true).clicked() {
                    acts.push(Action::Run("compose.keepAll", json!({})));
                    acts.push(Action::CloseAnyway);
                    keep = false;
                }
            });
            keep
        }
        // Closing never installs an update, or drops a download, without a word.
        Dialog::UpdateOnClose(state) => match state {
            UpdateState::Ready { version, admin } => {
                ui.set_width(440.0);
                title(ui, t, &tf!("Install MailCraft {version} as it closes?", version = version));
                let mut about = tl!("The update is downloaded and checked. It installs now, as MailCraft closes, which takes a moment; open MailCraft again when it's done.").to_string();
                if *admin {
                    about.push(' ');
                    about.push_str(tl!("Windows asks for an administrator's permission."));
                }
                ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
                ui.add_space(10.0);
                let mut keep = true;
                button_row(ui, |ui| {
                    if ui.button(tl!("Cancel")).clicked() {
                        keep = false;
                    }
                    if ui.button(tl!("Close without updating")).on_hover_text(tl!("It installs the next time you close MailCraft.")).clicked() {
                        acts.push(Action::UpdateOnClose(false));
                        keep = false;
                    }
                    if widgets::accent_button(ui, Icon::Sync, "Update and close", t, true).clicked() {
                        acts.push(Action::UpdateOnClose(true));
                        keep = false;
                    }
                });
                keep
            }
            UpdateState::Downloading { version, done, total } => {
                ui.set_width(440.0);
                title(ui, t, "An update is still downloading");
                let so_far = if *total > 0 {
                    tf!(
                        "MailCraft {version} is downloading ({done} of {total}).",
                        version = version,
                        done = widgets::human_size(usize::try_from(*done).unwrap_or(usize::MAX)),
                        total = widgets::human_size(usize::try_from(*total).unwrap_or(usize::MAX))
                    )
                } else {
                    tf!("MailCraft {version} is downloading.", version = version)
                };
                let about = format!(
                    "{so_far} {}",
                    tl!("If you close MailCraft now, the download pauses, and carries on from where it stopped the next time you open MailCraft.")
                );
                ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
                ui.add_space(10.0);
                let mut keep = true;
                button_row(ui, |ui| {
                    if ui.button(tl!("Keep MailCraft open")).clicked() {
                        keep = false;
                    }
                    if widgets::accent_button(ui, Icon::Close, "Close", t, true).clicked() {
                        // Kept for later: should it finish as MailCraft closes, it isn't installed
                        // without its own question.
                        acts.push(Action::UpdateOnClose(false));
                        keep = false;
                    }
                });
                keep
            }
            // Nothing on its way any more (the download failed): nothing to ask.
            UpdateState::None => false,
        },
        _ => true,
    }
}
