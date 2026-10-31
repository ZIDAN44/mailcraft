//! Dialogs: the category list (Outlook's Color Categories: each category's colour, name and
//! the keyword it is kept as on the server; new, edit, delete; those on mail that aren't in the
//! list yet), and a category made or changed.

use mailcraft_engine::model::organize::Category;
use mailcraft_engine::model::organize::keywords::{self, check_keyword};

use super::*;

impl Dialog {
    /// Make a category: its keyword left to be made from the name.
    pub fn new_category(s: &Session) -> Dialog {
        Dialog::NewCategory {
            name: String::new(),
            color: u8::try_from(s.prefs.categories.len() % 8).unwrap_or(0),
            keyword: String::new(),
            editing: None,
            error: None,
        }
    }

    /// Change a category in the list.
    pub fn edit_category(c: &Category) -> Dialog {
        Dialog::NewCategory { name: c.name.clone(), color: c.color, keyword: c.keyword.clone(), editing: Some(c.name.clone()), error: None }
    }

    /// Put a keyword that isn't in the list into it (Rename / choose colour): its keyword
    /// kept, so the mail that has it shows it, and other programs still see the same one.
    pub fn name_keyword(keyword: &str) -> Dialog {
        let c = Session::unlisted_category(keyword);
        Dialog::NewCategory { name: c.name, color: c.color, keyword: c.keyword, editing: None, error: None }
    }
}

/// A colour dot, as a button when `pick` (the one chosen ringed).
fn swatch(ui: &mut Ui, t: &Tokens, c: u8, chosen: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().circle_filled(rect.center(), 11.0, t.hover);
    }
    ui.painter().circle_filled(rect.center(), 8.0, theme::category_color(c, t.dark));
    if chosen {
        ui.painter().circle_stroke(rect.center(), 10.0, egui::Stroke::new(2.0, t.text));
    }
    resp.clicked()
}

/// One line of the list: a dot, the name, the keyword dim beside it; the buttons the caller
/// puts on the right.
fn row(ui: &mut Ui, t: &Tokens, c: &Category, odd: bool, extra: &str, buttons: impl FnOnce(&mut Ui)) {
    let (rect, hover) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    if hover.hovered() {
        ui.painter().rect_filled(rect, 0.0, t.hover_row);
    } else if odd {
        ui.painter().rect_filled(rect, 0.0, t.chrome);
    }
    let mut inner =
        ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut inner;
    crate::panes::organize::dot(ui, c.color, t.dark);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        buttons(ui);
        ui.add_space(6.0);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let mut job = egui::text::LayoutJob::default();
            job.append(&c.name, 0.0, egui::TextFormat { font_id: theme::bold(13.0), color: t.text, ..Default::default() });
            job.append(&c.keyword, 10.0, egui::TextFormat { font_id: egui::FontId::monospace(12.0), color: t.dim, ..Default::default() });
            if !extra.is_empty() {
                job.append(extra, 10.0, egui::TextFormat { font_id: theme::regular(12.0), color: t.faint, ..Default::default() });
            }
            job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
            ui.add(egui::Label::new(job).selectable(false)).on_hover_text(format!("{}\n{}", c.name, c.keyword));
        });
    });
}

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::NewCategory { name, color, keyword, editing, error } => {
            ui.set_width(380.0);
            title(ui, t, if editing.is_some() { "Edit category" } else { "New category" });
            let r = ui.add(egui::TextEdit::singleline(name).hint_text(tl!("Name")).desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.horizontal(|ui| {
                for c in 0..8u8 {
                    if swatch(ui, t, c, *color == c) {
                        *color = c;
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new(tl!("Keyword on the server")).small().color(t.dim));
            // Left empty, one made from the name as Thunderbird makes a tag's (a category being
            // changed keeps its own): shown as the hint.
            let made = match editing.as_ref().and_then(|e| s.category(e)) {
                Some(c) => c.keyword.clone(),
                None => keywords::new_keyword(name, &s.prefs.categories),
            };
            ui.add(egui::TextEdit::singleline(keyword).hint_text(made).font(egui::FontId::monospace(13.0)).desired_width(f32::INFINITY));
            let wanted = keyword.trim();
            let bad = !wanted.is_empty() && check_keyword(wanted).is_err();
            let taken = s.category_with_keyword(wanted).filter(|c| editing.as_ref().is_none_or(|e| !c.name.eq_ignore_ascii_case(e)));
            if bad {
                let why = if wanted.len() <= keywords::MAX_KEYWORD && mailcraft_engine::model::is_keyword(wanted) {
                    tl!("That keyword has another use.")
                } else {
                    tl!("Up to 50 letters, digits and symbols, without spaces, quotes, backslashes or ( ) { } ] % *")
                };
                ui.label(RichText::new(why).color(t.danger).small());
            } else if let Some(c) = taken {
                ui.label(RichText::new(tf!("{name} has that keyword already.", name = c.name)).color(t.danger).small());
            } else {
                let note = match editing.as_ref().and_then(|e| s.category(e)) {
                    Some(c) if !wanted.is_empty() && !wanted.eq_ignore_ascii_case(&c.keyword) => {
                        tl!("Mail that has the old keyword keeps it, shown as a category not in the list.")
                    }
                    _ => tl!("Other mail programs that set the same keyword show the same category."),
                };
                ui.label(RichText::new(note).color(t.faint).small());
            }
            if let Some(e) = error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ready = !name.trim().is_empty() && !bad && taken.is_none();
            let (ok, cancel) = buttons(ui, t, if editing.is_some() { "Save" } else { "Create" }, ready);
            if (ok || enter) && ready {
                *error = None;
                let mut p = json!({"color": color});
                if !wanted.is_empty() {
                    p["keyword"] = json!(wanted);
                }
                match editing {
                    Some(old) => {
                        p["name"] = json!(old);
                        p["newName"] = json!(name.trim());
                        acts.push(Action::Submit("category.edit", p));
                    }
                    None => {
                        p["name"] = json!(name.trim());
                        acts.push(Action::Submit("category.create", p));
                    }
                }
            }
            !cancel
        }
        Dialog::Categories => {
            ui.set_width(480.0);
            title(ui, t, "Categories");
            ui.label(
                RichText::new(tl!("Each category is kept with the mail on the server as a keyword, which other mail programs see too."))
                    .color(t.dim)
                    .small(),
            );
            ui.add_space(6.0);
            let unlisted = s.unlisted_keywords();
            egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if s.prefs.categories.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new(tl!("No categories yet.")).color(t.faint));
                        ui.add_space(8.0);
                    }
                    for (i, c) in s.prefs.categories.iter().enumerate() {
                        row(ui, t, c, i % 2 == 1, "", |ui| {
                            if widgets::tiny_icon_button(ui, Icon::Delete, "Delete", t).clicked() {
                                acts.push(Action::Run("category.delete", json!({"name": c.name})));
                            }
                            if widgets::tiny_icon_button(ui, Icon::Drafts, "Edit", t).clicked() {
                                acts.push(Action::Dialog(Dialog::edit_category(c)));
                            }
                        });
                    }
                    // Outlook's "not in the Master Category List": on mail here, from another
                    // program or another computer, until they're added.
                    if !unlisted.is_empty() {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.label(RichText::new(tl!("On mail here, not in the list")).color(t.dim).small().strong());
                        });
                        ui.add_space(2.0);
                        for (i, (k, n)) in unlisted.iter().enumerate() {
                            let c = Session::unlisted_category(k);
                            let count = tf!("Messages: {n}", n = n);
                            row(ui, t, &c, i % 2 == 1, &count, |ui| {
                                if widgets::tiny_icon_button(ui, Icon::Plus, "Add to the list", t).clicked() {
                                    acts.push(Action::Dialog(Dialog::name_keyword(k)));
                                }
                            });
                        }
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("New category…")).clicked() {
                    acts.push(Action::Dialog(Dialog::new_category(s)));
                }
                // The list to a file and back, for a team to use the same keywords.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!s.prefs.categories.is_empty(), egui::Button::new(tl!("Export…"))).clicked() {
                        acts.push(Action::ExportCategories);
                    }
                    if ui.button(tl!("Import…")).clicked() {
                        acts.push(Action::ImportCategories);
                    }
                });
            });
            ui.add_space(6.0);
            close_row(ui)
        }
        _ => true,
    }
}
