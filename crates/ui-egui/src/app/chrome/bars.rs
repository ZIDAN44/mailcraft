//! The top bar (the ribbon's tabs, search, Activity, Send / Receive, settings), and notices.

use crate::*;

/// A row of the top bar: its tabs, its buttons, the search box.
pub(crate) const BAR_ROW_H: f32 = 30.0;
/// The room the controls at the bar's end take: three of them (Undo's kept while there's nothing
/// to undo, so the box doesn't move when there is) and a margin.
const BAR_END: f32 = 3.0 * 34.0 + 8.0;
/// The narrowest the search box is beside the tabs (its edge and margins included); narrower,
/// it has a row of its own.
const MIN_SEARCH_BOX: f32 = 216.0;
const MAX_SEARCH_BOX: f32 = 656.0;

impl MailApp {
    /// The top bar: the ribbon's tabs, the search box, and at its end Send / Receive, Undo and
    /// settings (issue #22; the theme is under View and in Options, as other apps keep it). One row, where the app's name (the window's title already
    /// says it) and the bar's controls had a row of their own above the tabs. A window too narrow
    /// for the box beside the tabs has it on a row of its own under them.
    ///
    /// On Windows, Linux and the BSDs it is also the window's title bar ([`Services::own_title_bar`]): its free space
    /// drags the window, MailCraft's mark is at its start and the window's buttons at its end.
    pub(crate) fn top_bar(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let own = self.services.own_title_bar();
        let inner = ui.max_rect();
        // The panel's whole rect, its margins too: the window's buttons sit flush with its corner.
        let bar = inner.expand2(vec2(f32::from(super::frame::TOP_BAR_MARGIN.0), f32::from(super::frame::TOP_BAR_MARGIN.1)));
        // The bar's contents keep clear of the window's buttons.
        let t = self.ui.tokens;
        let full = if own { super::caption::title_bar(ui, &t, bar) } else { inner };
        let controls = ui.scope_builder(egui::UiBuilder::new().max_rect(full), |ui| self.top_bar_rows(ui, full, own, acts)).inner;
        if own {
            super::caption::note_bar(ui.ctx(), bar, super::caption::top_row(bar), controls);
        }
    }

    /// The top bar's tabs, search box and controls, in `full`; where they are (what isn't the
    /// bar's free space).
    fn top_bar_rows(&mut self, ui: &mut Ui, full: egui::Rect, own: bool, acts: &mut Vec<Action>) -> Vec<egui::Rect> {
        let row = vec2(full.width(), BAR_ROW_H);
        let mut boxed = 0.0;
        let mut controls = Vec::new();
        ui.allocate_ui_with_layout(row, bidi::row_from_start(Align::Center), |ui| {
            if own {
                let t = self.ui.tokens;
                super::caption::mark(ui, &t);
            }
            self.ribbon_tabs(ui, acts);
            controls.push(ui.min_rect());
            ui.spacing_mut().item_spacing.x = 4.0;
            // The room between the tabs and the controls at the end.
            let used = ui.min_rect().width();
            // The Activity button's room too, while it shows.
            let activity = if self.activity_shown_now(ui) { 34.0 } else { 0.0 };
            let free = full.width() - used - BAR_END - activity;
            let fits = free - 24.0 >= MIN_SEARCH_BOX;
            if self.ui.bar_two_rows == fits {
                self.ui.bar_two_rows = !fits;
                ui.ctx().request_repaint();
            }
            if fits {
                // The box, its edge and margins included: centred in the window when there's
                // room for that, else just after the tabs.
                boxed = (free - 24.0).min(MAX_SEARCH_BOX);
                let centred = (full.width() - boxed) / 2.0 - used;
                ui.add_space(centred.min(free - boxed - 12.0).max(12.0));
                controls.push(self.top_search(ui, boxed - 16.0, acts));
            }
            controls.push(self.bar_end(ui, acts));
        });
        if boxed == 0.0 {
            ui.allocate_ui_with_layout(row, Layout::top_down(Align::Center), |ui| {
                let boxed = (full.width() - 16.0).clamp(80.0, MAX_SEARCH_BOX);
                controls.push(self.top_search(ui, boxed - 16.0, acts));
            });
        }
        controls
    }

    /// The search box on the top bar, `search_w` wide inside its edge; where it is.
    fn top_search(&mut self, ui: &mut Ui, search_w: f32, acts: &mut Vec<Action>) -> egui::Rect {
        let t = self.ui.tokens;
        // On a plain bar the box needs its own edge to stand out.
        let edge = if t.bar == t.chrome { egui::Stroke::new(1.0, t.border) } else { egui::Stroke::NONE };
        let frame = egui::Frame::new()
            .fill(if t.dark { t.surface } else { Color32::WHITE })
            .stroke(edge)
            .corner_radius(crate::sizes::SURFACE_RADIUS)
            .inner_margin(Margin::symmetric(8, 3));
        frame
            .show(ui, |ui| {
                // Keep the search icon and field on the language's reading edge.
                ui.with_layout(bidi::row_from_start(Align::Center), |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), r, Icon::Search, t.dim);
                    // In the Calendar, People, Tasks and Notes it searches those (as Outlook's box
                    // searches where you are).
                    let module = self.session.module;
                    if matches!(module, Module::Calendar | Module::People | Module::Tasks | Module::Notes) {
                        let mut focus = std::mem::take(&mut self.ui.focus_search);
                        let (query, hint) = match module {
                            Module::People => (&mut self.ui.people_query, tl!("Search people")),
                            Module::Calendar => (&mut self.ui.cal_search.query, tl!("Search calendar (location:, organizer:, after:…)")),
                            Module::Tasks => (&mut self.ui.tasks_query, tl!("Search tasks")),
                            _ => (&mut self.ui.notes_query, tl!("Search notes")),
                        };
                        // An id of its own, as the mail's (issue #158): the bar's tabs before it change.
                        let resp = ui.add(
                            crate::widgets::line_edit(query)
                                .id(egui::Id::unique(("mailcraft-module-search", module as u8)))
                                .hint_text(hint)
                                .frame(egui::Frame::NONE)
                                .align(if i18n::current().rtl() { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER })
                                .desired_width(search_w - 60.0)
                                .text_color(t.text),
                        );
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("mailcraft-top-search-input-rect"), resp.rect));
                        // Asked for again (Ctrl+E): the query selected, so what's typed replaces it.
                        crate::look::fields::focus_selecting_all(ui, &resp, query, &mut focus);
                        if !query.is_empty() && widgets::tiny_icon_button(ui, Icon::Close, "Clear search", &t).clicked() {
                            query.clear();
                        }
                        return;
                    }
                    self.search_box(ui, search_w, acts);
                });
            })
            .response
            .rect
    }

    /// The top bar's end: settings, Send / Receive all folders, Activity while it shows, and Undo.
    fn bar_end(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) -> egui::Rect {
        let t = self.ui.tokens;
        let can_sync = self.session.registry().get("mail.sync").is_some_and(|c| (c.enabled)(&self.session).is_none());
        let can_undo = !self.session.undo.is_empty();
        ui.with_layout(bidi::row_from_end(Align::Center), |ui| {
            if widgets::bar_icon_button(ui, Icon::Gear, "Accounts and settings", &t).clicked() {
                acts.push(Action::Backstage(backstage::Page::Info));
            }
            // Quick access, as Outlook's title bar: Send / Receive all folders, and Undo the
            // last move, delete, flag or category (Ctrl+Z).
            if can_sync && widgets::bar_icon_button(ui, Icon::Sync, "Send / Receive all folders (F9)", &t).clicked() {
                acts.push(Action::Run("mail.sync", json!({})));
            }
            // What's going on in the background, and what needs you (`activity.rs`).
            if self.activity_shown_now(ui) {
                let a = self.ui.activity.current.clone();
                super::activity::button(ui, &t, &a, &mut self.ui.activity);
            } else {
                self.ui.activity.button = None;
                self.ui.activity.appeared = None;
            }
            if can_undo && widgets::bar_icon_button(ui, Icon::Back, "Undo (Ctrl+Z)", &t).clicked() {
                acts.push(Action::Run("edit.undo", json!({})));
            }
        })
        .response
        .rect
    }

    /// While a message sent waits in the Outbox to be undone (issue #135): how long it has, and
    /// Undo, at the bottom of the window; several at once, how many, the newest undone.
    pub(crate) fn undo_bar(&mut self, ctx: &egui::Context) {
        let now = self.session.clock.now;
        let holds: Vec<(u64, i64)> = self.session.undo_holds().map(|h| (h.id, h.until)).collect();
        let Some(&(newest, until)) = holds.last() else { return };
        // Each second, as it counts down, and as it goes.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        let t = self.ui.tokens;
        let left = until.saturating_sub(now).max(1);
        let text =
            if holds.len() > 1 { tf!("Sending {count} messages · {n} s", count = holds.len(), n = left) } else { tf!("Sending · {n} s", n = left) };
        let mut undo = false;
        egui::Area::new(egui::Id::unique("undo-send")).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -40.0)).order(egui::Order::Foreground).show(
            ctx,
            |ui| {
                egui::Frame::new()
                    .fill(t.surface)
                    .stroke(egui::Stroke::new(1.0, t.border))
                    .corner_radius(8)
                    .shadow(ui.visuals().popup_shadow)
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(text).color(t.text));
                            ui.add_space(12.0);
                            undo = ui.button(RichText::new(tl!("Undo")).strong().color(t.accent)).clicked();
                        });
                    });
            },
        );
        if undo {
            self.act(Action::Run("outbox.undoSend", json!({"id": newest})));
        }
    }

    pub(crate) fn toasts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.session.notices.len() < self.ui.seen_notices {
            self.ui.seen_notices = 0;
        }
        if let Some(n) = self.session.notices.get(self.ui.seen_notices..).and_then(|v| v.last()) {
            self.ui.toast = Some((n.text.clone(), n.error, now));
            // Its Undo, if it's the notice given with one.
            let count = self.session.notices.len();
            self.ui.toast_undo = self.ui.toast_undo.filter(|(n, ..)| *n == count);
            // Said to a screen reader too, as it comes (issue #175): the notice is in a corner
            // the keyboard never goes to. A failure is said at once.
            crate::a11y::announce(ctx, i18n::notice(&n.text), n.error);
        }
        self.ui.seen_notices = self.session.notices.len();
        let Some((text, error, at)) = self.ui.toast.clone() else { return };
        // Undo while what it undoes is still the last change.
        let s = &self.session;
        let undo = self.ui.toast_undo.is_some_and(|(_, given, depth)| given == s.undo_given && depth == s.undo.len());
        let ttl = if error || undo { 7.0 } else { 3.5 };
        if now - at > ttl {
            self.ui.toast = None;
            self.ui.toast_undo = None;
            return;
        }
        let mut undone = false;
        let t = self.ui.tokens;
        // In: up from a little below, settling; out: fading over its last moments.
        let since = (now - at) as f32;
        let (enter, leave) = if motion::enabled(ctx) {
            (
                motion::decelerate((since / motion::GENTLE).clamp(0.0, 1.0)),
                1.0 - motion::accelerate(((since - (ttl as f32 - motion::GENTLE)) / motion::GENTLE).clamp(0.0, 1.0)),
            )
        } else {
            (1.0, 1.0)
        };
        if enter < 1.0 || leave < 1.0 {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((at + ttl - motion::GENTLE as f64 - now).max(0.0)));
        }
        // In the corner where the window's lines end: the left one in a right-to-left interface.
        let (corner, aside) = if i18n::current().rtl() { (egui::Align2::LEFT_BOTTOM, 16.0) } else { (egui::Align2::RIGHT_BOTTOM, -16.0) };
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(corner, vec2(aside, -36.0 + 14.0 * (1.0 - enter)))
            .order(egui::Order::Foreground)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(enter.min(leave));
                egui::Frame::new()
                    .fill(if error { t.danger } else { t.text })
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(14, 10))
                    .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(60) })
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        let ink = if error { Color32::WHITE } else { t.surface };
                        ui.horizontal(|ui| {
                            // Notices from the engine are English; the ones with a fixed text have a translation.
                            ui.label(RichText::new(i18n::notice(&text)).color(ink));
                            if undo {
                                ui.add_space(8.0);
                                // A button, told from the words beside it: Semibold, in the accent
                                // that reads on the notice's dark (light, in dark mode) ground.
                                let word = RichText::new(tl!("Undo")).font(crate::theme::semibold(crate::sizes::BODY)).color(t.accent_soft);
                                let b = egui::Button::new(word).frame(false);
                                undone = ui.add(b).clicked();
                            }
                        });
                    });
            });
        if undone {
            self.ui.toast = None;
            self.ui.toast_undo = None;
            self.act(Action::Run("edit.undo", json!({})));
        }
    }
}
