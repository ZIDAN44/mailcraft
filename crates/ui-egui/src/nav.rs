//! The folder pane: Favorites, then each account's folders as a tree with unread counts.
//! Messages dragged from the list drop onto a folder to move them; a folder dragged onto another
//! moves inside it; an account dragged onto another takes its place. Right-click a folder to
//! create, rename, move, delete or pin it to Favorites. Favorites, accounts and folders with
//! folders in them collapse, and stay so from one start to the next.

use egui::{Color32, RichText, Sense, Ui, vec2};
use mailcraft_engine::model::{Folder, FolderRole};
use mailcraft_engine::{Session, View};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::list::DragKeys;
use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

fn role_icon(role: FolderRole) -> Icon {
    match role {
        FolderRole::Inbox => Icon::Inbox,
        FolderRole::Drafts => Icon::Drafts,
        FolderRole::Sent => Icon::Sent,
        FolderRole::Archive => Icon::Archive,
        FolderRole::Junk => Icon::Junk,
        FolderRole::Trash => Icon::Delete,
        FolderRole::Other => Icon::Folder,
    }
}

/// What a dragged folder carries.
pub struct DragFolder {
    pub account: String,
    pub path: String,
}

/// What a dragged account carries (its id).
pub struct DragAccount(pub String);

/// The key a folder is collapsed under (see `Prefs::collapsed`).
fn folder_key(account: &str, path: &str) -> String {
    format!("folder:{account}/{path}")
}

/// The folders a folder is inside (`A/B/C` → `A`, `A/B`).
fn ancestors(f: &Folder) -> Vec<String> {
    let d = f.delimiter.unwrap_or('/');
    let parts: Vec<&str> = f.path.split(d).collect();
    (1..parts.len()).map(|n| parts.get(..n).unwrap_or_default().join(&d.to_string())).collect()
}

/// A section's heading that collapses it (Favorites), with its arrow; whether it was clicked.
fn section_header(ui: &mut Ui, t: &Tokens, label: &str, open: bool) -> bool {
    ui.add_space(2.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover_row);
    }
    let chev = egui::Rect::from_min_size(egui::pos2(rect.min.x + 2.0, rect.center().y - 6.0), vec2(12.0, 12.0));
    icons::paint(ui.painter(), chev, if open { Icon::ChevronDown } else { Icon::ChevronRight }, t.faint);
    ui.painter().text(egui::pos2(rect.min.x + 18.0, rect.center().y), egui::Align2::LEFT_CENTER, tl!(label), theme::bold(12.0), t.faint);
    resp.clicked()
}

/// One row; returns its response.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    t: &Tokens,
    icon: Icon,
    label: &str,
    unread: u32,
    depth: usize,
    selected: bool,
    drop_ok: bool,
    draggable: bool,
) -> egui::Response {
    let h = 30.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), if draggable { Sense::click_and_drag() } else { Sense::click() });
    let hovering_drop = drop_ok && (resp.dnd_hover_payload::<DragKeys>().is_some() || resp.dnd_hover_payload::<DragFolder>().is_some());
    let fill = if hovering_drop {
        t.accent.gamma_multiply(0.3)
    } else if selected {
        t.accent_soft
    } else if resp.hovered() {
        t.hover_row
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    if selected {
        let bar = egui::Rect::from_min_size(rect.min + vec2(0.0, 7.0), vec2(3.0, h - 14.0));
        ui.painter().rect_filled(bar, 2.0, t.accent);
    }
    let x = rect.min.x + 10.0 + depth as f32 * 14.0;
    let ir = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 8.0), vec2(16.0, 16.0));
    icons::paint(ui.painter(), ir, icon, if selected { t.accent } else { t.dim });
    let font = if unread > 0 || selected { theme::bold(13.5) } else { theme::regular(13.5) };
    let text_right = rect.max.x - if unread > 0 { 40.0 } else { 8.0 };
    let galley = ui.painter().layout(tl!(label).to_string(), font, t.text, (text_right - x - 24.0).max(10.0));
    let gy = rect.center().y - galley.size().y / 2.0;
    ui.painter().with_clip_rect(egui::Rect::from_x_y_ranges(x..=text_right, rect.y_range())).galley(egui::pos2(x + 24.0, gy), galley, t.text);
    if unread > 0 {
        ui.painter().text(
            egui::pos2(rect.max.x - 10.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            unread.to_string(),
            theme::bold(12.5),
            t.accent,
        );
    }
    resp
}

pub fn show(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let favorites_open = !s.prefs.collapsed.contains("favorites");
        if section_header(ui, t, "Favorites", favorites_open) {
            acts.push(Action::Run("view.collapse", json!({"key": "favorites"})));
        }
        if favorites_open && s.accounts.len() > 1 {
            let unread: u32 = s.accounts.iter().filter_map(|a| a.folder(FolderRole::Inbox)).map(|f| f.unread).sum();
            let sel = s.view == View::Unified { role: FolderRole::Inbox } && s.search.is_none();
            if row(ui, t, Icon::Inbox, "All Inboxes", unread, 0, sel, false, false).clicked() {
                acts.push(Action::Run("folder.unified", json!({"role": "inbox"})));
            }
            let sel = s.view == View::Unified { role: FolderRole::Sent } && s.search.is_none();
            if row(ui, t, Icon::Sent, "All Sent", 0, 0, sel, false, false).clicked() {
                acts.push(Action::Run("folder.unified", json!({"role": "sent"})));
            }
        }
        for fav in s.favorites().into_iter().filter(|_| favorites_open) {
            if let Some(f) = s.folder(&fav.account, &fav.folder) {
                folder_row(s, t, ui, acts, f, 0, true, None);
            }
        }
        if !s.outbox.is_empty() {
            let sel = s.view == View::Outbox && s.search.is_none();
            let n = u32::try_from(s.outbox.len()).unwrap_or(u32::MAX);
            if row(ui, t, Icon::Outbox, "Outbox", n, 0, sel, false, false).on_hover_text(tl!("Messages waiting to be sent")).clicked() {
                acts.push(Action::Run("folder.outbox", json!({})));
            }
        }
        ui.add_space(8.0);
        for (at, a) in s.accounts.iter().enumerate() {
            let key = format!("account:{}", a.config.id);
            let open = !s.prefs.collapsed.contains(&key);
            // Dragged onto another account, it takes its place (above or below it).
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click_and_drag());
            if resp.hovered() || resp.dragged() {
                ui.painter().rect_filled(rect, 6.0, t.hover_row);
            }
            if resp.drag_started() {
                resp.dnd_set_drag_payload(DragAccount(a.config.id.clone()));
            }
            let below = ui.ctx().pointer_interact_pos().is_some_and(|p| p.y > rect.center().y);
            if resp.dnd_hover_payload::<DragAccount>().is_some_and(|d| d.0 != a.config.id) {
                let y = if below { rect.bottom() } else { rect.top() };
                ui.painter().line_segment([egui::pos2(rect.left() + 4.0, y), egui::pos2(rect.right() - 4.0, y)], egui::Stroke::new(2.0, t.accent));
            }
            if let Some(dragged) = resp.dnd_release_payload::<DragAccount>()
                && let Some(from) = s.accounts.iter().position(|x| x.config.id == dragged.0)
                && dragged.0 != a.config.id
            {
                let before = if below { at + 1 } else { at };
                let to = if from < before { before - 1 } else { before };
                if to != from {
                    acts.push(Action::Run("account.move", json!({"account": dragged.0, "by": to as i64 - from as i64})));
                }
            }
            let chev = egui::Rect::from_min_size(rect.min + vec2(6.0, 9.0), vec2(12.0, 12.0));
            icons::paint(ui.painter(), chev, if open { Icon::ChevronDown } else { Icon::ChevronRight }, t.dim);
            ui.painter().circle_filled(rect.min + vec2(26.0, 15.0), 4.0, theme::account_color(a.config.color));
            let label = if a.is_data_file() {
                a.config.email.clone()
            } else if a.is_local() {
                tf!("{email} (sample)", email = a.config.email)
            } else {
                a.config.email.clone()
            };
            ui.painter().with_clip_rect(rect).text(rect.min + vec2(36.0, 15.0), egui::Align2::LEFT_CENTER, label, theme::bold(13.0), t.text);
            if resp.clicked() {
                acts.push(Action::Run("view.collapse", json!({"key": key})));
            }
            let config = a.config.clone();
            resp.context_menu(|ui| {
                if ui.button(tl!("Send / Receive")).clicked() {
                    acts.push(Action::Run("mail.sync", json!({})));
                    ui.close();
                }
                if !a.is_local() && ui.button(tl!("Sign in again…")).clicked() {
                    acts.push(Action::SignInAgain(config.id.clone()));
                    ui.close();
                }
                // The account's data file, as Outlook's Open File Location; its settings.
                if let Some(file) = s.store.as_ref().map(|store| store.account_path(&config.id)).filter(|p| p.exists())
                    && ui.button(tl!("Open file location")).clicked()
                {
                    let dir = if file.is_dir() { file.clone() } else { file.parent().map_or_else(|| file.clone(), std::path::Path::to_path_buf) };
                    acts.push(Action::OpenFolder(dir));
                    ui.close();
                }
                if !a.is_local() && ui.button(tl!("Account settings…")).clicked() {
                    acts.push(Action::Dialog(Dialog::edit_account(&config)));
                    ui.close();
                }
                if ui.button(tl!("Signature…")).clicked() {
                    acts.push(Action::Dialog(Dialog::Signature { account: config.id.clone(), text: config.signature.clone() }));
                    ui.close();
                }
                if ui.button(tl!("New folder…")).clicked() {
                    // In the folder open now (as Outlook's New Folder starts), else at the top.
                    let here = match &s.view {
                        View::Folder { account, folder } if *account == config.id => Some(folder.as_str()),
                        _ => None,
                    };
                    acts.push(Action::Dialog(Dialog::new_folder(&config.id, here)));
                    ui.close();
                }
                // Its place among the accounts, as Outlook moves a mailbox up or down.
                if s.accounts.len() > 1 {
                    let at = s.accounts.iter().position(|x| x.config.id == config.id).unwrap_or(0);
                    ui.separator();
                    if ui.add_enabled(at > 0, egui::Button::new(tl!("Move up"))).clicked() {
                        acts.push(Action::Run("account.move", json!({"account": config.id, "by": -1})));
                        ui.close();
                    }
                    if ui.add_enabled(at + 1 < s.accounts.len(), egui::Button::new(tl!("Move down"))).clicked() {
                        acts.push(Action::Run("account.move", json!({"account": config.id, "by": 1})));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button(RichText::new(tl!("Remove account…")).color(t.danger)).clicked() {
                    acts.push(Action::Dialog(Dialog::RemoveAccount { account: config.id.clone(), email: config.email.clone() }));
                    ui.close();
                }
            });
            if !open {
                continue;
            }
            if a.folders.is_empty() {
                ui.label(RichText::new(tl!("  Loading folders…")).color(t.faint).small());
            }
            // Folders with folders in them collapse; what's inside a collapsed one is hidden.
            let parents: std::collections::HashSet<String> = a.folders.iter().flat_map(ancestors).collect();
            for f in &a.folders {
                if ancestors(f).iter().any(|up| s.prefs.collapsed.contains(&folder_key(&f.account, up))) {
                    continue;
                }
                let collapse = parents.contains(&f.path).then(|| s.prefs.collapsed.contains(&folder_key(&f.account, &f.path)));
                folder_row(s, t, ui, acts, f, f.depth() + 1, false, collapse);
            }
            ui.add_space(6.0);
        }
    });
}

/// A folder's row; `collapse` is whether it's collapsed, for one with folders in it (it gets
/// an arrow).
#[allow(clippy::too_many_arguments)]
fn folder_row(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>, f: &Folder, depth: usize, favorite: bool, collapse: Option<bool>) {
    let selected = s.search.is_none() && matches!(&s.view, View::Folder { account, folder } if *account == f.account && *folder == f.path);
    let name = match s.account(&f.account) {
        Some(a) if favorite && s.accounts.len() > 1 => {
            format!("{} · {}", crate::i18n::tc("folder", &f.display_name()), a.config.email.split('@').next().unwrap_or_default())
        }
        _ => crate::i18n::tc("folder", &f.display_name()).to_string(),
    };
    // Unread counts matter for mail you receive, not for Sent or Drafts.
    let unread = if matches!(f.role, FolderRole::Sent | FolderRole::Drafts | FolderRole::Trash | FolderRole::Archive) { 0 } else { f.unread };
    let user_folder = f.role == FolderRole::Other && !f.path.eq_ignore_ascii_case("INBOX");
    let resp = row(ui, t, role_icon(f.role), &name, unread, depth, selected, f.selectable, user_folder && !favorite);
    if let Some(collapsed) = collapse {
        let x = resp.rect.min.x + 10.0 + depth as f32 * 14.0 - 14.0;
        let chev = egui::Rect::from_min_size(egui::pos2(x, resp.rect.center().y - 6.0), vec2(12.0, 12.0));
        let c = ui.interact(chev.expand(4.0), resp.id.with("collapse"), Sense::click());
        icons::paint(ui.painter(), chev, if collapsed { Icon::ChevronRight } else { Icon::ChevronDown }, if c.hovered() { t.text } else { t.dim });
        if c.clicked() {
            acts.push(Action::Run("view.collapse", json!({"key": folder_key(&f.account, &f.path)})));
        }
    }
    if f.selectable && resp.clicked() {
        acts.push(Action::Run("folder.open", json!({"account": f.account, "folder": f.path})));
    }
    if f.selectable
        && let Some(p) = resp.dnd_release_payload::<DragKeys>()
        && p.0.iter().all(|k| k.account == f.account)
    {
        acts.push(Action::Run("mail.move", json!({"folder": f.path})));
    }
    if let Some(p) = resp.dnd_release_payload::<DragFolder>()
        && p.account == f.account
        && p.path != f.path
    {
        acts.push(Action::Run("folder.move", json!({"account": f.account, "folder": p.path, "parent": f.path})));
    }
    if user_folder && !favorite && resp.drag_started() {
        resp.dnd_set_drag_payload(DragFolder { account: f.account.clone(), path: f.path.clone() });
    }
    let path = f.path.clone();
    let account = f.account.clone();
    let is_favorite = s.favorites().iter().any(|r| r.account == f.account && r.folder == f.path);
    resp.context_menu(|ui| {
        if ui.button(tl!("Open")).clicked() {
            acts.push(Action::Run("folder.open", json!({"account": account, "folder": path})));
            ui.close();
        }
        if ui.button(tl!("Update folder")).clicked() {
            acts.push(Action::Run("folder.open", json!({"account": account, "folder": path})));
            acts.push(Action::Run("folder.sync", json!({})));
            ui.close();
        }
        if ui.button(tl!("Mark all as read")).clicked() {
            acts.push(Action::Run("folder.open", json!({"account": account, "folder": path})));
            acts.push(Action::Run("mail.markAllRead", json!({})));
            ui.close();
        }
        // Delete All, as Outlook's: Deleted Items and Junk are emptied for good, any other
        // folder's mail goes to Deleted Items.
        if f.selectable && ui.button(tl!("Delete all…")).clicked() {
            acts.push(Action::Dialog(Dialog::ConfirmEmpty {
                account: account.clone(),
                folder: path.clone(),
                name: crate::i18n::tc("folder", &f.display_name()).to_string(),
                for_good: matches!(f.role, FolderRole::Trash | FolderRole::Junk),
            }));
            ui.close();
        }
        ui.separator();
        if ui.button(tl!("New folder…")).clicked() {
            acts.push(Action::Dialog(Dialog::new_folder(&account, Some(&path))));
            ui.close();
        }
        if user_folder {
            if ui.button(tl!("Rename folder…")).clicked() {
                acts.push(Action::Dialog(Dialog::rename_folder(f)));
                ui.close();
            }
            if ui.button(tl!("Move folder…")).clicked() {
                acts.push(Action::Dialog(Dialog::move_folder(f)));
                ui.close();
            }
            if ui.button(RichText::new(tl!("Delete folder…")).color(t.danger)).clicked() {
                acts.push(Action::Dialog(Dialog::delete_folder(s, f)));
                ui.close();
            }
        }
        ui.separator();
        let label = tl!(if is_favorite { "Remove from Favorites" } else { "Show in Favorites" });
        if ui.button(label).clicked() {
            acts.push(Action::Run("folder.favorite", json!({"account": account, "folder": path, "on": !is_favorite})));
            ui.close();
        }
        ui.separator();
        if ui.button(tl!("Properties…")).clicked() {
            acts.push(Action::Dialog(Dialog::folder_properties(s, f)));
            ui.close();
        }
    });
}
