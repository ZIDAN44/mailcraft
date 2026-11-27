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

/// The category colours' names, in their order: said by a screen reader and on hover (the dots
/// had no name).
const COLOUR_NAMES: [&str; 8] = ["Red", "Orange", "Green", "Blue", "Purple", "Yellow", "Teal", "Pink"];

/// A colour dot to pick, the one chosen ringed: whether it was clicked.
fn swatch(ui: &mut Ui, t: &Tokens, c: u8, chosen: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    let name = tl!(COLOUR_NAMES.get(usize::from(c)).copied().unwrap_or("Red"));
    widgets::named(ui, &resp, name, true, Some(chosen));
    let resp = resp.on_hover_text(name);
    if resp.hovered() {
        ui.painter().circle_filled(rect.center(), 11.0, t.hover);
    }
    ui.painter().circle_filled(rect.center(), 8.0, theme::category_color(c, t.dark));
    if chosen {
        ui.painter().circle_stroke(rect.center(), 10.0, egui::Stroke::new(2.0, t.text));
    }
    resp.clicked()
}

/// The keywords on the mail listed that aren't categories in the list, with how many messages
/// have each ([`Session::unlisted_keywords`]), for the dialog: kept between frames, and counted
/// again only when the list (its revision) or the categories' keywords change.
struct Unlisted {
    revision: u64,
    keywords: Vec<String>,
    found: Vec<(String, usize)>,
}

/// The dialog's [`Unlisted`] for the list and categories now.
fn unlisted(ctx: &egui::Context, s: &Session) -> std::sync::Arc<Unlisted> {
    let id = egui::Id::unique("categories-unlisted");
    if let Some(kept) = ctx.data(|d| d.get_temp::<std::sync::Arc<Unlisted>>(id))
        && kept.revision == s.revision
        && kept.keywords.iter().eq(s.prefs.categories.iter().map(|c| &c.keyword))
    {
        return kept;
    }
    let keywords = s.prefs.categories.iter().map(|c| c.keyword.clone()).collect();
    let made = std::sync::Arc::new(Unlisted { revision: s.revision, keywords, found: s.unlisted_keywords() });
    ctx.data_mut(|d| d.insert_temp(id, made.clone()));
    made
}

/// One line of the list: a dot and the name (its keyword on hover: a column of "$label1"s read as
/// code); the buttons the caller puts on the right, seen while the row is pointed at.
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
        // Apart enough that Delete isn't hit for Edit beside it.
        ui.spacing_mut().item_spacing.x = 6.0;
        widgets::row_actions(ui, rect, buttons);
        ui.add_space(6.0);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                crate::i18n::category(&c.name),
                0.0,
                egui::TextFormat { font_id: theme::semibold(crate::sizes::BODY), color: t.text, ..Default::default() },
            );
            if !extra.is_empty() {
                job.append(extra, 10.0, egui::TextFormat { font_id: theme::regular(crate::sizes::CAPTION), color: t.faint, ..Default::default() });
            }
            let keyword = tf!("Keyword on the server: {keyword}", keyword = c.keyword);
            ui.add(egui::Label::new(job).selectable(false).truncate()).on_hover_text(format!("{}\n{keyword}", c.name));
        });
    });
}

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::NewCategory { name, color, keyword, editing, error } => {
            ui.set_width(crate::sizes::DIALOG_S);
            title(ui, t, if editing.is_some() { "Edit category" } else { "New category" });
            label_over(ui, t, "Name");
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
            // As it opens: editing, the name it has selected, so what's typed replaces it.
            let mut opening = ui.memory(|m| m.focused().is_none());
            crate::look::fields::focus_selecting_all(ui, &r, name, &mut opening);
            ui.horizontal(|ui| {
                for c in 0..8u8 {
                    if swatch(ui, t, c, *color == c) {
                        *color = c;
                    }
                }
            });
            ui.add_space(6.0);
            label_over(ui, t, "Keyword on the server");
            // Left empty, one made from the name as Thunderbird makes a tag's (a category being
            // changed keeps its own): shown as the hint.
            let current = editing.as_ref().and_then(|e| s.category(e));
            let made = match current {
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
                let note = match current {
                    Some(c) if !wanted.is_empty() && !wanted.eq_ignore_ascii_case(&c.keyword) => {
                        tl!("Mail that has the old keyword keeps it, shown as a category not in the list.")
                    }
                    _ => tl!("Other mail programs that set the same keyword show the same category."),
                };
                ui.label(RichText::new(note).color(t.faint).small());
            }
            if let Some(e) = error {
                field_error(ui, t, e);
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
            ui.set_width(crate::sizes::DIALOG_M);
            title(ui, t, "Categories");
            ui.label(
                RichText::new(tl!("Each category is kept with the mail on the server as a keyword, which other mail programs see too."))
                    .color(t.dim)
                    .small(),
            );
            ui.add_space(6.0);
            // What the list is worked with, over it; Close alone under it (Close was on a row of
            // its own under New, Import and Export).
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
            ui.add_space(4.0);
            let unlisted = unlisted(ui.ctx(), s);
            // The category just deleted, with Undo (deleting one asks nothing first), put back
            // where it was in the list.
            if let Some((c, at)) = deleted::last::<(Category, usize)>(ui, "categories")
                && deleted::undo_line(ui, t, &c.name)
            {
                deleted::forget::<(Category, usize)>(ui, "categories");
                acts.push(Action::Run("category.create", json!({"name": c.name, "color": c.color, "keyword": c.keyword, "at": at})));
            }
            egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(crate::sizes::SURFACE_RADIUS).show(ui, |ui| {
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
                                deleted::remember(ui, "categories", (c.clone(), i));
                                acts.push(Action::Run("category.delete", json!({"name": c.name})));
                            }
                            if widgets::tiny_icon_button(ui, Icon::Drafts, "Edit", t).clicked() {
                                acts.push(Action::Dialog(Dialog::edit_category(c)));
                            }
                        });
                    }
                    // Outlook's "not in the Master Category List": on mail here, from another
                    // program or another computer, until they're added.
                    if !unlisted.found.is_empty() {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.label(RichText::new(tl!("On mail here, not in the list")).color(t.dim).small().strong());
                        });
                        ui.add_space(2.0);
                        for (i, (k, n)) in unlisted.found.iter().enumerate() {
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
            close_row(ui)
        }
        _ => true,
    }
}
