//! Dialogs: moving mail, and making, renaming, moving, deleting and emptying folders.

use super::*;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::ConfirmDelete { count, one } => {
            ui.set_width(crate::sizes::DIALOG_S);
            title(ui, t, "Delete permanently?");
            let what = if *count == 1 {
                tl!("This message will be deleted for good. This can't be undone.").to_string()
            } else {
                tf!("These {n} messages will be deleted for good. This can't be undone.", n = count)
            };
            ui.label(RichText::new(what).color(t.dim));
            let (ok, cancel) = danger_buttons(ui, t, "Delete", true);
            if ok {
                // A row's own Delete: its message alone, as the button does.
                acts.push(match one {
                    Some(k) => Action::Run("list.rowAction", json!({"key": k.to_id(), "action": "delete", "permanent": true})),
                    None => Action::Run("mail.delete", json!({"permanent": true})),
                });
                return false;
            }
            !cancel
        }
        Dialog::MoveTo { filter, copy, chosen } => {
            let copy = *copy;
            ui.set_width(crate::sizes::DIALOG_S);
            title(ui, t, if copy { "Copy to folder" } else { "Move to folder" });
            let r = ui.add(egui::TextEdit::singleline(filter).hint_text(tl!("Search folders")).desired_width(f32::INFINITY));
            // Once, as it opens (it wasn't drawn the frame before), whatever had the keys:
            // asked for every frame, Tab never left the box.
            let (shown, now) = (egui::Id::unique("move-to-shown"), ui.ctx().cumulative_pass_nr());
            let before: Option<u64> = ui.data(|d| d.get_temp(shown));
            ui.data_mut(|d| d.insert_temp(shown, now));
            if before.is_none_or(|last| last.saturating_add(2) < now) {
                r.request_focus();
            }
            let mut accounts: Vec<String> = s.selected_iter().map(|m| m.key.account.clone()).collect();
            accounts.sort_unstable();
            accounts.dedup();
            let Some(account) = accounts.first().and_then(|a| s.account(a)) else {
                ui.label(RichText::new(tl!("Select a message first.")).color(t.dim));
                return close_row(ui);
            };
            let needle = filter.to_lowercase();
            // The messages' own account's folders, then every other account's, each under its
            // name: mail goes into any of them (issue #82).
            let own = account.config.id.clone();
            let others = s.accounts.iter().filter(|a| a.config.id != own);
            let folders: Vec<&Folder> = std::iter::once(account)
                .chain(others)
                .flat_map(|a| a.folders.iter())
                .filter(|f| f.selectable && folder_matches(f, &needle))
                .collect();
            // A click picks the folder and Move moves the mail there, as Move folder (issue #182):
            // a click moved it at once, and Move was never on. A double click still moves it.
            let mut now = None;
            // As wide as the dialog, its scroll bar at the edge, each row across it (issue #182).
            folder_list(ui, t, |ui| {
                let mut of = own.as_str();
                for f in &folders {
                    if f.account != of {
                        of = f.account.as_str();
                        // By its address, as the folder list names it: the name on its mail
                        // is the user's own, the same for each of their accounts.
                        let name = s.account(of).map(|a| a.config.email.clone()).unwrap_or_default();
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.label(RichText::new(name).color(t.dim).small());
                        });
                    }
                    let picked = chosen.as_ref().is_some_and(|(a, p)| *a == f.account && *p == f.path);
                    let row = folder_row(ui, t, crate::panes::nav::role_icon(f.look()), &crate::i18n::folder(f), f.depth(), picked);
                    if row.clicked() {
                        *chosen = Some((f.account.clone(), f.path.clone()));
                    }
                    if row.double_clicked() {
                        now = Some((f.account.clone(), f.path.clone()));
                    }
                }
            });
            // A folder picked that the search has since left out isn't moved to unseen.
            if chosen.as_ref().is_some_and(|(a, p)| !folders.iter().any(|f| f.account == *a && f.path == *p)) {
                *chosen = None;
            }
            let (ok, cancel) = buttons(ui, t, if copy { "Copy" } else { "Move" }, chosen.is_some());
            // Enter: the folder picked, or the one the search left.
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let only = (folders.len() == 1).then(|| folders.first().map(|f| (f.account.clone(), f.path.clone()))).flatten();
            let go = now.or_else(|| chosen.clone().filter(|_| ok || enter)).or_else(|| only.filter(|_| enter));
            if let Some((to, path)) = go {
                acts.push(Action::Run(if copy { "mail.copy" } else { "mail.move" }, json!({"folder": path, "account": to})));
                return false;
            }
            !cancel
        }
        Dialog::FolderName { account, parent, rename, name, error } => {
            ui.set_width(crate::sizes::DIALOG_S);
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
            label_over(ui, t, "Folder name");
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
            // As it opens: renaming, the name it has selected, so what's typed replaces it.
            let mut opening = ui.memory(|m| m.focused().is_none());
            crate::look::fields::focus_selecting_all(ui, &r, name, &mut opening);
            // One field: Enter means OK wherever the focus is.
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(e) = error {
                field_error(ui, t, e);
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
        Dialog::MoveFolder { account, folder, filter, chosen, error } => {
            ui.set_width(crate::sizes::DIALOG_S);
            let name = s.folder(account, folder).map(Folder::display_name).unwrap_or_default();
            title(ui, t, &tf!("Move “{folder}” to", folder = name));
            let r = ui.add(egui::TextEdit::singleline(filter).hint_text(tl!("Search folders")).desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            let needle = filter.to_lowercase();
            let Some(acc) = s.account(account) else { return false };
            let d = acc.folders.iter().find_map(|f| f.delimiter);
            // A click picks where it goes, and Move moves it there, as Outlook's Move Folder
            // (issue #182): a click moved it at once, and Move never turned on. A double click
            // still moves it at once.
            let mut now = false;
            // As wide as the dialog, its scroll bar at the edge, each row across it (issue #182).
            folder_list(ui, t, |ui| {
                // The top level, by the account's address as the folder pane names it (it was
                // "(Top level)" in italics).
                if needle.is_empty() {
                    let top = folder_row(ui, t, Icon::Folder, &acc.config.email, 0, *chosen == Some(None));
                    if top.clicked() {
                        *chosen = Some(None);
                    }
                    now |= top.double_clicked();
                }
                for f in acc.folders.iter().filter(|f| !mailcraft_engine::model::is_within(&f.path, folder, d) && folder_matches(f, &needle)) {
                    let picked = chosen.as_ref().is_some_and(|c| c.as_deref() == Some(f.path.as_str()));
                    // Under the account's row.
                    let row = folder_row(ui, t, crate::panes::nav::role_icon(f.look()), &crate::i18n::folder(f), f.depth() + 1, picked);
                    if row.clicked() {
                        *chosen = Some(Some(f.path.clone()));
                    }
                    now |= row.double_clicked();
                }
            });
            if let Some(e) = error {
                field_error(ui, t, e);
            }
            let (ok, cancel) = buttons(ui, t, "Move", chosen.is_some());
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(parent) = chosen.clone().filter(|_| ok || now || enter) {
                *error = None;
                acts.push(Action::Submit("folder.move", json!({"account": account, "folder": folder, "parent": parent})));
            }
            !cancel
        }
        Dialog::DeleteFolder { account, folder, name, error } => {
            ui.set_width(crate::sizes::DIALOG_S);
            title(ui, t, &tf!("Delete “{folder}”?", folder = name));
            let text = tl!("The folder, its subfolders and all the mail in them will be deleted for good. This can't be undone.");
            ui.label(RichText::new(text).color(t.dim));
            if let Some(e) = error {
                field_error(ui, t, e);
            }
            let (ok, cancel) = danger_buttons(ui, t, "Delete", true);
            if ok {
                *error = None;
                acts.push(Action::Submit("folder.delete", json!({"account": account, "folder": folder})));
            }
            !cancel
        }
        Dialog::FolderProperties { name, account, location, total, unread, size, archive } => {
            // One width with AutoArchive or without (it changed with what it showed).
            ui.set_width(crate::sizes::DIALOG_M);
            title(ui, t, &tf!("{folder} properties", folder = name));
            egui::Grid::new("folder-props").num_columns(2).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
                let rows = [
                    (tl!("Account"), account.clone()),
                    (tl!("Location"), location.clone()),
                    (tl!("Messages"), total.to_string()),
                    (tl!("Unread"), unread.to_string()),
                    (tl!("Size on this computer"), widgets::human_bytes(*size)),
                ];
                for (label, value) in rows {
                    ui.label(RichText::new(label).color(t.dim));
                    ui.label(value);
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            // Its AutoArchive setting (issue #180), as Outlook's AutoArchive tab.
            let Some(a) = archive else { return close_row(ui) };
            ui.separator();
            ui.label(RichText::new(tl!("AutoArchive")).strong());
            ui.radio_value(&mut a.mode, super::FolderMode::Off, tl!("Don't archive this folder"));
            ui.radio_value(&mut a.mode, super::FolderMode::Default, tl!("Archive it by the default settings"));
            ui.radio_value(&mut a.mode, super::FolderMode::Own, tl!("Archive it by these settings"));
            // Its settings under the choice, not set in again (they were two levels in).
            if a.mode == super::FolderMode::Own {
                ui.add_space(4.0);
                super::archiving::rule_fields(ui, &mut a.rule, "folder", acts);
            }
            let ready = a.mode != super::FolderMode::Own || a.rule.delete || !a.rule.path.trim().is_empty();
            if !ready {
                ui.label(RichText::new(tl!("Choose the file old mail goes to, or to delete it.")).color(t.dim).small());
            }
            let (ok, cancel) = buttons(ui, t, "Save", ready);
            if ok {
                acts.push(Action::Submit("autoArchive.folder", a.params()));
            }
            !cancel
        }
        Dialog::ConfirmEmpty { account, folder, name } => {
            ui.set_width(crate::sizes::DIALOG_S);
            title(ui, t, &tf!("Empty “{folder}”?", folder = name));
            ui.label(
                RichText::new(tl!(
                    "Everything in it will be deleted for good, including mail not downloaded to this computer. This can't be undone."
                ))
                .color(t.dim),
            );
            // For good: red, and not Enter's.
            let (ok, cancel) = danger_buttons(ui, t, "Empty", true);
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

/// The folders to pick from, as the folder pane has them, in a framed list that scrolls (they were
/// a bare list of names, indented with spaces).
fn folder_list(ui: &mut Ui, t: &Tokens, rows: impl FnOnce(&mut Ui)) {
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(crate::sizes::CONTROL_RADIUS).inner_margin(Margin::same(4)).show(
        ui,
        |ui| {
            egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                rows(ui);
            });
        },
    );
}

/// A folder's row to pick: its icon `depth` in, its name; `picked` lit as the folder pane lights
/// the folder open.
fn folder_row(ui: &mut Ui, t: &Tokens, icon: Icon, name: &str, depth: usize, picked: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    widgets::named(ui, &resp, name, true, Some(picked));
    if picked {
        ui.painter().rect_filled(rect, crate::sizes::CONTROL_RADIUS, t.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, crate::sizes::CONTROL_RADIUS, t.hover_row);
    }
    let m = crate::bidi::Mirror::new(rect);
    let x = rect.left() + 8.0 + 16.0 * depth.min(8) as f32;
    let icon_rect = m.rect(egui::Rect::from_center_size(egui::pos2(x + 8.0, rect.center().y), vec2(16.0, 16.0)));
    crate::icons::paint(ui.painter(), icon_rect, icon, if picked { t.accent } else { t.dim });
    let room = (rect.right() - x - 30.0).max(20.0);
    let galley = widgets::one_line(ui, name, theme::regular(crate::sizes::BODY), t.text, room);
    let at = m.rect(egui::Rect::from_min_size(egui::pos2(x + 24.0, rect.center().y - galley.size().y / 2.0), galley.size()));
    ui.painter().galley(at.min, galley, t.text);
    resp
}

/// A folder the search box's text (lower case) is in: in the name the row shows (translated,
/// or the server's own for a special folder), or in its path on the server.
fn folder_matches(f: &Folder, needle: &str) -> bool {
    needle.is_empty()
        || crate::i18n::folder(f).to_lowercase().contains(needle)
        || mailcraft_engine::model::utf7::decode(&f.path).to_lowercase().contains(needle)
}
