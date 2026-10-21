//! The folder pane: Favorites, then each account's folders as a tree with unread counts, and
//! after it the shared mailboxes it opens, each a mailbox of its own as Outlook shows them.
//! Messages dragged from the list drop onto a folder to move them; a folder dragged onto another
//! moves inside it; an account dragged onto another takes its place. Right-click a folder to
//! create, rename, move, delete or pin it to Favorites. Favorites, accounts and folders with
//! folders in them collapse, and stay so from one start to the next.

use egui::{RichText, Sense, Ui, vec2};
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

/// One row (its label as shown, translated); returns its response.
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
    let ctx = ui.ctx().clone();
    let sel = crate::motion::toward(&ctx, resp.id.with("selected"), selected, crate::motion::NORMAL);
    let hover = crate::motion::toward(&ctx, resp.id.with("hover"), resp.hovered(), crate::motion::FAST);
    let fill = if hovering_drop { t.accent.gamma_multiply(0.3) } else { crate::motion::mix(t.hover_row.gamma_multiply(hover), t.accent_soft, sel) };
    ui.painter().rect_filled(rect, 6.0, fill);
    if sel > 0.0 {
        let full = h - 14.0;
        let bar = egui::Rect::from_center_size(egui::pos2(rect.min.x + 1.5, rect.center().y), vec2(3.0, full * sel));
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
        let favorites_open = !s.prefs.collapsed.contains("favorites");
        if section_header(ui, t, "Favorites", favorites_open) {
            acts.push(Action::Run("view.collapse", json!({"key": "favorites"})));
        }
        if favorites_open && s.accounts.len() > 1 {
            let unread: u32 = s.accounts.iter().filter_map(|a| a.folder(FolderRole::Inbox)).map(|f| f.unread).sum();
            let sel = s.view == View::Unified { role: FolderRole::Inbox } && s.search.is_none();
            if row(ui, t, Icon::Inbox, tl!("All Inboxes"), unread, 0, sel, false, false).clicked() {
                acts.push(Action::Run("folder.unified", json!({"role": "inbox"})));
            }
            let sel = s.view == View::Unified { role: FolderRole::Sent } && s.search.is_none();
            if row(ui, t, Icon::Sent, tl!("All Sent"), 0, 0, sel, false, false).clicked() {
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
            if row(ui, t, Icon::Outbox, tl!("Outbox"), n, 0, sel, false, false).on_hover_text(tl!("Messages waiting to be sent")).clicked() {
                acts.push(Action::Run("folder.outbox", json!({})));
            }
        }
        ui.add_space(8.0);
        // Accounts, and the shared mailboxes they open, each a mailbox of its own in the order
        // the user put them (issue #1).
        let sections = s.pane_sections();
        for (at, sec) in sections.iter().enumerate() {
            let Some(a) = s.account(&sec.account) else { continue };
            if let Some(top) = sec.mailbox.as_ref().and_then(|p| a.folders.iter().find(|f| &f.path == p)) {
                shared_section(s, t, ui, acts, a, top, &sections, at);
                continue;
            }
            let key = format!("account:{}", a.config.id);
            let open = !s.prefs.collapsed.contains(&key);
            // Dragged onto another mailbox, it takes its place (above or below it).
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click_and_drag());
            if resp.hovered() || resp.dragged() {
                ui.painter().rect_filled(rect, 6.0, t.hover_row);
            }
            section_drag(ui, t, &resp, rect, &sections, at, acts);
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
                // A Microsoft account with a password (refused there) switches to Microsoft sign-in.
                let switch = mailcraft_engine::model::microsoft_password_account(&config) && s.oauth_available.iter().any(|p| p == "microsoft");
                if !a.is_local() && ui.button(if switch { tl!("Sign in with Microsoft…") } else { tl!("Sign in again…") }).clicked() {
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
                    acts.push(Action::EditSignature { account: config.id.clone(), insert: false });
                    ui.close();
                }
                // Exchange: another mailbox opened beside this one, as Outlook's Open these
                // additional mailboxes.
                if config.ews.is_some() && ui.button(tl!("Add shared mailbox…")).clicked() {
                    acts.push(Action::Dialog(Dialog::AddSharedMailbox { account: config.id.clone(), address: String::new(), error: None }));
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
                // Its place among the mailboxes, as Outlook moves a mailbox up or down.
                move_items(ui, &sections, at, acts);
                ui.separator();
                if ui.button(RichText::new(tl!("Remove account…")).color(t.danger)).clicked() {
                    acts.push(Action::Dialog(Dialog::RemoveAccount { account: config.id.clone(), email: config.email.clone() }));
                    ui.close();
                }
            });
            // Folders with folders in them collapse; what's inside a collapsed one is hidden.
            let parents: std::collections::HashSet<String> = a.folders.iter().flat_map(ancestors).collect();
            let hidden = |f: &Folder| ancestors(f).iter().any(|up| s.prefs.collapsed.contains(&folder_key(&f.account, up)));
            if open {
                if a.folders.is_empty() {
                    ui.label(RichText::new(tl!("  Loading folders…")).color(t.faint).small());
                }
                for f in a.folders.iter().filter(|f| f.mailbox.is_none()) {
                    if hidden(f) {
                        continue;
                    }
                    let collapse = parents.contains(&f.path).then(|| s.prefs.collapsed.contains(&folder_key(&f.account, &f.path)));
                    folder_row(s, t, ui, acts, f, f.depth() + 1, false, collapse);
                }
                ui.add_space(6.0);
            }
        }
        search_folders(s, t, ui, acts);
    });
}

/// Search Folders, after the mailboxes: each a saved search, opened with a click; New Search
/// Folder from the heading's menu (or the row when there are none).
fn search_folders(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    if s.accounts.is_empty() {
        return;
    }
    let open = !s.prefs.collapsed.contains("searchFolders");
    ui.add_space(4.0);
    let new = |ui: &mut Ui, acts: &mut Vec<Action>| {
        if ui.button(tl!("New Search Folder…")).clicked() {
            acts.push(Action::Dialog(Dialog::SearchFolder(Box::new(crate::dialogs::SearchFolderForm::new()))));
            ui.close();
        }
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover_row);
    }
    let chev = egui::Rect::from_min_size(egui::pos2(rect.min.x + 2.0, rect.center().y - 6.0), vec2(12.0, 12.0));
    icons::paint(ui.painter(), chev, if open { Icon::ChevronDown } else { Icon::ChevronRight }, t.faint);
    ui.painter().text(egui::pos2(rect.min.x + 18.0, rect.center().y), egui::Align2::LEFT_CENTER, tl!("Search Folders"), theme::bold(12.0), t.faint);
    if resp.clicked() {
        acts.push(Action::Run("view.collapse", json!({"key": "searchFolders"})));
    }
    resp.context_menu(|ui| new(ui, acts));
    if !open {
        return;
    }
    if s.prefs.search_folders.is_empty() {
        if row(ui, t, Icon::Plus, tl!("New Search Folder…"), 0, 0, false, false, false).clicked() {
            acts.push(Action::Dialog(Dialog::SearchFolder(Box::new(crate::dialogs::SearchFolderForm::new()))));
        }
        return;
    }
    let showing = s.open_search_folder_name();
    for f in &s.prefs.search_folders {
        let selected = showing == Some(f.name.as_str());
        let r = row(ui, t, Icon::Search, &f.name, 0, 0, selected, false, false).on_hover_text(&f.query);
        if r.clicked() {
            acts.push(Action::Run("searchFolder.open", json!({"name": f.name})));
        }
        r.context_menu(|ui| {
            if ui.button(tl!("Customize this Search Folder…")).clicked() {
                acts.push(Action::Dialog(Dialog::SearchFolder(Box::new(crate::dialogs::SearchFolderForm::edit(f)))));
                ui.close();
            }
            new(ui, acts);
            ui.separator();
            // Only the search goes; its mail stays where it is, so no question is asked.
            if ui.button(RichText::new(tl!("Delete Search Folder")).color(t.danger)).clicked() {
                acts.push(Action::Run("searchFolder.delete", json!({"name": f.name})));
                ui.close();
            }
        });
    }
}

/// A shared mailbox an account opens, as a mailbox of its own (issue #1): its top folder `top`
/// is its heading, its folders are under it as an account's are, and it moves among the
/// mailboxes as an account does.
#[allow(clippy::too_many_arguments)]
fn shared_section(
    s: &Session,
    t: &Tokens,
    ui: &mut Ui,
    acts: &mut Vec<Action>,
    a: &mailcraft_engine::AccountState,
    top: &Folder,
    sections: &[mailcraft_engine::PaneSection],
    at: usize,
) {
    let key = folder_key(&a.config.id, &top.path);
    let open = !s.prefs.collapsed.contains(&key);
    if shared_header(t, ui, acts, a, top, open, sections, at) {
        acts.push(Action::Run("view.collapse", json!({"key": key})));
    }
    if !open {
        return;
    }
    let parents: std::collections::HashSet<String> = a.folders.iter().flat_map(ancestors).collect();
    let hidden = |f: &Folder| ancestors(f).iter().any(|up| s.prefs.collapsed.contains(&folder_key(&f.account, up)));
    for f in a.folders.iter().filter(|f| f.mailbox == top.mailbox && f.depth() > 0) {
        if hidden(f) {
            continue;
        }
        let collapse = parents.contains(&f.path).then(|| s.prefs.collapsed.contains(&folder_key(&f.account, &f.path)));
        folder_row(s, t, ui, acts, f, f.depth(), false, collapse);
    }
    ui.add_space(6.0);
}

/// A mailbox's heading dragged onto another's: it takes its place (above or below it).
fn section_drag(
    ui: &Ui,
    t: &Tokens,
    resp: &egui::Response,
    rect: egui::Rect,
    sections: &[mailcraft_engine::PaneSection],
    at: usize,
    acts: &mut Vec<Action>,
) {
    let Some(me) = sections.get(at) else { return };
    if resp.drag_started() {
        resp.dnd_set_drag_payload(DragAccount(me.key.clone()));
    }
    let below = ui.ctx().pointer_interact_pos().is_some_and(|p| p.y > rect.center().y);
    if resp.dnd_hover_payload::<DragAccount>().is_some_and(|d| d.0 != me.key) {
        let y = if below { rect.bottom() } else { rect.top() };
        ui.painter().line_segment([egui::pos2(rect.left() + 4.0, y), egui::pos2(rect.right() - 4.0, y)], egui::Stroke::new(2.0, t.accent));
    }
    if let Some(dragged) = resp.dnd_release_payload::<DragAccount>()
        && let Some(from) = sections.iter().position(|x| x.key == dragged.0)
        && dragged.0 != me.key
    {
        let before = if below { at + 1 } else { at };
        let to = if from < before { before - 1 } else { before };
        if to != from {
            acts.push(Action::Run("folderPane.move", json!({"key": dragged.0, "by": to as i64 - from as i64})));
        }
    }
}

/// Move up and Move down in a mailbox's menu.
fn move_items(ui: &mut Ui, sections: &[mailcraft_engine::PaneSection], at: usize, acts: &mut Vec<Action>) {
    let Some(me) = sections.get(at) else { return };
    if sections.len() < 2 {
        return;
    }
    ui.separator();
    if ui.add_enabled(at > 0, egui::Button::new(tl!("Move up"))).clicked() {
        acts.push(Action::Run("folderPane.move", json!({"key": me.key, "by": -1})));
        ui.close();
    }
    if ui.add_enabled(at + 1 < sections.len(), egui::Button::new(tl!("Move down"))).clicked() {
        acts.push(Action::Run("folderPane.move", json!({"key": me.key, "by": 1})));
        ui.close();
    }
}

/// A shared mailbox's heading (its top folder `top`): its name, an arrow that collapses it, and
/// a menu. Whether it was clicked.
#[allow(clippy::too_many_arguments)]
fn shared_header(
    t: &Tokens,
    ui: &mut Ui,
    acts: &mut Vec<Action>,
    a: &mailcraft_engine::AccountState,
    top: &Folder,
    open: bool,
    sections: &[mailcraft_engine::PaneSection],
    at: usize,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click_and_drag());
    section_drag(ui, t, &resp, rect, sections, at, acts);
    let dropping = resp.dnd_hover_payload::<DragKeys>().is_some() || resp.dnd_hover_payload::<DragFolder>().is_some();
    if resp.hovered() || dropping {
        ui.painter().rect_filled(rect, 6.0, if dropping { t.accent.gamma_multiply(0.3) } else { t.hover_row });
    }
    let chev = egui::Rect::from_min_size(rect.min + vec2(6.0, 9.0), vec2(12.0, 12.0));
    icons::paint(ui.painter(), chev, if open { Icon::ChevronDown } else { Icon::ChevronRight }, t.dim);
    let ir = egui::Rect::from_min_size(rect.min + vec2(20.0, 8.0), vec2(14.0, 14.0));
    icons::paint(ui.painter(), ir, Icon::People, t.dim);
    let name = crate::i18n::folder(top);
    // Its unread mail, as Outlook shows a collapsed mailbox's Inbox.
    let unread: u32 =
        if open { 0 } else { a.folders.iter().filter(|f| f.mailbox == top.mailbox && f.mailbox_role == FolderRole::Inbox).map(|f| f.unread).sum() };
    let right = rect.max.x - if unread > 0 { 40.0 } else { 8.0 };
    ui.painter().with_clip_rect(egui::Rect::from_x_y_ranges(rect.min.x..=right, rect.y_range())).text(
        rect.min + vec2(40.0, 15.0),
        egui::Align2::LEFT_CENTER,
        &name,
        theme::bold(13.0),
        t.text,
    );
    if unread > 0 {
        ui.painter().text(
            egui::pos2(rect.max.x - 10.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            unread.to_string(),
            theme::bold(12.5),
            t.accent,
        );
    }
    let address = top.mailbox.clone().unwrap_or_default();
    let resp = resp.on_hover_text(tf!("Shared mailbox {address}, opened by {email}", address = address, email = a.config.email));
    // Dropped messages or folders go into its top folder, as onto a folder.
    if let Some(p) = resp.dnd_release_payload::<DragKeys>()
        && p.0.iter().all(|k| k.account == top.account)
    {
        acts.push(Action::Run("mail.move", json!({"folder": top.path})));
    }
    if let Some(p) = resp.dnd_release_payload::<DragFolder>()
        && p.account == top.account
    {
        acts.push(Action::Run("folder.move", json!({"account": top.account, "folder": p.path, "parent": top.path})));
    }
    let added = a.config.shared_mailboxes.iter().any(|m| m.eq_ignore_ascii_case(&address));
    let account = a.config.id.clone();
    let path = top.path.clone();
    resp.context_menu(|ui| {
        if ui.button(tl!("Send / Receive")).clicked() {
            acts.push(Action::Run("mail.sync", json!({})));
            ui.close();
        }
        if ui.button(tl!("New folder…")).clicked() {
            acts.push(Action::Dialog(Dialog::new_folder(&account, Some(&path))));
            ui.close();
        }
        ui.separator();
        // One added by hand is removed here; one the server opens is the server's to stop.
        if added {
            if ui.button(RichText::new(tl!("Remove shared mailbox")).color(t.danger)).clicked() {
                acts.push(Action::Run("account.removeSharedMailbox", json!({"account": account, "address": address})));
                ui.close();
            }
        } else {
            ui.add_enabled(false, egui::Button::new(tl!("Opened by the server for your access to it")));
        }
        if ui.button(tl!("Add shared mailbox…")).clicked() {
            acts.push(Action::Dialog(Dialog::AddSharedMailbox { account: account.clone(), address: String::new(), error: None }));
            ui.close();
        }
        move_items(ui, sections, at, acts);
    });
    resp.clicked()
}

/// A folder's row; `collapse` is whether it's collapsed, for one with folders in it (it gets
/// an arrow).
#[allow(clippy::too_many_arguments)]
fn folder_row(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>, f: &Folder, depth: usize, favorite: bool, collapse: Option<bool>) {
    let selected = s.search.is_none() && matches!(&s.view, View::Folder { account, folder } if *account == f.account && *folder == f.path);
    // In Favorites, a shared mailbox's folder says whose it is (its Inbox isn't the account's).
    let shared_top = f.mailbox.as_ref().and_then(|_| {
        let top = f.delimiter.and_then(|d| f.path.split(d).next()).unwrap_or(&f.path);
        s.folder(&f.account, top).map(crate::i18n::folder)
    });
    let name = match s.account(&f.account) {
        _ if favorite && let Some(top) = shared_top => format!("{} · {top}", crate::i18n::folder(f)),
        Some(a) if favorite && s.accounts.len() > 1 => {
            format!("{} · {}", crate::i18n::folder(f), a.config.email.split('@').next().unwrap_or_default())
        }
        _ => crate::i18n::folder(f),
    };
    // Unread counts matter for mail you receive, not for Sent or Drafts.
    let unread = if matches!(f.look(), FolderRole::Sent | FolderRole::Drafts | FolderRole::Trash | FolderRole::Archive) { 0 } else { f.unread };
    let user_folder = !f.is_fixed();
    let resp = row(ui, t, role_icon(f.look()), &name, unread, depth, selected, f.selectable, user_folder && !favorite);
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
                name: crate::i18n::folder(f),
                for_good: matches!(f.look(), FolderRole::Trash | FolderRole::Junk),
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
