//! People: the address books, searchable, with a card for the person picked.

use std::rc::Rc;

use egui::{Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::Session;
use serde_json::json;

use crate::icons::{self, Icon};
use crate::shown::{self, At, PeopleKey};
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
            Column::Book => {
                s.pim.get(p.account).and_then(|pim| pim.books.iter().find(|b| b.id == p.book)).map(|b| book_name(&b.id, &b.name)).unwrap_or_default()
            }
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

/// An address book's name as shown: the local one's, one with none and a server's own
/// "Contacts" in the language shown.
fn book_name(id: &str, name: &str) -> String {
    if id == mailcraft_engine::model::pim::LOCAL || name.is_empty() || name == "Contacts" { tl!("Contacts").to_string() } else { name.to_string() }
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
    let month_name = mm.parse::<u32>().ok().filter(|m| (1..=12).contains(m)).map(crate::calendar::month_name);
    match (month_name, dd.parse::<u32>()) {
        (Some(name), Ok(day)) => tf!("{day} {month}", day = day, month = name),
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

/// The people shown, kept between frames (`shown::Shown`): those the search finds in the address
/// book picked, in name order, and the side list's lines.
pub(crate) struct PeopleList {
    pub(crate) at: Vec<At>,
    /// The side list's lines: each letter's heading before the first name under it, and the
    /// people (their places in `at`).
    lines: Vec<Line>,
    /// The letters with names under them, in order (the A–Z index).
    letters: Vec<char>,
}

/// A line of the side list.
#[derive(Clone, Copy)]
enum Line {
    Letter(char),
    Person(usize),
}

/// The people shown: those matching the search, in the address book chosen, by name. Made again
/// only on the first frame after they or what picks them changed (see `shown`).
pub(crate) fn shown(s: &Session, st: &mut UiState) -> Rc<PeopleList> {
    let fresh = |k: &PeopleKey| k.0 == s.pim_revision && k.1 == st.people_query && k.2 == st.people_book;
    if let Some((k, list)) = &st.shown.people
        && fresh(k)
    {
        return list.clone();
    }
    let mut people = s.people(&st.people_query);
    if let Some((account, book)) = &st.people_book {
        people.retain(|p| p.account == *account && p.book == *book);
    }
    let (mut lines, mut letters) = (Vec::with_capacity(people.len() + 27), Vec::new());
    for (i, p) in people.iter().enumerate() {
        let letter = index_letter(&p.card.display());
        if letters.last() != Some(&letter) {
            letters.push(letter);
            lines.push(Line::Letter(letter));
        }
        lines.push(Line::Person(i));
    }
    let at = people.iter().map(|p| (p.account.to_string(), p.at)).collect();
    let list = Rc::new(PeopleList { at, lines, letters });
    st.shown.people = Some(((s.pim_revision, st.people_query.clone(), st.people_book.clone()), list.clone()));
    list
}

/// The List view's order: the people shown, sorted by the column picked. Made again when they
/// are, or the column, its direction or the language (a column's words) changed.
fn phone_order(s: &Session, st: &mut UiState) -> Rc<[At]> {
    let people = shown(s, st);
    let lang = crate::i18n::current().code();
    let list = &st.people_list;
    let fresh = |k: &(PeopleKey, Option<Column>, bool, &str)| {
        k.0.0 == s.pim_revision && k.0.1 == st.people_query && k.0.2 == st.people_book && k.1 == list.sort && k.2 == list.descending && k.3 == lang
    };
    if let Some((k, order)) = &st.shown.phone
        && fresh(k)
    {
        return order.clone();
    }
    let mut ps: Vec<mailcraft_engine::Person<'_>> = people.at.iter().filter_map(|at| shown::person(s, at)).collect();
    if let Some(col) = list.sort {
        ps.sort_by_cached_key(|p| (col.text(s, p).to_lowercase(), p.card.display().to_lowercase()));
    }
    if list.descending {
        ps.reverse();
    }
    let order: Rc<[At]> = ps.iter().map(|p| (p.account.to_string(), p.at)).collect();
    let key = ((s.pim_revision, st.people_query.clone(), st.people_book.clone()), list.sort, list.descending, lang);
    st.shown.phone = Some((key, order.clone()));
    order
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
        let mirror = crate::bidi::Mirror::new(r);
        if on || resp.hovered() {
            ui.painter().rect_filled(r, 5.0, if on { t.accent_soft } else { t.hover_row });
        }
        let icon = egui::Rect::from_center_size(mirror.pos(r.left_center() + vec2(14.0, 0.0)), vec2(14.0, 14.0));
        icons::paint(ui.painter(), icon, if shared { Icon::People } else { Icon::Person }, if on { t.accent } else { t.dim });
        // Limit Latin names too: right-aligned unbounded text would hide its start in RTL.
        let text = widgets::one_line(ui, label, theme::regular(13.0), t.text, (r.width() - 36.0).max(1.0));
        let at = egui::pos2(mirror.start(r.left() + 28.0, text.size().x), r.center().y - text.size().y / 2.0);
        ui.painter().with_clip_rect(r.shrink2(vec2(4.0, 0.0))).galley(at, text, t.text);
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
        let name = book_name(&b.id, &b.name);
        let label = if accounts > 1 && !b.read_only { format!("{name} ({email})") } else { name };
        let tip = if b.read_only { tf!("Shared with you through {account}: read-only", account = email) } else { email.clone() };
        if row(ui, on, &label, Some(tip), b.read_only) {
            st.people_book = Some((account.clone(), b.id.clone()));
        }
    }
    ui.add_space(6.0);
    ui.separator();
}

/// The side pane: everyone, A to Z (searched from the box at the top), under the address books.
pub fn nav(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    if widgets::accent_button(ui, Icon::Plus, "New contact", &t, !s.accounts.is_empty()).clicked() {
        acts.push(Action::Dialog(Dialog::new_contact()));
    }
    ui.add_space(8.0);
    books(s, st, ui);
    let people = shown(s, st);
    if people.at.is_empty() {
        ui.label(RichText::new(tl!(if st.people_query.trim().is_empty() { "No contacts yet." } else { "No one matches." })).color(t.faint));
    }
    if st.people_view != PeopleView::People {
        ui.label(RichText::new(tf!("{n} people", n = people.at.len())).color(t.faint).small());
        return;
    }
    // Outlook's A–Z index beside the list: a letter jumps to the first name under it.
    let letters = &people.letters;
    let jump_id = egui::Id::unique("people-jump");
    let jump: Option<char> = ui.ctx().data_mut(|d| d.remove_temp(jump_id));
    ui.with_layout(crate::bidi::row_from_start(egui::Align::Min), |ui| {
        // The list leaves room for the index and the gap before it exactly: a few pixels more and
        // the pane (shared by every module) grew a little each frame, up to its widest.
        let list_w = (ui.available_width() - INDEX_W - ui.spacing().item_spacing.x).max(80.0);
        ui.allocate_ui_with_layout(vec2(list_w, ui.available_height()), crate::bidi::column(), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).id_salt("people").show_viewport(ui, |ui, viewport| {
                // Only the lines in view are laid out, each where it would be with all of them:
                // a letter's heading takes 4 points and its letter's height, a person 40.
                let gap = ui.spacing().item_spacing.y;
                let heights: Vec<(char, f32)> = people
                    .letters
                    .iter()
                    .map(|&c| (c, 4.0 + ui.painter().layout_no_wrap(c.to_string(), theme::bold(12.0), t.accent).size().y + gap))
                    .collect();
                let heading = |c: char| heights.iter().find(|(x, _)| *x == c).map_or(0.0, |(_, h)| *h);
                let mut tops = Vec::with_capacity(people.lines.len() + 1);
                let mut y = 0.0;
                for line in &people.lines {
                    tops.push(y);
                    y += match line {
                        Line::Letter(c) => heading(*c),
                        Line::Person(_) => 40.0 + gap,
                    };
                }
                tops.push(y);
                let top = |k: usize| tops.get(k).copied().unwrap_or(y);
                if let Some(c) = jump
                    && let Some(k) = people.lines.iter().position(|l| matches!(l, Line::Letter(x) if *x == c))
                {
                    let origin = ui.min_rect().min;
                    let r = egui::Rect::from_min_size(origin + vec2(0.0, top(k) + 4.0), vec2(1.0, heading(c) - 4.0 - gap));
                    ui.scroll_to_rect(r, Some(egui::Align::TOP));
                }
                shown::rows(ui, viewport, people.lines.len(), top, None, |ui, k| match people.lines.get(k) {
                    Some(Line::Letter(c)) => {
                        ui.add_space(4.0);
                        ui.label(RichText::new(c.to_string()).font(theme::bold(12.0)).color(t.accent));
                    }
                    Some(Line::Person(i)) => match people.at.get(*i).and_then(|at| shown::person(s, at)) {
                        Some(p) => side_row(s, st, ui, &p, acts),
                        None => {
                            ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                        }
                    },
                    None => {}
                });
            });
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // All of it in the height there is: in a short window the letters that have names
            // only, closer together (it ran out under the window's edge, the last letters out
            // of reach).
            let room = ui.available_height();
            let shown: Vec<char> = ('A'..='Z').filter(|c| room >= 26.0 * 11.0 || letters.contains(c)).collect();
            let step = (room / shown.len().max(1) as f32).clamp(8.0, 16.0);
            for c in shown {
                let has = letters.contains(&c);
                let (r, resp) = ui.allocate_exact_size(vec2(INDEX_W, step), if has { Sense::click() } else { Sense::hover() });
                if resp.hovered() && has {
                    ui.painter().rect_filled(r, 3.0, t.hover_row);
                }
                ui.painter().text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    c.to_lowercase().to_string(),
                    theme::regular((step * 0.7).clamp(7.5, 11.0)),
                    if has { t.accent } else { t.faint },
                );
                if resp.clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(jump_id, c));
                }
            }
        });
    });
}

/// A person in the side list: their initials, name and address (or company); a click picks them,
/// a double-click edits them.
fn side_row(s: &Session, st: &mut UiState, ui: &mut Ui, p: &mailcraft_engine::Person<'_>, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let name = p.card.display();
    let selected = st.person.as_deref() == Some(p.card.uid.as_str());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let mirror = crate::bidi::Mirror::new(r);
    if selected || resp.hovered() {
        ui.painter().rect_filled(r, 6.0, if selected { t.accent_soft } else { t.hover_row });
    }
    let email = p.card.emails.first().map(|e| e.value.as_str()).unwrap_or_default();
    crate::photos::avatar(ui, s, mirror.pos(r.left_center() + vec2(20.0, 0.0)), 14.0, email, &initials(&name), theme::avatar_color(&name, t.dark));
    // Each on one line, "…" where the pane is too narrow (they were cut mid-letter).
    let room = r.width() - 46.0;
    let line = widgets::one_line(ui, &name, theme::regular(13.5), t.text, room);
    let at = egui::pos2(mirror.start(r.left() + 42.0, line.size().x), r.center().y - 7.0 - line.size().y / 2.0);
    ui.painter().galley(at, line, t.text);
    let sub = p.card.emails.first().map(|e| e.value.clone()).unwrap_or_else(|| p.card.org.clone());
    let line = widgets::one_line(ui, &sub, theme::regular(11.5), t.dim, room);
    let at = egui::pos2(mirror.start(r.left() + 42.0, line.size().x), r.center().y + 8.0 - line.size().y / 2.0);
    ui.painter().galley(at, line, t.dim);
    if resp.clicked() {
        st.person = Some(p.card.uid.clone());
    }
    if resp.double_clicked() && !s.person_read_only(&p.card.uid) {
        acts.push(Action::Dialog(Dialog::edit_contact(p.card)));
    }
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
        widgets::empty_state(ui, &t, Icon::People, tl!("Pick someone to see their details"));
        return;
    };
    let c = &p.card;
    let name = c.display();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 24 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                let email = c.emails.first().map(|e| e.value.as_str()).unwrap_or_default();
                crate::photos::avatar(ui, s, r.center(), 34.0, email, &initials(&name), theme::avatar_color(&name, t.dark));
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
                            acts.extend(write_to(&name, &e.value));
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
            if let Some(a) = s.account(p.account) {
                ui.label(RichText::new(tf!("In the address book of {account}", account = a.config.email)).color(t.faint).small());
            }
        });
    });
}

/// What a card's Email button does: a new message to the person, in Mail. The name is written
/// as a recipient's is (one with a comma quoted, as an Outlook "Last, First" name has):
/// pasted in bare, the comma parted it into two recipients, the first not an address.
pub(crate) fn write_to(name: &str, email: &str) -> [Action; 3] {
    let to = mailcraft_engine::model::EmailAddress::new(name, email).to_string();
    [Action::Run("mail.new", json!({})), Action::Run("compose.update", json!({ "to": to })), Action::Run("view.module", json!({"name": "mail"}))]
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
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("people-cards").show_viewport(ui, |ui, viewport| {
        egui::Frame::new().inner_margin(Margin::same(18)).show(ui, |ui| {
            if people.at.is_empty() {
                ui.label(RichText::new(tl!("No contacts yet.")).color(t.faint));
                return;
            }
            let card_w = 280.0;
            let card_h = 132.0;
            let gap = 12.0;
            let cols = (((ui.available_width() + gap) / (card_w + gap)).floor() as usize).max(1);
            // Only the rows of cards in view are laid out, each where it would be with all of them.
            let pitch = card_h + ui.spacing().item_spacing.y + gap;
            let rows = people.at.len().div_ceil(cols);
            shown::rows(
                ui,
                viewport,
                rows,
                |k| 18.0 + k as f32 * pitch,
                None,
                |ui, k| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for p in people.at.iter().skip(k.saturating_mul(cols)).take(cols).filter_map(|at| shown::person(s, at)) {
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
                            ui.painter().rect(
                                r,
                                8.0,
                                fill,
                                egui::Stroke::new(1.0, if selected { t.accent } else { t.border }),
                                egui::StrokeKind::Inside,
                            );
                            // A band in the person's colour down the left, with their initials.
                            let band = egui::Rect::from_min_size(r.min, vec2(74.0, card_h));
                            let color = theme::avatar_color(&name, t.dark);
                            ui.painter().rect_filled(band.shrink(1.0), egui::CornerRadius { nw: 7, sw: 7, ne: 0, se: 0 }, color.gamma_multiply(0.18));
                            let email = c.emails.first().map(|e| e.value.as_str()).unwrap_or_default();
                            crate::photos::avatar(ui, s, band.center(), 24.0, email, &initials(&name), color);
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
                },
            );
        });
    });
}

/// Everyone in a table (Outlook's List view): the name and the columns chosen (right-click the
/// header), sorted by the column clicked.
fn phone_list(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let people = phone_order(s, st);
    let list = st.people_list.clone();
    egui::Frame::new().inner_margin(Margin::same(14)).show(ui, |ui| {
        let w = ui.available_width();
        // Column widths: the name a share and a half, the others by their weight.
        let cols: Vec<Option<Column>> = std::iter::once(None).chain(list.columns.iter().copied().map(Some)).collect();
        let weight = |c: &Option<Column>| c.map_or(1.5, Column::weight);
        let total: f32 = cols.iter().map(weight).sum::<f32>().max(1.0);
        let widths: Vec<f32> = cols.iter().map(|c| w * weight(c) / total).collect();
        // A row, filled with `fill`, or (`lit`) lit up while it's pointed at.
        let row = |ui: &mut Ui, cells: &[String], header: bool, fill: Option<egui::Color32>, lit: bool| -> (egui::Response, Vec<egui::Rect>) {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
            let mirror = crate::bidi::Mirror::new(r);
            if let Some(f) = if lit && resp.hovered() { Some(t.hover_row) } else { fill } {
                ui.painter().rect_filled(r, 4.0, f);
            }
            let mut x = r.left() + 8.0;
            let mut rects = Vec::new();
            for (cw, text) in widths.iter().zip(cells) {
                let font = if header { theme::bold(12.5) } else { theme::regular(13.0) };
                let g = widgets::one_line(ui, text, font, if header { t.dim } else { t.text }, cw - 12.0);
                ui.painter().galley(egui::pos2(mirror.start(x, g.size().x), r.center().y - g.size().y / 2.0), g, t.text);
                rects.push(mirror.rect(egui::Rect::from_min_size(egui::pos2(x - 8.0, r.top()), vec2(*cw, r.height()))));
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
        let (head, rects) = row(ui, &heads, true, None, false);
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
        egui::ScrollArea::vertical().auto_shrink(false).id_salt("people-phone").show_viewport(ui, |ui, viewport| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // Only the rows in view are laid out, each where it would be with all of them.
            shown::rows(
                ui,
                viewport,
                people.len(),
                |k| k as f32 * 30.0,
                None,
                |ui, i| {
                    let Some(p) = people.get(i).and_then(|at| shown::person(s, at)) else {
                        ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
                        return;
                    };
                    let c = &p.card;
                    let cells: Vec<String> = cols.iter().map(|col| col.map_or_else(|| c.display(), |col| col.text(s, &p))).collect();
                    let selected = st.person.as_deref() == Some(c.uid.as_str());
                    let id = ui.scope_id().with(("phone-row", i));
                    // Picked, it stays so under the pointer; else every other row is shaded.
                    let fill = if selected {
                        Some(t.accent_soft)
                    } else if i % 2 == 1 {
                        Some(t.chrome)
                    } else {
                        None
                    };
                    let resp = ui.push_id(id, |ui| row(ui, &cells, false, fill, !selected).0).inner;
                    if resp.clicked() {
                        st.person = Some(c.uid.clone());
                    }
                    if resp.double_clicked() && !s.person_read_only(&c.uid) {
                        acts.push(Action::Dialog(Dialog::edit_contact(c)));
                    }
                },
            );
        });
    });
}
