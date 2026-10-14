//! The message list: date headings, rows with avatar, sender, subject and preview, unread and
//! flag markers, hover actions, multi-select (Ctrl/Shift), drag to a folder, right-click menu.
//! Only visible rows are laid out, so long folders scroll smoothly.

use egui::{Color32, Rect, RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::{FolderRole, MessageKey, Summary, addr, dates};
use mailcraft_engine::{ListFilter, Row, SearchScope, Session};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme;
use crate::widgets;
use crate::{Action, Dialog, UiState};

/// What a dragged row carries.
pub struct DragKeys(pub Vec<MessageKey>);

const HEADER_H: f32 = 30.0;

fn row_height(s: &Session) -> f32 {
    match (s.prefs.compact, s.prefs.show_preview) {
        (true, _) => 46.0,
        (false, true) => 76.0,
        (false, false) => 58.0,
    }
}

pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 10, top: 10, bottom: 6 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(crate::view_title(s)).font(theme::bold(18.0)).color(t.text));
        });
        ui.add_space(4.0);
        if s.focus_split() {
            ui.horizontal(|ui| {
                for (other, label) in [(false, "Focused"), (true, "Other")] {
                    let on = s.focus_other == other;
                    let text = RichText::new(tl!(label)).color(if on { t.text } else { t.dim }).font(if on {
                        theme::bold(14.0)
                    } else {
                        theme::regular(14.0)
                    });
                    let r = ui.add(egui::Button::new(text).frame(false));
                    if on {
                        ui.painter().line_segment(
                            [pos2(r.rect.left() + 2.0, r.rect.bottom() + 1.0), pos2(r.rect.right() - 2.0, r.rect.bottom() + 1.0)],
                            egui::Stroke::new(2.0, t.accent),
                        );
                    }
                    if r.clicked() && !on {
                        acts.push(Action::Run("list.focusTab", json!({"value": if other { "other" } else { "focused" }})));
                    }
                }
            });
            ui.add_space(2.0);
        }
        ui.horizontal(|ui| {
            for f in ListFilter::ALL {
                let on = s.filter == f;
                let text = RichText::new(tl!(f.label())).color(if on { t.accent } else { t.dim }).font(if on {
                    theme::bold(12.5)
                } else {
                    theme::regular(12.5)
                });
                let r = ui.add(egui::Button::new(text).frame(false));
                if on {
                    ui.painter().line_segment(
                        [pos2(r.rect.left() + 4.0, r.rect.bottom()), pos2(r.rect.right() - 4.0, r.rect.bottom())],
                        egui::Stroke::new(2.0, t.accent),
                    );
                }
                if r.clicked() {
                    acts.push(Action::Run("list.filter", json!({"value": f.code()})));
                }
            }
            if let Some((q, scope)) = &s.search {
                ui.add_space(8.0);
                let all = *scope == SearchScope::All;
                if ui.selectable_label(all, RichText::new(tl!("All folders")).small()).clicked() && !all {
                    acts.push(Action::Run("search.run", json!({"query": q, "scope": "all"})));
                }
                if ui.selectable_label(!all, RichText::new(tl!("This folder")).small()).clicked() && all {
                    acts.push(Action::Run("search.run", json!({"query": q, "scope": "current"})));
                }
            }
        });
    });
    let sep = ui.available_rect_before_wrap();
    ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));

    let rows = s.rows();
    if rows.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(60.0);
            let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            icons::paint(ui.painter(), r, if s.search.is_some() { Icon::Search } else { Icon::Inbox }, t.faint);
            ui.add_space(8.0);
            let msg = if s.search.is_some() {
                tl!("No results. Try fewer words, or search all folders.")
            } else if s.filter != ListFilter::All {
                tl!("Nothing matches this filter.")
            } else {
                tl!("All caught up: nothing in this folder.")
            };
            ui.label(RichText::new(msg).color(t.dim));
        });
        return;
    }

    let mh = row_height(s);
    let mut tops = Vec::with_capacity(rows.len());
    let mut y = 0.0;
    for r in &rows {
        tops.push(y);
        y += if matches!(r, Row::Header(_)) { HEADER_H } else { mh };
    }
    let total = y;
    let focus_index = s.focus.as_ref().and_then(|k| s.list.iter().position(|m| &m.key == k));
    let scroll_to = if st.scroll_to_focus {
        st.scroll_to_focus = false;
        focus_index.and_then(|fi| rows.iter().position(|r| matches!(r, Row::Message { index, .. } if *index == fi)))
    } else {
        None
    };
    let sent_view = matches!(s.view_role(), Some(FolderRole::Sent | FolderRole::Drafts)) || s.view == mailcraft_engine::View::Outbox;

    egui::ScrollArea::vertical().auto_shrink(false).id_salt("list").show_viewport(ui, |ui, viewport| {
        ui.set_height(total);
        let origin = ui.min_rect().min;
        let width = ui.available_width();
        if let Some(ri) = scroll_to
            && let Some(&top) = tops.get(ri)
        {
            let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, mh));
            ui.scroll_to_rect(r, None);
        }
        let first = tops.partition_point(|&top| top + mh < viewport.min.y);
        for (i, row) in rows.iter().enumerate().skip(first) {
            let top = tops.get(i).copied().unwrap_or(0.0);
            if top > viewport.max.y {
                break;
            }
            match row {
                Row::Header(label) => {
                    let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, HEADER_H));
                    ui.painter().text(r.left_bottom() + vec2(14.0, -8.0), egui::Align2::LEFT_BOTTOM, tl!(label), theme::bold(12.5), t.dim);
                }
                Row::Message { index, count, unread } => {
                    let Some(m) = s.list.get(*index) else { continue };
                    let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, mh));
                    message_row(s, &t, ui, r, m, *count, *unread, sent_view, acts);
                }
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn message_row(
    s: &Session,
    t: &theme::Tokens,
    ui: &mut Ui,
    r: Rect,
    m: &Summary,
    count: usize,
    unread_in_thread: usize,
    sent_view: bool,
    acts: &mut Vec<Action>,
) {
    let id = ui.make_persistent_id(("row", &m.key));
    let resp = ui.interact(r, id, Sense::click_and_drag());
    let selected = s.selection.contains(&m.key);
    let unread = if count > 1 { unread_in_thread > 0 } else { !m.flags.seen };
    let p = ui.painter_at(r);
    let fill = if selected {
        t.accent_soft
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    p.rect_filled(r.shrink2(vec2(4.0, 1.0)), 6.0, fill);
    if unread {
        p.rect_filled(Rect::from_min_size(r.min + vec2(4.0, 8.0), vec2(3.0, r.height() - 16.0)), 2.0, t.accent);
    }
    p.line_segment([r.left_bottom() + vec2(14.0, 0.0), r.right_bottom() - vec2(8.0, 0.0)], egui::Stroke::new(1.0, t.border.gamma_multiply(0.6)));

    let compact = s.prefs.compact;
    let mut x = r.min.x + 14.0;
    let who_addr = if sent_view { m.to.first().cloned().unwrap_or_default() } else { m.from.clone() };
    if !compact {
        let c = egui::pos2(x + 17.0, r.min.y + 26.0);
        widgets::avatar(ui, c, 17.0, &who_addr.initials(), theme::avatar_color(&who_addr.email, t.dark));
        x += 44.0;
    }
    let right = r.max.x - 14.0;
    let line1 = r.min.y + if compact { 6.0 } else { 9.0 };
    // Sender (or recipients in Sent) and time.
    let time = crate::i18n::date(&dates::list_time(m.date, &s.clock));
    let time_g = p.layout_no_wrap(time, if unread { theme::bold(12.0) } else { theme::regular(12.0) }, if unread { t.accent } else { t.dim });
    let tw = time_g.size().x;
    p.galley(egui::pos2(right - tw, line1 + 1.0), time_g, t.dim);
    let who = if sent_view { tf!("To: {names}", names = addr::display_list(&m.to)) } else { m.from.display().to_string() };
    let who = if count > 1 { format!("{who}  ({count})") } else { who };
    let who_g = p.layout(who, if unread { theme::bold(14.0) } else { theme::regular(14.0) }, t.text, (right - tw - 10.0 - x).max(20.0));
    p.with_clip_rect(Rect::from_x_y_ranges(x..=right - tw - 8.0, r.y_range())).galley(egui::pos2(x, line1), who_g, t.text);

    // Subject with markers.
    let line2 = line1 + 19.0;
    let mut icons_w = 0.0;
    let marker = |ic: Icon, color: Color32, w: &mut f32| {
        *w += 18.0;
        let ir = Rect::from_min_size(egui::pos2(right - *w + 3.0, line2 + 1.0), vec2(14.0, 14.0));
        icons::paint(&p, ir, ic, color);
    };
    let hovered = resp.hovered();
    if !hovered {
        if m.flags.pinned() {
            marker(Icon::Pin, t.accent, &mut icons_w);
        }
        if m.flags.flagged {
            marker(Icon::Flag, t.flag, &mut icons_w);
        }
        if m.has_attachments {
            marker(Icon::Attach, t.dim, &mut icons_w);
        }
        if m.flags.answered {
            marker(Icon::Reply, t.faint, &mut icons_w);
        } else if m.flags.forwarded {
            marker(Icon::Forward, t.faint, &mut icons_w);
        }
    }
    // Categories: a coloured dot each, before the markers.
    if !hovered {
        for c in s.categories_of(m).into_iter().rev().take(4) {
            icons_w += 13.0;
            p.circle_filled(egui::pos2(right - icons_w + 7.0, line2 + 8.0), 5.0, theme::category_color(c.color, t.dark));
        }
    }
    let subject = if m.subject.is_empty() { tl!("(no subject)").to_string() } else { m.subject.clone() };
    let subj_color = if unread { t.accent } else { t.text };
    let subj_g = p.layout_no_wrap(subject, if unread { theme::bold(13.0) } else { theme::regular(13.0) }, subj_color);
    let subj_right = right - icons_w.max(if hovered { 70.0 } else { 0.0 }) - 4.0;
    p.with_clip_rect(Rect::from_x_y_ranges(x..=subj_right, r.y_range())).galley(egui::pos2(x, line2), subj_g, subj_color);
    if s.prefs.show_preview && !compact && !m.preview.is_empty() {
        let pv = p.layout_no_wrap(m.preview.clone(), theme::regular(12.5), t.dim);
        p.with_clip_rect(Rect::from_x_y_ranges(x..=right, r.y_range())).galley(egui::pos2(x, line2 + 19.0), pv, t.dim);
    }

    // Hover actions.
    let mut clicked_action = false;
    if hovered {
        let mut bx = right - 22.0;
        let actions: [(Icon, &str, &'static str, serde_json::Value); 3] = [
            (Icon::Delete, "Delete", "mail.delete", json!({})),
            (Icon::Flag, if m.flags.flagged { "Unflag" } else { "Flag" }, "mail.flag", json!({})),
            (
                if m.flags.seen { Icon::Unread } else { Icon::Read },
                tl!(if m.flags.seen { "Mark unread" } else { "Mark read" }),
                "mail.toggleRead",
                json!({}),
            ),
        ];
        for (ic, tip, cmd, params) in actions {
            let br = Rect::from_min_size(egui::pos2(bx, line2 - 3.0), vec2(22.0, 22.0));
            let bresp = ui.interact(br, id.with(cmd), Sense::click()).on_hover_text(tip);
            if bresp.hovered() {
                ui.painter().rect_filled(br, 4.0, t.border);
            }
            let color = if cmd == "mail.flag" && m.flags.flagged { t.flag } else { t.dim };
            icons::paint(ui.painter(), icons::centered(br, 14.0), ic, color);
            if bresp.clicked() {
                clicked_action = true;
                if !selected {
                    acts.push(Action::Run("list.select", json!({"key": m.key.to_id()})));
                }
                acts.push(Action::Run(cmd, params));
            }
            bx -= 24.0;
        }
    }

    if resp.clicked() && !clicked_action {
        let mods = ui.input(|i| i.modifiers);
        if mods.command || mods.ctrl {
            acts.push(Action::Run("list.select", json!({"key": m.key.to_id(), "add": true})));
        } else if mods.shift {
            range_select(s, &m.key, acts);
        } else {
            acts.push(Action::Open(m.key.clone()));
        }
    }
    if resp.double_clicked() && s.view_role() == Some(FolderRole::Drafts) {
        acts.push(Action::Open(m.key.clone()));
    }
    if resp.drag_started() {
        if !selected {
            acts.push(Action::Run("list.select", json!({"key": m.key.to_id()})));
        }
        let keys = if selected { s.selection.clone() } else { vec![m.key.clone()] };
        resp.dnd_set_drag_payload(DragKeys(keys));
    }
    if resp.dragged() {
        let n = if selected { s.selection.len() } else { 1 };
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), id.with("drag"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(if n == 1 { tl!("Move 1 message").to_string() } else { tf!("Move {n} messages", n = n) });
        });
    }
    let key = m.key.clone();
    let flags = m.flags.clone();
    resp.context_menu(|ui| {
        if !s.selection.contains(&key) {
            acts.push(Action::Run("list.select", json!({"key": key.to_id()})));
        }
        let mut item = |ui: &mut Ui, label: &str, act: Action| {
            if ui.button(label).clicked() {
                acts.push(act);
                ui.close();
            }
        };
        item(ui, tl!("Reply"), Action::Run("mail.reply", json!({})));
        item(ui, tl!("Reply all"), Action::Run("mail.replyAll", json!({})));
        item(ui, tl!("Forward"), Action::Run("mail.forward", json!({})));
        ui.separator();
        item(
            ui,
            tl!(if flags.seen { "Mark as unread" } else { "Mark as read" }),
            Action::Run(if flags.seen { "mail.markUnread" } else { "mail.markRead" }, json!({})),
        );
        item(ui, tl!(if flags.flagged { "Unflag" } else { "Flag" }), Action::Run("mail.flag", json!({})));
        ui.separator();
        item(ui, tl!("Move to…"), Action::Dialog(Dialog::move_to()));
        item(ui, tl!("Archive"), Action::Run("mail.archive", json!({})));
        item(ui, tl!("Report junk"), Action::Run("mail.junk", json!({})));
        item(ui, tl!("Delete"), Action::Run("mail.delete", json!({})));
        ui.separator();
        crate::organize::context_items(ui, s, t, acts, flags.pinned());
    });
    // Reply actions need the message open.
    if resp.secondary_clicked() && !s.reading.as_ref().is_some_and(|r| r.key == m.key) {
        acts.push(Action::Open(m.key.clone()));
    }
}

fn range_select(s: &Session, to: &MessageKey, acts: &mut Vec<Action>) {
    let anchor = s.focus.clone().or_else(|| s.selection.first().cloned());
    let Some(anchor) = anchor else {
        acts.push(Action::Run("list.select", json!({"key": to.to_id()})));
        return;
    };
    let a = s.list.iter().position(|m| m.key == anchor);
    let b = s.list.iter().position(|m| &m.key == to);
    let (Some(a), Some(b)) = (a, b) else { return };
    let (lo, hi) = (a.min(b), a.max(b));
    for m in s.list.iter().take(hi + 1).skip(lo) {
        if !s.selection.contains(&m.key) {
            acts.push(Action::Run("list.select", json!({"key": m.key.to_id(), "add": true})));
        }
    }
}
