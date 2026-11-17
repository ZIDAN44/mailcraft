//! Keyboard shortcuts, Outlook's, dispatched to commands.

use crate::*;

mod alt_code;
pub use alt_code::{alt_code_pages, alt_codes};

impl MailApp {
    pub(crate) fn shortcuts(&mut self, ctx: &egui::Context) {
        let back_key = self.services.back_key();
        let system_back = back_key.is_some_and(|key| ctx.input(|input| input.key_pressed(key)));
        // A message's text had the keyboard last frame (it says so again while it keeps it).
        let text_keys = std::mem::take(&mut self.ui.text_selection.keys);
        // Android's Back should unwind the topmost layer before leaving the screen: a context
        // menu or selector is lighter than the folder drawer, a dialog or the app itself.
        if system_back && ctx.any_popup_open() {
            egui::Popup::close_all(ctx);
            return;
        }
        if system_back && self.ui.mobile_folder_open {
            self.ui.mobile_folder_open = false;
            return;
        }
        if system_back && let Some(Dialog::DiscardForm(form)) = self.ui.dialog.as_ref() {
            self.ui.dialog = Some((**form).clone());
            return;
        }
        if system_back
            && self.ui.phone_mode
            && self.ui.dialog.as_ref().is_some_and(|dialog| {
                matches!(dialog, Dialog::Event(form) if !form.read_only) || matches!(dialog, Dialog::Task(_) | Dialog::Contact(_))
            })
            && let Some(form) = self.ui.dialog.take()
        {
            self.ui.dialog = Some(Dialog::DiscardForm(Box::new(form)));
            return;
        }
        let escape = ctx.input(|input| input.key_pressed(Key::Escape)) || system_back;
        if self.ui.backstage.is_none() {
            self.ui.backstage_typing = false;
        }
        if let Some(d) = &self.ui.dialog {
            // Not the part of a compaction that has a data file to itself: it can't be put away.
            let stays = matches!(d, Dialog::Compacting { account, .. } if self.session.compaction.exclusive(account));
            if !stays && escape {
                self.ui.dialog = None;
            }
            return;
        }
        if self.ui.backstage.is_some() {
            // Esc in one of its fields leaves the field; from outside one, the page (as
            // GridCraft's File page).
            if escape && !std::mem::take(&mut self.ui.backstage_typing) {
                self.ui.backstage = None;
            }
            return;
        }
        let typing = ctx.text_edit_focused();
        // Android routes Back to the app only after its input method has handled Back itself.
        // A text field can stay focused after the keyboard is dismissed, so don't let that stale
        // focus prevent the user's next Back from leaving the current screen.
        if system_back {
            if self.ui.phone_mode && self.session.compose.is_some() {
                let sending = self.session.compose.as_ref().is_some_and(|c| c.sending);
                if !sending {
                    if self.session.compose_written() {
                        self.ui.dialog = Some(Dialog::SaveChanges);
                        self.ui.dialog_for = self.session.compose.as_ref().map(|c| c.id);
                    } else {
                        self.run("compose.close", json!({"save": false}));
                    }
                }
            } else if self.ui.attachment_view.is_some() {
                reading::close_attachment_view(&mut self.ui);
            } else if self.ui.phone_mode && self.ui.phone_reader_open {
                self.ui.phone_reader_open = false;
            } else if self.ui.phone_mode && self.session.module == Module::People && self.ui.person.is_some() {
                self.ui.person = None;
            } else if self.ui.phone_mode && self.session.module == Module::Tasks && self.ui.task.is_some() {
                self.ui.task = None;
            } else if self.ui.phone_mode && self.session.module == Module::Notes && self.ui.note_edit.is_some() {
                self.checkpoint_note_edit();
            } else if self.ui.attachment_selected.as_ref().is_some_and(|(_, selected)| !selected.is_empty()) {
                self.ui.attachment_selected = None;
                self.ui.attachment_selection_mode = false;
                self.ui.attachments_active = false;
            } else if self.clear_module_search() {
                // Back clears the active module's filter before it leaves the screen.
            } else if self.ui.search_focused || !self.ui.search_text.is_empty() || self.session.search.is_some() {
                self.ui.search_text.clear();
                self.ui.search_focused = false;
                if self.session.search.is_some() {
                    self.run("search.clear", json!({}));
                }
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        // A press anywhere but on an attachment hands the keyboard back from the attachments and
        // clears their selection; not one in a menu over them (Save Selected… is one).
        if ctx.input(|i| i.pointer.primary_pressed()) && !self.ui.attachment_hovered {
            let in_menu =
                ctx.input(|i| i.pointer.interact_pos()).and_then(|p| ctx.layer_id_at(p)).is_some_and(|l| l.order != egui::Order::Background);
            if !in_menu {
                self.ui.attachments_active = false;
                self.ui.attachment_selected = None;
                self.ui.attachment_selection_mode = false;
            }
        }
        self.ui.attachment_hovered = false;
        // Text selected in a message (its body, headers): Ctrl+A and Ctrl+C are the text's.
        let text_selected = ctx.plugin_opt::<egui::text_selection::LabelSelectionState>().is_some_and(|p| p.lock().has_selection());
        let pressed: Vec<(Key, egui::Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    // The digit row by where it is: with Shift, or on a layout whose top row types
                    // other characters (AZERTY), Ctrl+Shift+1 is still 1, not `!` or `&`.
                    egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } => {
                        let key = if Some(*key) == back_key { Key::Escape } else { *key };
                        let digit = physical_key.filter(|p| {
                            matches!(
                                p,
                                Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9
                            )
                        });
                        Some((digit.unwrap_or(key), *modifiers))
                    }
                    _ => None,
                })
                .collect()
        });
        let reg = self.session.registry();
        // A key held down repeats faster than a frame is drawn: the presses of Down or Up (J,
        // K, with Shift too) queued in a row go as one step of so many messages, so only the
        // message landed on is read, and a frame costs one step whatever the repeat rate.
        let mut runs: Vec<(Key, egui::Modifiers, String, i64)> = Vec::new();
        for (key, m) in pressed {
            let combo = combo_of(key, m);
            let step = |c: &str| reg.by_shortcut(c).map(|spec| spec.id).filter(|id| steps(id));
            if let Some(last) = runs.last_mut()
                && let Some(id) = step(&combo)
                && step(&last.2) == Some(id)
            {
                last.3 = last.3.saturating_add(1);
                continue;
            }
            runs.push((key, m, combo, 1));
        }
        for (key, m, combo, n) in runs {
            let plain = !(m.command || m.ctrl || m.alt);
            // Single keys type text when a text box has focus.
            if typing && plain && !matches!(key, Key::F9 | Key::Escape) {
                continue;
            }
            // Writing a message: its own keys (formatting, Ctrl+R aligns right…), not the main
            // window's; Ctrl+Shift+B opens the address book from any of its boxes.
            if typing && let Some(c) = self.session.compose.as_ref().filter(|c| compose_inline(c, ctx)) {
                if combo == "Mod+Shift+B" {
                    self.ui.dialog = Some(Dialog::select_names(c, "to"));
                }
                if !matches!(combo.as_str(), "Mod+S" | "Mod+Enter") {
                    continue;
                }
            }
            if combo == "Mod+E" {
                self.ui.focus_search = true;
                continue;
            }
            if combo == "Escape" {
                // Escape lets go of selected attachments first.
                if self.ui.attachment_selected.as_ref().is_some_and(|(_, v)| !v.is_empty()) {
                    self.ui.attachment_selected = None;
                    self.ui.attachment_selection_mode = false;
                    self.ui.attachments_active = false;
                    continue;
                }
                if self.session.search.is_some() {
                    self.ui.search_text.clear();
                    self.run("search.clear", json!({}));
                }
                continue;
            }
            // In a text box these edit the text (select all, undo, redo).
            if typing && matches!(combo.as_str(), "Mod+A" | "Mod+Z" | "Mod+Y" | "Mod+Shift+Z") {
                continue;
            }
            // Ctrl+A in the attachments selects them all, as Outlook's does.
            if combo == "Mod+A" && self.ui.attachments_active {
                self.select_all_attachments();
                continue;
            }
            if text_selected && matches!(combo.as_str(), "Mod+A" | "Mod+C") {
                continue;
            }
            // Clicked into a message's text: Ctrl+A, the copy and Shift with the arrows, Home
            // and End (Ctrl too) select in it, not in the message list.
            if text_keys && text_key(&combo) {
                continue;
            }
            if let Some(spec) = reg.by_shortcut(&combo) {
                let id = spec.id;
                if id == "search.run" {
                    continue;
                }
                // The message list's keys, and those acting on the mail selected, are Mail's: in
                // Calendar, People or Notes the list isn't there, and End read its last message,
                // Delete or Backspace took mail away unseen (found testing issue #26).
                if self.session.module != Module::Mail && list_key(id) {
                    continue;
                }
                if id == "mail.delete" && self.needs_delete_confirm() {
                    self.ui.dialog = Some(Dialog::ConfirmDelete { count: self.session.selection.len() });
                    continue;
                }
                if id == "mail.move" {
                    self.ui.dialog = Some(Dialog::move_to());
                    continue;
                }
                // Page Up and Page Down go as many rows as the list shows.
                let params = if id.ends_with("PageUp") || id.ends_with("PageDown") {
                    json!({"rows": self.ui.list_page})
                } else if steps(id) {
                    json!({"by": n})
                } else {
                    json!({})
                };
                self.run(id, params);
                if moves_focus(id) {
                    self.ui.scroll_to_focus = true;
                }
            }
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::Enter)) && self.session.compose.is_some() {
            self.run("compose.send", json!({}));
        }
    }

    /// Clear the current non-mail module's search before Android Back leaves its screen.
    fn clear_module_search(&mut self) -> bool {
        let query = match self.session.module {
            Module::Calendar => &mut self.ui.cal_search.query,
            Module::People => &mut self.ui.people_query,
            Module::Tasks => &mut self.ui.tasks_query,
            Module::Notes => &mut self.ui.notes_query,
            Module::Mail => return false,
        };
        if query.is_empty() {
            return false;
        }
        query.clear();
        true
    }

    /// Deleting from Deleted Items is permanent: ask first.
    pub fn needs_delete_confirm(&self) -> bool {
        self.session.prefs.confirm_permanent_delete
            && !self.session.selection.is_empty()
            && self.session.selected_iter().all(|m| {
                self.session
                    .account(&m.key.account)
                    .and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Trash))
                    .is_some_and(|t| t.path == m.key.folder)
            })
    }
}

/// Whether a shortcut's command is the message list's or acts on the mail selected in it (new
/// mail and Send/Receive aren't: they work from anywhere).
fn list_key(id: &str) -> bool {
    id.starts_with("list.") || id.starts_with("quickStep.") || (id.starts_with("mail.") && !matches!(id, "mail.new" | "mail.sync"))
}

/// The keys a message's text takes while it has the keyboard.
fn text_key(combo: &str) -> bool {
    let shifted = combo.strip_prefix("Mod+").unwrap_or(combo).strip_prefix("Shift+");
    matches!(combo, "Mod+A" | "Mod+C") || shifted.is_some_and(|k| matches!(k, "Left" | "Right" | "Up" | "Down" | "Home" | "End"))
}

/// A shortcut as the registry names it: `Mod+Shift+Down`.
fn combo_of(key: Key, m: egui::Modifiers) -> String {
    let mut combo = String::new();
    if m.command || m.ctrl {
        combo.push_str("Mod+");
    }
    if m.alt {
        combo.push_str("Alt+");
    }
    if m.shift {
        combo.push_str("Shift+");
    }
    combo.push_str(match key {
        Key::ArrowDown => "Down",
        Key::ArrowUp => "Up",
        Key::Enter => "Enter",
        k => k.name(),
    });
    combo
}

/// The commands that go a message at a time (`{"by": n}`: so many at once).
fn steps(id: &str) -> bool {
    matches!(id, "list.next" | "list.prev" | "list.extendNext" | "list.extendPrev")
}

/// Whether a command moves the list's focus (which is then scrolled into view).
pub(crate) fn moves_focus(id: &str) -> bool {
    matches!(id, "list.next" | "list.prev" | "list.first" | "list.last" | "list.pageUp" | "list.pageDown") || id.starts_with("list.extend")
}
