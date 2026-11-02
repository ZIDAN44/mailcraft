//! Dialogs: what closing MailCraft would leave behind (messages being written, what online-only
//! accounts hold in memory, a data file being imported, an update about to be installed or
//! still downloading), and a data file compacted, as MailCraft closes or otherwise.

use mailcraft_engine::model::job::{CompactPhase, DataFileHealth};

use super::*;
use crate::UpdateState;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
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
        // An import pauses as MailCraft closes, and goes on as it opens again.
        Dialog::ImportOnClose { file, permille } => {
            ui.set_width(440.0);
            title(ui, t, "Close MailCraft?");
            let about = match permille {
                Some(p) => tf!(
                    "MailCraft is importing {file} ({percent}%). If you close it now, the import goes on the next time you open MailCraft.",
                    file = file,
                    percent = crate::progress::percent(*p)
                ),
                None => tf!("MailCraft is importing {file}. If you close it now, the import goes on the next time you open MailCraft.", file = file),
            };
            ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
            ui.add_space(10.0);
            let mut keep = true;
            button_row(ui, |ui| {
                if ui.button(tl!("Keep importing")).clicked() {
                    keep = false;
                }
                if widgets::accent_button(ui, Icon::Close, "Close MailCraft", t, true).clicked() {
                    // Paused, its place kept: it goes on from there as MailCraft opens.
                    acts.push(Action::Run("account.stopImport", json!({"all": true, "how": "pause"})));
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
                        done = widgets::human_bytes(*done),
                        total = widgets::human_bytes(*total)
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
        // A data file being compacted; it ends by itself (nothing left to show).
        Dialog::Compacting { account, closing, since } => {
            let Some(c) = s.compaction.running.get(account.as_str()) else { return false };
            ui.set_width(440.0);
            let file = crate::app::compacting::file_name(s, account);
            title(ui, t, &if *closing { tf!("Compacting {file} before closing…", file = file) } else { tf!("Compacting {file}…", file = file) });
            let (words, permille) = crate::app::compacting::phase_words(c, *since, ui.input(|i| i.time));
            ui.label(RichText::new(words).color(t.text));
            ui.add_space(6.0);
            crate::progress::bar(ui, t, permille, vec2(ui.available_width(), 6.0));
            ui.add_space(6.0);
            let exclusive = c.phase == CompactPhase::Exclusive;
            let about = if exclusive {
                tl!(
                    "MailCraft has the data file to itself for this part, so it can't be stopped. Nothing is lost if the computer goes off meanwhile: the file is checked the next time MailCraft opens."
                )
            } else if *closing {
                tl!("Nothing is lost if you close now: what's done is kept, and the rest is done another time.")
            } else {
                tl!("It goes on in the background if you hide this. Your mail stays as it is: only the room it takes changes.")
            };
            ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
            ui.add_space(10.0);
            let mut keep = true;
            button_row(ui, |ui| {
                if *closing {
                    if widgets::accent_button(ui, Icon::Close, "Close now", t, true).clicked() {
                        acts.push(Action::CompactOnClose(false));
                        keep = false;
                    }
                } else if exclusive {
                    ui.add_enabled(false, egui::Button::new(tl!("Finishing…")));
                } else {
                    if ui.button(tl!("Hide")).clicked() {
                        keep = false;
                    }
                    if ui.button(tl!("Stop compacting")).clicked() {
                        acts.push(Action::Run("dataFile.cancelCompact", json!({ "account": account })));
                    }
                }
            });
            keep
        }
        // Closing waits only so long: a longer one is offered by hand, saying how long it takes.
        Dialog::CompactOnClose { account, seconds } => {
            ui.set_width(460.0);
            let file = crate::app::compacting::file_name(s, account);
            title(ui, t, &tf!("Compact {file} before closing?", file = file));
            let health = s.compaction.health.get(account.as_str());
            let saving = widgets::human_bytes(health.map_or(0, DataFileHealth::saving));
            let how_long = match health.and_then(DataFileHealth::by_hand_s).unwrap_or(*seconds) {
                secs if secs < 60 => tf!("about {n} s", n = secs.max(1)),
                secs => tf!("about {n} min", n = secs.div_ceil(60)),
            };
            let copied = health.is_some_and(|h| h.by_hand_s().is_none());
            let about = if copied {
                tf!(
                    "It has about {size} to give back. That's done by copying it into a new file, which can take many minutes for a large file; MailCraft closes when it's done.",
                    size = saving
                )
            } else {
                tf!(
                    "It has about {size} to give back, but that takes {time}, longer than MailCraft waits as it closes. Meanwhile nothing of this account can be opened.",
                    size = saving,
                    time = how_long
                )
            };
            ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(tl!("You can also compact it later from File › Info, or turn this off in File › Options.")).color(t.dim).small(),
                )
                .wrap(),
            );
            ui.add_space(10.0);
            let mut keep = true;
            button_row(ui, |ui| {
                if ui.button(tl!("Close without compacting")).clicked() {
                    acts.push(Action::CompactOnClose(false));
                    keep = false;
                }
                if widgets::accent_button(ui, Icon::Sync, "Compact, then close", t, true).clicked() {
                    acts.push(Action::CompactOnClose(true));
                    keep = false;
                }
            });
            keep
        }
        _ => true,
    }
}
