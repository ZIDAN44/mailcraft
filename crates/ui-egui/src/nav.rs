//! The folder pane: Favorites, then each account's folders as a tree with unread counts.
//! Messages dragged from the list drop onto a folder to move them.

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

/// One row; returns its response.
fn row(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, unread: u32, depth: usize, selected: bool, drop_ok: bool) -> egui::Response {
    let h = 30.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let hovering_drop = drop_ok && resp.dnd_hover_payload::<DragKeys>().is_some();
    let fill = if hovering_drop {
        t.accent.gamma_multiply(0.3)
    } else if selected {
        t.accent_soft
    } else if resp.hovered() {
        t.hover
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
    let galley = ui.painter().layout(label.to_string(), font, t.text, (text_right - x - 24.0).max(10.0));
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
        section(ui, t, "Favorites");
        if s.accounts.len() > 1 {
            let unread: u32 = s.accounts.iter().filter_map(|a| a.folder(FolderRole::Inbox)).map(|f| f.unread).sum();
            let sel = s.view == View::Unified { role: FolderRole::Inbox } && s.search.is_none();
            if row(ui, t, Icon::Inbox, "All Inboxes", unread, 0, sel, false).clicked() {
                acts.push(Action::Run("folder.unified", json!({"role": "inbox"})));
            }
            let sel = s.view == View::Unified { role: FolderRole::Sent } && s.search.is_none();
            if row(ui, t, Icon::Sent, "All Sent", 0, 0, sel, false).clicked() {
                acts.push(Action::Run("folder.unified", json!({"role": "sent"})));
            }
        } else if let Some(a) = s.accounts.first() {
            for role in [FolderRole::Inbox, FolderRole::Sent, FolderRole::Drafts] {
                if let Some(f) = a.folder(role) {
                    folder_row(s, t, ui, acts, f, 0, true);
                }
            }
        }
        ui.add_space(8.0);
        for a in &s.accounts {
            let id = ui.make_persistent_id(("acct", &a.config.id));
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
            let open = state.is_open();
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 6.0, t.hover);
            }
            let chev = egui::Rect::from_min_size(rect.min + vec2(6.0, 9.0), vec2(12.0, 12.0));
            icons::paint(ui.painter(), chev, if open { Icon::ChevronDown } else { Icon::ChevronRight }, t.dim);
            ui.painter().circle_filled(rect.min + vec2(26.0, 15.0), 4.0, theme::account_color(a.config.color));
            let label = if a.is_local() { format!("{} (sample)", a.config.email) } else { a.config.email.clone() };
            ui.painter().with_clip_rect(rect).text(rect.min + vec2(36.0, 15.0), egui::Align2::LEFT_CENTER, label, theme::bold(13.0), t.text);
            if resp.clicked() {
                state.toggle(ui);
            }
            let config = a.config.clone();
            resp.context_menu(|ui| {
                if ui.button("Send / Receive").clicked() {
                    acts.push(Action::Run("mail.sync", json!({})));
                    ui.close();
                }
                if !a.is_local() && ui.button("Sign in again…").clicked() {
                    acts.push(Action::Dialog(Dialog::password(&config)));
                    ui.close();
                }
                if ui.button("Signature…").clicked() {
                    acts.push(Action::Dialog(Dialog::Signature { account: config.id.clone(), text: config.signature.clone() }));
                    ui.close();
                }
                ui.separator();
                if ui.button(RichText::new("Remove account…").color(t.danger)).clicked() {
                    acts.push(Action::Dialog(Dialog::RemoveAccount { account: config.id.clone(), email: config.email.clone() }));
                    ui.close();
                }
            });
            state.store(ui.ctx());
            if !open {
                continue;
            }
            if a.folders.is_empty() {
                ui.label(RichText::new("  Loading folders…").color(t.faint).small());
            }
            for f in &a.folders {
                folder_row(s, t, ui, acts, f, f.depth() + 1, false);
            }
            ui.add_space(6.0);
        }
    });
}

fn section(ui: &mut Ui, t: &Tokens, label: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(label).font(theme::bold(12.0)).color(t.faint));
}

fn folder_row(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>, f: &Folder, depth: usize, favorite: bool) {
    let selected = s.search.is_none() && matches!(&s.view, View::Folder { account, folder } if *account == f.account && *folder == f.path);
    let name = if favorite && s.accounts.len() > 1 { format!("{} ({})", f.display_name(), f.account) } else { f.display_name() };
    // Unread counts matter for mail you receive, not for Sent or Drafts.
    let unread = if matches!(f.role, FolderRole::Sent | FolderRole::Drafts | FolderRole::Trash | FolderRole::Archive) { 0 } else { f.unread };
    let resp = row(ui, t, role_icon(f.role), &name, unread, depth, selected, f.selectable);
    if f.selectable && resp.clicked() {
        acts.push(Action::Run("folder.open", json!({"account": f.account, "folder": f.path})));
    }
    if f.selectable
        && let Some(p) = resp.dnd_release_payload::<DragKeys>()
        && p.0.iter().all(|k| k.account == f.account)
    {
        acts.push(Action::Run("mail.move", json!({"folder": f.path})));
    }
    let path = f.path.clone();
    let account = f.account.clone();
    resp.context_menu(|ui| {
        if ui.button("Open").clicked() {
            acts.push(Action::Run("folder.open", json!({"account": account, "folder": path})));
            ui.close();
        }
        if ui.button("Update folder").clicked() {
            acts.push(Action::Run("folder.open", json!({"account": account, "folder": path})));
            acts.push(Action::Run("folder.sync", json!({})));
            ui.close();
        }
        if ui.button("Mark all as read").clicked() {
            acts.push(Action::Run("folder.open", json!({"account": account, "folder": path})));
            acts.push(Action::Run("mail.markAllRead", json!({})));
            ui.close();
        }
    });
}
