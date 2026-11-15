//! The ribbon: its tabs and their buttons.

use crate::*;

mod mail_tabs;
mod modules;

impl MailApp {
    /// The ribbon's tabs: File, Home, Send / Receive, View, and Search while searching. They sit
    /// on the top bar beside the search box (issue #22), one row where there were two, the
    /// bar's and the tabs'.
    pub(crate) fn ribbon_tabs(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
        // The Search tab goes when the search does (and the box isn't being typed in, nor the
        // pointer on its way from the box to the tab's buttons).
        let pointer = ui.ctx().pointer_latest_pos();
        let under = |r: Option<egui::Rect>| r.zip(pointer).is_some_and(|(r, p)| r.contains(p));
        let near = under(self.ui.ribbon_rect) || under(self.ui.top_bar_rect);
        if self.ui.tab == RibbonTab::Search && s.search.is_none() && !self.ui.search_focused && !near {
            self.ui.tab = RibbonTab::Home;
        }
        let searching = self.ui.tab == RibbonTab::Search || s.search.is_some();
        ui.spacing_mut().item_spacing.x = 0.0;
        // Alt pressed: each tab's key tip under it (`keytips`).
        let tips = self.ui.keytips.prefix() == Some("");
        let file = tab_button(ui, &t, tl!("File"), None);
        if let Some(letters) = crate::keytips::table::letters(crate::keytips::table::Top::File).filter(|_| tips) {
            crate::keytips::badge(ui.ctx(), file.rect, letters, true, &t);
        }
        if file.clicked() {
            acts.push(Action::Backstage(backstage::Page::Info));
        }
        let tabs = [(RibbonTab::Home, "Home"), (RibbonTab::SendReceive, "Send / Receive"), (RibbonTab::View, "View"), (RibbonTab::Search, "Search")];
        for (tab, label) in tabs.into_iter().filter(|(tab, _)| *tab != RibbonTab::Search || searching) {
            let on = self.ui.tab == tab;
            let r = tab_button(ui, &t, tl!(label), Some(on));
            if let Some(letters) = crate::keytips::table::tab_letters(tab).filter(|_| tips) {
                crate::keytips::badge(ui.ctx(), r.rect, letters, true, &t);
            }
            if on {
                // The underline glides from the tab left to the one chosen.
                let rect = r.rect;
                let ctx = ui.ctx().clone();
                let left = motion::glide(&ctx, egui::Id::unique("ribbon-tab-left"), rect.left() + 8.0, motion::NORMAL);
                let right = motion::glide(&ctx, egui::Id::unique("ribbon-tab-right"), rect.right() - 8.0, motion::NORMAL);
                let y = rect.bottom() - 2.0;
                ui.painter().line_segment([egui::pos2(left, y), egui::pos2(right, y)], egui::Stroke::new(2.0, tab_colors(&t).0));
            }
            if r.clicked() {
                self.ui.tab = tab;
            }
        }
    }

    /// The ribbon's buttons: those of the module shown (`ribbon/modules.rs`), or in Mail the tab
    /// chosen's (`ribbon/mail_tabs.rs`).
    pub(crate) fn ribbon(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        // The buttons sit on a card with an edge, as Outlook's (issue #8): its place is kept here
        // and drawn under them once they're laid out (`card_under_ribbon`).
        let slot = ui.painter().add(egui::Shape::Noop);
        let top = ui.cursor().top();
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(RIBBON_CARD), Some((slot, top))));
        ui.add_space(3.0);
        if self.session.module != Module::Mail {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                match self.session.module {
                    Module::Calendar => self.ribbon_calendar(ui, acts),
                    Module::Tasks => self.ribbon_tasks(ui, acts),
                    Module::Notes => self.ribbon_notes(ui, acts),
                    _ => self.ribbon_people(ui, acts),
                }
            });
            return;
        }
        let s = &self.session;
        let ust = &mut self.ui;
        // A narrow window scrolls the ribbon sideways rather than cutting buttons off; each tab
        // keeps its own place (one shared place opened View scrolled as far as Home was, which
        // gridcraft's ribbon found).
        // Right to left it starts scrolled to the right, where the row starts.
        let rtl = i18n::current().rtl();
        egui::ScrollArea::horizontal().id_salt(("ribbon", ust.tab)).stick_to_right(rtl).show(ui, |ui| {
            widgets::scrolled_row(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                widgets::start_group(ui);
                match ust.tab {
                    RibbonTab::Home => mail_tabs::ribbon_home(ui, s, t, acts),
                    RibbonTab::SendReceive => mail_tabs::ribbon_send_receive(ui, s, t, ust.has_files, acts),
                    RibbonTab::Search => mail_tabs::ribbon_search(ui, s, ust, t, acts),
                    RibbonTab::View => mail_tabs::ribbon_view(ui, s, t, ust.dark, acts),
                }
            });
        });
    }
}

/// A tab's colours on the top bar, the one shown and the others: white on the coloured bar
/// (the one shown told by its weight and underline), the accent and grey on a plain one.
pub(crate) fn tab_colors(t: &Tokens) -> (egui::Color32, egui::Color32) {
    if t.bar == t.chrome { (t.accent, t.dim) } else { (t.on_bar, t.on_bar) }
}

/// A ribbon tab on the top bar (`on`: whether it's the one shown; `None` for File, which opens
/// a page instead): its name, bold when shown, on a wash under the pointer that reads on the
/// coloured bar as on a plain one.
fn tab_button(ui: &mut Ui, t: &Tokens, label: &str, on: Option<bool>) -> egui::Response {
    let shown = on == Some(true);
    let (on_color, off_color) = tab_colors(t);
    let color = if shown { on_color } else { off_color };
    let font = if shown { theme::bold(13.5) } else { theme::regular(13.5) };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
    let height = ui.available_height().clamp(22.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, height), Sense::click());
    widgets::named(ui, &resp, label, true, on);
    if resp.hovered() {
        ui.painter().rect_filled(rect.shrink2(vec2(1.0, 1.0)), 5.0, widgets::bar_wash(t));
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.shrink(1.0), 5.0, egui::Stroke::new(1.5, on_color), egui::StrokeKind::Inside);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

/// Where the ribbon's card is drawn: its shape's place and its top (see `card_under_ribbon`).
const RIBBON_CARD: &str = "ribbon-card";

/// Draw the card under the ribbon's buttons, from where the tabs end to the bottom of what was
/// laid out, across the ribbon.
pub(crate) fn card_under_ribbon(ui: &Ui, t: &Tokens, room: egui::Rect) {
    let kept = ui.ctx().data_mut(|d| d.remove_temp::<Option<(egui::layers::ShapeIdx, f32)>>(egui::Id::unique(RIBBON_CARD)));
    let Some((slot, top)) = kept.flatten() else { return };
    let bottom = ui.min_rect().bottom() + 3.0;
    if bottom <= top {
        return;
    }
    let rect = egui::Rect::from_min_max(egui::pos2(room.left() - 5.0, top), egui::pos2(room.right() + 5.0, bottom));
    let fill = if t.dark { t.surface } else { egui::Color32::WHITE };
    ui.painter().set(slot, egui::epaint::RectShape::new(rect, 8.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside));
}

/// The ribbon's height (its row of buttons), the same whichever tab is shown.
pub(crate) const RIBBON_H: f32 = 30.0;
/// The classic ribbon's: its tabs, then big buttons with the groups' names under them.
pub(crate) const RIBBON_H_CLASSIC: f32 = widgets::CLASSIC_BUTTON_H + widgets::CLASSIC_CAPTION_H;

/// The modules (Mail, Calendar, People, Tasks, Notes), as Outlook orders them.
const MODULES: [(Module, Icon, &str, &str); 5] = [
    (Module::Mail, Icon::Unread, "Mail", "mail"),
    (Module::Calendar, Icon::Calendar, "Calendar", "calendar"),
    (Module::People, Icon::People, "People", "people"),
    (Module::Tasks, Icon::Task, "Tasks", "tasks"),
    (Module::Notes, Icon::Note, "Notes", "notes"),
];

/// Mail, Calendar, People, Tasks and Notes, at the bottom of the side pane: narrower buttons
/// when the pane is.
pub(crate) fn module_switch(ui: &mut Ui, t: &Tokens, module: Module, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let n = MODULES.len() as f32;
        let w = ((ui.available_width() - 4.0 * (n - 1.0)) / n).clamp(26.0, 40.0);
        for each in MODULES {
            module_button(ui, t, module, each, w, acts);
        }
    });
}

/// The modules one above another (the minimized folder pane).
pub(crate) fn module_switch_vertical(ui: &mut Ui, t: &Tokens, module: Module, acts: &mut Vec<Action>) {
    for each in MODULES {
        module_button(ui, t, module, each, 40.0, acts);
    }
}

/// One module's button, `width` wide (of [`MODULES`]; `module`: the one shown): its icon, named
/// and on hover; a click shows the module.
fn module_button(ui: &mut Ui, t: &Tokens, module: Module, (m, icon, label, code): (Module, Icon, &str, &str), width: f32, acts: &mut Vec<Action>) {
    let (r, resp) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
    let on = m == module;
    widgets::named(ui, &resp, tl!(label), true, Some(on));
    if on || resp.hovered() {
        ui.painter().rect_filled(r, 6.0, if on { t.accent_soft } else { t.hover });
    }
    icons::paint(ui.painter(), icons::centered(r, 19.0), icon, if on { t.accent } else { t.dim });
    if resp.on_hover_text(tl!(label)).clicked() {
        acts.push(Action::Run("view.module", json!({"name": code})));
    }
}

/// The Quick Steps menu: each step (done to the selection), then making and changing them.
pub(crate) fn quick_steps_menu(ui: &mut egui::Ui, s: &Session, acts: &mut Vec<Action>) {
    let has = !s.selection.is_empty();
    if s.prefs.quick_steps.is_empty() {
        ui.label(RichText::new(tl!("No Quick Steps yet.")).weak());
    }
    for (i, q) in s.prefs.quick_steps.iter().enumerate() {
        let mut b = egui::Button::new(q.name.as_str());
        if i < 9 {
            b = b.shortcut_text(format!("Ctrl+Shift+{}", i + 1));
        }
        if ui.add_enabled(has, b).clicked() {
            acts.push(Action::Run("quickStep.run", json!({"index": i})));
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tl!("New Quick Step…")).clicked() {
        acts.push(Action::Dialog(Dialog::QuickStep(Box::new(crate::dialogs::QuickStepForm::new(s)))));
        ui.close();
    }
    if !s.prefs.quick_steps.is_empty() {
        ui.menu_button(tl!("Edit a Quick Step"), |ui| {
            for q in &s.prefs.quick_steps {
                if ui.button(q.name.as_str()).clicked() {
                    acts.push(Action::Dialog(Dialog::QuickStep(Box::new(crate::dialogs::QuickStepForm::edit(s, q)))));
                    ui.close();
                }
            }
        });
    }
}

/// The Junk menu: block the selection's senders, or never block them (or their domain), and
/// the lists themselves (File › Options).
pub(crate) fn junk_menu(ui: &mut egui::Ui, s: &Session, acts: &mut Vec<Action>) {
    let has = !s.selection.is_empty();
    for (id, label, tip) in [
        ("junk.blockSender", "Block sender", "Their mail goes to Junk Email from now on"),
        ("junk.neverBlockSender", "Never block sender", "Their mail is never treated as junk"),
        ("junk.neverBlockDomain", "Never block sender's domain", "Mail from anyone at their domain is never treated as junk"),
    ] {
        if ui.add_enabled(has, egui::Button::new(tl!(label))).on_hover_text(tl!(tip)).clicked() {
            acts.push(Action::Run(id, json!({})));
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tl!("Junk Email options…")).clicked() {
        acts.push(Action::Backstage(backstage::Page::Options));
        ui.close();
    }
}
