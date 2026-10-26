//! The mail search box (issue #4, as Outlook's): where to look on its left, the recent searches
//! dropping down from it, and advanced search under the chevron on its right.

use mailcraft_engine::model::search::Advanced;

use crate::*;

/// How long the search box waits after the last key before searching, in seconds.
const SEARCH_PAUSE: f64 = 0.3;

/// The search box's menu's name for where it looks.
pub(crate) fn scope_label(scope: SearchScope) -> &'static str {
    match scope {
        SearchScope::Current => tl!("Current Folder"),
        SearchScope::Subfolders => tl!("Subfolders"),
        SearchScope::Mailbox => tl!("Current Mailbox"),
        SearchScope::All => tl!("All Mailboxes"),
    }
}

/// The search box's dropdowns: the recent searches, or advanced search.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum SearchDrop {
    #[default]
    None,
    Recent,
    Advanced(Advanced),
}

impl MailApp {
    /// The mail search box itself, `width` wide.
    pub(crate) fn search_box(&mut self, ui: &mut Ui, width: f32, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let scope = self.session.search.as_ref().map_or(self.session.prefs.search_scope, |(_, s)| *s);
        // Where to look, as Outlook's menu at the box's left.
        let menu = ui.menu_button(RichText::new(format!("{} ▾", scope_label(scope))).small().color(t.dim), |ui| {
            for s in SearchScope::ALL {
                if ui.selectable_label(s == scope, scope_label(s)).clicked() {
                    acts.push(Action::Run("search.run", json!({"query": self.ui.search_text, "scope": s.code()})));
                    ui.close();
                }
            }
        });
        menu.response.on_hover_text(tl!("Look in"));
        // The words get what's left: the box keeps one width whatever the menu says, and room for
        // its two buttons (Clear's kept when it isn't shown).
        let button = 22.0 + ui.spacing().item_spacing.x;
        let id = ui.make_persistent_id("search-box");
        widgets::cursor_within(ui.ctx(), id, &self.ui.search_text);
        let te = egui::TextEdit::singleline(&mut self.ui.search_text)
            .id(id)
            .hint_text(tl!("Search mail (from:, subject:, has:attachment…)"))
            .frame(egui::Frame::NONE)
            .desired_width((width - ui.min_rect().width() - 2.0 * button - ui.spacing().item_spacing.x).max(80.0))
            .text_color(t.text);
        let resp = ui.add(te);
        // Into the search box: the Search tab, as Outlook switches to it, and the recent searches.
        if resp.gained_focus() {
            self.ui.tab = RibbonTab::Search;
        }
        if (resp.gained_focus() || resp.clicked()) && !self.session.recent_searches().is_empty() && self.ui.search_drop == SearchDrop::None {
            self.ui.search_drop = SearchDrop::Recent;
        }
        self.ui.search_focused = resp.has_focus();
        if self.ui.search_cursor_end {
            self.ui.search_cursor_end = false;
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                let end = egui::text::CCursor::new(self.ui.search_text.chars().count());
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), resp.id);
            }
        }
        if self.ui.focus_search {
            resp.request_focus();
            self.ui.focus_search = false;
        }
        // Searched a moment after the last key (Enter: at once), not on every key.
        let now = ui.input(|i| i.time);
        if resp.changed() {
            self.ui.search_due = Some(now + SEARCH_PAUSE);
        }
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        match self.ui.search_due {
            Some(due) if enter || now >= due || self.ui.search_text.is_empty() => {
                self.ui.search_due = None;
                let q = self.ui.search_text.clone();
                acts.push(Action::Run("search.run", json!({"query": q, "remember": enter})));
            }
            Some(due) => ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((due - now).max(0.01))),
            None if enter => acts.push(Action::Run("search.remember", json!({"query": self.ui.search_text}))),
            None => {}
        }
        // Leaving the box with a search in it (to a result, say): one of the recent searches.
        if resp.lost_focus() && !enter && !self.ui.search_text.trim().is_empty() {
            acts.push(Action::Run("search.remember", json!({"query": self.ui.search_text})));
        }
        if self.ui.search_text.is_empty() {
            ui.add_space(button);
        } else if widgets::tiny_icon_button(ui, Icon::Close, "Clear search", &t).clicked() {
            acts.push(Action::Run("search.clear", json!({})));
            self.ui.search_drop = SearchDrop::None;
        }
        let open = matches!(self.ui.search_drop, SearchDrop::Advanced(_));
        if widgets::tiny_icon_button(ui, Icon::ChevronDown, "Advanced search", &t).clicked() {
            self.ui.search_drop = if open { SearchDrop::None } else { SearchDrop::Advanced(Advanced::from_query(&self.ui.search_text)) };
        }
        // Under the box (its menu to its chevron, and the frame round them), as wide as it.
        let row = ui.min_rect();
        let below = egui::Rect::from_min_size(row.left_bottom() + vec2(-8.0, 6.0), vec2(row.width() + 16.0, 0.0));
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.ui.search_drop = SearchDrop::None;
        }
        let area = match std::mem::take(&mut self.ui.search_drop) {
            SearchDrop::None => None,
            SearchDrop::Recent => self.recent_searches(ui.ctx(), below, acts),
            SearchDrop::Advanced(mut form) => self.advanced_search(ui.ctx(), below, &mut form, acts),
        };
        // A click anywhere else (not the box or its buttons) closes it.
        if let Some(rect) = area {
            let pressed_outside =
                ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !row.contains(p)));
            if pressed_outside {
                self.ui.search_drop = SearchDrop::None;
            }
        }
    }

    /// The recent searches, the ones with what's typed so far; picking one searches it. Where it
    /// was drawn (`None`: nothing to show, closed).
    fn recent_searches(&mut self, ctx: &egui::Context, at: egui::Rect, acts: &mut Vec<Action>) -> Option<egui::Rect> {
        let t = self.ui.tokens;
        let typed = self.ui.search_text.trim().to_lowercase();
        let shown: Vec<String> = self
            .session
            .recent_searches()
            .iter()
            .filter(|r| r.to_lowercase().contains(&typed) && r.trim() != self.ui.search_text.trim())
            .cloned()
            .collect();
        if shown.is_empty() {
            return None;
        }
        self.ui.search_drop = SearchDrop::Recent;
        let area = egui::Area::new(egui::Id::unique("search-recent")).order(egui::Order::Foreground).fixed_pos(at.min).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).inner_margin(Margin::same(6)).show(ui, |ui| {
                ui.set_width(at.width() - 12.0);
                ui.label(RichText::new(tl!("Recent searches")).small().color(t.dim));
                for r in shown {
                    let item = ui.add(egui::Button::selectable(false, RichText::new(&r).color(t.text)).min_size(vec2(ui.available_width(), 0.0)));
                    if item.clicked() {
                        self.ui.search_text = r.clone();
                        self.ui.search_due = None;
                        self.ui.search_drop = SearchDrop::None;
                        acts.push(Action::Run("search.run", json!({"query": r, "remember": true})));
                    }
                }
                ui.separator();
                if ui.link(RichText::new(tl!("Clear recent searches")).small().color(t.accent)).clicked() {
                    acts.push(Action::Run("search.forgetRecent", json!({})));
                    self.ui.search_drop = SearchDrop::None;
                }
            });
        });
        Some(area.response.rect)
    }

    /// Advanced search: a field for each part of the query, filled from what's in the box.
    fn advanced_search(&mut self, ctx: &egui::Context, at: egui::Rect, form: &mut Advanced, acts: &mut Vec<Action>) -> Option<egui::Rect> {
        let t = self.ui.tokens;
        let mut close = false;
        let area = egui::Area::new(egui::Id::unique("search-advanced")).order(egui::Order::Foreground).fixed_pos(at.min).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width((at.width() - 24.0).max(320.0));
                ui.label(RichText::new(tl!("Advanced search")).strong().color(t.text));
                ui.add_space(4.0);
                let mut enter = false;
                egui::Grid::new("search-advanced-fields").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                    for (label, value) in [
                        (tl!("Words"), &mut form.words),
                        (tl!("From"), &mut form.from),
                        (tl!("To"), &mut form.to),
                        (tl!("Subject"), &mut form.subject),
                        (tl!("Message text"), &mut form.body),
                        (tl!("Other terms"), &mut form.rest),
                    ] {
                        ui.label(RichText::new(label).color(t.dim));
                        let r = ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
                        enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut form.attachments, tl!("Has attachments"));
                    ui.checkbox(&mut form.unread, tl!("Unread"));
                    ui.checkbox(&mut form.flagged, tl!("Flagged"));
                    ui.checkbox(&mut form.important, tl!("High importance"));
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(tl!("Search")).clicked() || enter {
                        let q = form.to_query();
                        self.ui.search_text = q.clone();
                        self.ui.search_due = None;
                        acts.push(Action::Run("search.run", json!({"query": q, "remember": true})));
                        close = true;
                    }
                    if ui.button(tl!("Clear")).clicked() {
                        *form = Advanced::default();
                    }
                });
            });
        });
        if close {
            return None;
        }
        self.ui.search_drop = SearchDrop::Advanced(form.clone());
        Some(area.response.rect)
    }
}
