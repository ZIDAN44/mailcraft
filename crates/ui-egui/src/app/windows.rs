//! The windows of their own: messages being written, and messages popped out.

use crate::*;

impl MailApp {
    /// Make message `id` the one acted on, with its editor (selection, undo) at hand.
    pub(crate) fn use_compose(&mut self, id: u64) -> bool {
        if !self.session.activate_compose(id) {
            return false;
        }
        use_editor(&mut self.ui, id);
        true
    }

    /// The message written in the main window, if any, made the one acted on (another's window
    /// may have taken over last frame); editors of messages gone are dropped.
    pub(crate) fn settle_composes(&mut self, ctx: &egui::Context) {
        let live: std::collections::HashSet<u64> = self.session.all_composes().map(|c| c.id).collect();
        self.ui.editors.retain(|id, _| live.contains(id));
        self.ui.compose_shown.retain(|id| live.contains(id));
        if self.ui.dialog_for.is_some_and(|id| !live.contains(&id)) {
            self.ui.dialog_for = None;
        }
        let inline = self.session.all_composes().find(|c| compose_inline(c, ctx)).map(|c| c.id);
        if let Some(id) = inline {
            self.use_compose(id);
        }
    }

    /// Each message being written in a window of its own (new mail, popped out, or set aside
    /// when another was started), as Outlook keeps every one open. Closing one keeps it in Drafts.
    pub(crate) fn compose_windows(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let before = self.session.compose.as_ref().map(|c| c.id);
        let ids: Vec<u64> = self.session.all_composes().filter(|c| !compose_inline(c, ctx)).map(|c| c.id).collect();
        // A window opening goes a step down and right of those already open.
        let open = ids.len().saturating_sub(1);
        for id in ids {
            if self.use_compose(id) {
                let mut mine = Vec::new();
                self.compose_window(ctx, open, &mut mine);
                acts.extend(mine.into_iter().map(|a| Action::ForCompose(id, Box::new(a))));
            }
        }
        if let Some(id) = before {
            self.use_compose(id);
        }
    }

    /// The window of the message acted on now, opening `n` steps down and right of the last
    /// place (as many as are open already).
    pub(crate) fn compose_window(&mut self, ctx: &egui::Context, n: usize, acts: &mut Vec<Action>) {
        let Some(c) = self.session.compose.as_ref() else { return };
        let mine = c.id;
        let t = self.ui.tokens;
        let subject = if c.subject.trim().is_empty() { tl!("Untitled").to_string() } else { c.subject.trim().to_string() };
        let id = egui::ViewportId::from_hash_of(("compose-window", c.id));
        let builder = egui::ViewportBuilder::default()
            .with_title(tf!("{subject} – Message", subject = subject))
            .with_inner_size([900.0, 720.0])
            .with_min_inner_size([520.0, 420.0]);
        // Where the last one was, and its size; each further one a step down and right.
        let step = 28.0 * (n % 8) as f32;
        let builder = match self.ui.places.get("compose") {
            Some(p) if opening(ctx, id) => WindowPlace { x: p.x + step, y: p.y + step, ..*p }.apply(builder),
            _ => builder,
        };
        let (session, st) = (&mut self.session, &mut self.ui);
        let close = ctx.show_viewport_immediate(id, builder, |ui, _| {
            let mut place = st.places.get("compose").copied();
            after_open(ui, place);
            WindowPlace::note(&mut place, ui);
            if let Some(p) = place {
                st.places.insert("compose".into(), p);
            }
            let mut mine_acts: Vec<Action> = Vec::new();
            let mut closing = ui.input(|i| i.viewport().close_requested());
            // Something written: ask first, as Outlook does.
            if closing && session.compose_written() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
                st.dialog = Some(Dialog::SaveChanges);
                st.dialog_for = Some(mine);
                closing = false;
            }
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                reading::compose_window(session, st, ui, &mut mine_acts);
            });
            // A dialog for a message shows in that message's window only.
            let here = st.dialog_for.is_none_or(|f| f == mine);
            if let Some(mut d) = st.dialog.take() {
                if d.for_compose() && here {
                    let keep = dialogs::show(ui.ctx(), &mut d, session, &t, &mut mine_acts);
                    if keep && st.dialog.is_none() {
                        st.dialog = Some(d);
                    }
                } else {
                    st.dialog = Some(d);
                }
            }
            acts.extend(mine_acts);
            closing
        });
        if close {
            forget_open(ctx, id);
            acts.push(Action::Run("compose.close", json!({"save": false})));
        }
    }

    /// Show only one message, as its own window does (`key` from [`MessageKey::to_id`]).
    pub fn show_message_page(&mut self, key: &str) {
        let Some(k) = MessageKey::from_id(key) else { return };
        if self.session.run("message.popOut", &json!({"key": key})).is_ok() {
            self.ui.page_message = Some(k);
        }
    }

    /// A message's own window: its toolbar and the message; a reply written here; closed when done.
    pub(crate) fn message_page(&mut self, ui: &mut Ui, key: &MessageKey, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        if self.session.compose.is_some() {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                reading::show(&mut self.session, &mut self.ui, ui, acts);
            });
            return;
        }
        let Some(w) = self.session.windows.iter().find(|w| &w.key == key).cloned() else {
            // Done with: the window goes (a browser may refuse; then this says so).
            self.services.close_page();
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                ui.centered_and_justified(|ui| ui.label(RichText::new(tl!("You can close this window.")).color(t.dim)));
            });
            return;
        };
        let subject =
            w.parsed.as_ref().map(|p| p.subject.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| tl!("(no subject)").to_string());
        let title = tf!("{subject} – Message", subject = subject);
        if title != self.ui.title_shown {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.ui.title_shown = title;
        }
        egui::Panel::top("message-page-bar")
            .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(8, 6)).stroke(egui::Stroke::new(1.0, t.border)))
            .show(ui, |ui| window_toolbar(ui, &self.session, &t, key, acts));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
            reading::window(&mut self.session, &mut self.ui, ui, w, acts);
        });
    }

    /// Messages opened in windows of their own (double-click), each a native window with the
    /// message's own toolbar; in the browser, each in a browser window of its own.
    pub(crate) fn message_windows(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        if self.ui.page_message.is_none() {
            for w in self.session.windows.clone() {
                if self.services.open_message_window(&w.key.to_id()) {
                    self.session.close_window(&w.key);
                }
            }
        }
        for w in self.session.windows.clone() {
            let key = w.key.clone();
            let subject =
                w.parsed.as_ref().map(|p| p.subject.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| tl!("(no subject)").to_string());
            let id = egui::ViewportId::from_hash_of(("message-window", key.to_id()));
            let builder = egui::ViewportBuilder::default()
                .with_title(tf!("{subject} – Message", subject = subject))
                .with_inner_size([940.0, 740.0])
                .with_min_inner_size([480.0, 360.0]);
            // Where the last one was, and its size.
            let builder = match self.ui.places.get("message") {
                Some(p) if opening(ctx, id) => p.apply(builder),
                _ => builder,
            };
            let (session, st) = (&mut self.session, &mut self.ui);
            let close = ctx.show_viewport_immediate(id, builder, |ui, _| {
                let mut place = st.places.get("message").copied();
                after_open(ui, place);
                WindowPlace::note(&mut place, ui);
                if let Some(p) = place {
                    st.places.insert("message".into(), p);
                }
                let mut mine: Vec<Action> = Vec::new();
                let closing = ui.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape));
                egui::Panel::top(egui::Id::unique(("message-window-bar", key.to_id())))
                    .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(8, 6)).stroke(egui::Stroke::new(1.0, t.border)))
                    .show(ui, |ui| window_toolbar(ui, session, &t, &key, &mut mine));
                egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                    reading::window(session, st, ui, w.clone(), &mut mine);
                });
                let leave = mine.iter().any(|a| matches!(a, Action::Run("message.closeWindow", _)));
                // What works on "the open message" works on this window's message.
                acts.extend(mine.into_iter().map(|a| match a {
                    Action::OpenAttachment(_)
                    | Action::SaveAttachment(_)
                    | Action::OpenInCraft(..)
                    | Action::DragOut(_)
                    | Action::SaveAll
                    | Action::Print
                    | Action::ExportPdf
                    | Action::SaveMessage
                    | Action::Run("attachment.preview", _) => Action::InWindow(key.clone(), Box::new(a)),
                    a => a,
                }));
                closing && !leave
            });
            if close {
                forget_open(ctx, id);
                acts.push(Action::Run("message.closeWindow", json!({"key": key.to_id()})));
            }
        }
    }
}

/// Where a window was and how big (in points), and whether it was maximized: its normal place is
/// kept while maximized, so Restore goes back to it, as Outlook does.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindowPlace {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub maximized: bool,
    /// The size of the monitor it was on, so a window whose monitor went (an external one
    /// unplugged) is brought back on screen ([`WindowPlace::rescue`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<[f32; 2]>,
}

/// Room a window leaves on each side of its monitor for the taskbar, the Dock or a desktop panel,
/// which the monitor size (all egui reports) includes.
pub(crate) const SYSTEM_BAR: egui::Vec2 = egui::vec2(48.0, 48.0);

/// The smallest a window is made to fit its monitor (the main window's minimum size).
pub(crate) const FIT_MIN: egui::Vec2 = egui::vec2(760.0, 480.0);

impl WindowPlace {
    /// Note where a window is now (nothing while it's minimized; while maximized, only that).
    pub fn note(place: &mut Option<WindowPlace>, ui: &Ui) {
        let v = ui.input(|i| i.viewport().clone());
        if v.minimized == Some(true) {
            return;
        }
        let maximized = v.maximized == Some(true);
        if maximized {
            if let Some(p) = place.as_mut() {
                p.maximized = true;
            }
            return;
        }
        if let (Some(outer), Some(inner)) = (v.outer_rect, v.inner_rect) {
            let monitor = v.monitor_size.filter(|m| m.is_finite() && m.x > 0.0 && m.y > 0.0).map(|m| [m.x, m.y]);
            *place = Some(WindowPlace { x: outer.min.x, y: outer.min.y, w: inner.width(), h: inner.height(), maximized: false, monitor });
        }
    }

    /// Where to move a restored window, once its monitor is known (the other Craft apps' fix): a
    /// window whose monitor differs from the one it was saved on (an external monitor unplugged)
    /// may sit where no monitor is any more, so it's centred on the primary one, whose top-left
    /// corner is the origin. A maximized window fills whichever monitor the system picks.
    pub fn rescue(self, info: &egui::ViewportInfo) -> Option<egui::Pos2> {
        if self.maximized {
            return None;
        }
        let saved = egui::Vec2::from(self.monitor?);
        let now = info.monitor_size.filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0)?;
        if (now - saved).length() <= 1.0 {
            return None;
        }
        let free = now - egui::vec2(self.w, self.h);
        Some(egui::pos2((free.x / 2.0).max(0.0), (free.y / 2.0).max(0.0)))
    }

    /// Once the monitor is known, a smaller size and a centred position for a window that doesn't
    /// fit on it, frame included: one saved on a bigger screen, or the default size on a laptop,
    /// reached under the taskbar or past the screen's edge (WordCraft found). `restored`: it was
    /// put where a saved place says (perhaps another monitor), so only its size counts; one the
    /// system placed is on the primary monitor, so its position counts too.
    pub fn fit(info: &egui::ViewportInfo, restored: bool) -> Option<(egui::Pos2, egui::Vec2)> {
        if info.maximized == Some(true) || info.fullscreen == Some(true) || info.minimized == Some(true) {
            return None;
        }
        let monitor = info.monitor_size.filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0)?;
        let inner = info.inner_rect.filter(|r| r.is_finite())?;
        let outer = info.outer_rect.filter(|r| r.is_finite()).unwrap_or(inner);
        let frame = (outer.size() - inner.size()).max(egui::Vec2::ZERO);
        let room = monitor - 2.0 * SYSTEM_BAR;
        let too_big = outer.width() > room.x || outer.height() > room.y;
        let edge = monitor - SYSTEM_BAR;
        let past_edge = !restored && (outer.max.x > edge.x || outer.max.y > edge.y);
        if !too_big && !past_edge {
            return None;
        }
        let size = (room - frame).min(inner.size()).max(FIT_MIN);
        let free = monitor - (size + frame);
        Some((egui::pos2((free.x / 2.0).max(0.0), (free.y / 2.0).max(0.0)), size))
    }

    /// A place that's sensible to open at: a real size, and on a screen as far as can be told.
    pub fn usable(self) -> Option<WindowPlace> {
        let fine = |v: f32| v.is_finite();
        (fine(self.x) && fine(self.y) && self.w >= 200.0 && self.h >= 150.0 && self.w < 20_000.0 && self.h < 20_000.0)
            .then_some(self)
            .filter(|p| p.x > -2_000.0 && p.y > -200.0 && p.x < 20_000.0 && p.y < 20_000.0)
    }

    /// Open a window where this place says.
    pub fn apply(self, b: egui::ViewportBuilder) -> egui::ViewportBuilder {
        match self.usable() {
            // On Windows, winit 0.30.13 showed a window created maximized at once, though eframe
            // keeps it hidden until its first frame: it appeared, vanished and appeared again. The
            // patched winit (storytold/winit) maximizes a hidden window only as it's shown.
            Some(p) => b.with_position([p.x, p.y]).with_inner_size([p.w, p.h]).with_maximized(p.maximized),
            None => b,
        }
    }
}

/// What a window does once it has opened, from its place as it opened (`place`), called at the
/// top of each of its frames before the place is noted again: on the first frame that knows its
/// monitor, one restored where no monitor is any more is brought back ([`WindowPlace::rescue`]),
/// and one too big for its screen is made to fit ([`WindowPlace::fit`]).
///
/// [`forget_open`] when it closes, so it does the same when it opens again.
pub fn after_open(ui: &Ui, place: Option<WindowPlace>) {
    let ctx = ui.ctx();
    let id = egui::Id::unique(("after-open", ctx.viewport_id()));
    let place = place.and_then(WindowPlace::usable);
    // Absent: its first frame; Some(pending): still to check once the monitor is known.
    let pending: Option<Option<WindowPlace>> = ctx.data(|d| d.get_temp(id));
    // A window with no place (the system put it) is checked too.
    let pending = pending.unwrap_or_else(|| Some(place.unwrap_or_default()));
    let info = ui.input(|i| i.viewport().clone());
    let known = info.monitor_size.is_some_and(|m| m.is_finite() && m.x > 0.0 && m.y > 0.0);
    let pending = match pending {
        Some(p) if known => {
            let restored = place.is_some();
            if let Some(pos) = p.rescue(&info) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            } else if !p.maximized
                && let Some((pos, size)) = WindowPlace::fit(&info, restored)
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            }
            None
        }
        other => other,
    };
    ctx.data_mut(|d| d.insert_temp(id, pending));
}

/// True until [`after_open`] has run for this viewport. Saved geometry goes into the builder
/// only then; egui turns a changed builder position into a move command, and windows of one
/// kind share a saved place, so passing it on later frames dragged sibling windows around.
pub(crate) fn opening(ctx: &egui::Context, viewport: egui::ViewportId) -> bool {
    ctx.data(|d| d.get_temp::<Option<WindowPlace>>(egui::Id::unique(("after-open", viewport))).is_none())
}

/// A closed window opens afresh next time ([`after_open`]).
pub(crate) fn forget_open(ctx: &egui::Context, viewport: egui::ViewportId) {
    ctx.data_mut(|d| d.remove::<Option<WindowPlace>>(egui::Id::unique(("after-open", viewport))));
}

/// The toolbar of a message window. Replying and forwarding happen in the main window, so
/// they close this one; so do Delete and Archive.
pub(crate) fn window_toolbar(ui: &mut Ui, s: &Session, t: &Tokens, key: &MessageKey, acts: &mut Vec<Action>) {
    let close = || Action::Run("message.closeWindow", json!({"key": key.to_id()}));
    let flagged = s.summary(key).is_some_and(|m| m.flags.flagged);
    let unread = s.summary(key).is_some_and(|m| !m.flags.seen);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (id, icon, label) in
            [("mail.reply", Icon::Reply, "Reply"), ("mail.replyAll", Icon::ReplyAll, "Reply all"), ("mail.forward", Icon::Forward, "Forward")]
        {
            if widgets::tool_button(ui, icon, label, t, true).clicked() {
                acts.push(Action::Open(key.clone()));
                acts.push(Action::Run(id, json!({})));
                acts.push(Action::FocusMain);
                acts.push(close());
            }
        }
        widgets::group_sep(ui, t);
        for (id, icon, label) in [("mail.delete", Icon::Delete, "Delete"), ("mail.archive", Icon::Archive, "Archive")] {
            if widgets::tool_button(ui, icon, label, t, true).clicked() {
                acts.push(Action::Open(key.clone()));
                acts.push(Action::Run(id, json!({})));
                acts.push(close());
            }
        }
        if widgets::tool_button(ui, Icon::Move, "Move to", t, true).clicked() {
            acts.push(Action::Open(key.clone()));
            acts.push(Action::Dialog(Dialog::move_to()));
            acts.push(Action::FocusMain);
            acts.push(close());
        }
        widgets::group_sep(ui, t);
        let (ricon, rlabel) = if unread { (Icon::Read, "Mark read") } else { (Icon::Unread, "Mark unread") };
        if widgets::tool_button(ui, ricon, rlabel, t, true).clicked() {
            acts.push(Action::Run("list.select", json!({"key": key.to_id()})));
            acts.push(Action::Run(if unread { "mail.markRead" } else { "mail.markUnread" }, json!({})));
        }
        if widgets::tool_button(ui, Icon::Flag, if flagged { "Unflag" } else { "Flag" }, t, true).clicked() {
            acts.push(Action::Run("list.select", json!({"key": key.to_id()})));
            acts.push(Action::Run("mail.flag", json!({"on": !flagged})));
        }
        widgets::group_sep(ui, t);
        if widgets::tool_button(ui, Icon::Print, "Print", t, true).clicked() {
            acts.push(Action::Open(key.clone()));
            acts.push(Action::Print);
        }
        // Drawn inside the page (the browser), the window has no title bar to close it from.
        if ui.ctx().embed_viewports() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::tool_button(ui, Icon::Close, "Close", t, true).clicked() {
                    acts.push(close());
                }
            });
        }
    });
}
