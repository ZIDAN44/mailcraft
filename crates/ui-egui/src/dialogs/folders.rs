//! Dialogs: moving mail, and making, renaming, moving, deleting and emptying folders.

use super::*;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::ConfirmDelete { count } => {
            ui.set_width(360.0);
            title(ui, t, "Delete permanently?");
            let what = if *count == 1 {
                tl!("This message will be deleted for good. This can't be undone.").to_string()
            } else {
                tf!("These {n} messages will be deleted for good. This can't be undone.", n = count)
            };
            ui.label(RichText::new(what).color(t.dim));
            let (ok, cancel) = buttons(ui, t, "Delete", true);
            if ok {
                acts.push(Action::Run("mail.delete", json!({"permanent": true})));
                return false;
            }
            !cancel
        }
        Dialog::MoveTo { filter } => {
            ui.set_width(340.0);
            title(ui, t, "Move to folder");
            let r = ui.add(egui::TextEdit::singleline(filter).hint_text(tl!("Search folders")).desired_width(f32::INFINITY));
            r.request_focus();
            let mut accounts: Vec<String> = s.selected().into_iter().map(|m| m.key.account).collect();
            accounts.sort_unstable();
            accounts.dedup();
            let Some(account) = accounts.first().and_then(|a| s.account(a)) else {
                ui.label(RichText::new(tl!("Select a message first.")).color(t.dim));
                return !buttons(ui, t, "Close", false).1;
            };
            if accounts.len() > 1 {
                ui.label(RichText::new(tl!("Pick messages from one account to move them to a folder.")).color(t.warn).small());
            }
            let needle = filter.to_lowercase();
            let folders: Vec<&Folder> = account
                .folders
                .iter()
                .filter(|f| f.selectable && mailcraft_engine::model::utf7::decode(&f.path).to_lowercase().contains(&needle))
                .collect();
            let mut chosen = None;
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                for f in &folders {
                    let indent = "    ".repeat(f.depth());
                    if ui.selectable_label(false, format!("{indent}{}", crate::i18n::folder(f))).clicked() {
                        chosen = Some(f.path.clone());
                    }
                }
            });
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter && folders.len() == 1 {
                chosen = folders.first().map(|f| f.path.clone());
            }
            if let Some(path) = chosen {
                acts.push(Action::Run("mail.move", json!({"folder": path})));
                return false;
            }
            !buttons(ui, t, "Move", false).1
        }
        Dialog::FolderName { account, parent, rename, name, error } => {
            ui.set_width(360.0);
            title(ui, t, if rename.is_some() { "Rename folder" } else { "New folder" });
            if rename.is_none() {
                let place = parent.as_deref().and_then(|p| s.folder(account, p)).map(Folder::display_name);
                let who = s.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
                let text = match place {
                    Some(p) => tf!("Inside {folder} ({account})", folder = p, account = who),
                    None => tf!("At the top of {account}", account = who),
                };
                ui.label(RichText::new(text).color(t.dim).small());
                ui.add_space(4.0);
            }
            let r = ui.add(egui::TextEdit::singleline(name).hint_text(tl!("Folder name")).desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            // One field: Enter means OK wherever the focus is.
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(e) = error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let ready = !name.trim().is_empty();
            let (ok, cancel) = buttons(ui, t, if rename.is_some() { "Rename" } else { "Create" }, ready);
            if (ok || enter) && ready {
                *error = None;
                match rename {
                    Some(path) => acts.push(Action::Submit("folder.rename", json!({"account": account, "folder": path, "name": name.trim()}))),
                    None => acts.push(Action::Submit("folder.create", json!({"account": account, "parent": parent, "name": name.trim()}))),
                }
            }
            !cancel
        }
        Dialog::MoveFolder { account, folder, filter, error } => {
            ui.set_width(340.0);
            let name = s.folder(account, folder).map(Folder::display_name).unwrap_or_default();
            title(ui, t, &tf!("Move “{folder}” to", folder = name));
            let r = ui.add(egui::TextEdit::singleline(filter).hint_text(tl!("Search folders")).desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            let needle = filter.to_lowercase();
            let Some(acc) = s.account(account) else { return false };
            let d = acc.folders.iter().find_map(|f| f.delimiter);
            let mut chosen: Option<Option<String>> = None;
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                if needle.is_empty() && ui.selectable_label(false, RichText::new(tl!("(Top level)")).italics()).clicked() {
                    chosen = Some(None);
                }
                for f in acc.folders.iter().filter(|f| !mailcraft_engine::model::is_within(&f.path, folder, d)) {
                    if !mailcraft_engine::model::utf7::decode(&f.path).to_lowercase().contains(&needle) {
                        continue;
                    }
                    let indent = "    ".repeat(f.depth());
                    if ui.selectable_label(false, format!("{indent}{}", crate::i18n::folder(f))).clicked() {
                        chosen = Some(Some(f.path.clone()));
                    }
                }
            });
            if let Some(e) = error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            if let Some(parent) = chosen {
                *error = None;
                acts.push(Action::Submit("folder.move", json!({"account": account, "folder": folder, "parent": parent})));
            }
            !buttons(ui, t, "Move", false).1
        }
        Dialog::DeleteFolder { account, folder, name, permanent, error } => {
            ui.set_width(380.0);
            title(ui, t, &tf!("Delete “{folder}”?", folder = name));
            let text = if *permanent {
                tl!("The folder, its subfolders and all the mail in them will be deleted for good. This can't be undone.")
            } else {
                tl!("The folder and everything in it will be moved to Deleted Items.")
            };
            ui.label(RichText::new(text).color(t.dim));
            if let Some(e) = error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let (ok, cancel) = buttons(ui, t, "Delete", true);
            if ok {
                *error = None;
                acts.push(Action::Submit("folder.delete", json!({"account": account, "folder": folder})));
            }
            !cancel
        }
        Dialog::FolderProperties { name, account, location, total, unread, size } => {
            ui.set_width(380.0);
            title(ui, t, &tf!("{folder} Properties", folder = name));
            egui::Grid::new("folder-props").num_columns(2).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
                let rows = [
                    (tl!("Account"), account.clone()),
                    (tl!("Location"), location.clone()),
                    (tl!("Messages"), total.to_string()),
                    (tl!("Unread"), unread.to_string()),
                    (tl!("Size on this computer"), widgets::human_size(usize::try_from(*size).unwrap_or(usize::MAX))),
                ];
                for (label, value) in rows {
                    ui.label(RichText::new(label).color(t.dim));
                    ui.label(value);
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            close_row(ui)
        }
        Dialog::ConfirmEmpty { account, folder, name, for_good } => {
            ui.set_width(400.0);
            if *for_good {
                title(ui, t, &tf!("Empty “{folder}”?", folder = name));
                ui.label(
                    RichText::new(tl!(
                        "Everything in it will be deleted for good, including mail not downloaded to this computer. This can't be undone."
                    ))
                    .color(t.dim),
                );
            } else {
                title(ui, t, &tf!("Delete everything in “{folder}”?", folder = name));
                ui.label(RichText::new(tl!("Every message in it will be moved to Deleted Items.")).color(t.dim));
            }
            let (ok, cancel) = buttons(ui, t, if *for_good { "Empty" } else { "Delete all" }, true);
            if ok {
                acts.push(Action::Run("folder.empty", json!({"account": account, "folder": folder})));
                return false;
            }
            !cancel
        }
        // Every other kind is shown by its own group (`body`).
        _ => true,
    }
}
