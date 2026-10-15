//! People: the address books, searchable, with a card for the person picked.

use egui::{Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::Session;
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState, widgets};

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
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("people").show(ui, |ui| {
        let mut letter: Option<char> = None;
        for p in &people {
            let name = p.card.display();
            let first = name.chars().next().map(|c| c.to_uppercase().next().unwrap_or(c));
            if first != letter {
                letter = first;
                ui.add_space(4.0);
                ui.label(RichText::new(first.map(String::from).unwrap_or_default()).font(theme::bold(12.0)).color(t.accent));
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
}

/// The main pane: the person picked.
pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
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
