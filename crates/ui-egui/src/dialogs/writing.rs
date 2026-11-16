//! Dialogs: while writing (Address Book, saving changes, no subject, word count).

use super::*;
use crate::icons::Icon;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::SelectNames(f) => select_names(ui, f, t, acts),
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
    title(ui, t, "Select Names");
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
        egui::ComboBox::from_id_salt("address-book")
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
        egui::Grid::new("names-head").num_columns(2).min_col_width(250.0).show(ui, |ui| {
            ui.label(RichText::new(tl!("Name")).color(t.dim).small());
            ui.label(RichText::new(tl!("Email Address")).color(t.dim).small());
        });
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
                    let name_w = 244.0;
                    let cut_name = cell(ui, &name, 6.0, name_w, t.text);
                    let cut_email = cell(ui, &a.email, 256.0, (rect.width() - 262.0).max(40.0), t.dim);
                    if cut_name || cut_email { r.on_hover_text(format!("{name} <{}>", a.email)) } else { r }
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
    for (field, label) in [(Recipients::To, "To ->"), (Recipients::Cc, "Cc ->"), (Recipients::Bcc, "Bcc ->")] {
        ui.horizontal(|ui| {
            if ui.add_sized(vec2(72.0, 24.0), egui::Button::new(tl!(label))).clicked() {
                add = Some(field);
            }
            ui.add(egui::TextEdit::singleline(field.of(&mut f.to, &mut f.cc, &mut f.bcc)).desired_width(f32::INFINITY));
        });
    }
    if let (Some(target), Some(a)) = (add, f.selected.and_then(|i| f.list.get(i)).cloned()) {
        add_to(target.of(&mut f.to, &mut f.cc, &mut f.bcc), &a);
    }
    let (ok, cancel) = buttons(ui, t, "OK", true);
    if ok {
        acts.push(Action::Submit("compose.update", json!({"to": f.to.trim(), "cc": f.cc.trim(), "bcc": f.bcc.trim()})));
        return true;
    }
    !cancel
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
