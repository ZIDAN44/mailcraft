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

    /// Messages dragged from the list over a message being written in a window of its own (issue
    /// #270): the drag says it attaches them, and let go there they're attached and that window
    /// comes forward. Taken before the panes are drawn, so a folder under that window (the main
    /// window's) doesn't take them too.
    pub(crate) fn messages_onto_windows(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        if !reading::dragged::dragging(ctx) {
            return;
        }
        let windows: Vec<(u64, egui::ViewportId)> = self
            .session
            .all_composes()
            .filter(|c| !compose_inline(c, ctx))
            .map(|c| (c.id, egui::ViewportId::from_hash_of(("compose-window", c.id))))
            .collect();
        if windows.is_empty() {
            return;
        }
        let now = reading::dragged::pointer_on_screen(ctx, self.services.pointer_on_screen());
        let at = reading::dragged::last_place(ctx, now);
        let Some((id, viewport)) = at.and_then(|at| reading::dragged::window_under(ctx, &windows, at)) else { return };
        reading::dragged::note_over(ctx);
        if ctx.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(ctx);
            acts.push(Action::ForCompose(id, Box::new(Action::Run("compose.attachMessages", json!({})))));
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
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
        let zoom = ctx.zoom_factor();
        let builder = crate::os_window()
            .with_title(tf!("{subject} – Message", subject = subject))
            .with_inner_size(at_zoom([900.0, 720.0], zoom))
            .with_min_inner_size(at_zoom([520.0, 420.0], zoom));
        // Where the last one was, and its size; each further one a step down and right.
        let step = 28.0 * (n % 8) as f32;
        let builder = match self.ui.places.get("compose") {
            Some(p) if opening(ctx, id) => WindowPlace { x: p.x + step, y: p.y + step, ..*p }.apply(builder, zoom),
            _ => builder,
        };
        let (session, st, services) = (&mut self.session, &mut self.ui, &mut self.services);
        // Under the frame's guard: a panic in this window's own pass is said once it's over.
        let mut caught = None;
        let mut draw = |ui: &mut egui::Ui| {
            track_place(&mut st.places, "compose", ui);
            // Started from another app: in front. One set aside when another was started stays
            // behind the one being written.
            if st.compose_to_front == Some(mine) {
                raise_once(ui.ctx(), id);
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
            for p in super::dropped::pasted_files(ui.ctx(), services.as_mut(), session) {
                mine_acts.push(Action::Run("compose.attach", json!({"path": p.to_string_lossy()})));
            }
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                reading::compose_window(session, st, ui, &mut mine_acts);
            });
            // A dialog for a message shows in that message's window only.
            let here = st.dialog_for.is_none_or(|f| f == mine);
            if let Some(mut d) = st.dialog.take() {
                if d.for_compose() && here {
                    let keep = dialogs::show(ui.ctx(), &mut d, session, &mut st.pictures, &t, &mut mine_acts);
                    if keep && st.dialog.is_none() {
                        st.dialog = Some(d);
                    }
                } else {
                    st.dialog = Some(d);
                }
            }
            acts.extend(mine_acts);
            closing
        };
        let close = ctx.show_viewport_immediate(id, builder, |ui, _| super::frame::guarded_window(&mut caught, || draw(ui)));
        self.window_drawn(ctx, caught);
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
            .show(ui, |ui| window_toolbar(ui, &self.session, &self.ui, key, acts));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
            reading::window(&mut self.session, &mut self.ui, ui, w, acts);
        });
    }

    /// Messages opened in windows of their own (double-click), each a native window with the
    /// message's own toolbar; in the browser, each in a browser window of its own.
    pub(crate) fn message_windows(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        if self.ui.page_message.is_none() {
            // An opened file isn't handed to a page of its own, which couldn't find it again.
            let handed: Vec<MessageKey> =
                self.session.windows.iter().map(|w| w.key.clone()).filter(|k| self.session.opened_file_name(k).is_none()).collect();
            for key in handed {
                if self.services.open_message_window(&key.to_id()) {
                    self.session.close_window(&key);
                }
            }
        }
        // Under the frame's guard: a panic in a window's own pass is said once they're drawn.
        let mut caught = None;
        for w in self.session.windows.clone() {
            let key = w.key.clone();
            let subject =
                w.parsed.as_ref().map(|p| p.subject.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| tl!("(no subject)").to_string());
            let id = egui::ViewportId::from_hash_of(("message-window", key.to_id()));
            let zoom = ctx.zoom_factor();
            let builder = crate::os_window()
                .with_title(tf!("{subject} – Message", subject = subject))
                .with_inner_size(at_zoom([940.0, 740.0], zoom))
                .with_min_inner_size(at_zoom([480.0, 360.0], zoom));
            // Where the last one was, and its size.
            let builder = match self.ui.places.get("message") {
                Some(p) if opening(ctx, id) => p.apply(builder, zoom),
                _ => builder,
            };
            let (session, st) = (&mut self.session, &mut self.ui);
            let mut draw = |ui: &mut egui::Ui| {
                // Reading here is the user being back, as much as in the main window.
                st.compact.note_input(ui);
                raise_once(ui.ctx(), id);
                track_place(&mut st.places, "message", ui);
                let mut mine: Vec<Action> = Vec::new();
                let closing = ui.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape));
                egui::Panel::top(egui::Id::unique(("message-window-bar", key.to_id())))
                    .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(8, 6)).stroke(egui::Stroke::new(1.0, t.border)))
                    .show(ui, |ui| window_toolbar(ui, session, st, &key, &mut mine));
                // An opened file is in no cache: its pictures come out of what was read of it.
                if let Some(raw) = session.opened_file_source(&key) {
                    st.pictures.keep_source(&key, raw);
                }
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
            };
            let close = ctx.show_viewport_immediate(id, builder, |ui, _| super::frame::guarded_window(&mut caught, || draw(ui)));
            if close {
                forget_open(ctx, id);
                acts.push(Action::Run("message.closeWindow", json!({"key": key.to_id()})));
            }
        }
        self.window_drawn(ctx, caught);
    }
}

/// Where a window was and how big (in points), and whether it was maximized: its normal place is
/// kept while maximized, so Restore goes back to it, as Outlook does.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
/// Where a window was and its size, in its monitor's own points (as the system has them, whatever
/// the interface's text size): a window keeps its size and place on the screen as the text is made
/// larger or smaller, and opens again so.
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
/// which the monitor size (all egui reports) includes, where the system doesn't say where they are
/// ([`Monitors::work`]).
pub(crate) const SYSTEM_BAR: egui::Vec2 = egui::vec2(48.0, 48.0);

/// How far past a work area's sides and bottom a window it fits may reach: Windows draws the
/// resize border of a window it frames outside it, unseen (7 pixels at 100 %, in the outer rect).
const UNSEEN_BORDERS: f32 = 16.0;

/// The smallest a window is made to fit its monitor (the main window's minimum size).
pub(crate) const FIT_MIN: egui::Vec2 = egui::vec2(760.0, 480.0);

/// The monitors as the system lists them, in physical pixels: every one, and the primary one
/// where the system says which (not on Wayland, nor in the browser).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Monitors {
    pub all: Vec<egui::Rect>,
    pub primary: Option<egui::Rect>,
    /// The part of each that windows may use, without the taskbar, the Dock or the menu bar,
    /// where the system says (Windows, macOS).
    pub work: Vec<egui::Rect>,
}

fn monitors_id() -> egui::Id {
    egui::Id::unique("system-monitors")
}

/// Tell every window where the monitors are (the app asks the system every so often), for a
/// window opening where none is any more ([`WindowPlace::rescue`]).
pub fn set_monitors(ctx: &egui::Context, monitors: Option<Monitors>) {
    ctx.data_mut(|d| d.insert_temp(monitors_id(), monitors.map(std::sync::Arc::new)));
}

/// What [`set_monitors`] said last.
fn monitors(ctx: &egui::Context) -> Option<std::sync::Arc<Monitors>> {
    ctx.data(|d| d.get_temp::<Option<std::sync::Arc<Monitors>>>(monitors_id())).flatten()
}

/// How much of a window's top edge must be on one monitor for the pointer to drag it back.
const GRIP: egui::Vec2 = egui::vec2(96.0, 16.0);

/// Whether a [`GRIP`] of `window`'s top edge is on one of `monitors` (all in one unit).
pub(crate) fn reachable(window: egui::Rect, monitors: &[egui::Rect]) -> bool {
    let top = egui::Rect::from_min_size(window.min, egui::vec2(window.width(), window.height().min(GRIP.y)));
    let need = GRIP.min(top.size());
    monitors.iter().any(|m| {
        let on = top.intersect(*m);
        on.is_positive() && on.width() >= need.x && on.height() >= need.y
    })
}

/// Light or dark as chosen in MailCraft (none: as the system is), for the title bars the system
/// draws on its windows: they followed the system alone, a dark bar on a light MailCraft (issue
/// #74). Said once a frame by the main window; each window tells the system when it changes.
pub(crate) fn title_bars_in(ctx: &egui::Context, dark: Option<bool>) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("title-bars-in"), dark));
}

/// This window's title bar in MailCraft's light or dark ([`title_bars_in`]).
fn title_bar_follows(ctx: &egui::Context) {
    let Some(dark) = ctx.data(|d| d.get_temp::<Option<bool>>(egui::Id::unique("title-bars-in"))) else { return };
    // Told again over a window's first frames: one that's just opening doesn't take it yet (on
    // X11 it stayed as it opened).
    const TIMES: u8 = 3;
    let told = egui::Id::unique(("title-bar-told", ctx.viewport_id()));
    let times = match ctx.data(|d| d.get_temp::<(Option<bool>, u8)>(told)) {
        Some((was, times)) if was == dark => times,
        _ => 0,
    };
    if times >= TIMES {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(told, (dark, times + 1)));
    ctx.request_repaint();
    let theme = match dark {
        Some(true) => egui::SystemTheme::Dark,
        Some(false) => egui::SystemTheme::Light,
        None => egui::SystemTheme::SystemDefault,
    };
    ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(theme));
}

impl WindowPlace {
    /// Note where a window is now (nothing while it's minimized; while maximized, only that).
    /// Every window comes by here each frame: its title bar is kept in MailCraft's light or dark.
    pub fn note(place: &mut Option<WindowPlace>, ui: &Ui) {
        title_bar_follows(ui.ctx());
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
            let monitor = monitor_of(&v).map(|m| [m.x, m.y]);
            let here = WindowPlace { x: outer.min.x, y: outer.min.y, w: inner.width(), h: inner.height(), maximized: false, monitor };
            *place = Some(here.scaled(ui.ctx().zoom_factor()));
        }
    }

    /// Where to move a restored window, once its monitor is known (the other Craft apps' fix): a
    /// window whose monitor differs from the one it was saved on (an external monitor unplugged),
    /// or that the system's `monitors` say no monitor holds enough of to be dragged by, is
    /// centred on the primary monitor (whose top-left corner is the origin where the system
    /// doesn't say where it is). Sizes alone can't tell: a monitor unplugged can leave one of
    /// the same size (WordCraft found). A maximized window fills whichever monitor the system
    /// picks.
    /// The place and `monitors` in the window's points at text size `zoom` (egui's zoom factor).
    pub fn rescue(self, info: &egui::ViewportInfo, monitors: Option<&Monitors>, zoom: f32) -> Option<egui::Pos2> {
        if self.maximized {
            return None;
        }
        let now = monitor_of(info)?;
        let other = self.monitor.is_some_and(|saved| (now - egui::Vec2::from(saved)).length() > 1.0);
        // In the monitors' physical pixels.
        let ppp = pixels_per_point(info, zoom);
        let lost = match (monitors.filter(|m| !m.all.is_empty()), info.outer_rect.filter(|r| r.is_finite())) {
            (Some(m), Some(outer)) => !reachable(egui::Rect::from_min_max(outer.min * ppp, outer.max * ppp), &m.all),
            _ => false,
        };
        if !other && !lost {
            return None;
        }
        let primary = monitors.and_then(|m| m.primary).map(|r| egui::Rect::from_min_max(r.min / ppp, r.max / ppp));
        let area = primary.unwrap_or_else(|| egui::Rect::from_min_size(egui::Pos2::ZERO, now));
        let free = area.size() - egui::vec2(self.w, self.h);
        Some(area.min + (free / 2.0).max(egui::Vec2::ZERO))
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
        let monitor = monitor_of(info)?;
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

    /// As [`WindowPlace::fit`], where the system says what part of the monitor windows may use
    /// (`work`, in points): a window that doesn't fit in it, frame included, is made smaller and
    /// centred there; one whose title bar is above it (under a taskbar or the menu bar along the
    /// top, or past the screen's top) is moved down into it, as little as it takes; one the system
    /// placed past its edge is centred there. A window that fits it to within the borders the
    /// system leaves unseen round a window it frames is left as it is.
    pub fn fit_in(info: &egui::ViewportInfo, restored: bool, work: egui::Rect) -> Option<(egui::Pos2, egui::Vec2)> {
        if info.maximized == Some(true) || info.fullscreen == Some(true) || info.minimized == Some(true) {
            return None;
        }
        let inner = info.inner_rect.filter(|r| r.is_finite())?;
        let outer = info.outer_rect.filter(|r| r.is_finite()).unwrap_or(inner);
        let frame = (outer.size() - inner.size()).max(egui::Vec2::ZERO);
        let too_big = outer.width() > work.width() + UNSEEN_BORDERS || outer.height() > work.height() + UNSEEN_BORDERS;
        // The top edge has no unseen border: a title bar above the work area can't be grabbed.
        let above = outer.min.y < work.min.y - 1.0;
        let past = !restored
            && (outer.min.x < work.min.x - UNSEEN_BORDERS || outer.max.x > work.max.x + UNSEEN_BORDERS || outer.max.y > work.max.y + UNSEEN_BORDERS);
        if !too_big && !above && !past {
            return None;
        }
        let room = (work.size() - frame).max(egui::Vec2::ZERO);
        let size = if too_big { inner.size().min(room).max(FIT_MIN.min(room)) } else { inner.size() };
        let free = (work.size() - (size + frame)).max(egui::Vec2::ZERO);
        let pos = if too_big || past {
            work.min + free / 2.0
        } else {
            egui::pos2(outer.min.x.clamp(work.min.x, work.min.x + free.x), outer.min.y.clamp(work.min.y, work.min.y + free.y))
        };
        Some((pos, size))
    }

    /// A place that's sensible to open at: a real size, and on a screen as far as can be told.
    pub fn usable(self) -> Option<WindowPlace> {
        let fine = |v: f32| v.is_finite();
        (fine(self.x) && fine(self.y) && self.w >= 200.0 && self.h >= 150.0 && self.w < 20_000.0 && self.h < 20_000.0)
            .then_some(self)
            .filter(|p| p.x > -2_000.0 && p.y > -200.0 && p.x < 20_000.0 && p.y < 20_000.0)
    }

    /// This place, kept in the monitor's own points, in the points of text size `zoom` (egui's
    /// zoom factor), which windows are placed and measured in; or back, by `1 / zoom`.
    pub fn at_zoom(self, zoom: f32) -> WindowPlace {
        self.scaled(1.0 / zoom_or_own(zoom))
    }

    fn scaled(self, by: f32) -> WindowPlace {
        let by = zoom_or_own(by);
        WindowPlace {
            x: self.x * by,
            y: self.y * by,
            w: self.w * by,
            h: self.h * by,
            maximized: self.maximized,
            monitor: self.monitor.map(|[w, h]| [w * by, h * by]),
        }
    }

    /// Open a window where this place says, at text size `zoom` (egui's zoom factor; 1 for the
    /// main window, opened before the text size is set).
    pub fn apply(self, b: egui::ViewportBuilder, zoom: f32) -> egui::ViewportBuilder {
        match self.at_zoom(zoom).usable() {
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
    let zoom = ctx.zoom_factor();
    let place = place.map(|p| p.at_zoom(zoom)).and_then(WindowPlace::usable);
    // Absent: its first frame; Some(pending): still to check once the monitor is known.
    let pending: Option<Option<WindowPlace>> = ctx.data(|d| d.get_temp(id));
    // A window with no place (the system put it) is checked too.
    let pending = pending.unwrap_or_else(|| Some(place.unwrap_or_default()));
    let info = ui.input(|i| i.viewport().clone());
    let known = monitor_of(&info).is_some();
    let pending = match pending {
        Some(p) if known => {
            let restored = place.is_some();
            if let Some(pos) = p.rescue(&info, monitors(ctx).as_deref(), zoom) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            } else if !p.maximized
                && let Some((pos, size)) = match work_area(&info, monitors(ctx).as_deref(), zoom) {
                    Some(work) => WindowPlace::fit_in(&info, restored, work),
                    None => WindowPlace::fit(&info, restored),
                }
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

/// What every window does at the top of each of its frames: [`after_open`] from where windows of
/// its `kind` ("main", "message", "compose", "source") were last, then where it is now noted
/// there for the next one.
pub(crate) fn track_place(places: &mut std::collections::BTreeMap<String, WindowPlace>, kind: &str, ui: &Ui) {
    let mut place = places.get(kind).copied();
    after_open(ui, place);
    WindowPlace::note(&mut place, ui);
    if let Some(p) = place {
        // The kind's key made only the once, not at every frame.
        match places.get_mut(kind) {
            Some(kept) => *kept = p,
            None => {
                places.insert(kind.to_string(), p);
            }
        }
    }
}

/// The work area ([`Monitors::work`]) of the monitor most of a window is on, in its points at
/// text size `zoom` (egui's zoom factor).
pub(crate) fn work_area(info: &egui::ViewportInfo, monitors: Option<&Monitors>, zoom: f32) -> Option<egui::Rect> {
    let ppp = pixels_per_point(info, zoom);
    let outer = info.outer_rect.filter(|r| r.is_finite())?;
    let window = egui::Rect::from_min_max(outer.min * ppp, outer.max * ppp);
    let on = |w: &egui::Rect| {
        let both = w.intersect(window);
        if both.is_positive() { both.area() } else { 0.0 }
    };
    let work = monitors?.work.iter().filter(|w| w.is_finite() && w.is_positive() && on(w) > 0.0).max_by(|a, b| on(a).total_cmp(&on(b)))?;
    Some(egui::Rect::from_min_max(work.min / ppp, work.max / ppp))
}

/// The physical pixels in one of a window's points: the monitor's scale, times the text size
/// (egui's zoom factor), as egui measures windows and monitors.
fn pixels_per_point(info: &egui::ViewportInfo, zoom: f32) -> f32 {
    info.native_pixels_per_point.filter(|p| p.is_finite() && *p > 0.0).unwrap_or(1.0) * zoom_or_own(zoom)
}

/// A zoom factor to measure by: one that makes no sense is the interface's own.
fn zoom_or_own(zoom: f32) -> f32 {
    if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 }
}

/// A window's size in the monitor's own points as egui's points at text size `zoom`: the size a
/// window opens at, and the least it can be, stay as they are on the screen at any text size (at
/// 200 % a message window's least height was taller than a laptop's screen).
pub(crate) fn at_zoom(size: [f32; 2], zoom: f32) -> [f32; 2] {
    let zoom = zoom_or_own(zoom);
    [size[0] / zoom, size[1] / zoom]
}

/// The size of the monitor a window is on, when the system has told a sensible one.
fn monitor_of(info: &egui::ViewportInfo) -> Option<egui::Vec2> {
    info.monitor_size.filter(|m| m.is_finite() && m.x > 0.0 && m.y > 0.0)
}

/// True until [`after_open`] has run for this viewport. Saved geometry goes into the builder
/// only then; egui turns a changed builder position into a move command, and windows of one
/// kind share a saved place, so passing it on later frames dragged sibling windows around.
pub(crate) fn opening(ctx: &egui::Context, viewport: egui::ViewportId) -> bool {
    ctx.data(|d| d.get_temp::<Option<WindowPlace>>(egui::Id::unique(("after-open", viewport))).is_none())
}

/// A closed window opens afresh next time ([`after_open`]).
pub(crate) fn forget_open(ctx: &egui::Context, viewport: egui::ViewportId) {
    ctx.data_mut(|d| {
        d.remove::<Option<WindowPlace>>(egui::Id::unique(("after-open", viewport)));
        d.remove::<Raised>(egui::Id::unique(("raised", viewport)));
    });
}

/// How far a window just opened has come to the front ([`raise_once`]).
#[derive(Clone, Copy)]
struct Raised {
    /// Its frames so far, counted to two.
    frames: u8,
    /// When it opened (egui's input time).
    opened: f64,
    /// Done with: brought forward again, or something done in the main window.
    settled: bool,
}

/// How long after a window opens the main window taking the focus from it is the system's doing,
/// not the user's.
const RAISE_FOR: f64 = 2.0;

/// A message window just opened comes to the front when it's on screen (its second frame), and
/// once more if the main window takes the focus from it in its first [`RAISE_FOR`] seconds with
/// nothing clicked or typed there. The window of a mail file handed to a running MailCraft opened
/// behind the main window, which that launch had just brought forward; so did one for a file
/// MailCraft was started with: on X11 (Openbox), the main window, shown once it has drawn, took
/// the focus 40 ms after the message window had it.
fn raise_once(ctx: &egui::Context, viewport: egui::ViewportId) {
    let id = egui::Id::unique(("raised", viewport));
    let now = ctx.input(|i| i.time);
    let mut r = ctx.data(|d| d.get_temp::<Raised>(id)).unwrap_or(Raised { frames: 0, opened: now, settled: false });
    if r.frames == 1 {
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    } else if r.frames > 1 && !r.settled {
        let acted = ctx.input_for(egui::ViewportId::ROOT, |i| {
            i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::PointerButton { .. } | egui::Event::Text(_)))
        });
        let taken =
            ctx.input_for(egui::ViewportId::ROOT, |i| i.viewport().focused == Some(true)) && ctx.input(|i| i.viewport().focused != Some(true));
        if acted || now - r.opened > RAISE_FOR {
            r.settled = true;
        } else if taken {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            r.settled = true;
        }
    }
    r.frames = r.frames.saturating_add(1).min(2);
    ctx.data_mut(|d| d.insert_temp(id, r));
}

/// The toolbar of a message window. Replying and forwarding happen in the main window, so
/// they close this one; so do Delete and Archive.
pub(crate) fn window_toolbar(ui: &mut Ui, s: &Session, st: &UiState, key: &MessageKey, acts: &mut Vec<Action>) {
    if s.opened_file_name(key).is_some() {
        opened_file_toolbar(ui, s, st, key, acts);
        return;
    }
    widgets::fitted_toolbar(ui, egui::Id::unique(("message-toolbar-fit", ui.ctx().viewport_id())), |ui| toolbar_row(ui, s, st, key, acts));
}

/// The row of a message window's toolbar (what [`window_toolbar`] fits to the window).
fn toolbar_row(ui: &mut Ui, s: &Session, st: &UiState, key: &MessageKey, acts: &mut Vec<Action>) {
    let t = &st.tokens;
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
        more_and_close(ui, s, st, key, acts);
    });
}

/// The end of a message window's toolbar: More, as the reading pane's ⋯ (what's done to this
/// message alone, staying open), and Close where the window has no title bar to close it from
/// (drawn inside the page, the browser's).
fn more_and_close(ui: &mut Ui, s: &Session, st: &UiState, key: &MessageKey, acts: &mut Vec<Action>) {
    let t = &st.tokens;
    let more = widgets::more_pill(ui, "More actions", t);
    egui::Popup::menu(&more).show(|ui| {
        if ui.button(tl!("View message source")).clicked() {
            acts.push(Action::Run("message.source", json!({"key": key.to_id()})));
            ui.close();
        }
        if let Some(p) = s.windows.iter().find(|w| &w.key == key).and_then(|w| w.parsed.as_ref()) {
            reading::plain_text_check(s, st, ui, key, p, acts);
        }
    });
    if ui.ctx().embed_viewports() {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::tool_button(ui, Icon::Close, "Close", t, true).clicked() {
                acts.push(Action::Run("message.closeWindow", json!({"key": key.to_id()})));
            }
        });
    }
}

/// The toolbar of an opened message file's window (issue #56): what looks at it or keeps a copy
/// of it. It's in no folder and no account, so nothing answers, files or marks it.
fn opened_file_toolbar(ui: &mut Ui, s: &Session, st: &UiState, key: &MessageKey, acts: &mut Vec<Action>) {
    let t = &st.tokens;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if widgets::tool_button(ui, Icon::Print, "Print", t, true).on_hover_text(tl!("Open as a PDF to print")).clicked() {
            acts.push(Action::Print);
        }
        let save = widgets::tool_button(ui, Icon::File, "Save", t, true);
        egui::Popup::menu(&save).show(|ui| {
            if ui.button(tl!("Save as PDF…")).clicked() {
                acts.push(Action::ExportPdf);
                ui.close();
            }
            if ui.button(tl!("Save as email (.eml)…")).clicked() {
                acts.push(Action::SaveMessage);
                ui.close();
            }
        });
        more_and_close(ui, s, st, key, acts);
    });
}
