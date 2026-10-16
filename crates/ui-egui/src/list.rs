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
                    let r = ui.add(egui::Button::new(text).frame_when_inactive(false));
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
            // How the list is arranged, kept at the right (as Outlook's "By Date" above its list):
            // the tabs wrap in what's left, so long words (Dutch, German…) never run into it.
            let arrange = tf!("By {field} ⏷", field = tl!(s.prefs.arrange.label()));
            let right = if s.search.is_none() {
                ui.painter().layout_no_wrap(arrange.clone(), egui::TextStyle::Small.resolve(ui.style()), t.dim).size().x + 54.0
            } else {
                0.0
            };
            let left_w = (ui.available_width() - right).max(120.0);
            ui.allocate_ui_with_layout(vec2(left_w, 22.0), egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
                // The tabs, and a filter chosen from the Home tab's Filter menu when it's another.
                let shown: Vec<ListFilter> = ListFilter::ALL.into_iter().chain((!ListFilter::ALL.contains(&s.filter)).then_some(s.filter)).collect();
                for f in shown {
                    let on = s.filter == f;
                    let text = RichText::new(tl!(f.label())).color(if on { t.accent } else { t.dim }).font(if on {
                        theme::bold(12.5)
                    } else {
                        theme::regular(12.5)
                    });
                    let r = ui.add(egui::Button::new(text).frame_when_inactive(false));
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
            if s.search.is_none() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let rev = s.prefs.reverse_sort;
                    if ui
                        .add(egui::Button::new(RichText::new(if rev { "↓" } else { "↑" }).color(t.dim)).frame_when_inactive(false))
                        .on_hover_text(tl!("Reverse sort"))
                        .clicked()
                    {
                        acts.push(Action::Run("view.reverseSort", json!({})));
                    }
                    let r = ui.add(egui::Button::new(RichText::new(arrange).color(t.dim).small()).frame_when_inactive(false));
                    egui::Popup::menu(&r).show(|ui| arrange_menu(ui, s, acts));
                });
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
        y += if matches!(r, Row::Header(_) | Row::Group(_)) { HEADER_H } else { mh };
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
                Row::Group(name) => {
                    let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, HEADER_H));
                    let (name, _) = crate::bidi::shown(name);
                    ui.painter().text(r.left_bottom() + vec2(14.0, -8.0), egui::Align2::LEFT_BOTTOM, name.as_ref(), theme::bold(12.5), t.dim);
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

/// Arrange the list by…, as a menu (the list's own and the View tab's).
pub(crate) fn arrange_menu(ui: &mut Ui, s: &Session, acts: &mut Vec<Action>) {
    for a in mailcraft_engine::Arrange::ALL {
        if ui.selectable_label(s.prefs.arrange == a, tl!(a.label())).clicked() {
            acts.push(Action::Run("view.arrange", json!({"by": a.code()})));
            ui.close();
        }
    }
    ui.separator();
    if ui.selectable_label(s.prefs.reverse_sort, tl!("Reverse sort")).clicked() {
        acts.push(Action::Run("view.reverseSort", json!({})));
        ui.close();
    }
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
    // The pointer anywhere on the row, its own buttons included (egui hovers only the button
    // under the pointer, and the buttons must stay while they're pointed at).
    let hovered = ui.rect_contains_pointer(r) && !ui.ctx().is_being_dragged(id) || resp.hovered();
    let selected = s.selection.contains(&m.key);
    let unread = if count > 1 { unread_in_thread > 0 } else { !m.flags.seen };
    let p = ui.painter_at(r);
    let fill = if selected {
        t.accent_soft
    } else if hovered {
        t.hover_row
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
    let (who, who_rtl) = crate::bidi::shown(&who);
    let who_g = p.layout_no_wrap(who.into_owned(), if unread { theme::bold(14.0) } else { theme::regular(14.0) }, t.text);
    // Right-to-left text starts at the right, so that's where it goes when it doesn't fit.
    let who_x = if who_rtl { (right - tw - 10.0 - who_g.size().x).max(x) } else { x };
    p.with_clip_rect(Rect::from_x_y_ranges(x..=right - tw - 8.0, r.y_range())).galley(egui::pos2(who_x, line1), who_g, t.text);

    // Subject with markers.
    let line2 = line1 + 19.0;
    let mut icons_w = 0.0;
    let marker = |ic: Icon, color: Color32, w: &mut f32| {
        *w += 18.0;
        let ir = Rect::from_min_size(egui::pos2(right - *w + 3.0, line2 + 1.0), vec2(14.0, 14.0));
        icons::paint(&p, ir, ic, color);
    };
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
        // Importance the sender set: a red ! for high, a ↓ for low.
        if m.importance != 0 {
            icons_w += 12.0;
            let (mark, color) = if m.importance > 0 { ("!", t.danger) } else { ("↓", t.accent) };
            p.text(egui::pos2(right - icons_w + 6.0, line2 + 8.0), egui::Align2::CENTER_CENTER, mark, theme::bold(13.0), color);
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
    let (subject, subj_rtl) = crate::bidi::shown(&subject);
    let subj_g = p.layout_no_wrap(subject.into_owned(), if unread { theme::bold(13.0) } else { theme::regular(13.0) }, subj_color);
    let subj_right = right - icons_w.max(if hovered { 70.0 } else { 0.0 }) - 4.0;
    let subj_x = if subj_rtl { (subj_right - subj_g.size().x).max(x) } else { x };
    p.with_clip_rect(Rect::from_x_y_ranges(x..=subj_right, r.y_range())).galley(egui::pos2(subj_x, line2), subj_g, subj_color);
    if s.prefs.show_preview && !compact && !m.preview.is_empty() {
        let (preview, pv_rtl) = crate::bidi::shown(&m.preview);
        let pv = p.layout_no_wrap(preview.into_owned(), theme::regular(12.5), t.dim);
        let pv_x = if pv_rtl { (right - pv.size().x).max(x) } else { x };
        p.with_clip_rect(Rect::from_x_y_ranges(x..=right, r.y_range())).galley(egui::pos2(pv_x, line2 + 19.0), pv, t.dim);
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
    if resp.double_clicked() {
        // Drafts carry on being written; other mail opens in a window of its own.
        if s.view_role() == Some(FolderRole::Drafts) {
            acts.push(Action::Open(m.key.clone()));
        } else {
            acts.push(Action::Run("message.popOut", json!({"key": m.key.to_id()})));
        }
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
    let (subject, sender) = (m.subject.clone(), m.from.email.clone());
    resp.context_menu(|ui| {
        if !s.selection.contains(&key) {
            acts.push(Action::Run("list.select", json!({"key": key.to_id()})));
        }
        let item = |ui: &mut Ui, acts: &mut Vec<Action>, label: &str, act: Action| {
            if ui.button(label).clicked() {
                acts.push(act);
                ui.close();
            }
        };
        item(ui, acts, tl!("Open in new window"), Action::Run("message.popOut", json!({"key": key.to_id()})));
        ui.separator();
        item(ui, acts, tl!("Reply"), Action::Run("mail.reply", json!({})));
        item(ui, acts, tl!("Reply all"), Action::Run("mail.replyAll", json!({})));
        item(ui, acts, tl!("Forward"), Action::Run("mail.forward", json!({})));
        item(ui, acts, tl!("Quick print"), Action::Print);
        ui.separator();
        item(
            ui,
            acts,
            tl!(if flags.seen { "Mark as unread" } else { "Mark as read" }),
            Action::Run(if flags.seen { "mail.markUnread" } else { "mail.markRead" }, json!({})),
        );
        ui.menu_button(tl!("Follow up"), |ui| crate::organize::follow_up_menu(ui, s, t, acts));
        ui.separator();
        item(ui, acts, tl!("Move to…"), Action::Dialog(Dialog::move_to()));
        item(ui, acts, tl!("Archive"), Action::Run("mail.archive", json!({})));
        item(ui, acts, tl!("Report junk"), Action::Run("mail.junk", json!({})));
        item(ui, acts, tl!("Ignore"), Action::Run("mail.ignore", json!({})));
        item(ui, acts, tl!("Delete"), Action::Run("mail.delete", json!({})));
        ui.separator();
        // Find related, as Outlook's: the conversation, or everything from the sender.
        ui.menu_button(tl!("Find related"), |ui| {
            let topic = mailcraft_engine::model::thread::base_subject(&subject);
            let searches = [
                ("Messages in this conversation", format!("subject:\"{}\"", topic.replace('"', ""))),
                ("Messages from sender", format!("from:{sender}")),
            ];
            for (label, query) in searches {
                if ui.button(tl!(label)).clicked() {
                    acts.push(Action::Search(query));
                    ui.close();
                }
            }
        });
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
