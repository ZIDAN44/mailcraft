//! The Activity panel, under its button: what needs you, what's under way and how far it has
//! come, and what MailCraft said lately, each with what can be done about it. An import's details
//! open in it, under the import's line (the folder pane's line under a data file opens them too).

use super::*;

/// The panel's width.
pub(crate) const PANEL_W: f32 = 360.0;
/// The notices it keeps under Recent, newest first.
const RECENT: usize = 5;
/// The rows' margin from the panel's edges.
const EDGE: f32 = 14.0;

impl MailApp {
    /// The panel, while it's open (or an import's details were asked for), under the button;
    /// a click outside it or Esc puts it away.
    pub(crate) fn activity_panel(&mut self, ctx: &egui::Context) {
        if !self.ui.activity.open && self.ui.import_details.is_none() {
            return;
        }
        // Not where it opens from (the File page has no top bar): put away.
        let Some(button) = self.ui.activity.button.filter(|_| self.ui.backstage.is_none()) else {
            self.close_activity();
            return;
        };
        let t = self.ui.tokens;
        let a = self.ui.activity.current.clone();
        let screen = ctx.content_rect();
        let x = if i18n::current().rtl() { button.left() } else { button.right() - PANEL_W };
        let at = egui::pos2(x.clamp(8.0, (screen.right() - PANEL_W - 8.0).max(8.0)), button.bottom() + 6.0);
        let tall = (screen.bottom() - at.y - 16.0).max(160.0);
        // Opening, it eases down from just under the button and in.
        let now = ctx.input(|i| i.time);
        let opened = *self.ui.activity.opened.get_or_insert(now);
        let come = motion::come(ctx, now - opened);
        let at = at - vec2(0.0, 6.0 * (1.0 - come));
        let mut acts: Vec<Action> = Vec::new();
        let mut clear = false;
        let mut close = false;
        let shown = egui::Area::new(egui::Id::unique("activity-panel")).order(egui::Order::Foreground).fixed_pos(at).fade_in(false).show(ctx, |ui| {
            ui.multiply_opacity(come);
            egui::Frame::new()
                .fill(t.surface)
                .stroke(egui::Stroke::new(1.0, t.border))
                .corner_radius(crate::sizes::SURFACE_RADIUS)
                .shadow(ui.visuals().popup_shadow)
                .inner_margin(Margin::symmetric(0, 10))
                .show(ui, |ui| {
                    ui.set_width(PANEL_W);
                    // The room it may take, down to the window's bottom: an Area gives what's in
                    // it the size it had last frame, so details opened under a line were cut off
                    // by the scroll area, which never grew past that.
                    ui.set_max_height(tall + 48.0);
                    let recent = self.recent();
                    ui.horizontal(|ui| {
                        ui.add_space(EDGE);
                        ui.label(RichText::new(tl!("Activity")).font(theme::semibold(crate::sizes::BODY)).color(t.text));
                        ui.with_layout(bidi::row_from_end(Align::Center), |ui| {
                            ui.add_space(EDGE - 6.0);
                            close |= widgets::tiny_icon_button(ui, Icon::Close, "Close", &t).clicked();
                            if !recent.is_empty()
                                && ui.add(egui::Button::new(RichText::new(tl!("Clear")).color(t.accent).small()).frame(false)).clicked()
                            {
                                clear = true;
                            }
                        });
                    });
                    egui::ScrollArea::vertical().max_height(tall).show(ui, |ui| {
                        self.panel_rows(ui, &t, &a, &recent, &mut acts);
                    });
                });
        });
        let panel = shown.response.rect;
        let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !panel.contains(p) && !button.contains(p)));
        if close || outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.close_activity();
        }
        if clear {
            self.ui.activity.cleared = self.session.notices.len();
        }
        // What was asked for (a dialog, signing in) takes the panel's place; an import's details
        // stay in it.
        let only_details = acts.iter().all(|a| matches!(a, Action::ImportDetails(_)));
        if !acts.is_empty() && !only_details {
            self.close_activity();
        }
        for act in acts {
            if let Action::ImportDetails(id) = &act
                && self.ui.import_details.as_deref() == Some(id.as_str())
            {
                // Its details again: they close.
                self.ui.import_details = None;
                continue;
            }
            self.act(act);
        }
    }

    pub(crate) fn close_activity(&mut self) {
        self.ui.activity.open = false;
        self.ui.activity.opened = None;
        self.ui.import_details = None;
    }

    /// The notices not cleared, newest first, as many as Recent keeps: their words, whether it
    /// went wrong, and when.
    fn recent(&self) -> Vec<(String, bool, i64)> {
        let from = self.ui.activity.cleared.min(self.session.notices.len());
        self.session.notices.get(from..).unwrap_or_default().iter().rev().take(RECENT).map(|n| (n.text.clone(), n.error, n.at)).collect()
    }

    fn panel_rows(&self, ui: &mut Ui, t: &Tokens, a: &Activity, recent: &[(String, bool, i64)], acts: &mut Vec<Action>) {
        let s = &self.session;
        if !a.needs.is_empty() {
            heading(ui, t, "Needs you");
            for p in &a.needs {
                let color = match p.severity {
                    Severity::Error => t.danger,
                    Severity::Warning => t.warn,
                    Severity::Info => t.accent,
                };
                row(ui, t, p.icon, color, &p.title, &p.detail, None, p.action.as_ref(), acts);
                self.details_under(ui, t, &p.key, acts);
            }
        }
        if !a.work.is_empty() {
            heading(ui, t, "In progress");
            let now = ui.input(|i| i.time);
            for w in &a.work {
                // Fading in as it comes and out a moment after it's done (`Held`).
                let seen = self.ui.activity.opacity(ui.ctx(), &w.key, now);
                ui.scope(|ui| {
                    ui.multiply_opacity(seen);
                    row(ui, t, w.icon, t.dim, &w.title, &w.detail, Some(w.permille), w.action.as_ref(), acts);
                });
                self.details_under(ui, t, &w.key, acts);
            }
        }
        if !recent.is_empty() {
            heading(ui, t, "Recent");
            for (text, error, at) in recent {
                let (icon, color) = if *error { (Icon::Warning, t.danger) } else { (Icon::Done, t.ok) };
                let when = i18n::date(&mailcraft_engine::model::dates::list_time(*at, &s.clock));
                row(ui, t, icon, color, &i18n::notice(text), &when, None, None, acts);
            }
        }
        if a.needs.is_empty() && a.work.is_empty() && recent.is_empty() {
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(tl!("Nothing's going on in the background")).color(t.dim));
            });
            ui.add_space(10.0);
        }
    }

    /// Under an import's line, its details while they're open.
    fn details_under(&self, ui: &mut Ui, t: &Tokens, key: &str, acts: &mut Vec<Action>) {
        let Some(account) = key.strip_prefix("import:") else { return };
        if self.ui.import_details.as_deref() != Some(account) {
            return;
        }
        let Some(st) = self.session.import_state(account).filter(|st| !st.removing) else { return };
        egui::Frame::new()
            .fill(t.chrome)
            .corner_radius(crate::sizes::CONTROL_RADIUS)
            .inner_margin(Margin::same(10))
            .outer_margin(Margin::symmetric(10, 4))
            .show(ui, |ui| crate::app::importing::details_ui(ui, &self.session, t, account, st, acts));
    }
}

/// A section's heading.
fn heading(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(EDGE);
        ui.label(RichText::new(tl!(text)).font(theme::semibold(crate::sizes::CAPTION)).color(t.dim));
    });
    ui.add_space(2.0);
}

/// One line of the panel: its icon, what it is, a bar of how far (`progress`: `Some` for work,
/// its thousandths `None` while that isn't known), a detail under it, and its button.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    t: &Tokens,
    icon: Icon,
    color: Color32,
    title: &str,
    detail: &str,
    progress: Option<Option<u16>>,
    action: Option<&(&'static str, Do)>,
    acts: &mut Vec<Action>,
) {
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.add_space(EDGE);
        let (ir, _) = ui.allocate_exact_size(vec2(16.0, 20.0), Sense::hover());
        icons::paint(ui.painter(), icons::centered(ir, 16.0), icon, color);
        ui.add_space(8.0);
        let button = action.map(|(words, _)| {
            let w = ui.painter().layout_no_wrap(tl!(words).to_string(), theme::regular(crate::sizes::CAPTION), t.accent).size().x;
            w + 16.0
        });
        let width = (ui.available_width() - button.unwrap_or(0.0) - EDGE).max(80.0);
        ui.vertical(|ui| {
            ui.set_width(width);
            // Wrapped, not cut off: the panel is narrow, and the end of what it says matters
            // ("…until you go online", how long is left).
            ui.add(egui::Label::new(RichText::new(title).color(t.text)).wrap());
            if let Some(permille) = progress {
                ui.add_space(3.0);
                progress::bar(ui, t, permille, vec2(width, 4.0));
                ui.add_space(1.0);
            }
            if !detail.is_empty() {
                ui.add(egui::Label::new(RichText::new(detail).color(t.dim).small()).wrap());
            }
        });
        if let Some((words, what)) = action
            && ui.add(egui::Button::new(RichText::new(tl!(words)).color(t.accent).small()).frame(false)).clicked()
        {
            acts.push(what.action());
        }
    });
    ui.add_space(4.0);
}
