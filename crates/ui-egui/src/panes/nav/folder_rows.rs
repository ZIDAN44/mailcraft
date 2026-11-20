//! A folder's row in the folder pane: what's dropped on it, dragging it, and its menu.

use super::*;

/// Where a folder dragged over a row of one of the user's folders lands (issue #183): over the
/// row's top quarter, before it (`false`); its bottom quarter, after it (`true`); its middle,
/// inside it (none, issue #181).
fn place(ui: &Ui, rect: egui::Rect) -> Option<bool> {
    let y = ui.ctx().pointer_interact_pos()?.y;
    let edge = rect.height() / 4.0;
    if y < rect.top() + edge {
        Some(false)
    } else if y > rect.bottom() - edge {
        Some(true)
    } else {
        None
    }
}

/// The selection dropped on a folder: moved there, or copied with Ctrl held (issue #82). The
/// folder may be another account's (a drop there did nothing, and said nothing).
pub(super) fn dropped_mail(ui: &Ui, account: &str, folder: &str) -> Action {
    let copy = ui.input(|i| i.modifiers.command);
    Action::Run(if copy { "mail.copy" } else { "mail.move" }, json!({"folder": folder, "account": account}))
}

/// A folder's row; `collapse` is whether it's collapsed, for one with folders in it (it gets
/// an arrow).
pub(super) fn folder_row(
    s: &Session,
    t: &Tokens,
    ui: &mut Ui,
    acts: &mut Vec<Action>,
    f: &Folder,
    depth: usize,
    favorite: bool,
    collapse: Option<bool>,
) {
    let selected = s.search.is_none() && matches!(&s.view, View::Folder { account, folder } if *account == f.account && *folder == f.path);
    // In Favorites, a shared mailbox's folder says whose it is (its Inbox isn't the account's).
    let shared_top = f.mailbox.as_ref().and_then(|_| {
        let top = f.delimiter.and_then(|d| f.path.split(d).next()).unwrap_or(&f.path);
        s.folder(&f.account, top).map(crate::i18n::folder)
    });
    let name = match s.account(&f.account) {
        _ if favorite && let Some(top) = shared_top => format!("{} · {top}", crate::i18n::folder(f)),
        Some(a) if favorite && several_mailboxes(s) => {
            format!("{} · {}", crate::i18n::folder(f), a.config.email.split('@').next().unwrap_or_default())
        }
        _ => crate::i18n::folder(f),
    };
    // Unread counts matter for mail you receive, not for Sent or Drafts.
    let unread = if matches!(f.look(), FolderRole::Sent | FolderRole::Drafts | FolderRole::Trash | FolderRole::Archive) { 0 } else { f.unread };
    let user_folder = !f.is_fixed();
    // Drafts says how many it holds, as Outlook's (a saved draft is never unread).
    let count = if f.look() == FolderRole::Drafts { Count::Items(f.total) } else { Count::Unread(unread) };
    let resp = row(ui, t, role_icon(f.look()), &name, count, depth, RowIs { selected, drop_ok: f.selectable, draggable: user_folder && !favorite });
    if let Some(collapsed) = collapse {
        let indent = super::rows::sizes(ui).indent;
        let x = resp.rect.min.x + 10.0 + depth as f32 * indent - 14.0;
        let m = crate::bidi::Mirror::new(resp.rect);
        let chev = m.rect(egui::Rect::from_min_size(egui::pos2(x, resp.rect.center().y - 6.0), vec2(12.0, 12.0)));
        let c = ui.interact(chev.expand(4.0), resp.id.with("collapse"), Sense::click());
        crate::widgets::named(ui, &c, &format!("{name}: {}", tl!(if collapsed { "Expand" } else { "Collapse" })), true, None);
        icons::paint(ui.painter(), chev, if collapsed { m.closed() } else { Icon::ChevronDown }, if c.hovered() { t.text } else { t.dim });
        if c.clicked() {
            acts.push(Action::Run("view.collapse", json!({"key": folder_key(&f.account, &f.path)})));
        }
    }
    if f.selectable && resp.clicked() {
        acts.push(Action::Run("folder.open", json!({"account": f.account, "folder": f.path})));
    }
    if f.selectable && dropped::<DragKeys>(&resp).is_some() {
        acts.push(dropped_mail(ui, &f.account, &f.path));
    }
    // A folder over one of the user's: at its edges, put beside it (a line says where); over
    // the middle, inside it.
    let orderable = mailcraft_engine::model::is_orderable(f) && !favorite;
    let beside = if orderable { place(ui, resp.rect) } else { None };
    if let Some(after) = beside
        && resp.dnd_hover_payload::<DragFolder>().is_some_and(|p| p.account == f.account && p.path != f.path)
    {
        let y = if after { resp.rect.bottom() } else { resp.rect.top() };
        ui.painter().line_segment([egui::pos2(resp.rect.left() + 4.0, y), egui::pos2(resp.rect.right() - 4.0, y)], egui::Stroke::new(2.0, t.accent));
    }
    if let Some(p) = dropped::<DragFolder>(&resp)
        && p.account == f.account
        && p.path != f.path
    {
        acts.push(match beside {
            Some(after) => {
                let side = if after { "after" } else { "before" };
                Action::Run("folder.order", json!({"account": f.account, "folder": p.path, side: f.path}))
            }
            None => Action::Run("folder.move", json!({"account": f.account, "folder": p.path, "parent": f.path})),
        });
    }
    if user_folder && !favorite && resp.drag_started() {
        resp.dnd_set_drag_payload(DragFolder { account: f.account.clone(), path: f.path.clone() });
    }
    // The menu borrows the folder: nothing is copied or looked up unless it's open.
    let (account, path) = (&f.account, &f.path);
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
        // That folder, all of it, where it is: it isn't opened (issue #197).
        if ui.button(tl!("Mark all as read")).clicked() {
            acts.push(Action::Run("mail.markAllRead", json!({"account": account, "folder": path})));
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
            acts.push(Action::Dialog(Dialog::new_folder(account, Some(path))));
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
            // Its place among those beside it (issue #183), as Outlook's Move up and Move down.
            if orderable {
                for (by, label, id) in [(-1, "Move up", "folder.moveUp"), (1, "Move down", "folder.moveDown")] {
                    if ui.add_enabled(s.can_reorder_folder(account, path, by), egui::Button::new(tl!(label))).clicked() {
                        acts.push(Action::Run(id, json!({"account": account, "folder": path})));
                        ui.close();
                    }
                }
            }
            if ui.button(RichText::new(tl!("Delete folder…")).color(t.danger)).clicked() {
                acts.push(Action::Dialog(Dialog::delete_folder(s, f)));
                ui.close();
            }
        }
        ui.separator();
        let is_favorite = s.favorites().iter().any(|r| r.account == *account && r.folder == *path);
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
