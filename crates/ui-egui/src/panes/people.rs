//! People: the address books, searchable, with a card for the person picked.

mod books;
mod cards;
mod list;
mod system_contact;
use system_contact::add_contact_action;

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
    /// A table of the columns chosen (Outlook's List view; kept as `phone` in saved settings).
    Phone,
}

/// A column the list can show besides the name (Outlook's List view fields).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Column {
    Company,
    Title,
    Email,
    Business,
    Mobile,
    Home,
    Notes,
    Book,
    Fax,
    BusinessAddress,
    HomeAddress,
    Birthday,
}

impl Column {
    pub const ALL: [Column; 12] = [
        Column::Company,
        Column::Title,
        Column::Email,
        Column::Business,
        Column::Mobile,
        Column::Home,
        Column::Fax,
        Column::BusinessAddress,
        Column::HomeAddress,
        Column::Birthday,
        Column::Notes,
        Column::Book,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Column::Company => "Company",
            Column::Title => "Job title",
            Column::Email => "Email",
            Column::Business => "Business phone",
            Column::Mobile => "Mobile phone",
            Column::Home => "Home phone",
            Column::Notes => "Notes",
            Column::Book => "Address book",
            Column::Fax => "Business fax",
            Column::BusinessAddress => "Business address",
            Column::HomeAddress => "Home address",
            Column::Birthday => "Birthday",
        }
    }

    /// Its share of the width against the others shown.
    fn weight(self) -> f32 {
        match self {
            Column::Email | Column::Notes => 1.4,
            Column::BusinessAddress | Column::HomeAddress => 1.6,
            Column::Birthday => 0.7,
            Column::Book => 1.1,
            _ => 1.0,
        }
    }

    /// What it shows for someone.
    fn text(self, s: &Session, p: &mailcraft_engine::Person) -> String {
        let c = &p.card;
        let phone = |kinds: &[&str]| c.phones.iter().find(|x| kinds.contains(&x.kind.as_str())).map(|x| x.value.clone()).unwrap_or_default();
        match self {
            Column::Company => c.org.clone(),
            Column::Title => c.title.clone(),
            Column::Email => c.emails.first().map(|e| e.value.clone()).unwrap_or_default(),
            Column::Business => phone(&["work", "voice", "pref"]),
            Column::Fax => phone(&["fax"]),
            Column::BusinessAddress => c.addresses.iter().find(|a| a.kind == "work").map(|a| a.one_line()).unwrap_or_default(),
            Column::HomeAddress => c.addresses.iter().find(|a| a.kind == "home").map(|a| a.one_line()).unwrap_or_default(),
            Column::Birthday => birthday_shown(&c.birthday),
            Column::Mobile => phone(&["cell", "mobile"]),
            Column::Home => phone(&["home"]),
            Column::Notes => c.note.lines().next().unwrap_or_default().to_string(),
            Column::Book => s
                .pim
                .get(p.account)
                .and_then(|pim| pim.books.iter().find(|b| b.id == p.book))
                .map(|b| if b.id == mailcraft_engine::model::pim::LOCAL { tl!("Contacts").to_string() } else { b.name.clone() })
                .unwrap_or_default(),
        }
    }
}

/// What the list shows and how it's sorted (kept with the window's settings).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ListSettings {
    pub columns: Vec<Column>,
    /// Sorted by this column (`None`: by name).
    pub sort: Option<Column>,
    pub descending: bool,
}

impl Default for ListSettings {
    fn default() -> Self {
        ListSettings { columns: vec![Column::Company, Column::Title, Column::Email, Column::Business, Column::Mobile], sort: None, descending: false }
    }
}

/// Columns to tick (Outlook's Field Chooser), for the header's right-click and the ribbon.
pub fn column_menu(ui: &mut Ui, list: &mut ListSettings) {
    for c in Column::ALL {
        let mut on = list.columns.contains(&c);
        if ui.checkbox(&mut on, tl!(c.label())).changed() {
            if on {
                // In the order the menu lists them.
                list.columns.push(c);
                list.columns.sort_by_key(|x| Column::ALL.iter().position(|y| y == x));
            } else {
                list.columns.retain(|x| *x != c);
                if list.sort == Some(c) {
                    list.sort = None;
                }
            }
        }
    }
}

impl PeopleView {
    pub const ALL: [PeopleView; 3] = [PeopleView::People, PeopleView::Cards, PeopleView::Phone];

    pub fn label(self) -> &'static str {
        match self {
            PeopleView::People => "People",
            PeopleView::Cards => "Cards",
            PeopleView::Phone => "List",
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

/// Display form of a stored birthday: `--MM-DD` becomes a localized "20 February"; full
/// dates are shown as stored.
fn birthday_shown(b: &str) -> String {
    let Some(b) = mailcraft_engine::model::vcard::birthday_of(b) else { return b.to_string() };
    let Some((mm, dd)) = b.strip_prefix("--").and_then(|md| md.split_once('-')) else { return b };
    let month_name = mm.parse::<usize>().ok().and_then(|m| m.checked_sub(1)).and_then(|i| crate::calendar::MONTHS.get(i).copied());
    match (month_name, dd.parse::<u32>()) {
        (Some(name), Ok(day)) => tf!("{day} {month}", day = day, month = crate::i18n::tc("month", name)),
        _ => b.clone(),
    }
}

/// A name's index letter: its first letter, or `#` for one that isn't a letter.
fn index_letter(name: &str) -> char {
    match name.chars().next() {
        Some(c) if c.is_alphabetic() => c.to_uppercase().next().unwrap_or(c),
        _ => '#',
    }
}

/// The side pane: everyone, A to Z (searched from the box at the top).
/// The people shown: those matching the search, in the address book chosen.
pub(crate) fn shown<'a>(s: &'a Session, st: &UiState) -> Vec<mailcraft_engine::Person<'a>> {
    let mut people = s.people(&st.people_query);
    if let Some((account, book)) = &st.people_book {
        people.retain(|p| p.account == *account && p.book == *book);
    }
    people
}

pub fn nav(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    if !st.phone_mode && widgets::accent_button(ui, Icon::Plus, "New contact", &t, !s.accounts.is_empty()).clicked() {
        acts.push(Action::Dialog(Dialog::new_contact()));
    }
    ui.add_space(8.0);
    books::show(s, st, ui);
    let people = shown(s, st);
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
    let jump_id = egui::Id::unique("people-jump");
    let jump: Option<char> = ui.ctx().data_mut(|d| d.remove_temp(jump_id));
    let touch = ui.ctx().input(|input| input.has_touch_screen());
    ui.horizontal_top(|ui| {
        // The list leaves room for the index and the gap before it exactly: a few pixels more and
        // the pane (shared by every module) grew a little each frame, up to its widest.
        let index_w = if letters.is_empty() {
            0.0
        } else if touch {
            44.0
        } else {
            INDEX_W
        };
        let index_gap = if letters.is_empty() { 0.0 } else { ui.spacing().item_spacing.x };
        let list_w = (ui.available_width() - index_w - index_gap).max(80.0);
        ui.allocate_ui_with_layout(vec2(list_w, ui.available_height()), crate::bidi::column(), |ui| {
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
                    let row_height = if touch { 48.0 } else { 40.0 };
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
                    let sub = p.card.emails.first().map(|e| e.value.clone()).unwrap_or_else(|| p.card.org.clone());
                    widgets::named(ui, &resp, &format!("{name}, {sub}"), true, Some(selected));
                    if selected || resp.hovered() {
                        ui.painter().rect_filled(r, 6.0, if selected { t.accent_soft } else { t.hover_row });
                    }
                    let mirror = crate::bidi::Mirror::new(r);
                    let initials: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
                    widgets::avatar(ui, mirror.pos(r.left_center() + vec2(20.0, 0.0)), 14.0, &initials, theme::avatar_color(&name, t.dark));
                    ui.painter().with_clip_rect(r.shrink2(vec2(2.0, 0.0))).text(
                        mirror.pos(r.left_center() + vec2(42.0, -7.0)),
                        mirror.align(egui::Align2::LEFT_CENTER),
                        &name,
                        theme::regular(13.5),
                        t.text,
                    );
                    ui.painter().with_clip_rect(r.shrink2(vec2(2.0, 0.0))).text(
                        mirror.pos(r.left_center() + vec2(42.0, 8.0)),
                        mirror.align(egui::Align2::LEFT_CENTER),
                        sub,
                        theme::regular(11.5),
                        t.dim,
                    );
                    if resp.clicked() {
                        st.person = Some(p.card.uid.clone());
                    }
                    if resp.double_clicked() && !s.person_read_only(&p.card.uid) {
                        acts.push(Action::Dialog(Dialog::edit_contact(p.card)));
                    }
                }
            });
        });
        if touch {
            if !letters.is_empty() {
                let index = ui.add_sized(vec2(index_w, 44.0), egui::Button::new("A–Z"));
                widgets::named(ui, &index, "A–Z", true, None);
                crate::bidi::popup_menu(&index).show(|ui| {
                    ui.set_max_width(220.0);
                    ui.horizontal_wrapped(|ui| {
                        for c in &letters {
                            let letter = c.to_string();
                            let choice = ui.add_sized(vec2(44.0, 44.0), egui::Button::new(c.to_lowercase().to_string()));
                            widgets::named(ui, &choice, &letter, true, None);
                            if choice.clicked() {
                                ui.ctx().data_mut(|d| d.insert_temp(jump_id, *c));
                                ui.close();
                            }
                        }
                    });
                });
            }
        } else if !letters.is_empty() {
            crate::bidi::vertical(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for c in 'A'..='Z' {
                    let has = letters.contains(&c);
                    let (r, resp) = ui.allocate_exact_size(vec2(index_w, 16.0), if has { Sense::click() } else { Sense::hover() });
                    let letter = c.to_string();
                    widgets::named(ui, &resp, &letter, has, None);
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
        }
    });
}

/// The main pane: the person picked, or everyone as cards or a phone list.
pub fn show(
    s: &Session,
    st: &mut UiState,
    ui: &mut Ui,
    acts: &mut Vec<Action>,
    can_dial_phone: bool,
    can_open_map: bool,
    can_share_contacts: bool,
    can_export_contacts: bool,
    can_add_to_system_contacts: bool,
) {
    let t = st.tokens;
    if st.phone_mode && st.person.is_none() {
        match st.people_view {
            PeopleView::Cards => return cards::show(s, st, ui, acts, can_share_contacts, can_export_contacts, can_add_to_system_contacts),
            PeopleView::Phone => return list::show(s, st, ui, acts, can_dial_phone, can_open_map),
            PeopleView::People => {
                nav(s, st, ui, acts);
                return;
            }
        }
    }
    if !st.phone_mode {
        match st.people_view {
            PeopleView::Cards => return cards::show(s, st, ui, acts, can_share_contacts, can_export_contacts, can_add_to_system_contacts),
            PeopleView::Phone => return list::show(s, st, ui, acts, can_dial_phone, can_open_map),
            PeopleView::People => {}
        }
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
    let inset = if st.phone_mode { 16 } else { 28 };
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: inset, right: inset, top: inset, bottom: inset }).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                let initials: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
                widgets::avatar(ui, r.center(), 34.0, &initials, theme::avatar_color(&name, t.dark));
                ui.add_space(10.0);
                crate::bidi::vertical(ui, |ui| {
                    ui.label(RichText::new(&name).font(theme::bold(22.0)).color(t.text));
                    let role = [c.title.as_str(), c.org.as_str()].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" · ");
                    if !role.is_empty() {
                        ui.label(RichText::new(role).color(t.dim));
                    }
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if let Some(e) = c.emails.first()
                            && widgets::pill_button(ui, Icon::NewMail, "Email", &t).clicked()
                        {
                            acts.push(Action::Run("mail.new", json!({})));
                            acts.push(Action::Run("compose.update", json!({"to": format!("{name} <{}>", e.value)})));
                            acts.push(Action::Run("view.module", json!({"name": "mail"})));
                        }
                        if can_share_contacts && contact_action_button(ui, "Share contact").clicked() {
                            acts.push(Action::ShareContact {
                                title: tl!("Share contact").to_owned(),
                                vcard: mailcraft_engine::model::vcard::write(c),
                            });
                        }
                        if can_export_contacts && contact_action_button(ui, "Save contact").clicked() {
                            acts.push(Action::ExportContact { name: name.clone(), vcard: mailcraft_engine::model::vcard::write(c) });
                        }
                        if can_add_to_system_contacts && contact_action_button(ui, "Add to Android contacts").clicked() {
                            acts.push(add_contact_action(c));
                        }
                        // Someone in a shared address book (an organisation's) is only read.
                        if s.person_read_only(&c.uid) {
                            ui.label(RichText::new(tl!("In a shared address book: read-only")).color(t.faint).small());
                        } else {
                            if widgets::pill_button(ui, Icon::Drafts, "Edit", &t).clicked() {
                                acts.push(Action::Dialog(Dialog::edit_contact(c)));
                            }
                            if widgets::pill_button(ui, Icon::Delete, "Delete", &t).clicked() {
                                acts.push(Action::Dialog(Dialog::ConfirmDeleteContact { uid: c.uid.clone(), name: c.display() }));
                            }
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
            for (index, e) in c.emails.iter().enumerate() {
                let action = crate::mailto::link(&e.value).map(Action::OpenUrl);
                contact_detail_row(
                    ui,
                    18.0,
                    &e.value,
                    ("contact-email", &c.uid, index),
                    action,
                    Some(tl!("Copy address")),
                    Some(&e.value),
                    &t,
                    acts,
                    |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        icons::paint(ui.painter(), r, Icon::Unread, t.accent);
                        if crate::mailto::link(&e.value).is_some() {
                            ui.label(RichText::new(&e.value).color(t.accent).underline());
                        } else {
                            ui.label(RichText::new(&e.value).color(t.text));
                        }
                        if !e.kind.is_empty() {
                            ui.label(RichText::new(&e.kind).color(t.faint).small());
                        }
                    },
                );
            }
            // Each number with what it is (Outlook's Business, Home, Mobile…).
            for (index, ph) in c.phones.iter().enumerate() {
                let action = can_dial_phone.then(|| Action::DialPhone(ph.value.clone()));
                contact_detail_row(
                    ui,
                    18.0,
                    &ph.value,
                    ("contact-phone", &c.uid, index),
                    action,
                    Some(tl!("Copy phone number")),
                    Some(&ph.value),
                    &t,
                    acts,
                    |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        icons::paint(ui.painter(), r, Icon::Phone, t.accent);
                        if can_dial_phone {
                            ui.label(RichText::new(&ph.value).color(t.accent).underline());
                        } else {
                            ui.label(RichText::new(&ph.value).color(t.text));
                        }
                        ui.label(RichText::new(phone_kind(&ph.kind)).color(t.faint).small());
                    },
                );
            }
            for (index, a) in c.addresses.iter().enumerate().filter(|(_, a)| !a.is_empty()) {
                let kind = match a.kind.as_str() {
                    "work" => tl!("Business"),
                    "home" => tl!("Home"),
                    _ => tl!("Other"),
                };
                let lines =
                    [a.street.clone(), format!("{} {}", a.postcode.trim(), a.city.trim()).trim().to_string(), a.region.clone(), a.country.clone()]
                        .into_iter()
                        .filter(|x| !x.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                let text_height = lines.lines().count().max(1) as f32 * 18.0;
                let action = can_open_map.then(|| Action::OpenMap(lines.clone()));
                contact_detail_row(
                    ui,
                    text_height,
                    &lines,
                    ("contact-address", &c.uid, index),
                    action,
                    Some(tl!("Copy address")),
                    Some(&lines),
                    &t,
                    acts,
                    |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        icons::paint(ui.painter(), r, Icon::Location, t.accent);
                        if can_open_map {
                            ui.label(RichText::new(&lines).color(t.accent).underline());
                        } else {
                            ui.label(RichText::new(&lines).color(t.text));
                        }
                        if !kind.is_empty() {
                            ui.label(RichText::new(kind).color(t.faint).small());
                        }
                    },
                );
            }
            if !c.birthday.is_empty() {
                field(ui, &t, Icon::Calendar, &birthday_shown(&c.birthday), tl!("Birthday"));
            }
            if !c.note.is_empty() {
                ui.add_space(10.0);
                ui.label(RichText::new(tl!("Notes")).font(theme::bold(13.0)).color(t.dim));
                ui.label(RichText::new(&c.note).color(t.text));
            }
            ui.add_space(14.0);
            if let Some(a) = s.account(p.account) {
                ui.label(RichText::new(tf!("In the address book of {account}", account = a.config.email)).color(t.faint).small());
            }
        });
    });
}

/// What a number is, as Outlook names it.
/// Lay out a contact's detail row, with its whole height as the action target.
fn contact_detail_row(
    ui: &mut Ui,
    content_height: f32,
    label: &str,
    id: impl std::hash::Hash + std::fmt::Debug,
    action: Option<Action>,
    copy_label: Option<&str>,
    copy_text: Option<&str>,
    t: &Tokens,
    acts: &mut Vec<Action>,
    add: impl FnOnce(&mut Ui),
) {
    let touch = ui.ctx().input(|input| input.has_touch_screen());
    let height = if touch { content_height.max(44.0) } else { content_height.max(26.0) };
    let copy_width = 44.0;
    let mut copy_clicked = false;
    let mut content_rect = egui::Rect::NOTHING;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), height), crate::bidi::row_from_start(egui::Align::Center), |row_ui| {
        let show_copy = touch && copy_label.is_some() && copy_text.is_some();
        let reserved_width = if show_copy { copy_width + row_ui.spacing().item_spacing.x } else { 0.0 };
        let content_width = (row_ui.available_width() - reserved_width).max(0.0);
        let content = row_ui.allocate_ui_with_layout(vec2(content_width, height), crate::bidi::row_from_start(egui::Align::Center), add);
        content_rect = content.response.rect;
        if show_copy && let (Some(copy_label), Some(copy_text)) = (copy_label, copy_text) {
            let response = widgets::tiny_icon_button(row_ui, Icon::Copy, copy_label, t);
            if response.clicked() {
                row_ui.ctx().copy_text(copy_text.to_owned());
                copy_clicked = true;
            }
        }
    });
    if action.is_some() || copy_text.is_some() {
        let response = ui.interact(content_rect, ui.scope_id().with(id), Sense::click());
        let response = if action.is_some() { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), label));
        if !copy_clicked
            && let Some(action) = action
            && response.clicked()
        {
            acts.push(action);
        }
        if let (Some(copy_label), Some(copy_text)) = (copy_label, copy_text) {
            response.context_menu(|ui| {
                if ui.button(copy_label).clicked() {
                    ui.ctx().copy_text(copy_text.to_owned());
                    ui.close();
                }
            });
        }
    }
}

/// A detail action, tall enough to tap on a phone, with the ordinary desktop sizing elsewhere.
fn contact_action_button(ui: &mut Ui, label: &str) -> egui::Response {
    let button = egui::Button::new(tl!(label));
    if ui.ctx().input(|input| input.has_touch_screen()) { ui.add(button.min_size(vec2(0.0, 44.0))) } else { ui.add(button) }
}

fn phone_kind(kind: &str) -> &'static str {
    match kind {
        "work" | "voice" => tl!("Business"),
        "home" => tl!("Home"),
        "cell" | "mobile" => tl!("Mobile"),
        "fax" => tl!("Fax"),
        "pager" => tl!("Pager"),
        _ => tl!("Other"),
    }
}
