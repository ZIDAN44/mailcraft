//! People: the address books, searchable, with a card for the person picked.

use egui::{Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::Session;
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState, widgets};

/// How the contacts are laid out, as Outlook's Change View offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PeopleView {
    /// Everyone in the side pane, the one picked as a card.
    #[default]
    People,
    /// Business cards, side by side.
    Cards,
    /// A table: name, company, job title, email, phone.
    Phone,
}

impl PeopleView {
    pub const ALL: [PeopleView; 3] = [PeopleView::People, PeopleView::Cards, PeopleView::Phone];

    pub fn label(self) -> &'static str {
        match self {
            PeopleView::People => "People",
            PeopleView::Cards => "Cards",
            PeopleView::Phone => "Phone list",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            PeopleView::People => Icon::Person,
            PeopleView::Cards => Icon::Layout,
            PeopleView::Phone => Icon::Phone,
        }
    }
}

fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
}

/// The A–Z index's width.
const INDEX_W: f32 = 18.0;

/// A name's index letter: its first letter, or `#` for one that isn't a letter.
fn index_letter(name: &str) -> char {
    match name.chars().next() {
        Some(c) if c.is_alphabetic() => c.to_uppercase().next().unwrap_or(c),
        _ => '#',
    }
}

/// The side pane: search, then everyone, A to Z.
pub fn nav(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    if widgets::accent_button(ui, Icon::Plus, "New contact", &t, !s.accounts.is_empty()).clicked() {
        acts.push(Action::Dialog(Dialog::new_contact()));
    }
    ui.add_space(8.0);
    ui.add(egui::TextEdit::singleline(&mut st.people_query).hint_text(tl!("Search people")).desired_width(f32::INFINITY));
    ui.add_space(6.0);
    let people = s.people(&st.people_query);
    if people.is_empty() {
        ui.label(RichText::new(tl!(if st.people_query.trim().is_empty() { "No contacts yet." } else { "No one matches." })).color(t.faint));
    }
    if st.people_view != PeopleView::People {
        ui.label(RichText::new(tf!("{n} people", n = people.len())).color(t.faint).small());
        return;
    }
    // Outlook's A–Z index beside the list: a letter jumps to the first name under it.
    let letters: Vec<char> = {
        let mut v: Vec<char> = people.iter().map(|p| index_letter(&p.card.display())).collect();
        v.dedup();
        v
    };
    let jump_id = egui::Id::new("people-jump");
    let jump: Option<char> = ui.ctx().data_mut(|d| d.remove_temp(jump_id));
    ui.horizontal_top(|ui| {
        // The list leaves room for the index and the gap before it exactly: a few pixels more and
        // the pane (shared by every module) grew a little each frame, up to its widest.
        let list_w = (ui.available_width() - INDEX_W - ui.spacing().item_spacing.x).max(80.0);
        ui.allocate_ui_with_layout(vec2(list_w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).id_salt("people").show(ui, |ui| {
                let mut letter: Option<char> = None;
                for p in &people {
                    let name = p.card.display();
                    let first = Some(index_letter(&name));
                    if first != letter {
                        letter = first;
                        ui.add_space(4.0);
                        let r = ui.label(RichText::new(first.map(String::from).unwrap_or_default()).font(theme::bold(12.0)).color(t.accent));
                        if jump == first {
                            r.scroll_to_me(Some(egui::Align::TOP));
                        }
                    }
                    let selected = st.person.as_deref() == Some(p.card.uid.as_str());
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                    if selected || resp.hovered() {
                        ui.painter().rect_filled(r, 6.0, if selected { t.accent_soft } else { t.hover_row });
                    }
                    let initials: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
                    widgets::avatar(ui, r.left_center() + vec2(20.0, 0.0), 14.0, &initials, theme::avatar_color(&name, t.dark));
                    ui.painter().text(r.left_center() + vec2(42.0, -7.0), egui::Align2::LEFT_CENTER, &name, theme::regular(13.5), t.text);
                    let sub = p.card.emails.first().map(|e| e.value.clone()).unwrap_or_else(|| p.card.org.clone());
                    ui.painter().text(r.left_center() + vec2(42.0, 8.0), egui::Align2::LEFT_CENTER, sub, theme::regular(11.5), t.dim);
                    if resp.clicked() {
                        st.person = Some(p.card.uid.clone());
                    }
                    if resp.double_clicked() {
                        acts.push(Action::Dialog(Dialog::edit_contact(&p.card)));
                    }
                }
            });
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for c in 'A'..='Z' {
                let has = letters.contains(&c);
                let (r, resp) = ui.allocate_exact_size(vec2(INDEX_W, 16.0), if has { Sense::click() } else { Sense::hover() });
                if resp.hovered() && has {
                    ui.painter().rect_filled(r, 3.0, t.hover_row);
                }
                ui.painter().text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    c.to_lowercase().to_string(),
                    theme::regular(11.0),
                    if has { t.accent } else { t.faint },
                );
                if resp.clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(jump_id, c));
                }
            }
        });
    });
}

/// The main pane: the person picked, or everyone as cards or a phone list.
pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    match st.people_view {
        PeopleView::Cards => return cards(s, st, ui, acts),
        PeopleView::Phone => return phone_list(s, st, ui, acts),
        PeopleView::People => {}
    }
    let Some(p) = st.person.as_deref().and_then(|u| s.find_person(u)) else {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.35);
                let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::People, t.border);
                ui.add_space(8.0);
                ui.label(RichText::new(tl!("Pick someone to see their details")).color(t.faint));
            });
        });
        return;
    };
    let c = &p.card;
    let name = c.display();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 24 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                let initials: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
                widgets::avatar(ui, r.center(), 34.0, &initials, theme::avatar_color(&name, t.dark));
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&name).font(theme::bold(22.0)).color(t.text));
                    let role = [c.title.as_str(), c.org.as_str()].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" · ");
                    if !role.is_empty() {
                        ui.label(RichText::new(role).color(t.dim));
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if let Some(e) = c.emails.first()
                            && widgets::pill_button(ui, Icon::NewMail, "Email", &t).clicked()
                        {
                            acts.push(Action::Run("mail.new", json!({})));
                            acts.push(Action::Run("compose.update", json!({"to": format!("{name} <{}>", e.value)})));
                            acts.push(Action::Run("view.module", json!({"name": "mail"})));
                        }
                        if widgets::pill_button(ui, Icon::Drafts, "Edit", &t).clicked() {
                            acts.push(Action::Dialog(Dialog::edit_contact(c)));
                        }
                        if widgets::pill_button(ui, Icon::Delete, "Delete", &t).clicked() {
                            acts.push(Action::Run("contact.delete", json!({"uid": c.uid})));
                        }
                    });
                });
            });
            ui.add_space(18.0);
            let field = |ui: &mut Ui, t: &Tokens, icon: Icon, value: &str, kind: &str| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    icons::paint(ui.painter(), r, icon, t.accent);
                    ui.label(RichText::new(value).color(t.text));
                    if !kind.is_empty() {
                        ui.label(RichText::new(kind).color(t.faint).small());
                    }
                });
            };
            for e in &c.emails {
                field(ui, &t, Icon::Unread, &e.value, &e.kind);
            }
            for ph in &c.phones {
                field(ui, &t, Icon::Phone, &ph.value, &ph.kind);
            }
            if !c.note.is_empty() {
                ui.add_space(10.0);
                ui.label(RichText::new(tl!("Notes")).font(theme::bold(13.0)).color(t.dim));
                ui.label(RichText::new(&c.note).color(t.text));
            }
            ui.add_space(14.0);
            if let Some(a) = s.account(&p.account) {
                ui.label(RichText::new(tf!("In the address book of {account}", account = a.config.email)).color(t.faint).small());
            }
        });
    });
}

/// Everyone as business cards, side by side (Outlook's Business Card view): a click picks one,
/// a double-click edits it.
fn cards(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let people = s.people(&st.people_query);
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("people-cards").show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::same(18)).show(ui, |ui| {
            if people.is_empty() {
                ui.label(RichText::new(tl!("No contacts yet.")).color(t.faint));
                return;
            }
            let card_w = 280.0;
            let card_h = 132.0;
            let gap = 12.0;
            let cols = (((ui.available_width() + gap) / (card_w + gap)).floor() as usize).max(1);
            for row in people.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for p in row {
                        let c = &p.card;
                        let name = c.display();
                        let selected = st.person.as_deref() == Some(c.uid.as_str());
                        let (r, resp) = ui.allocate_exact_size(vec2(card_w, card_h), Sense::click());
                        let fill = if selected {
                            t.accent_soft
                        } else if resp.hovered() {
                            t.hover_row
                        } else {
                            t.surface
                        };
                        ui.painter().rect(r, 8.0, fill, egui::Stroke::new(1.0, if selected { t.accent } else { t.border }), egui::StrokeKind::Inside);
                        // A band in the person's colour down the left, with their initials.
                        let band = egui::Rect::from_min_size(r.min, vec2(74.0, card_h));
                        let color = theme::avatar_color(&name, t.dark);
                        ui.painter().rect_filled(band.shrink(1.0), egui::CornerRadius { nw: 7, sw: 7, ne: 0, se: 0 }, color.gamma_multiply(0.18));
                        widgets::avatar(ui, band.center(), 24.0, &initials(&name), color);
                        let x = band.right() + 12.0;
                        let w = r.right() - x - 10.0;
                        let mut y = r.top() + 12.0;
                        let mut line = |text: &str, font: egui::FontId, color: egui::Color32| {
                            if text.is_empty() {
                                return;
                            }
                            let g = widgets::one_line(ui, text, font, color, w);
                            let h = g.size().y;
                            ui.painter().galley(egui::pos2(x, y), g, color);
                            y += h + 3.0;
                        };
                        line(&name, theme::bold(14.5), t.text);
                        line(&c.title, theme::regular(12.0), t.dim);
                        line(&c.org, theme::regular(12.0), t.dim);
                        for e in c.emails.iter().take(2) {
                            line(&e.value, theme::regular(12.0), t.accent);
                        }
                        for ph in c.phones.iter().take(1) {
                            line(&ph.value, theme::regular(12.0), t.text);
                        }
                        if resp.clicked() {
                            st.person = Some(c.uid.clone());
                        }
                        if resp.double_clicked() {
                            acts.push(Action::Dialog(Dialog::edit_contact(c)));
                        }
                    }
                });
                ui.add_space(gap);
            }
        });
    });
}

/// Everyone in a table (Outlook's Phone view): name, company, job title, email and phone.
fn phone_list(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let people = s.people(&st.people_query);
    egui::Frame::new().inner_margin(Margin::same(14)).show(ui, |ui| {
        let w = ui.available_width();
        // Column widths: a share of the pane each.
        let cols = [("Full name", 0.24), ("Company", 0.18), ("Job title", 0.16), ("Email", 0.26), ("Phone", 0.16)];
        let row = |ui: &mut Ui, cells: [&str; 5], header: bool, fill: Option<egui::Color32>| -> egui::Response {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), if header { Sense::hover() } else { Sense::click() });
            if let Some(f) = fill {
                ui.painter().rect_filled(r, 4.0, f);
            }
            let mut x = r.left() + 8.0;
            for ((_, share), text) in cols.iter().zip(cells) {
                let cw = w * share;
                let font = if header { theme::bold(12.5) } else { theme::regular(13.0) };
                let g = widgets::one_line(ui, text, font, if header { t.dim } else { t.text }, cw - 12.0);
                ui.painter().galley(egui::pos2(x, r.center().y - g.size().y / 2.0), g, t.text);
                x += cw;
            }
            resp
        };
        row(ui, cols.map(|(h, _)| tl!(h)), true, None);
        let line = ui.available_rect_before_wrap();
        ui.painter().line_segment([line.left_top(), line.right_top()], egui::Stroke::new(1.0, t.border));
        ui.add_space(2.0);
        if people.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(tl!("No contacts yet.")).color(t.faint));
        }
        egui::ScrollArea::vertical().auto_shrink(false).id_salt("people-phone").show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, p) in people.iter().enumerate() {
                let c = &p.card;
                let name = c.display();
                let email = c.emails.first().map_or("", |e| e.value.as_str());
                let phone = c.phones.first().map_or("", |e| e.value.as_str());
                let selected = st.person.as_deref() == Some(c.uid.as_str());
                let id = ui.id().with(("phone-row", i));
                let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
                let fill = if selected {
                    Some(t.accent_soft)
                } else if hovered {
                    Some(t.hover_row)
                } else if i % 2 == 1 {
                    Some(t.chrome)
                } else {
                    None
                };
                let resp = ui.push_id(id, |ui| row(ui, [&name, &c.org, &c.title, email, phone], false, fill)).inner;
                if resp.clicked() {
                    st.person = Some(c.uid.clone());
                }
                if resp.double_clicked() {
                    acts.push(Action::Dialog(Dialog::edit_contact(c)));
                }
            }
        });
    });
}
