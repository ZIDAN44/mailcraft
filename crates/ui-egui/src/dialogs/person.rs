//! Everything with one person (issue #178): the mail the user and they have exchanged, by
//! conversation, its files and links, the meetings with them and the others on that mail; what
//! their signature gives, offered for their contact; all of it narrowed by a search. Someone
//! else is found from the box at its top.

use egui::Sense;
use mailcraft_engine::PersonActivity;
use mailcraft_engine::model::EmailAddress;
use mailcraft_engine::model::contacts::Contact;
use mailcraft_engine::model::dates;

use super::*;

/// The most rows a part shows: the search narrows to the rest.
const ROWS_SHOWN: usize = 200;
/// The most messages shown under a conversation.
const MESSAGES_SHOWN: usize = 20;
/// The rows of each part the overview shows.
const OVERVIEW_ROWS: usize = 5;
/// The most people the box at the top offers.
pub(crate) const PEOPLE_OFFERED: usize = 6;

/// The search within it (a test puts the keyboard there).
pub(crate) fn search_id() -> egui::Id {
    egui::Id::unique("mailcraft-person-search")
}

/// The box that finds someone else.
pub(crate) fn lookup_id() -> egui::Id {
    egui::Id::unique("mailcraft-person-lookup")
}

/// The parts, as the tabs show them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PersonTab {
    /// A few of each.
    #[default]
    Overview,
    Mail,
    Files,
    Meetings,
    Links,
    People,
}

const TABS: [PersonTab; 6] = [PersonTab::Overview, PersonTab::Mail, PersonTab::Files, PersonTab::Meetings, PersonTab::Links, PersonTab::People];

/// What the dialog keeps: all there is with them, what the search narrows it to, the part shown,
/// and someone else being looked for.
#[derive(Clone, Debug)]
pub struct PersonForm {
    pub all: PersonActivity,
    pub shown: PersonActivity,
    pub query: String,
    pub tab: PersonTab,
    /// The name or address typed to find someone else, and who it finds.
    pub lookup: String,
    pub people_found: Vec<Contact>,
}

impl PersonForm {
    pub fn new(s: &mut Session, email: &str) -> PersonForm {
        let all = s.person_activity(email);
        PersonForm { shown: all.clone(), all, query: String::new(), tab: PersonTab::Overview, lookup: String::new(), people_found: Vec::new() }
    }

    /// Search within it (an empty search shows it all).
    pub fn search(&mut self, query: &str) {
        query.clone_into(&mut self.query);
        self.shown = self.all.refined(query);
    }
}

pub(super) fn draw(ui: &mut Ui, f: &mut PersonForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    let w = (ui.ctx().content_rect().width() * 0.6).clamp(420.0, 760.0);
    ui.set_width(w);
    lookup(ui, f, t, acts);
    ui.add_space(8.0);
    header(ui, f, s, t, acts);
    found(ui, f, s, t, acts);
    ui.add_space(8.0);
    let r = ui.add(
        egui::TextEdit::singleline(&mut f.query).id(search_id()).hint_text(tl!("Search within everything with them")).desired_width(f32::INFINITY),
    );
    if r.changed() {
        let q = f.query.clone();
        f.search(&q);
    }
    ui.add_space(6.0);
    let labels: Vec<String> = TABS.iter().map(|tab| label(*tab, &f.shown)).collect();
    let names: Vec<&str> = labels.iter().map(String::as_str).collect();
    let selected = TABS.iter().position(|x| *x == f.tab).unwrap_or(0);
    if let Some(tab) = widgets::segmented(ui, t, "person-parts", &names, selected).and_then(|i| TABS.get(i)) {
        f.tab = *tab;
    }
    ui.add_space(6.0);
    let tall = (ui.ctx().content_rect().height() * 0.45).max(160.0);
    egui::ScrollArea::vertical().id_salt(("person-part", f.tab as u8)).max_height(tall).auto_shrink([false, true]).show(ui, |ui| {
        let a = &f.shown;
        let empty = match f.tab {
            PersonTab::Overview => overview(ui, f, s, t, acts),
            PersonTab::Mail => mail(ui, a, s, t, acts, ROWS_SHOWN, true),
            PersonTab::Files => files(ui, a, s, t, acts, ROWS_SHOWN),
            PersonTab::Meetings => meetings(ui, a, s, t, acts, ROWS_SHOWN),
            PersonTab::Links => links(ui, a, s, t, acts, ROWS_SHOWN),
            PersonTab::People => people(ui, a, t, acts, ROWS_SHOWN),
        };
        if empty {
            let why = if f.query.trim().is_empty() { tl!("Nothing here.") } else { tl!("Nothing here matches the search.") };
            ui.label(RichText::new(why).color(t.dim));
        }
    });
    // What it's made of: the mail on this computer.
    if f.all.not_downloaded > 0 {
        ui.add_space(4.0);
        let n = f.all.not_downloaded;
        let note = tf!("Files and links come from the mail on this computer: {n} of these messages aren't downloaded.", n = n);
        ui.label(RichText::new(note).color(t.dim).small());
    }
    if f.all.cut_short {
        ui.label(RichText::new(tl!("Only the newest of the mail with them is shown.")).color(t.dim).small());
    }
    ui.add_space(6.0);
    close_row(ui)
}

/// A part's name with how many it holds.
fn label(tab: PersonTab, a: &PersonActivity) -> String {
    match tab {
        PersonTab::Overview => tl!("Overview").to_string(),
        PersonTab::Mail => tf!("Mail ({n})", n = a.message_count()),
        PersonTab::Files => tf!("Files ({n})", n = a.files.len()),
        PersonTab::Meetings => tf!("Meetings ({n})", n = a.meetings.len()),
        PersonTab::Links => tf!("Links ({n})", n = a.links.len()),
        PersonTab::People => tf!("People ({n})", n = a.people.len()),
    }
}

/// Someone else, by name or address: those mail was exchanged with and the contacts (the app
/// asks, [`Action::PersonLookup`]); a click shows everything with them.
fn lookup(ui: &mut Ui, f: &mut PersonForm, t: &Tokens, acts: &mut Vec<Action>) {
    let r = ui.add(
        egui::TextEdit::singleline(&mut f.lookup).id(lookup_id()).hint_text(tl!("Find someone by name or address")).desired_width(f32::INFINITY),
    );
    if r.changed() {
        acts.push(Action::PersonLookup(f.lookup.clone()));
    }
    if f.lookup.trim().is_empty() {
        return;
    }
    if f.people_found.is_empty() {
        ui.label(RichText::new(tl!("No one found.")).color(t.dim));
    }
    for (i, c) in f.people_found.iter().take(PEOPLE_OFFERED).enumerate() {
        let a = &c.address;
        let name = if a.name.trim().is_empty() { a.email.clone() } else { format!("{} <{}>", a.name.trim(), a.email) };
        if ui.push_id(("someone", i), |ui| row(ui, t, &name, false, "", tl!("Everything with them"))).inner.clicked() {
            acts.push(Action::Person(a.email.clone()));
        }
    }
}

/// Who they are, their addresses, and what can be done with them.
fn header(ui: &mut Ui, f: &PersonForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    let a = &f.all;
    let known = s.contact_by_address(&a.email);
    let who = EmailAddress::new(&a.name, &a.email);
    ui.horizontal(|ui| {
        let (ar, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
        crate::photos::avatar(ui, s, ar.center(), 23.0, &a.email, &who.initials(), theme::avatar_color(&a.email, t.dark));
        ui.add_space(6.0);
        ui.vertical(|ui| {
            let r = ui.label(RichText::new(&a.name).font(theme::bold(18.0)).color(t.text));
            // What the dialog is called, to a screen reader.
            ui.ctx().data_mut(|d| d.insert_temp(title_id(), Some(r.id)));
            if let Some(c) = known {
                let what = [c.title.as_str(), c.org.as_str()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
                if !what.is_empty() {
                    ui.label(RichText::new(what).color(t.dim));
                }
            }
            ui.label(RichText::new(a.addresses.join(", ")).color(t.dim).small());
        });
    });
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if widgets::pill_button(ui, Icon::NewMail, "Send email", t).clicked() {
            acts.push(Action::Run("mail.new", json!({"to": who.to_string()})));
        }
        let (label, dialog) = match known {
            Some(card) => ("Edit contact…", Dialog::edit_contact(card)),
            None => ("Add to contacts…", Dialog::contact_from(&who)),
        };
        if widgets::pill_button(ui, Icon::Person, label, t).clicked() {
            acts.push(Action::Dialog(dialog));
        }
    });
}

/// What their signature gives that their contact doesn't have: offered, never put in by itself.
fn found(ui: &mut Ui, f: &PersonForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    let got = &f.all.found;
    if got.is_empty() {
        return;
    }
    ui.add_space(8.0);
    egui::Frame::new().fill(t.bg).corner_radius(6).inner_margin(Margin::same(8)).stroke(egui::Stroke::new(1.0, t.border)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(tl!("In their signature")).color(t.dim).small());
        ui.horizontal_wrapped(|ui| {
            for phone in &got.phones {
                ui.label(RichText::new(phone).color(t.text));
                ui.add_space(8.0);
            }
            for site in &got.websites {
                let r = ui.add(egui::Label::new(RichText::new(site).color(t.accent).underline()).sense(Sense::click()));
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    let url = if site.contains("://") { site.clone() } else { format!("https://{site}") };
                    acts.push(Action::OpenUrl(url));
                }
                ui.add_space(8.0);
            }
        });
        if !got.phones.is_empty() && ui.button(tl!("Add to their contact…")).clicked() {
            acts.push(Action::Dialog(with_phones(s, f)));
        }
    });
}

/// Their contact's form (a new one when they have none) with the numbers their signature gives
/// put in its empty places.
fn with_phones(s: &Session, f: &PersonForm) -> Dialog {
    let a = &f.all;
    let mut form = match s.contact_by_address(&a.email) {
        Some(card) => crate::pim_dialogs::ContactForm::edit(card),
        None => crate::pim_dialogs::ContactForm::from_address(&EmailAddress::new(&a.name, &a.email)),
    };
    // Business, then Mobile, then Other, then Home.
    for phone in &a.found.phones {
        match [0, 2, 4, 1].into_iter().find(|i| form.phones.get(*i).is_some_and(|p| p.trim().is_empty())).and_then(|i| form.phones.get_mut(i)) {
            Some(place) => phone.clone_into(place),
            None => form.more_phones.push(mailcraft_engine::model::vcard::Typed { value: phone.clone(), kind: String::new() }),
        }
    }
    Dialog::Contact(Box::new(form))
}

/// A row of a part: what it is (cut to the width) and, at its end, where, when or how much; a
/// click on it is what it does.
fn row(ui: &mut Ui, t: &Tokens, main: &str, strong: bool, side: &str, hint: &str) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    let spoken = if side.is_empty() { main.to_string() } else { format!("{main}, {side}") };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &spoken));
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    let m = crate::bidi::Mirror::new(rect);
    let side_g = ui.painter().layout_no_wrap(side.to_string(), theme::regular(12.0), t.dim);
    let main_w = (w - side_g.size().x - 24.0).max(40.0);
    let font = if strong { theme::bold(13.0) } else { theme::regular(13.0) };
    let g = widgets::one_line(ui, main, font, t.text, main_w);
    let y = |h: f32| rect.center().y - h / 2.0;
    ui.painter().galley(egui::pos2(m.start(rect.left() + 6.0, g.size().x), y(g.size().y)), g, t.text);
    let side_w = side_g.size().x;
    ui.painter().galley(egui::pos2(m.start(rect.right() - 6.0 - side_w, side_w), y(side_g.size().y)), side_g, t.dim);
    resp.on_hover_text(hint).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A few of each part, under a heading that shows all of it.
fn overview(ui: &mut Ui, f: &mut PersonForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    let mut empty = true;
    let mut go = None;
    for tab in TABS.into_iter().skip(1) {
        let a = &f.shown;
        let has = match tab {
            PersonTab::Overview => false,
            PersonTab::Mail => !a.threads.is_empty(),
            PersonTab::Files => !a.files.is_empty(),
            PersonTab::Meetings => !a.meetings.is_empty(),
            PersonTab::Links => !a.links.is_empty(),
            PersonTab::People => !a.people.is_empty(),
        };
        if !has {
            continue;
        }
        if !empty {
            ui.add_space(8.0);
        }
        empty = false;
        let heading = ui.add(egui::Label::new(RichText::new(label(tab, a)).font(theme::bold(14.0)).color(t.accent)).sense(Sense::click()));
        if heading.on_hover_text(tl!("Show all")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            go = Some(tab);
        }
        ui.push_id(("overview", tab as u8), |ui| match tab {
            PersonTab::Overview => true,
            PersonTab::Mail => mail(ui, a, s, t, acts, OVERVIEW_ROWS, false),
            PersonTab::Files => files(ui, a, s, t, acts, OVERVIEW_ROWS),
            PersonTab::Meetings => meetings(ui, a, s, t, acts, OVERVIEW_ROWS),
            PersonTab::Links => links(ui, a, s, t, acts, OVERVIEW_ROWS),
            PersonTab::People => people(ui, a, t, acts, OVERVIEW_ROWS),
        });
    }
    if let Some(tab) = go {
        f.tab = tab;
    }
    empty
}

/// Where a message is, and when: its folder's name, as the folder pane shows it.
fn where_and_when(s: &Session, key: &mailcraft_engine::model::MessageKey, when: i64) -> String {
    let date = crate::i18n::date(&dates::list_time(when, &s.clock));
    match s.folder(&key.account, &key.folder) {
        Some(folder) => format!("{} · {date}", crate::i18n::folder(folder)),
        None => date,
    }
}

/// The mail by conversation (each one's messages under it, `messages`): a click on a message
/// opens it in a window of its own (the main window is under this dialog).
fn mail(ui: &mut Ui, a: &PersonActivity, s: &Session, t: &Tokens, acts: &mut Vec<Action>, limit: usize, messages: bool) -> bool {
    for (i, th) in a.threads.iter().take(limit).enumerate() {
        if i > 0 && messages {
            ui.add_space(4.0);
        }
        let subject = if th.subject.trim().is_empty() { tl!("(no subject)").to_string() } else { th.subject.clone() };
        ui.push_id(("thread", i), |ui| {
            if let Some(newest) = th.messages.first() {
                let side = if messages && th.messages.len() > 1 {
                    tf!("{n} messages", n = th.messages.len())
                } else {
                    where_and_when(s, &newest.key, newest.when())
                };
                if row(ui, t, &subject, messages, &side, tl!("Open the newest")).clicked() {
                    acts.push(Action::Run("message.popOut", json!({"key": newest.key.to_id()})));
                }
            }
            if !messages {
                return;
            }
            for (j, m) in th.messages.iter().take(MESSAGES_SHOWN).enumerate() {
                let who = if m.from.display().trim().is_empty() { m.from.email.clone() } else { m.from.display().to_string() };
                let side = where_and_when(s, &m.key, m.when());
                ui.push_id(j, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        if row(ui, t, &who, false, &side, tl!("Open in new window")).clicked() {
                            acts.push(Action::Run("message.popOut", json!({"key": m.key.to_id()})));
                        }
                    });
                });
            }
            if th.messages.len() > MESSAGES_SHOWN {
                ui.label(RichText::new(tf!("and {n} more", n = th.messages.len() - MESSAGES_SHOWN)).color(t.dim).small());
            }
        });
    }
    a.threads.is_empty()
}

/// The files: a click opens one, its message in a window of its own beside it.
fn files(ui: &mut Ui, a: &PersonActivity, s: &Session, t: &Tokens, acts: &mut Vec<Action>, limit: usize) -> bool {
    for (i, file) in a.files.iter().take(limit).enumerate() {
        let side = format!("{} · {}", widgets::human_size(file.size), crate::i18n::date(&dates::list_time(file.date, &s.clock)));
        let hint = if file.from_them { tl!("Open (they sent it)") } else { tl!("Open") };
        if ui.push_id(("file", i), |ui| row(ui, t, &file.name, false, &side, hint)).inner.clicked() {
            acts.push(Action::Run("message.popOut", json!({"key": file.key.to_id()})));
            acts.push(Action::InWindow(file.key.clone(), Box::new(Action::OpenAttachment(file.index))));
        }
    }
    a.files.is_empty()
}

/// The meetings they organise or are asked to: a click opens one.
fn meetings(ui: &mut Ui, a: &PersonActivity, s: &Session, t: &Tokens, acts: &mut Vec<Action>, limit: usize) -> bool {
    for (i, m) in a.meetings.iter().take(limit).enumerate() {
        let when = if m.all_day { dates::day_text(m.start.div_euclid(86_400), &s.clock) } else { dates::full_time(m.start, &s.clock) };
        let summary = if m.summary.trim().is_empty() { tl!("(no subject)").to_string() } else { m.summary.clone() };
        let hint = if m.theirs { tl!("They organise it") } else { tl!("They're invited") };
        if ui.push_id(("meeting", i), |ui| row(ui, t, &summary, false, &crate::i18n::date(&when), hint)).inner.clicked()
            && let Some(d) = Dialog::open_item(s, &m.uid, m.start)
        {
            acts.push(Action::Dialog(d));
        }
    }
    a.meetings.is_empty()
}

/// The web addresses in the mail: a click opens one in the browser.
fn links(ui: &mut Ui, a: &PersonActivity, s: &Session, t: &Tokens, acts: &mut Vec<Action>, limit: usize) -> bool {
    for (i, l) in a.links.iter().take(limit).enumerate() {
        let when = crate::i18n::date(&dates::list_time(l.date, &s.clock));
        if ui.push_id(("link", i), |ui| row(ui, t, &l.url, false, &when, &l.url)).inner.clicked() {
            acts.push(Action::OpenUrl(l.url.clone()));
        }
    }
    a.links.is_empty()
}

/// The others on the mail with them: a click shows everything with that one.
fn people(ui: &mut Ui, a: &PersonActivity, t: &Tokens, acts: &mut Vec<Action>, limit: usize) -> bool {
    for (i, p) in a.people.iter().take(limit).enumerate() {
        let name = if p.name.is_empty() { p.email.clone() } else { format!("{} <{}>", p.name, p.email) };
        let side = if p.count == 1 { tl!("One message").to_string() } else { tf!("{n} messages", n = p.count) };
        if ui.push_id(("person", i), |ui| row(ui, t, &name, false, &side, tl!("Everything with them"))).inner.clicked() {
            acts.push(Action::Person(p.email.clone()));
        }
    }
    a.people.is_empty()
}
