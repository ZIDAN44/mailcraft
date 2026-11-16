//! Keyboard shortcuts, Outlook's, dispatched to commands.

use crate::*;

/// Where the keyboard was this frame, for the keys the window's parts keep for themselves.
#[derive(Clone, Copy)]
struct Keyboard {
    /// A text box has focus.
    typing: bool,
    /// Text is selected in a message (its body, headers).
    text_selected: bool,
    /// A message's text had the keyboard last frame.
    text_keys: bool,
}

impl MailApp {
    pub(crate) fn shortcuts(&mut self, ctx: &egui::Context) {
        // A message's text had the keyboard last frame (it says so again while it keeps it).
        let text_keys = std::mem::take(&mut self.ui.text_selection.keys);
        if self.ui.backstage.is_none() {
            self.ui.backstage_typing = false;
        }
        if self.modal_keys(ctx) {
            return;
        }
        let typing = ctx.text_edit_focused();
        self.attachments_let_go(ctx);
        // Text selected in a message (its body, headers): Ctrl+A and Ctrl+C are the text's.
        let text_selected = ctx.plugin_opt::<egui::text_selection::LabelSelectionState>().is_some_and(|p| p.lock().has_selection());
        let keyboard = Keyboard { typing, text_selected, text_keys };
        let reg = self.session.registry();
        for (key, m, combo, n) in runs(&reg, pressed(ctx)) {
            if self.key_taken(ctx, key, m, &combo, keyboard) {
                continue;
            }
            let Some(spec) = reg.by_shortcut(&combo) else { continue };
            let id = spec.id;
            if self.intercepted(ctx, id, &combo) {
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

    /// A dialog or the File page has the keys: Esc puts it away. Whether one had them.
    fn modal_keys(&mut self, ctx: &egui::Context) -> bool {
        if let Some(d) = &self.ui.dialog {
            // Not the part of a compaction that has a data file to itself: it can't be put away.
            let stays = matches!(d, Dialog::Compacting { account, .. } if self.session.compaction.exclusive(account));
            if !stays && ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.ui.dialog = None;
            }
            return true;
        }
        if self.ui.backstage.is_some() {
            // Esc in one of its fields leaves the field; from outside one, the page (as
            // GridCraft's File page).
            if ctx.input(|i| i.key_pressed(Key::Escape)) && !std::mem::take(&mut self.ui.backstage_typing) {
                self.ui.backstage = None;
            }
            return true;
        }
        false
    }

    /// A press anywhere but on an attachment hands the keyboard back from the attachments and
    /// clears their selection; not one in a menu over them (Save Selected… is one).
    fn attachments_let_go(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.pointer.primary_pressed()) && !self.ui.attachment_hovered {
            let in_menu =
                ctx.input(|i| i.pointer.interact_pos()).and_then(|p| ctx.layer_id_at(p)).is_some_and(|l| l.order != egui::Order::Background);
            if !in_menu {
                self.ui.attachments_active = false;
                self.ui.attachment_selected = None;
            }
        }
        self.ui.attachment_hovered = false;
    }

    /// Whether a key is kept by where the keyboard is (a text box, the message being written,
    /// the attachments, a message's text) or done here (Ctrl+E, Esc): not a command's then.
    fn key_taken(&mut self, ctx: &egui::Context, key: Key, m: egui::Modifiers, combo: &str, keyboard: Keyboard) -> bool {
        let Keyboard { typing, text_selected, text_keys } = keyboard;
        let plain = !(m.command || m.ctrl || m.alt);
        // Single keys type text when a text box has focus.
        if typing && plain && !matches!(key, Key::F9 | Key::Escape) {
            return true;
        }
        // Writing a message: its own keys (formatting, Ctrl+R aligns right…), not the main
        // window's; Ctrl+Shift+B opens the address book from any of its boxes.
        if typing && let Some(c) = self.session.compose.as_ref().filter(|c| compose_inline(c, ctx)) {
            if combo == "Mod+Shift+B" {
                self.ui.dialog = Some(Dialog::select_names(c, crate::dialogs::Recipients::To));
            }
            if !matches!(combo, "Mod+S" | "Mod+Enter") {
                return true;
            }
        }
        if combo == "Mod+E" {
            self.ui.focus_search = true;
            return true;
        }
        if combo == "Escape" {
            // Escape lets go of selected attachments first.
            if self.ui.attachment_selected.as_ref().is_some_and(|(_, v)| !v.is_empty()) {
                self.ui.attachment_selected = None;
                self.ui.attachments_active = false;
                return true;
            }
            if self.session.search.is_some() {
                self.ui.search_text.clear();
                self.run("search.clear", json!({}));
            }
            return true;
        }
        // In a text box these edit the text (select all, undo, redo).
        if typing && matches!(combo, "Mod+A" | "Mod+Z" | "Mod+Y" | "Mod+Shift+Z") {
            return true;
        }
        // Ctrl+A in the attachments selects them all, as Outlook's does.
        if combo == "Mod+A" && self.ui.attachments_active {
            self.select_all_attachments();
            return true;
        }
        if text_selected && matches!(combo, "Mod+A" | "Mod+C") {
            return true;
        }
        // Clicked into a message's text: Ctrl+A, the copy and Shift with the arrows, Home
        // and End (Ctrl too) select in it, not in the message list.
        text_keys && text_key(combo)
    }

    /// The commands whose shortcut is done otherwise, or not here: whether `id`'s was (and so
    /// isn't run).
    fn intercepted(&mut self, ctx: &egui::Context, id: &str, combo: &str) -> bool {
        // Ctrl+Enter sends the message being written beside the list, as its Send
        // button does: asked first when it has no subject (the command's own words
        // came up instead, in English, braces and all), and not while the keys are
        // the search box's, a dialog's or another part of MailCraft's (it went, half
        // written, from Calendar or with a search typed).
        if id == "compose.send" {
            // With the keys in the message (or nowhere): not in a box beside it, a
            // new task's in the To-Do bar.
            let area = self.ui.compose_area.filter(|(pass, _)| pass.saturating_add(3) > ctx.cumulative_pass_nr()).map(|(_, r)| r);
            let in_it = ctx.memory(|m| m.focused()).is_none_or(|id| match (area, ctx.read_response(id)) {
                (Some(area), Some(r)) => area.contains(r.rect.center()),
                _ => true,
            });
            let here = self.session.module == Module::Mail && !self.ui.search_focused && self.ui.dialog.is_none() && in_it;
            let Some(c) = self.session.compose.as_ref().filter(|c| here && !c.sending && compose_inline(c, ctx)) else { return true };
            // Ctrl+Enter asks first, or does nothing, as set (issue #115); Alt+S sends as ever.
            if combo == "Mod+Enter" && self.session.prefs.ctrl_enter != mailcraft_engine::CtrlEnter::Send {
                if self.session.prefs.ctrl_enter == mailcraft_engine::CtrlEnter::Ask {
                    self.ui.dialog = Some(Dialog::CtrlEnter { remember: false });
                }
                return true;
            }
            if c.subject.trim().is_empty() && self.session.compose_send_block().is_none() {
                self.ui.dialog = Some(Dialog::EmptySubject);
                return true;
            }
        }
        if id == "search.run" {
            return true;
        }
        // The message list's keys, and those acting on the mail selected, are Mail's: in
        // Calendar, People or Notes the list isn't there, and End read its last message,
        // Delete or Backspace took mail away unseen (found testing issue #26).
        if self.session.module != Module::Mail && list_key(id) {
            return true;
        }
        if id == "mail.delete" && self.ask_before_deleting() {
            return true;
        }
        if id == "mail.move" {
            self.ui.dialog = Some(Dialog::move_to());
            return true;
        }
        false
    }

    /// Deleting from Deleted Items is for good: asked first, wherever it's asked from (a row's
    /// bin, the menu, the ribbon, Delete, a message's own window and the reading pane's button).
    /// Whether it was asked (and the delete waits for the answer).
    pub(crate) fn ask_before_deleting(&mut self) -> bool {
        if !self.needs_delete_confirm() {
            return false;
        }
        self.ui.dialog = Some(Dialog::ConfirmDelete { count: self.session.selection.len() });
        true
    }

    /// Deleting from Deleted Items is permanent: ask first.
    pub fn needs_delete_confirm(&self) -> bool {
        self.session.prefs.confirm_permanent_delete
            && !self.session.selection.is_empty()
            // (One whose message is in the list: a message's own window may show one that isn't.)
            && self.session.selected_iter().next().is_some()
            && self.session.selected_iter().all(|m| {
                self.session
                    .account(&m.key.account)
                    .and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Trash))
                    .is_some_and(|t| t.path == m.key.folder)
            })
    }
}

/// The keys pressed this frame, with their modifiers.
fn pressed(ctx: &egui::Context) -> Vec<(Key, egui::Modifiers)> {
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                // The digit row by where it is: with Shift, or on a layout whose top row types
                // other characters (AZERTY), Ctrl+Shift+1 is still 1, not `!` or `&`.
                egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } => {
                    let digit = physical_key.filter(|p| {
                        matches!(
                            p,
                            Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9
                        )
                    });
                    Some((digit.unwrap_or(*key), *modifiers))
                }
                _ => None,
            })
            .collect()
    })
}

/// The keys pressed, each with its shortcut and how many times in a row it goes. A key held
/// down repeats faster than a frame is drawn: the presses of Down or Up (J, K, with Shift too)
/// queued in a row go as one step of so many messages, so only the message landed on is read,
/// and a frame costs one step whatever the repeat rate.
fn runs(reg: &mailcraft_engine::Registry, pressed: Vec<(Key, egui::Modifiers)>) -> Vec<(Key, egui::Modifiers, String, i64)> {
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
    runs
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
