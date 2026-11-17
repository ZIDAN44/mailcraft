use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    s: &Session,
    t: &theme::Tokens,
    ui: &mut Ui,
    r: Rect,
    m: &Summary,
    count: usize,
    unread_in_thread: usize,
    sent_view: bool,
    drafting: bool,
    acts: &mut Vec<Action>,
) {
    let id = ui.make_persistent_id(("row", &m.key));
    let touch = ui.ctx().input(|input| input.has_touch_screen());
    let resp = ui.interact(r, id, Sense::click_and_drag());
    let long_touch = touch && resp.long_touched();
    let is_draft = m.flags.draft || s.view_role() == Some(FolderRole::Drafts);
    // The pointer anywhere on the row, its own buttons included (egui hovers only the button
    // under the pointer, and the buttons must stay while they're pointed at).
    let hovered = ui.rect_contains_pointer(r) && !ui.ctx().is_being_dragged(id) || resp.hovered();
    let selected = s.selection.contains(&m.key);
    let unread = if count > 1 { unread_in_thread > 0 } else { !m.flags.seen };
    // Who the sender is (issue #29): their address beside their name, a warning, External.
    let marks = if sent_view { mailcraft_engine::SenderMarks::default() } else { s.sender_marks(m) };
    // To a screen reader a row is one item of the list: who, the subject, when, and what its
    // markers show (unread, flagged, attachments).
    resp.widget_info(|| {
        let mut who = if sent_view { m.to.first().map(|a| a.display().to_string()).unwrap_or_default() } else { m.from.display().to_string() };
        if marks.address {
            who = format!("{who} <{}>", m.from.email.trim());
        }
        let subject = if m.subject.is_empty() { tl!("(no subject)") } else { m.subject.as_str() };
        let mut spoken = vec![who, subject.to_string(), crate::i18n::date(&dates::list_time(m.when(), &s.clock))];
        if drafting {
            spoken.insert(0, tl!("[Draft]").to_string());
        }
        for (on, word) in [
            (marks.warning, "Check who sent it"),
            (marks.external, "External"),
            (unread, "Unread"),
            (m.flags.flagged, "Flagged"),
            (m.flags.follow_up_complete(), "Completed"),
            (m.has_attachments, "Has attachments"),
        ] {
            if on {
                spoken.push(tl!(word).to_string());
            }
        }
        egui::WidgetInfo::selected(egui::Role::ListItem, true, selected, spoken.join(", "))
    });
    let p = ui.painter_at(r);
    let sel = crate::motion::toward(ui.ctx(), id.with("selected"), selected, crate::motion::NORMAL);
    let hover = crate::motion::toward(ui.ctx(), id.with("hover"), hovered, crate::motion::FAST);
    let fill = crate::motion::mix(t.hover_row.gamma_multiply(hover), t.accent_soft, sel);
    p.rect_filled(r.shrink2(vec2(4.0, 1.0)), 6.0, fill);
    // Laid out left to right, mirrored for a right-to-left interface: the avatar on the right,
    // the time and markers on the left.
    let mr = crate::bidi::Mirror::new(r);
    if unread {
        p.rect_filled(mr.rect(Rect::from_min_size(r.min + vec2(4.0, 8.0), vec2(3.0, r.height() - 16.0))), 2.0, t.accent);
    }
    let rule = [mr.pos(r.left_bottom() + vec2(14.0, 0.0)), mr.pos(r.right_bottom() - vec2(8.0, 0.0))];
    p.line_segment(rule, egui::Stroke::new(1.0, t.border.gamma_multiply(0.6)));

    let compact = s.prefs.compact;
    let mut x = r.min.x + 14.0;
    let who_addr = if sent_view { m.to.first().cloned().unwrap_or_default() } else { m.from.clone() };
    if !compact {
        let c = mr.pos(egui::pos2(x + 17.0, r.min.y + 26.0));
        widgets::avatar(ui, c, 17.0, &who_addr.initials(), theme::avatar_color(&who_addr.email, t.dark));
        x += 44.0;
    }
    let right = r.max.x - 14.0;
    let action_size = if touch { 44.0 } else { 22.0 };
    let action_gap = if touch { 0.0 } else { 2.0 };
    let action_width = (action_size + action_gap) * 3.0 - action_gap;
    // A phone has no hover to reveal a row's actions: keep a compact, visible menu target there.
    // Desktop keeps its three direct hover buttons.
    let content_right = right
        - if touch {
            action_size
        } else if hovered {
            action_width
        } else {
            0.0
        };
    let line1 = r.min.y + if compact { 6.0 } else { 9.0 };
    // Sender (or recipients in Sent) and time.
    let time = crate::i18n::date(&dates::list_time(m.when(), &s.clock));
    let time_g = p.layout_no_wrap(time, if unread { theme::bold(12.0) } else { theme::regular(12.0) }, if unread { t.accent } else { t.dim });
    let mut tw = time_g.size().x;
    p.galley(egui::pos2(mr.start(content_right - tw, tw), line1 + 1.0), time_g, t.dim);
    // A search over several folders: each result says where it is, before its time.
    let several =
        s.search.as_ref().is_some_and(|(_, scope)| *scope != SearchScope::Current) || matches!(s.view, mailcraft_engine::View::Unified { .. });
    if let Some(f) = s.folder(&m.key.account, &m.key.folder).filter(|_| several) {
        let name: String = crate::i18n::folder(f).chars().take(24).collect();
        let fg = p.layout_no_wrap(name, theme::regular(11.5), t.faint);
        let fw = fg.size().x;
        p.galley(egui::pos2(mr.start(content_right - tw - 10.0 - fw, fw), line1 + 2.0), fg, t.faint);
        tw += fw + 10.0;
    }
    // From outside the organisation: a small tag before the time.
    if marks.external {
        let eg = p.layout_no_wrap(tl!("External").to_string(), theme::regular(11.0), t.dim);
        let ew = eg.size().x + 10.0;
        let er = mr.rect(Rect::from_min_size(egui::pos2(content_right - tw - 10.0 - ew, line1), vec2(ew, 17.0)));
        p.rect_stroke(er, 4.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        p.galley(er.center() - eg.size() / 2.0, eg, t.dim);
        tw += ew + 10.0;
    }
    let mut who_x = x;
    // Something about the sender doesn't add up: a warning sign before the name (the reading
    // pane says what).
    if marks.warning {
        let ir = mr.rect(Rect::from_min_size(egui::pos2(who_x, line1 + 1.0), vec2(15.0, 15.0)));
        icons::paint(&p, ir, Icon::Warning, t.danger);
        who_x += 19.0;
    }
    // A reply begun and left in Drafts: [Draft] before the name, in red, as Outlook writes it.
    if drafting {
        let dg = p.layout_no_wrap(tl!("[Draft]").to_string(), if unread { theme::bold(14.0) } else { theme::regular(14.0) }, t.danger);
        let dw = dg.size().x;
        p.galley(egui::pos2(mr.start(who_x, dw), line1), dg, t.danger);
        who_x += dw + 6.0;
    }
    let who = if sent_view { tf!("To: {names}", names = addr::display_list(&m.to)) } else { m.from.display().to_string() };
    let who = if count > 1 && !marks.address { format!("{who}  ({count})") } else { who };
    let (who, who_rtl) = crate::bidi::shown(&who);
    let who_right = content_right - tw - 10.0;
    let who_font = if unread { theme::bold(14.0) } else { theme::regular(14.0) };
    // A line between `from` and `to`, at its start: right-to-left text starts at the right, as
    // everything does in a right-to-left interface. What doesn't fit is cut from its end.
    let place = |rtl: bool, from: f32, to: f32, w: f32| mr.start(if rtl && !mr.on() { to - w } else { from }, w);
    if marks.address {
        // The name, then the address it really came from, dimmer. The address is what tells, so
        // it's laid out first (cut to its domain if it must be: `<…@example.com>`), but the name
        // keeps up to nine twentieths of the room, so a narrow row still says who.
        let room = (who_right - who_x).max(1.0);
        let email = m.from.email.trim();
        let more = if count > 1 { format!("  ({count})") } else { String::new() };
        let font = theme::regular(12.5);
        let name_w = p.layout_no_wrap(who.to_string(), who_font.clone(), t.text).size().x;
        let cap = (room - name_w.min(room * 0.45) - 6.0).clamp(1.0, room * 0.7);
        let full = format!("<{email}>{more}");
        let fits = p.layout_no_wrap(full.clone(), font.clone(), t.dim).size().x <= cap;
        let address = match email.rsplit_once('@') {
            Some((_, domain)) if !fits => format!("<…@{domain}>{more}"),
            _ => full,
        };
        let ag = widgets::one_line(ui, &address, font, t.dim, cap);
        let aw = ag.size().x;
        let who_g = widgets::one_line(ui, &who, who_font, t.text, (room - aw - 6.0).max(1.0));
        let nw = who_g.size().x;
        p.galley(egui::pos2(mr.start(who_x, nw), line1), who_g, t.text);
        p.galley(egui::pos2(mr.start(who_x + nw + 6.0, aw), line1 + 1.5), ag, t.dim);
    } else {
        let who_g = widgets::one_line(ui, &who, who_font, t.text, who_right - who_x);
        p.galley(egui::pos2(place(who_rtl, who_x, who_right, who_g.size().x), line1), who_g, t.text);
    }

    // Subject with markers.
    let line2 = line1 + 19.0;
    let mut icons_w = 0.0;
    let marker = |ic: Icon, color: Color32, w: &mut f32| {
        *w += 18.0;
        let ir = mr.rect(Rect::from_min_size(egui::pos2(right - *w + 3.0, line2 + 1.0), vec2(14.0, 14.0)));
        icons::paint(&p, ir, ic, color);
    };
    if !hovered {
        if m.flags.pinned() {
            marker(Icon::Pin, t.accent, &mut icons_w);
        }
        if m.flags.flagged {
            marker(Icon::Flag, t.flag, &mut icons_w);
        } else if m.flags.follow_up_complete() {
            marker(Icon::Done, t.dim, &mut icons_w);
        }
        if m.has_attachments {
            marker(Icon::Attach, t.dim, &mut icons_w);
        }
        // S/MIME, as the header says (checked once it's opened).
        match m.smime {
            mailcraft_engine::model::Smime::Encrypted => marker(Icon::Lock, t.dim, &mut icons_w),
            mailcraft_engine::model::Smime::Signed => marker(Icon::Seal, t.dim, &mut icons_w),
            mailcraft_engine::model::Smime::None => {}
        }
        // Importance the sender set: a red ! for high, a ↓ for low.
        if m.importance != 0 {
            icons_w += 12.0;
            let (mark, color) = if m.importance > 0 { ("!", t.danger) } else { ("↓", t.accent) };
            p.text(mr.pos(egui::pos2(right - icons_w + 6.0, line2 + 8.0)), egui::Align2::CENTER_CENTER, mark, theme::bold(13.0), color);
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
            p.circle_filled(mr.pos(egui::pos2(right - icons_w + 7.0, line2 + 8.0)), 5.0, theme::category_color(c.color, t.dark));
        }
    }
    let subject = if m.subject.is_empty() { tl!("(no subject)").to_string() } else { m.subject.clone() };
    let subj_color = if unread { t.accent } else { t.text };
    let (subject, subj_rtl) = crate::bidi::shown(&subject);
    let subj_right = content_right - icons_w.max(if hovered && !touch { 70.0 } else { 0.0 }) - 4.0;
    let subj_g = widgets::one_line(ui, &subject, if unread { theme::bold(13.0) } else { theme::regular(13.0) }, subj_color, subj_right - x);
    p.galley(egui::pos2(place(subj_rtl, x, subj_right, subj_g.size().x), line2), subj_g, subj_color);
    if s.prefs.show_preview && !compact && !m.preview.is_empty() {
        let (preview, pv_rtl) = crate::bidi::shown(&m.preview);
        let pv = widgets::one_line(ui, &preview, theme::regular(12.5), t.dim, content_right - x);
        p.galley(egui::pos2(place(pv_rtl, x, content_right, pv.size().x), line2 + 19.0), pv, t.dim);
    }

    // Hover actions.
    let mut clicked_action = false;
    if touch || hovered {
        let mut bx = right - action_size;
        let by = r.min.y + (r.height() - action_size) / 2.0;
        if touch {
            let br = mr.rect(Rect::from_min_size(egui::pos2(bx, by), vec2(action_size, action_size)));
            let response = ui.put(br, egui::Button::new(RichText::new("⋯").size(22.0))).on_hover_text(tl!("More actions"));
            widgets::named(ui, &response, tl!("More actions"), true, None);
            clicked_action = response.clicked();
            context::touch_message_menu(&response, s, t, m, selected, acts);
        } else {
            let actions: [(Icon, &str, &'static str, serde_json::Value); 3] = [
                (Icon::Delete, tl!("Delete"), "mail.delete", json!({})),
                (Icon::Flag, tl!(if m.flags.flagged { "Unflag" } else { "Flag" }), "mail.flag", json!({})),
                (
                    if m.flags.seen { Icon::Unread } else { Icon::Read },
                    tl!(if m.flags.seen { "Mark unread" } else { "Mark read" }),
                    "mail.toggleRead",
                    json!({}),
                ),
            ];
            for (ic, tip, cmd, params) in actions {
                let br = mr.rect(Rect::from_min_size(egui::pos2(bx, by), vec2(action_size, action_size)));
                let bresp = ui.interact(br, id.with(cmd), Sense::click()).on_hover_text(tip);
                crate::widgets::named(ui, &bresp, tip, true, None);
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
                bx -= action_size + action_gap;
            }
        }
    }

    if long_touch {
        // Long-press starts the phone's selection mode; subsequent taps add or remove rows.
        acts.push(Action::Run("list.select", json!({"key": m.key.to_id(), "add": true})));
    } else if resp.clicked() && !clicked_action {
        let mods = ui.input(|i| i.modifiers);
        // Shift: from the anchor to here (with Ctrl, added to what's selected); Ctrl: this one in
        // or out (issue #26).
        if mods.shift {
            acts.push(Action::Run("list.selectRange", json!({"key": m.key.to_id(), "add": mods.command || mods.ctrl})));
        } else if mods.command || mods.ctrl || (touch && !s.selection.is_empty()) {
            acts.push(Action::Run("list.select", json!({"key": m.key.to_id(), "add": true})));
        } else if touch && (s.view_role() == Some(FolderRole::Drafts) || m.flags.draft) {
            acts.push(Action::Run("draft.edit", json!({"key": m.key.to_id()})));
        } else {
            acts.push(Action::Open(m.key.clone()));
        }
    }
    if !touch && resp.double_clicked() {
        // Drafts carry on being written; other mail opens in a window of its own.
        if s.view_role() == Some(FolderRole::Drafts) || m.flags.draft {
            acts.push(Action::Run("draft.edit", json!({"key": m.key.to_id()})));
        } else {
            acts.push(Action::Run("message.popOut", json!({"key": m.key.to_id()})));
        }
    }
    // Touch keeps horizontal drags for quick mail actions; desktop retains drag-to-move.
    if !touch && resp.drag_started() {
        if !selected {
            acts.push(Action::Run("list.select", json!({"key": m.key.to_id()})));
        }
        let keys = if selected { s.selection.clone() } else { vec![m.key.clone()] };
        resp.dnd_set_drag_payload(DragKeys(keys));
    }
    if !touch && resp.dragged() {
        let n = if selected { s.selection.len() } else { 1 };
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), id.with("drag"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(if n == 1 { tl!("Move 1 message").to_string() } else { tf!("Move {n} messages", n = n) });
        });
    }
    if touch && resp.dragged() {
        let delta = resp.drag_delta();
        if delta.x.abs() >= 16.0 && delta.x.abs() > delta.y.abs() * 1.25 {
            let width = delta.x.abs().min(r.width());
            let left = if delta.x < 0.0 { r.right() - width } else { r.left() };
            let swipe = Rect::from_min_max(egui::pos2(left, r.top()), egui::pos2(left + width, r.bottom()));
            p.rect_filled(swipe, 6.0, t.accent_soft);
            let (icon, label) = if delta.x < 0.0 && is_draft {
                (Icon::Delete, tl!("Delete"))
            } else if delta.x < 0.0 {
                (Icon::Archive, tl!("Archive"))
            } else if unread {
                (Icon::Read, tl!("Mark as read"))
            } else {
                (Icon::Unread, tl!("Mark as unread"))
            };
            let center = swipe.center();
            icons::paint(&p, icons::centered(Rect::from_center_size(center, vec2(20.0, 20.0)), 18.0), icon, t.accent);
            if width >= 118.0 {
                p.text(center + vec2(0.0, 15.0), egui::Align2::CENTER_TOP, label, theme::regular(12.0), t.accent);
            }
        }
    }
    if touch && resp.drag_stopped() {
        let delta = resp.drag_delta();
        if let Some(action) = touch_swipe_action(delta, !unread) {
            if !selected {
                acts.push(Action::Run("list.select", json!({"key": m.key.to_id()})));
            }
            if count > 1 {
                for key in s.thread_of(&m.key) {
                    if key != m.key && (!selected || !s.selection.contains(&key)) {
                        acts.push(Action::Run("list.select", json!({"key": key.to_id(), "add": true})));
                    }
                }
            }
            let id = match action {
                TouchSwipeAction::Archive if is_draft => "mail.delete",
                TouchSwipeAction::Archive => "mail.archive",
                TouchSwipeAction::MarkRead => "mail.markRead",
                TouchSwipeAction::MarkUnread => "mail.markUnread",
            };
            acts.push(Action::Run(id, json!({})));
        }
    }
    if !long_touch {
        context::message_menu(&resp, s, t, m, selected, acts);
    }
    // Reply actions need the message open; not one of several selected, which the menu acts on
    // all of (opening it selected it alone: Delete took only that one).
    let one_of_several = selected && s.selection.len() > 1;
    if !touch && resp.secondary_clicked() && !one_of_several && !s.reading.as_ref().is_some_and(|r| r.key == m.key) {
        acts.push(Action::Open(m.key.clone()));
    }
}
