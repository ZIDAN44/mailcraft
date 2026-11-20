//! The folder pane's mailboxes: a shared mailbox's section and heading, and a mailbox's place
//! among the others (dragged, or Move up and Move down).

use super::*;

/// A shared mailbox an account opens, as a mailbox of its own (issue #1): its top folder `top`
/// is its heading, its folders are under it as an account's are, and it moves among the
/// mailboxes as an account does.
pub(super) fn shared_section(
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
    let tree = Tree::new(s, a);
    for f in a.folders.iter().filter(|f| f.mailbox == top.mailbox && f.depth() > 0) {
        if tree.hidden(f) {
            continue;
        }
        folder_row(s, t, ui, acts, f, f.depth(), false, tree.collapse(f));
    }
    ui.add_space(6.0);
}

/// A mailbox's heading dragged onto another's: it takes its place (above or below it).
pub(super) fn section_drag(
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
    if let Some(dragged) = dropped::<DragAccount>(resp)
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
pub(super) fn move_items(ui: &mut Ui, sections: &[mailcraft_engine::PaneSection], at: usize, acts: &mut Vec<Action>) {
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
pub(super) fn shared_header(
    t: &Tokens,
    ui: &mut Ui,
    acts: &mut Vec<Action>,
    a: &mailcraft_engine::AccountState,
    top: &Folder,
    open: bool,
    sections: &[mailcraft_engine::PaneSection],
    at: usize,
) -> bool {
    let size = super::rows::sizes(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), size.row), Sense::click_and_drag());
    let inset = corner_inset(ui, rect);
    section_drag(ui, t, &resp, rect, sections, at, acts);
    let dropping = resp.dnd_hover_payload::<DragKeys>().is_some() || resp.dnd_hover_payload::<DragFolder>().is_some();
    if resp.hovered() || dropping {
        ui.painter().rect_filled(rect, 6.0, if dropping { t.accent.gamma_multiply(0.3) } else { t.hover_row });
    }
    let m = crate::bidi::Mirror::new(rect);
    let chev = m.rect(egui::Rect::from_min_size(egui::pos2(rect.min.x + 6.0, rect.center().y - 6.0), vec2(12.0, 12.0)));
    icons::paint(ui.painter(), chev, if open { Icon::ChevronDown } else { m.closed() }, t.dim);
    let ir = m.rect(egui::Rect::from_min_size(egui::pos2(rect.min.x + 20.0, rect.center().y - 7.0), vec2(14.0, 14.0)));
    icons::paint(ui.painter(), ir, Icon::People, t.dim);
    let name = crate::i18n::folder(top);
    crate::widgets::named(ui, &resp, &name, true, Some(open));
    // Its unread mail, as Outlook shows a collapsed mailbox's Inbox.
    let unread: u32 =
        if open { 0 } else { a.folders.iter().filter(|f| f.mailbox == top.mailbox && f.mailbox_role == FolderRole::Inbox).map(|f| f.unread).sum() };
    let right = rect.max.x - inset - if unread > 0 { 40.0 } else { 8.0 };
    let g = crate::widgets::one_line(ui, &name, theme::bold(size.heading), t.text, right - rect.min.x - 40.0);
    ui.painter().galley(egui::pos2(m.start(rect.min.x + 40.0, g.size().x), rect.center().y - g.size().y / 2.0), g, t.text);
    if unread > 0 {
        ui.painter().text(
            m.pos(egui::pos2(rect.max.x - 10.0 - inset, rect.center().y)),
            m.align(egui::Align2::RIGHT_CENTER),
            unread.to_string(),
            theme::bold(12.5),
            t.accent,
        );
    }
    let address = top.mailbox.clone().unwrap_or_default();
    let resp = resp.on_hover_text(tf!("Shared mailbox {address}, opened by {email}", address = address, email = a.config.email));
    // Dropped messages or folders go into its top folder, as onto a folder.
    if dropped::<DragKeys>(&resp).is_some() {
        acts.push(dropped_mail(ui, &top.account, &top.path));
    }
    if let Some(p) = dropped::<DragFolder>(&resp)
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
