//! One message's row in the list: what a screen reader hears of it, its ground, the sender line,
//! the subject line with its markers, the buttons shown on hover, clicks and drags, and its
//! right-click menu.

use super::*;

/// The message a row shows, and where.
pub(super) struct RowOf<'a> {
    pub(super) m: &'a Summary,
    pub(super) r: Rect,
    /// How many messages its conversation has in the list, and how many of them are unread.
    pub(super) count: usize,
    pub(super) unread_in_thread: usize,
    /// In Sent, Drafts or the Outbox: the row says who it went to.
    pub(super) sent_view: bool,
    /// A reply or forward to it was begun and left in Drafts.
    pub(super) drafting: bool,
    /// The list holds several folders' mail (a search over more than this one, or the unified
    /// Inbox): each row says where it is.
    pub(super) several_folders: bool,
    /// Tab stops here: the list is one stop, on the message it's on (issue #175).
    pub(super) tab_stop: bool,
}

/// What a row's pieces share, worked out once.
struct MessageRow<'a> {
    s: &'a Session,
    t: &'a theme::Tokens,
    m: &'a Summary,
    id: egui::Id,
    r: Rect,
    /// Laid out left to right, mirrored for a right-to-left interface: the avatar on the right,
    /// the time and markers on the left.
    mr: crate::bidi::Mirror,
    count: usize,
    sent_view: bool,
    drafting: bool,
    several_folders: bool,
    unread: bool,
    selected: bool,
    hovered: bool,
    marks: mailcraft_engine::SenderMarks,
    /// Where the text starts (past the avatar) and ends, and the top of its first line.
    x: f32,
    right: f32,
    line1: f32,
    /// The whole row on that first line (the single-line list): the sender in a column of its
    /// own, then the subject, the markers and the time.
    single: bool,
}

/// The single-line list: the least room the subject keeps before a tag is left out.
const SUBJECT_LEAST: f32 = 120.0;

pub(super) fn message_row(s: &Session, t: &theme::Tokens, ui: &mut Ui, of: RowOf<'_>, acts: &mut Vec<Action>) -> egui::Response {
    let RowOf { m, r, count, unread_in_thread, sent_view, drafting, several_folders, tab_stop } = of;
    let id = ui.make_persistent_id(("row", &m.key));
    let mut sense = Sense::click_and_drag();
    if !tab_stop {
        sense -= Sense::FOCUSABLE;
    }
    let resp = ui.interact(r, id, sense);
    // The pointer anywhere on the row, its own buttons included (egui hovers only the button
    // under the pointer, and the buttons must stay while they're pointed at).
    let hovered = ui.rect_contains_pointer(r) && !ui.ctx().is_being_dragged(id) || resp.hovered();
    let selected = s.selection.contains(&m.key);
    let unread = if count > 1 { unread_in_thread > 0 } else { !m.flags.seen };
    // Who the sender is (issue #29): their address beside their name, a warning, External.
    let marks = if sent_view { mailcraft_engine::SenderMarks::default() } else { s.sender_marks(m) };
    let single = s.prefs.single_line;
    let compact = s.prefs.compact || single;
    let first_line = match (single, compact) {
        (true, _) => 5.0,
        (false, true) => 6.0,
        (false, false) => 9.0,
    };
    let row = MessageRow {
        s,
        t,
        m,
        id,
        r,
        mr: crate::bidi::Mirror::new(r),
        count,
        sent_view,
        drafting,
        several_folders,
        unread,
        selected,
        hovered,
        marks,
        x: r.min.x + 14.0 + if compact { 0.0 } else { 44.0 },
        right: r.max.x - 14.0,
        line1: r.min.y + first_line,
        single,
    };
    resp.widget_info(|| row.spoken());
    let p = ui.painter_at(r);
    row.ground(ui, &p);
    let end = row.sender(ui, &p);
    let icons_w = row.markers(&p, end);
    row.subject(ui, &p, end, icons_w);
    let clicked_action = hovered && row.hover_actions(ui, end, acts);
    row.clicks(ui, &resp, clicked_action, acts);
    row.menu(&resp, acts);
    // Reply actions need the message open; not one of several selected, which the menu acts on
    // all of (opening it selected it alone: Delete took only that one).
    let one_of_several = selected && s.selection.len() > 1;
    if resp.secondary_clicked() && !one_of_several && !s.reading.as_ref().is_some_and(|r| r.key == m.key) {
        acts.push(Action::Open(m.key.clone()));
    }
    resp
}

impl MessageRow<'_> {
    /// To a screen reader a row is one item of the list: who, the subject, when, and what its
    /// markers show (unread, flagged, attachments).
    fn spoken(&self) -> egui::WidgetInfo {
        let (s, m, marks) = (self.s, self.m, self.marks);
        let mut who = if self.sent_view { m.to.first().map(|a| a.display().to_string()).unwrap_or_default() } else { m.from.display().to_string() };
        if marks.address {
            who = format!("{who} <{}>", m.from.email.trim());
        }
        let subject = if m.subject.is_empty() { tl!("(no subject)") } else { m.subject.as_str() };
        let mut spoken = vec![who, subject.to_string(), crate::i18n::date(&dates::list_time(m.when(), &s.clock))];
        if self.drafting {
            spoken.insert(0, tl!("[Draft]").to_string());
        }
        for (on, word) in [
            (marks.warning, "Check who sent it"),
            (marks.external, "External"),
            (self.unread, "Unread"),
            (m.flags.flagged, "Flagged"),
            (m.flags.follow_up_complete(), "Completed"),
            (m.has_attachments, "Has attachments"),
        ] {
            if on {
                spoken.push(tl!(word).to_string());
            }
        }
        egui::WidgetInfo::selected(egui::Role::ListItem, true, self.selected, spoken.join(", "))
    }

    /// The row's fill (selected, pointed at), the unread bar and the rule under it.
    fn ground(&self, ui: &Ui, p: &egui::Painter) {
        let (t, r, mr, id) = (self.t, self.r, self.mr, self.id);
        let sel = crate::motion::toward(ui.ctx(), id.with("selected"), self.selected, crate::motion::NORMAL);
        let hover = crate::motion::toward(ui.ctx(), id.with("hover"), self.hovered, crate::motion::FAST);
        let fill = crate::motion::mix(t.hover_row.gamma_multiply(hover), t.accent_soft, sel);
        p.rect_filled(r.shrink2(vec2(4.0, 1.0)), 6.0, fill);
        if self.unread {
            p.rect_filled(mr.rect(Rect::from_min_size(r.min + vec2(4.0, 8.0), vec2(3.0, r.height() - 16.0))), 2.0, t.accent);
        }
        let rule = [mr.pos(r.left_bottom() + vec2(14.0, 0.0)), mr.pos(r.right_bottom() - vec2(8.0, 0.0))];
        p.line_segment(rule, egui::Stroke::new(1.0, t.border.gamma_multiply(0.6)));
    }

    /// A line between `from` and `to`, at its start: right-to-left text starts at the right, as
    /// everything does in a right-to-left interface. What doesn't fit is cut from its end.
    fn place(&self, rtl: bool, from: f32, to: f32, w: f32) -> f32 {
        self.mr.start(if rtl && !self.mr.on() { to - w } else { from }, w)
    }

    /// Where the subject's line is: the second, or the first in the single-line list.
    fn line2(&self) -> f32 {
        if self.single { self.line1 } else { self.line1 + 19.0 }
    }

    /// Where the subject starts: under the sender, or after the sender's column in the
    /// single-line list (about a quarter of the row, so the subjects line up down the list).
    fn subject_x(&self) -> f32 {
        if self.single { self.x + ((self.right - self.x) * 0.26).clamp(96.0, 220.0) + 12.0 } else { self.x }
    }

    /// The avatar, then the first line: the sender (or recipients in Sent) and the time, with
    /// the folder of a search's result, the External tag, a warning and [Draft]. Where the
    /// subject's line ends: the row's end, or before the time in the single-line list.
    fn sender(&self, ui: &mut Ui, p: &egui::Painter) -> f32 {
        let (s, t, m, r, mr, marks) = (self.s, self.t, self.m, self.r, self.mr, self.marks);
        let (unread, count, x, right, line1) = (self.unread, self.count, self.x, self.right, self.line1);
        if !s.prefs.compact && !self.single {
            let nobody = mailcraft_engine::model::EmailAddress::default();
            let who_addr = if self.sent_view { m.to.first().unwrap_or(&nobody) } else { &m.from };
            let c = mr.pos(egui::pos2(r.min.x + 14.0 + 17.0, r.min.y + 26.0));
            widgets::avatar(ui, c, 17.0, &who_addr.initials(), theme::avatar_color(&who_addr.email, t.dark));
        }
        // Sender (or recipients in Sent) and time.
        let time = crate::i18n::date(&dates::list_time(m.when(), &s.clock));
        let time_g = p.layout_no_wrap(time, if unread { theme::bold(12.0) } else { theme::regular(12.0) }, if unread { t.accent } else { t.dim });
        let mut tw = time_g.size().x;
        p.galley(egui::pos2(mr.start(right - tw, tw), line1 + 1.0), time_g, t.dim);
        // In a narrow list the sender comes first: where a tag before the time would leave the name
        // (and the address beside it) no room to be read, the tag isn't drawn.
        let who_least = if marks.address { 150.0 } else { 90.0 };
        // On one line it's the subject a tag would leave no room.
        let (single, subject_x) = (self.single, self.subject_x());
        let fits = |tw: f32, w: f32| {
            let left = right - tw - 10.0 - w - 10.0;
            if single { left - subject_x >= SUBJECT_LEAST } else { left - x >= who_least }
        };
        // A search over several folders: each result says where it is, before its time.
        if let Some(f) = s.folder(&m.key.account, &m.key.folder).filter(|_| self.several_folders) {
            let name: String = crate::i18n::folder(f).chars().take(24).collect();
            let fg = p.layout_no_wrap(name, theme::regular(11.5), t.faint);
            let fw = fg.size().x;
            if fits(tw, fw) {
                p.galley(egui::pos2(mr.start(right - tw - 10.0 - fw, fw), line1 + 2.0), fg, t.faint);
                tw += fw + 10.0;
            }
        }
        // From outside the organisation: a small tag before the time.
        if marks.external {
            let eg = p.layout_no_wrap(tl!("External").to_string(), theme::regular(11.0), t.dim);
            let ew = eg.size().x + 10.0;
            if fits(tw, ew) {
                let er = mr.rect(Rect::from_min_size(egui::pos2(right - tw - 10.0 - ew, line1), vec2(ew, 17.0)));
                p.rect_stroke(er, 4.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                p.galley(er.center() - eg.size() / 2.0, eg, t.dim);
                tw += ew + 10.0;
            }
        }
        let mut who_x = x;
        // Something about the sender doesn't add up: a warning sign before the name (the reading
        // pane says what).
        if marks.warning {
            let ir = mr.rect(Rect::from_min_size(egui::pos2(who_x, line1 + 1.0), vec2(15.0, 15.0)));
            icons::paint(p, ir, Icon::Warning, t.danger);
            who_x += 19.0;
        }
        // A reply begun and left in Drafts: [Draft] before the name, in red, as Outlook writes it.
        if self.drafting {
            let dg = p.layout_no_wrap(tl!("[Draft]").to_string(), if unread { theme::bold(14.0) } else { theme::regular(14.0) }, t.danger);
            let dw = dg.size().x;
            p.galley(egui::pos2(mr.start(who_x, dw), line1), dg, t.danger);
            who_x += dw + 6.0;
        }
        let who = if self.sent_view { tf!("To: {names}", names = addr::display_list(&m.to)) } else { m.from.display().to_string() };
        let who = if count > 1 && !marks.address { format!("{who}  ({count})") } else { who };
        let who_rtl = crate::bidi::reads_rtl(&who);
        let who_right = if single { subject_x - 12.0 } else { right - tw - 10.0 };
        let who_font = if unread { theme::bold(14.0) } else { theme::regular(14.0) };
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
            p.galley(egui::pos2(self.place(who_rtl, who_x, who_right, who_g.size().x), line1), who_g, t.text);
        }
        if single { right - tw - 10.0 } else { right }
    }

    /// The subject line's markers at its end (`right`), unless the row is pointed at (its buttons
    /// go there): pinned, flagged, attachments, S/MIME, importance, answered, then a dot per
    /// category. How wide they are.
    fn markers(&self, p: &egui::Painter, right: f32) -> f32 {
        let (t, m, mr) = (self.t, self.m, self.mr);
        let line2 = self.line2();
        let mut icons_w = 0.0;
        let marker = |ic: Icon, color: Color32, w: &mut f32| {
            *w += 18.0;
            let ir = mr.rect(Rect::from_min_size(egui::pos2(right - *w + 3.0, line2 + 1.0), vec2(14.0, 14.0)));
            icons::paint(p, ir, ic, color);
        };
        if !self.hovered {
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
        if !self.hovered {
            for c in self.s.categories_of(m).into_iter().rev().take(4) {
                icons_w += 13.0;
                p.circle_filled(mr.pos(egui::pos2(right - icons_w + 7.0, line2 + 8.0)), 5.0, theme::category_color(c.color, t.dark));
            }
        }
        icons_w
    }

    /// The subject, before the markers (`icons_w` wide, ending at `end`), and under it the
    /// preview.
    fn subject(&self, ui: &mut Ui, p: &egui::Painter, end: f32, icons_w: f32) {
        let (t, m, x, right) = (self.t, self.m, self.x, self.right);
        let (line2, subject_x) = (self.line2(), self.subject_x());
        let unread = self.unread;
        let subject = if m.subject.is_empty() { tl!("(no subject)").to_string() } else { m.subject.clone() };
        let subj_color = if unread { t.accent } else { t.text };
        let subj_rtl = crate::bidi::reads_rtl(&subject);
        let subj_right = end - icons_w.max(if self.hovered { 70.0 } else { 0.0 }) - 4.0;
        let subj_font = if unread { theme::bold(13.0) } else { theme::regular(13.0) };
        let marks = crate::reading::subject_marks(self.s, &subject);
        let subj_g = marked_line(ui, &subject, &marks, subj_font, subj_color, subj_right - subject_x);
        p.galley(egui::pos2(self.place(subj_rtl, subject_x, subj_right, subj_g.size().x), line2), subj_g, subj_color);
        if self.s.prefs.show_preview && !self.s.prefs.compact && !self.single && !m.preview.is_empty() {
            let pv_rtl = crate::bidi::reads_rtl(&m.preview);
            let marks = crate::reading::text_marks(self.s, &m.preview);
            let pv = marked_line(ui, &m.preview, &marks, theme::regular(12.5), t.dim, right - x);
            p.galley(egui::pos2(self.place(pv_rtl, x, right, pv.size().x), line2 + 19.0), pv, t.dim);
        }
    }

    /// Delete, Flag and Mark read, at the subject line's end (`end`) while the row is pointed at.
    /// Whether one was clicked.
    fn hover_actions(&self, ui: &mut Ui, end: f32, acts: &mut Vec<Action>) -> bool {
        let (t, m, mr, id) = (self.t, self.m, self.mr, self.id);
        let line2 = self.line2();
        let mut clicked_action = false;
        let mut bx = end - 22.0;
        let actions: [(Icon, &str, &'static str); 3] = [
            (Icon::Delete, tl!("Delete"), "mail.delete"),
            (Icon::Flag, tl!(if m.flags.flagged { "Unflag" } else { "Flag" }), "mail.flag"),
            (if m.flags.seen { Icon::Unread } else { Icon::Read }, tl!(if m.flags.seen { "Mark unread" } else { "Mark read" }), "mail.toggleRead"),
        ];
        for (ic, tip, cmd) in actions {
            let br = mr.rect(Rect::from_min_size(egui::pos2(bx, line2 - 3.0), vec2(22.0, 22.0)));
            let bresp = ui.interact(br, id.with(cmd), Sense::click()).on_hover_text(tip);
            crate::widgets::named(ui, &bresp, tip, true, None);
            if bresp.hovered() {
                ui.painter().rect_filled(br, 4.0, t.border);
            }
            let color = if cmd == "mail.flag" && m.flags.flagged { t.flag } else { t.dim };
            icons::paint(ui.painter(), icons::centered(br, 14.0), ic, color);
            if bresp.clicked() {
                clicked_action = true;
                if !self.selected {
                    acts.push(Action::Run("list.select", json!({"key": m.key.to_id()})));
                }
                acts.push(Action::Run(cmd, json!({})));
            }
            bx -= 24.0;
        }
        clicked_action
    }

    /// A click opens or selects (Ctrl, Shift), a double click opens in a window (or carries on
    /// writing a draft), a drag carries the selection to a folder.
    fn clicks(&self, ui: &Ui, resp: &egui::Response, clicked_action: bool, acts: &mut Vec<Action>) {
        let (s, m, selected) = (self.s, self.m, self.selected);
        if resp.clicked() && !clicked_action {
            let mods = ui.input(|i| i.modifiers);
            // Shift: from the anchor to here (with Ctrl, added to what's selected); Ctrl: this one in
            // or out (issue #26).
            if mods.shift {
                acts.push(Action::Run("list.selectRange", json!({"key": m.key.to_id(), "add": mods.command || mods.ctrl})));
            } else if mods.command || mods.ctrl {
                acts.push(Action::Run("list.select", json!({"key": m.key.to_id(), "add": true})));
            } else {
                acts.push(Action::Open(m.key.clone()));
            }
        }
        if resp.double_clicked() {
            // Drafts carry on being written; other mail opens in a window of its own.
            if s.view_role() == Some(FolderRole::Drafts) || m.flags.draft {
                acts.push(Action::Run("draft.edit", json!({"key": m.key.to_id()})));
            } else {
                acts.push(Action::Run("message.popOut", json!({"key": m.key.to_id()})));
            }
        }
        if resp.drag_started() {
            if !selected {
                acts.push(Action::Run("list.select", json!({"key": m.key.to_id()})));
            }
            resp.dnd_set_drag_payload(DragKeys);
        }
        if resp.dragged() {
            let n = if selected { s.selection.len() } else { 1 };
            egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), self.id.with("drag"), egui::PopupAnchor::Pointer).show(|ui| {
                // With Ctrl held, dropped on a folder they're copied there.
                let copy = ui.input(|i| i.modifiers.command);
                ui.label(match (copy, n) {
                    (false, 1) => tl!("Move 1 message").to_string(),
                    (false, n) => tf!("Move {n} messages", n = n),
                    (true, 1) => tl!("Copy 1 message").to_string(),
                    (true, n) => tf!("Copy {n} messages", n = n),
                });
            });
        }
    }

    /// The row's right-click menu.
    fn menu(&self, resp: &egui::Response, acts: &mut Vec<Action>) {
        let (s, t, m, selected) = (self.s, self.t, self.m, self.selected);
        let (key, flags) = (&m.key, &m.flags);
        resp.context_menu(|ui| {
            if !s.selection.contains(key) {
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
            // One of several selected: answering and printing are for one message (they failed, or
            // asked where to save a PDF and then said nothing is open).
            let several = selected && s.selection.len() > 1;
            for (label, act) in [
                (tl!("Reply"), Action::Run("mail.reply", json!({}))),
                (tl!("Reply all"), Action::Run("mail.replyAll", json!({}))),
                (tl!("Forward"), Action::Run("mail.forward", json!({}))),
                (tl!("Quick print"), Action::Print),
            ] {
                if ui.add_enabled(!several, egui::Button::new(label)).clicked() {
                    acts.push(act);
                    ui.close();
                }
            }
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
            item(ui, acts, tl!("Copy to…"), Action::Dialog(Dialog::copy_to()));
            item(ui, acts, tl!("Archive"), Action::Run("mail.archive", json!({})));
            item(ui, acts, tl!("Report junk"), Action::Run("mail.junk", json!({})));
            ui.menu_button(tl!("Junk"), |ui| crate::junk_menu(ui, s, acts));
            item(ui, acts, tl!("Ignore"), Action::Run("mail.ignore", json!({})));
            item(ui, acts, tl!("Delete"), Action::Run("mail.delete", json!({})));
            ui.separator();
            // Find related, as Outlook's: the conversation, or everything from the sender.
            ui.menu_button(tl!("Find related"), |ui| {
                let topic = mailcraft_engine::model::thread::base_subject(&m.subject);
                let searches = [
                    ("Messages in this conversation", format!("subject:\"{}\"", topic.replace('"', ""))),
                    ("Messages from sender", format!("from:{}", m.from.email)),
                ];
                for (label, query) in searches {
                    if ui.button(tl!(label)).clicked() {
                        acts.push(Action::Search(query));
                        ui.close();
                    }
                }
            });
            crate::organize::context_items(ui, s, t, acts, flags.pinned());
            ui.separator();
            item(ui, acts, tl!("View message source"), Action::Run("message.source", json!({"key": key.to_id()})));
        });
    }
}

/// A line of a row ([`widgets::one_line`]), what the search on found in it marked (issue #173).
fn marked_line(
    ui: &Ui,
    text: &str,
    marks: &[std::ops::Range<usize>],
    font: egui::FontId,
    color: egui::Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    if marks.is_empty() {
        return widgets::one_line(ui, text, font, color, width);
    }
    let mut job = crate::reading::marked_job(text, marks, font, color);
    job.wrap = egui::text::TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}
