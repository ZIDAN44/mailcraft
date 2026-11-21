//! Dialogs: while writing (Address Book, Check Names, saving changes, no subject, word count).

use serde_json::Value;

use super::*;
use crate::icons::Icon;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::SelectNames(f) => select_names(ui, f, t, acts),
        Dialog::CheckNames(f) => check_names(ui, f, t, acts),
        Dialog::SaveChanges => {
            ui.set_width(380.0);
            title(ui, t, "Want to save your changes?");
            ui.label(RichText::new(tl!("The message is kept in Drafts if you save it.")).color(t.dim));
            ui.add_space(10.0);
            let mut keep = true;
            button_row(ui, |ui| {
                if ui.button(tl!("Cancel")).clicked() {
                    keep = false;
                }
                if ui.button(tl!("No")).clicked() {
                    acts.push(Action::Run("compose.close", json!({"save": false})));
                    keep = false;
                }
                if widgets::accent_button(ui, crate::icons::Icon::Drafts, "Yes", t, true).clicked() {
                    acts.push(Action::Run("compose.close", json!({"save": true})));
                    keep = false;
                }
            });
            keep
        }
        Dialog::EmptySubject => {
            ui.set_width(360.0);
            title(ui, t, "Send without a subject?");
            ui.label(RichText::new(tl!("Messages without a subject are easy to overlook.")).color(t.dim));
            let (ok, cancel) = buttons(ui, t, "Send anyway", true);
            if ok {
                acts.push(Action::Run("compose.send", json!({"allowEmptySubject": true})));
                return false;
            }
            !cancel
        }
        Dialog::CtrlEnter { remember } => {
            ui.set_width(380.0);
            title(ui, t, "Send this message?");
            ui.label(RichText::new(tl!("Ctrl+Enter sends the message you're writing.")).color(t.dim));
            ui.add_space(6.0);
            ui.checkbox(remember, tl!("Don't ask again"));
            ui.add_space(10.0);
            let (mut send, mut not) = (false, false);
            // Right to left: Send rightmost, as each dialog's main button.
            button_row(ui, |ui| {
                send = widgets::accent_button(ui, Icon::Send, "Send", t, true).clicked();
                not = ui.button(tl!("Don't send")).clicked();
            });
            if send || not {
                if *remember {
                    acts.push(Action::Run("options.ctrlEnter", json!({"does": if send { "send" } else { "nothing" }})));
                }
                if send {
                    // Sent as Send sends it: asked first about a missing subject.
                    let empty = s.compose.as_ref().is_some_and(|c| c.subject.trim().is_empty());
                    if empty && s.compose_send_block().is_none() {
                        acts.push(Action::Dialog(Dialog::EmptySubject));
                    } else {
                        acts.push(Action::Run("compose.send", json!({})));
                    }
                }
                return false;
            }
            true
        }
        Dialog::SendCheck(found) => {
            use mailcraft_engine::SendCheck;
            ui.set_width(440.0);
            title(ui, t, "Check before sending");
            let mut exposed = false;
            // What was found can't be shown (see `MailApp::run`): that there is something.
            let unread = found.is_empty().then(|| {
                tl!("Something about the recipients was found to look at, but it can't be shown. Look them over before sending.").to_string()
            });
            let lines = found.iter().map(|f| match f {
                SendCheck::Lookalike { address, like } => {
                    tf!("{address} looks like an address at {like}, but isn't one. Is it the right one?", address = address, like = like)
                }
                SendCheck::Exposed { count } => {
                    exposed = true;
                    tf!("{n} people who may not know each other will all see each other's addresses.", n = count)
                }
            });
            for line in unread.into_iter().chain(lines) {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                    crate::icons::paint(ui.painter(), r, crate::icons::Icon::Warning, t.warn);
                    ui.add(egui::Label::new(RichText::new(line).color(t.text)).wrap());
                });
                ui.add_space(4.0);
            }
            if exposed {
                ui.add_space(4.0);
                if ui
                    .button(tl!("Put everyone in Bcc"))
                    .on_hover_text(tl!("Each of them gets the message without seeing who else did; you're in To"))
                    .clicked()
                {
                    acts.push(Action::Run("compose.hideRecipients", json!({})));
                    return false;
                }
            }
            let (ok, cancel) = buttons(ui, t, "Send anyway", true);
            if ok {
                acts.push(Action::Run("compose.send", json!({"allowEmptySubject": true, "checked": true})));
                return false;
            }
            !cancel
        }
        Dialog::WordCount(n, selection) => {
            ui.set_width(320.0);
            title(ui, t, "Word count");
            if *selection {
                ui.label(RichText::new(tl!("The selected text")).color(t.dim).small());
                ui.add_space(4.0);
            }
            egui::Grid::new("word-count").num_columns(2).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                for (label, value) in [
                    (tl!("Words"), n.words),
                    (tl!("Characters (no spaces)"), n.chars_no_spaces),
                    (tl!("Characters (with spaces)"), n.chars),
                    (tl!("Paragraphs"), n.paragraphs),
                    (tl!("Lines"), n.lines),
                ] {
                    ui.label(RichText::new(label).color(t.dim));
                    ui.label(value.to_string());
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            close_row(ui)
        }
        // Every other kind is shown by its own group (`body`).
        _ => true,
    }
}

/// Check Names' question (issue #211): a name typed that could be several people, and who.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameChoice {
    /// The box it's in (`to`, `cc`, `bcc`), as typed, and the people it could be (name, address).
    pub field: String,
    pub typed: String,
    pub people: Vec<(String, String)>,
}

impl NameChoice {
    /// One of `compose.checkNames`' `several`.
    pub fn from_json(v: &Value) -> Option<NameChoice> {
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let people = v.get("people")?.as_array()?.iter().filter_map(|p| Some((s(p, "name").unwrap_or_default(), s(p, "email")?))).collect();
        Some(NameChoice { field: s(v, "field")?, typed: s(v, "typed")?, people })
    }
}

/// The names Check Names asks about, the first shown; which of its people is picked.
#[derive(Clone, Debug, Default)]
pub struct CheckNamesForm {
    pub asks: Vec<NameChoice>,
    pub picked: Option<usize>,
}

/// Check Names: the people a name typed could be, one picked (a double-click picks and goes on);
/// then the next name. Cancel leaves the rest as typed.
fn check_names(ui: &mut Ui, f: &mut CheckNamesForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    let Some(ask) = f.asks.first() else { return false };
    ui.set_width(440.0);
    title(ui, t, "Check Names");
    ui.add(
        egui::Label::new(RichText::new(tf!("More than one person could be “{typed}”. Pick the one you meant:", typed = ask.typed)).color(t.text))
            .wrap(),
    );
    ui.add_space(6.0);
    let mut go = false;
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(Margin::same(4)).show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
            // Rows the list's width, read from the start as the address book's are.
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                for (i, (name, email)) in ask.people.iter().enumerate() {
                    let label = if name.trim().is_empty() { email.clone() } else { format!("{name}  <{email}>") };
                    let r = ui.add(egui::Button::selectable(f.picked == Some(i), label).min_size(vec2(0.0, 24.0)));
                    if r.clicked() {
                        f.picked = Some(i);
                    }
                    if r.double_clicked() {
                        f.picked = Some(i);
                        go = true;
                    }
                }
            });
        });
    });
    let (ok, cancel) = buttons(ui, t, "OK", f.picked.is_some());
    if cancel {
        return false;
    }
    if (ok || go)
        && let Some((name, email)) = f.picked.and_then(|i| ask.people.get(i))
    {
        acts.push(Action::Run("compose.pickName", json!({"field": ask.field, "typed": ask.typed, "email": email, "name": name})));
        f.asks.remove(0);
        f.picked = None;
        return !f.asks.is_empty();
    }
    true
}

/// The address book being looked at.
#[derive(Clone, Debug, Default)]
pub struct NamesForm {
    pub query: String,
    /// The address book shown: `all` (everyone known), `people` (every contact) or one book
    /// (`book:<account>:<id>`); empty until the one it opens with is known.
    pub source: String,
    /// The one it opens with.
    pub default_source: String,
    /// Each account's address books: (source, name, the account's address).
    pub books: Vec<(String, String, String)>,
    pub list: Vec<mailcraft_engine::model::EmailAddress>,
    /// Each one's job title and department, as the directory gave them (issue #213), in the
    /// list's order: empty for one it didn't.
    pub details: Vec<(String, String)>,
    /// The list must be read again (the search or the book changed).
    pub stale: bool,
    /// An account has an organisation directory to search (issue #9).
    pub directory: bool,
    /// The directory answers the list was read with (the session's `directory.rev`).
    pub directory_rev: u64,
    /// A directory is still being asked about the search, and why one couldn't answer.
    pub searching: bool,
    pub directory_errors: Vec<String>,
    /// An Exchange Online account needs the user's permission to search its directory (issue #9).
    pub consent: bool,
    pub selected: Option<usize>,
    /// The field a double-click adds to.
    pub target: Recipients,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    /// What the names are picked for: a message (To, Cc, Bcc), or calendars (issue #140).
    pub purpose: NamesFor,
    /// A calendar group's name, as typed ([`NamesFor::Group`]).
    pub group: String,
    /// People are being added to the group already there, of that name (issue #209: its menu
    /// had only Delete): said so, its name not to be changed here.
    pub adding: bool,
    /// Why what was picked couldn't be done (a calendar's or group's command refused it).
    pub error: Option<String>,
}

/// What the address book's names are picked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NamesFor {
    /// The message being written: its To, Cc and Bcc.
    #[default]
    Message,
    /// People whose calendars open (Outlook's Open Calendar › From Address Book).
    Calendars,
    /// A new calendar group of them, or people added to one.
    Group,
}

/// A message's recipient fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Recipients {
    #[default]
    To,
    Cc,
    Bcc,
}

impl Recipients {
    /// Its name in the commands' parameters and the widgets' ids: `to`, `cc` or `bcc`.
    pub fn code(self) -> &'static str {
        match self {
            Recipients::To => "to",
            Recipients::Cc => "cc",
            Recipients::Bcc => "bcc",
        }
    }

    /// This field of the three given (To, Cc and Bcc, in that order).
    pub fn of<'a>(self, to: &'a mut String, cc: &'a mut String, bcc: &'a mut String) -> &'a mut String {
        match self {
            Recipients::To => to,
            Recipients::Cc => cc,
            Recipients::Bcc => bcc,
        }
    }
}

/// Add `a` to a recipient field (`a; b` as Outlook writes them), unless it's there: an address
/// in the field the same as its own, not one it's a part of (`joann@…`) or in a name.
pub(super) fn add_to(field: &mut String, a: &mailcraft_engine::model::EmailAddress) {
    let email = a.email.trim().to_lowercase();
    if email.is_empty() || mailcraft_engine::model::addr::parse_valid(field).iter().any(|x| x.email.to_lowercase() == email) {
        return;
    }
    let f = field.trim().trim_end_matches([';', ',']).trim().to_string();
    *field = if f.is_empty() { format!("{a}; ") } else { format!("{f}; {a}; ") };
}

pub(super) fn select_names(ui: &mut Ui, f: &mut NamesForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(620.0);
    title(
        ui,
        t,
        match f.purpose {
            NamesFor::Message => "Select Names",
            NamesFor::Calendars => "Open a shared calendar",
            NamesFor::Group if f.adding => "Add people to the group",
            NamesFor::Group => "New calendar group",
        },
    );
    if f.purpose == NamesFor::Group {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Group name")).strong());
            if f.adding {
                ui.label(RichText::new(&f.group).color(t.text));
            } else {
                ui.add(egui::TextEdit::singleline(&mut f.group).desired_width(240.0).hint_text(tl!("Design team")));
            }
        });
        ui.add_space(4.0);
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Search")).strong());
        let w = if f.directory { 186.0 } else { 240.0 };
        let r = ui.add(egui::TextEdit::singleline(&mut f.query).desired_width(w).hint_text(tl!("Name or address")));
        if ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        if r.changed() {
            f.stale = true;
            f.selected = None;
        }
        // The directory is asked as the search is typed; Enter or Go asks again now, whatever it
        // answered before, and for a search of one or two letters too (Outlook's Go).
        if f.directory {
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let go = ui.add_enabled(!f.query.trim().is_empty(), egui::Button::new("→")).on_hover_text(tl!("Search the directory now"));
            if (enter || go.clicked()) && !f.query.trim().is_empty() {
                acts.push(Action::Run("contact.searchDirectory", json!({"query": f.query.trim()})));
                f.stale = true;
            }
            if widgets::tiny_icon_button(ui, Icon::Sync, "Refresh: search the directory again and read the address books from the server", t)
                .clicked()
            {
                acts.push(Action::Run("contact.refreshDirectory", json!({})));
                f.stale = true;
            }
        }
        ui.add_space(12.0);
        let book = ui.label(RichText::new(tl!("Address Book")).strong());
        // Everyone (the people written to or heard from, and contacts), every contact, or one
        // address book (each account's Contacts, an organisation's shared ones).
        let accounts = f.books.iter().map(|(_, _, a)| a).collect::<std::collections::BTreeSet<_>>().len();
        let mut choices: Vec<(String, String, &'static str)> = vec![
            ("all".into(), tl!("All addresses").to_string(), tl!("Everyone you've written to or heard from, and your contacts")),
            ("people".into(), tl!("All contacts").to_string(), tl!("Every contact, in every address book")),
        ];
        if f.directory {
            choices.push((
                "directory".into(),
                tl!("Global Address List").to_string(),
                tl!("Everyone in your organisation's directory, asked of the server"),
            ));
        }
        if f.books.len() > 1 {
            for (source, name, account) in &f.books {
                let name = if name.is_empty() || name == "Contacts" { tl!("Contacts").to_string() } else { name.clone() };
                choices.push((source.clone(), if accounts > 1 { format!("{name} ({account})") } else { name }, ""));
            }
        }
        let shown = choices.iter().find(|(s, _, _)| *s == f.source).map_or_else(|| tl!("All addresses").to_string(), |(_, l, _)| l.clone());
        let r = egui::ComboBox::from_id_salt("address-book")
            .width(220.0)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (source, label, tip) in &choices {
                    let mut r = ui.selectable_label(f.source == *source, label);
                    if !tip.is_empty() {
                        r = r.on_hover_text(*tip);
                    }
                    if r.clicked() && f.source != *source {
                        f.source = source.clone();
                        f.stale = true;
                        f.selected = None;
                    }
                }
            })
            .response
            .labelled_by(book.id);
        let sources: Vec<String> = choices.iter().map(|(source, _, _)| source.clone()).collect();
        let mut at = f.source.clone();
        crate::drop_down::keys(ui, &r, &mut at, &sources);
        if at != f.source {
            f.source = at;
            f.stale = true;
            f.selected = None;
        }
    });
    // Outlook's "Show this address list first".
    if !f.source.is_empty() && f.source != f.default_source {
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() - 260.0);
            if ui.link(RichText::new(tl!("Always open with this address book")).small()).clicked() {
                acts.push(Action::Run("contact.defaultBook", json!({"source": f.source})));
                f.default_source = f.source.clone();
            }
        });
    }
    ui.add_space(6.0);
    let mut add: Option<Recipients> = None;
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4).show(ui, |ui| {
        ui.set_min_size(vec2(ui.available_width(), 230.0));
        // A job title and department where the directory gives any (issue #213), as Outlook's
        // Address Book has them: namesakes are told apart.
        let titled = f.details.iter().any(|(title, department)| !title.is_empty() || !department.is_empty());
        let cols: &[(&str, f32)] = if titled {
            &[("Name", 6.0), ("Job title", 176.0), ("Department", 312.0), ("Email Address", 432.0)]
        } else {
            &[("Name", 6.0), ("Email Address", 256.0)]
        };
        let head = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), egui::Sense::hover()).0;
        for (label, x) in cols {
            let g = ui.painter().layout_no_wrap(tl!(label).to_string(), egui::TextStyle::Small.resolve(ui.style()), t.dim);
            ui.painter().galley(egui::pos2(head.min.x + x, head.center().y - g.size().y / 2.0), g, t.dim);
        }
        ui.separator();
        // Only the rows in view are laid out: the list holds up to 2,000 people. A row is as tall
        // as `horizontal` makes it: its 22 points, or a control's height if that's more.
        let row_height = ui.spacing().interact_size.y.max(22.0);
        egui::ScrollArea::vertical().max_height(210.0).auto_shrink([false, false]).show_rows(ui, row_height, f.list.len(), |ui, range| {
            if f.list.is_empty() && !f.stale && !f.searching {
                let hint = if f.directory && f.source == "directory" && f.query.trim().is_empty() {
                    tl!("Type a name to search your organisation's directory.")
                } else {
                    tl!("No one found.")
                };
                ui.label(RichText::new(hint).color(t.faint));
            }
            for (i, a) in f.list.iter().enumerate().take(range.end).skip(range.start) {
                let on = f.selected == Some(i);
                // A row: name and address, left-aligned as in Outlook's list.
                let r = ui.horizontal(|ui| {
                    let name = if a.name.is_empty() { a.email.clone() } else { a.name.clone() };
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), egui::Sense::click());
                    if on || r.hovered() {
                        ui.painter().rect_filled(rect, 3.0, if on { t.accent_soft } else { t.hover_row });
                    }
                    // Each in its column, a long one cut short with "…" (the whole of it on hover).
                    let cell = |ui: &Ui, text: &str, x: f32, w: f32, c: egui::Color32| {
                        let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), theme::regular(13.5), c);
                        job.wrap.max_width = w;
                        job.wrap.max_rows = 1;
                        job.wrap.break_anywhere = true;
                        job.wrap.overflow_character = Some('…');
                        let g = ui.fonts_mut(|f| f.layout_job(job));
                        let cut = g.rows.first().is_some_and(|row| row.glyphs.len() < text.chars().count());
                        let pos = egui::pos2(rect.min.x + x, rect.center().y - g.size().y / 2.0);
                        ui.painter()
                            .with_clip_rect(Rect::from_min_size(egui::pos2(rect.min.x + x, rect.min.y), vec2(w + 2.0, rect.height())))
                            .galley(pos, g, c);
                        cut
                    };
                    let (name_w, email_x) = if titled { (164.0, 432.0) } else { (244.0, 256.0) };
                    let mut cut = cell(ui, &name, 6.0, name_w, t.text);
                    let (title, department) = f.details.get(i).map(|(t, d)| (t.as_str(), d.as_str())).unwrap_or_default();
                    if titled {
                        cut |= cell(ui, title, 176.0, 128.0, t.dim);
                        cut |= cell(ui, department, 312.0, 112.0, t.dim);
                    }
                    cut |= cell(ui, &a.email, email_x, (rect.width() - email_x - 6.0).max(40.0), t.dim);
                    let more = [title, department].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
                    if !cut {
                        r
                    } else if more.is_empty() {
                        r.on_hover_text(format!("{name} <{}>", a.email))
                    } else {
                        r.on_hover_text(format!("{name} <{}>\n{more}", a.email))
                    }
                });
                if r.inner.clicked() {
                    f.selected = Some(i);
                }
                if r.inner.double_clicked() {
                    f.selected = Some(i);
                    add = Some(f.target);
                }
            }
        });
    });
    // The directory: still being asked, or why it couldn't answer (what's here is shown meanwhile).
    if f.searching {
        ui.label(RichText::new(tl!("Searching your organisation's directory…")).color(t.dim).small());
    }
    if f.consent {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tl!("Your organisation's directory needs your permission to be searched from MailCraft.")).color(t.dim).small());
            if ui
                .button(tl!("Allow searching the directory…"))
                .on_hover_text(tl!("Microsoft asks in your browser; your mail's sign-in stays as it is"))
                .clicked()
            {
                acts.push(Action::Run("contact.allowDirectory", json!({})));
            }
        });
    }
    for e in f.directory_errors.iter().take(3) {
        ui.add(egui::Label::new(RichText::new(tf!("The directory couldn't be searched: {error}", error = e)).color(t.warn).small()).wrap());
    }
    ui.add_space(8.0);
    // A message's three fields; the people whose calendars open, or who are in the group.
    let fields: &[(Recipients, &str)] = match f.purpose {
        NamesFor::Message => &[(Recipients::To, "To ->"), (Recipients::Cc, "Cc ->"), (Recipients::Bcc, "Bcc ->")],
        NamesFor::Calendars | NamesFor::Group => &[(Recipients::To, "Add ->")],
    };
    for (field, label) in fields {
        ui.horizontal(|ui| {
            if ui.add_sized(vec2(72.0, 24.0), egui::Button::new(tl!(label))).clicked() {
                add = Some(*field);
            }
            if f.purpose == NamesFor::Message {
                ui.add(egui::TextEdit::singleline(field.of(&mut f.to, &mut f.cc, &mut f.bcc)).desired_width(f32::INFINITY));
            } else {
                crate::bidi::vertical(ui, |ui| people_field(ui, t, &mut f.to));
            }
        });
    }
    if let (Some(target), Some(a)) = (add, f.selected.and_then(|i| f.list.get(i)).cloned()) {
        add_to(target.of(&mut f.to, &mut f.cc, &mut f.bcc), &a);
    }
    if let Some(e) = &f.error {
        ui.label(RichText::new(e.as_str()).color(t.danger).small());
    }
    let ready = match f.purpose {
        NamesFor::Message => true,
        NamesFor::Calendars => f.to.contains('@'),
        NamesFor::Group => f.to.contains('@') && !f.group.trim().is_empty(),
    };
    let (ok, cancel) = buttons(ui, t, "OK", ready);
    if ok {
        match f.purpose {
            NamesFor::Message => acts.push(Action::Submit("compose.update", json!({"to": f.to.trim(), "cc": f.cc.trim(), "bcc": f.bcc.trim()}))),
            NamesFor::Calendars => acts.push(Action::Submit("calendar.openShared", json!({"email": f.to.trim()}))),
            NamesFor::Group => acts.push(Action::Submit("calendar.groupCreate", json!({"name": f.group.trim(), "members": f.to.trim()}))),
        }
        return true;
    }
    !cancel
}

/// The people picked to open the calendars of or put in a group (issue #210): each a chip with
/// their name (their address on hover, × takes them out), in rows that wrap and scroll past four,
/// and the box under them for who's still being typed (after the last `;`). The box kept one
/// size, so not everyone added could be seen, each as name and address.
fn people_field(ui: &mut Ui, t: &Tokens, to: &mut String) {
    let (done, typing) = match to.rfind(';') {
        Some(i) => (to.get(..i).unwrap_or(""), to.get(i + 1..).unwrap_or("")),
        None => ("", to.as_str()),
    };
    let mut people: Vec<mailcraft_engine::model::EmailAddress> = Vec::new();
    let mut unread: Vec<String> = Vec::new();
    // Read whole first: a name in quotes may hold a `;` of its own.
    let whole = mailcraft_engine::model::addr::parse_list(done).ok().filter(|l| l.iter().all(|a| a.email.contains('@')));
    let parts: Vec<&str> = if whole.is_some() { Vec::new() } else { done.split(';').map(str::trim).filter(|p| !p.is_empty()).collect() };
    people.extend(whole.into_iter().flatten());
    for part in parts {
        match mailcraft_engine::model::addr::parse_list(part) {
            Ok(list) if !list.is_empty() && list.iter().all(|a| a.email.contains('@')) => people.extend(list),
            _ => unread.push(part.to_string()),
        }
    }
    let mut typing = [unread.join("; "), typing.trim().to_string()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("; ");
    let mut remove = None;
    widgets::chip_grid(ui, ui.scope_id().with("people"), people.len(), 150.0, 240.0, 26.0, 4, |ui, i, w| {
        let Some(a) = people.get(i) else { return };
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
        let x_rect = Rect::from_center_size(egui::pos2(rect.right() - 13.0, rect.center().y), vec2(20.0, 20.0));
        let x = ui.interact(x_rect, resp.id.with("remove"), Sense::click()).on_hover_text(tl!("Remove"));
        let name = if a.name.trim().is_empty() { a.email.as_str() } else { a.name.trim() };
        widgets::named(ui, &x, &format!("{}: {name}", tl!("Remove")), true, None);
        let p = ui.painter();
        p.rect(
            rect,
            4.0,
            if resp.hovered() { t.hover_row } else { t.surface },
            egui::Stroke::new(1.0, t.faint.gamma_multiply(0.55)),
            egui::StrokeKind::Inside,
        );
        if x.hovered() {
            p.rect_filled(x_rect, 3.0, t.hover);
        }
        crate::icons::paint(p, crate::icons::centered(x_rect, 10.0), crate::icons::Icon::Close, if x.hovered() { t.text } else { t.dim });
        let g = widgets::one_line(ui, name, theme::regular(12.5), t.text, x_rect.left() - 4.0 - (rect.left() + 8.0));
        p.galley(egui::pos2(rect.left() + 8.0, rect.center().y - g.size().y / 2.0), g, t.text);
        resp.on_hover_text(a.email.clone());
        if x.clicked() {
            remove = Some(i);
        }
    });
    ui.add(egui::TextEdit::singleline(&mut typing).desired_width(f32::INFINITY));
    if let Some(i) = remove.filter(|i| *i < people.len()) {
        people.remove(i);
    }
    let picked: String = people.iter().map(|a| format!("{a}; ")).collect();
    *to = format!("{picked}{}", typing.trim_start());
}

/// Select Names adds someone whose address is part of another's, or of a name, in the field;
/// not someone already there, whatever the case.
#[cfg(test)]
#[test]
fn an_address_is_added_unless_it_is_there_whole() {
    let ann = mailcraft_engine::model::EmailAddress::new("Ann", "ann@x.example");
    let added = |field: &str| {
        let mut f = field.to_string();
        add_to(&mut f, &ann);
        f
    };
    assert_eq!(added(""), "Ann <ann@x.example>; ");
    assert_eq!(added("joann@x.example; "), "joann@x.example; Ann <ann@x.example>; ");
    assert_eq!(added("\"ann@x.example's desk\" <desk@x.example>"), "\"ann@x.example's desk\" <desk@x.example>; Ann <ann@x.example>; ");
    assert_eq!(added("bo@x.example, zo"), "bo@x.example, zo; Ann <ann@x.example>; ");
    for there in ["ann@x.example", "Ann Lee <ANN@x.example>; ", "bo@x.example; Ann@X.example; "] {
        assert_eq!(added(there), there);
    }
}
