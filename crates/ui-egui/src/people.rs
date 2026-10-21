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
                .get(&p.account)
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
pub(crate) fn shown(s: &Session, st: &UiState) -> Vec<mailcraft_engine::Person> {
    let mut people = s.people(&st.people_query);
    if let Some((account, book)) = &st.people_book {
        people.retain(|p| p.account == *account && p.book == *book);
    }
    people
}

/// Each account's address books, as Outlook lists contact folders (its Contacts, an
/// organisation's shared ones in public folders): a click shows that one alone.
fn books(s: &Session, st: &mut UiState, ui: &mut Ui) {
    let t = st.tokens;
    let all: Vec<(String, mailcraft_engine::model::pim::BookInfo, String)> = s
        .accounts
        .iter()
        .filter_map(|a| s.pim.get(&a.config.id).map(|p| (a, p)))
        .flat_map(|(a, p)| p.books.iter().map(move |b| (a.config.id.clone(), b.clone(), a.config.email.clone())))
        .collect();
    if all.len() < 2 {
        if st.people_book.is_some() {
            st.people_book = None;
        }
        return;
    }
    // A book gone (an account removed): everyone again.
    if st.people_book.as_ref().is_some_and(|(a, b)| !all.iter().any(|(x, y, _)| x == a && y.id == *b)) {
        st.people_book = None;
    }
    let accounts = all.iter().map(|(a, _, _)| a).collect::<std::collections::BTreeSet<_>>().len();
    let row = |ui: &mut Ui, on: bool, label: &str, tip: Option<String>, shared: bool| {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
        if on || resp.hovered() {
            ui.painter().rect_filled(r, 5.0, if on { t.accent_soft } else { t.hover_row });
        }
        let icon = egui::Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(14.0, 14.0));
        icons::paint(ui.painter(), icon, if shared { Icon::People } else { Icon::Person }, if on { t.accent } else { t.dim });
        ui.painter().with_clip_rect(r.shrink2(vec2(4.0, 0.0))).text(
            r.left_center() + vec2(28.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            theme::regular(13.0),
            t.text,
        );
        match tip {
            Some(tip) => resp.on_hover_text(tip).clicked(),
            None => resp.clicked(),
        }
    };
    ui.label(RichText::new(tl!("Address books")).font(theme::bold(12.0)).color(t.dim));
    if row(ui, st.people_book.is_none(), tl!("All contacts"), None, false) {
        st.people_book = None;
    }
    for (account, b, email) in &all {
        let on = st.people_book.as_ref().is_some_and(|(a, x)| a == account && *x == b.id);
        let name = if b.id == mailcraft_engine::model::pim::LOCAL || b.name.is_empty() || b.name == "Contacts" {
            tl!("Contacts").to_string()
        } else {
            b.name.clone()
        };
        let label = if accounts > 1 && !b.read_only { format!("{name} ({email})") } else { name };
        let tip = if b.read_only { tf!("Shared with you through {account}: read-only", account = email) } else { email.clone() };
        if row(ui, on, &label, Some(tip), b.read_only) {
            st.people_book = Some((account.clone(), b.id.clone()));
        }
    }
    ui.add_space(6.0);
    ui.separator();
}

pub fn nav(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    if widgets::accent_button(ui, Icon::Plus, "New contact", &t, !s.accounts.is_empty()).clicked() {
        acts.push(Action::Dialog(Dialog::new_contact()));
    }
    ui.add_space(8.0);
    books(s, st, ui);
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
                    ui.painter().with_clip_rect(r.shrink2(vec2(2.0, 0.0))).text(
                        r.left_center() + vec2(42.0, -7.0),
                        egui::Align2::LEFT_CENTER,
                        &name,
                        theme::regular(13.5),
                        t.text,
                    );
                    let sub = p.card.emails.first().map(|e| e.value.clone()).unwrap_or_else(|| p.card.org.clone());
                    ui.painter().with_clip_rect(r.shrink2(vec2(2.0, 0.0))).text(
                        r.left_center() + vec2(42.0, 8.0),
                        egui::Align2::LEFT_CENTER,
                        sub,
                        theme::regular(11.5),
                        t.dim,
                    );
                    if resp.clicked() {
                        st.person = Some(p.card.uid.clone());
                    }
                    if resp.double_clicked() && !s.person_read_only(&p.card.uid) {
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
            for e in &c.emails {
                field(ui, &t, Icon::Unread, &e.value, &e.kind);
            }
            // Each number with what it is (Outlook's Business, Home, Mobile…).
            for ph in &c.phones {
                field(ui, &t, Icon::Phone, &ph.value, phone_kind(&ph.kind));
            }
            for a in c.addresses.iter().filter(|a| !a.is_empty()) {
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
                field(ui, &t, Icon::Location, &lines, kind);
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
            if let Some(a) = s.account(&p.account) {
                ui.label(RichText::new(tf!("In the address book of {account}", account = a.config.email)).color(t.faint).small());
            }
        });
    });
}

/// What a number is, as Outlook names it.
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

/// Everyone as business cards, side by side (Outlook's Business Card view): a click picks one,
/// a double-click edits it.
fn cards(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let people = shown(s, st);
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
                        if resp.double_clicked() && !s.person_read_only(&c.uid) {
                            acts.push(Action::Dialog(Dialog::edit_contact(c)));
                        }
                    }
                });
                ui.add_space(gap);
            }
        });
    });
}

/// Everyone in a table (Outlook's List view): the name and the columns chosen (right-click the
/// header), sorted by the column clicked.
fn phone_list(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let mut people = shown(s, st);
    let list = st.people_list.clone();
    if let Some(col) = list.sort {
        people.sort_by_cached_key(|p| (col.text(s, p).to_lowercase(), p.card.display().to_lowercase()));
    }
    if list.descending {
        people.reverse();
    }
    egui::Frame::new().inner_margin(Margin::same(14)).show(ui, |ui| {
        let w = ui.available_width();
        // Column widths: the name a share and a half, the others by their weight.
        let cols: Vec<Option<Column>> = std::iter::once(None).chain(list.columns.iter().copied().map(Some)).collect();
        let weight = |c: &Option<Column>| c.map_or(1.5, Column::weight);
        let total: f32 = cols.iter().map(weight).sum::<f32>().max(1.0);
        let widths: Vec<f32> = cols.iter().map(|c| w * weight(c) / total).collect();
        let row = |ui: &mut Ui, cells: &[String], header: bool, fill: Option<egui::Color32>| -> (egui::Response, Vec<egui::Rect>) {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
            if let Some(f) = fill {
                ui.painter().rect_filled(r, 4.0, f);
            }
            let mut x = r.left() + 8.0;
            let mut rects = Vec::new();
            for (cw, text) in widths.iter().zip(cells) {
                let font = if header { theme::bold(12.5) } else { theme::regular(13.0) };
                let g = widgets::one_line(ui, text, font, if header { t.dim } else { t.text }, cw - 12.0);
                ui.painter().galley(egui::pos2(x, r.center().y - g.size().y / 2.0), g, t.text);
                rects.push(egui::Rect::from_min_size(egui::pos2(x - 8.0, r.top()), vec2(*cw, r.height())));
                x += cw;
            }
            (resp, rects)
        };
        // The header: click a column to sort by it (again: the other way); right-click to choose.
        let arrow = if list.descending { " ↓" } else { " ↑" };
        let heads: Vec<String> = cols
            .iter()
            .map(|c| {
                let name = c.map_or_else(|| tl!("Full name").to_string(), |c| tl!(c.label()).to_string());
                if *c == list.sort { format!("{name}{arrow}") } else { name }
            })
            .collect();
        let (head, rects) = row(ui, &heads, true, None);
        if head.clicked()
            && let Some(p) = head.interact_pointer_pos()
            && let Some(i) = rects.iter().position(|r| r.contains(p))
            && let Some(c) = cols.get(i)
        {
            if *c == st.people_list.sort {
                st.people_list.descending = !st.people_list.descending;
            } else {
                st.people_list.sort = *c;
                st.people_list.descending = false;
            }
        }
        head.on_hover_text(tl!("Click a column to sort by it; right-click to choose the columns"))
            .context_menu(|ui| column_menu(ui, &mut st.people_list));
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
                let cells: Vec<String> = cols.iter().map(|col| col.map_or_else(|| c.display(), |col| col.text(s, p))).collect();
                let selected = st.person.as_deref() == Some(c.uid.as_str());
                let id = ui.scope_id().with(("phone-row", i));
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
                let resp = ui.push_id(id, |ui| row(ui, &cells, false, fill).0).inner;
                if resp.clicked() {
                    st.person = Some(c.uid.clone());
                }
                if resp.double_clicked() && !s.person_read_only(&c.uid) {
                    acts.push(Action::Dialog(Dialog::edit_contact(c)));
                }
            }
        });
    });
}
