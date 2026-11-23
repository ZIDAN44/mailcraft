//! Dialogs: accounts (adding one, its settings, its password, removing it, its other addresses,
//! its calendars and contacts given by hand, its server's certificate), and the rows the account
//! dialogs share.

use super::*;

mod adding;
pub(super) mod addresses;
pub(super) mod dav;
mod editing;
mod forms;
pub(super) mod trust;

use adding::add_account;
use editing::edit_account;
pub use forms::*;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::AddAccount(f) if f.calendar_only => super::dav::add_calendar_only(ui, f, t, acts),
        Dialog::AddAccount(f) => add_account(ui, f, s, t, acts),
        Dialog::Password { account, email, user, password } => {
            ui.set_width(380.0);
            title(ui, t, "Sign in again");
            // An account that signs in with a user name of its own says so: asked for "the
            // password for" its address, it read as signing in with the address (issue #88).
            let about = if user.is_empty() {
                tf!("Enter the password for {email}.", email = email)
            } else {
                tf!("Enter the password for {email}, which signs in as {user}.", email = email, user = user)
            };
            ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
            if let Some(a) = s.account(account).filter(|a| mailcraft_engine::model::google_mail(&a.config.email, &a.config.imap.host)) {
                let refused = matches!(&a.status, mailcraft_engine::model::SyncStatus::NeedsPassword { error } if error == mailcraft_engine::model::GOOGLE_APP_PASSWORD_NEEDED);
                google_app_password(ui, t, acts, refused);
            }
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(password).password(true).desired_width(f32::INFINITY));
            // Before the focus request below, which would hide the Enter that ended input.
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            // The one thing to type: ready for it.
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            let (ok, cancel) = buttons(ui, t, "Sign in", !password.is_empty());
            if (ok || enter) && !password.is_empty() {
                acts.push(Action::Run("account.setPassword", json!({"account": account, "password": password})));
                return false;
            }
            !cancel
        }
        Dialog::RemoveAccount { account, email } => {
            ui.set_width(380.0);
            title(ui, t, "Remove account?");
            // As Outlook removes one: its data file stays, to open again from Open & Export.
            let file = s.store.as_ref().and_then(|st| st.data_file(account)).filter(|f| f.exists());
            // A mailbox with no server (an opened data file) has nothing there to keep.
            let local = s.account(account).is_some_and(|a| a.is_local());
            let text = match file.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()) {
                Some(file) if local => tf!(
                    "{email} will be removed from MailCraft. Its data file, {file}, stays in your MailCraft Files folder: open it from File › Open & Export to see its mail again.",
                    email = email,
                    file = file
                ),
                Some(file) => tf!(
                    "{email} will be removed from MailCraft. Its data file, {file}, stays in your MailCraft Files folder: open it from File › Open & Export to see its mail again. Nothing is deleted on the server.",
                    email = email,
                    file = file
                ),
                None if local => tf!("{email} will be removed from MailCraft.", email = email),
                None => tf!("{email} will be removed from MailCraft. Nothing is deleted on the server.", email = email),
            };
            ui.add(egui::Label::new(RichText::new(text).color(t.dim)).wrap());
            let (ok, cancel) = buttons(ui, t, "Remove", true);
            if ok {
                acts.push(Action::Run("account.remove", json!({"account": account})));
                return false;
            }
            !cancel
        }
        Dialog::MailboxClosed { account, email, error } => {
            ui.set_width(440.0);
            title(ui, t, "The mailbox won't open");
            ui.label(RichText::new(email.as_str()).strong());
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(tl!(error.as_str())).color(t.dim)).wrap());
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(
                    RichText::new(tl!(
                        "Microsoft Graph reaches the mailbox without IMAP, and brings its calendar and contacts too. The account starts again through it: its mail comes down from the server."
                    ))
                    .color(t.faint)
                    .small(),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            if ui.link(RichText::new(tl!("Turned IMAP on, or picked another account in the browser? Sign in again")).small()).clicked() {
                acts.push(Action::Run("account.signIn", json!({"account": account})));
                return false;
            }
            let (ok, cancel) = buttons(ui, t, "Use Microsoft Graph", true);
            if ok {
                acts.push(Action::Run("account.useGraph", json!({"account": account})));
                return false;
            }
            !cancel
        }
        Dialog::EditAccount(f) => edit_account(ui, f, s, t, acts),
        Dialog::AddSharedMailbox { account, address, error } => {
            ui.set_width(420.0);
            title(ui, t, "Add a shared mailbox");
            ui.add(
                egui::Label::new(
                    RichText::new(tl!("Its folders show under this account. You need access to it on the Exchange server; the ones you have full access to open by themselves."))
                        .color(t.dim),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(address).hint_text("sales@example.com").desired_width(f32::INFINITY));
            // Before the focus request below, which would hide the Enter that ended input.
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            if let Some(e) = error {
                ui.add_space(4.0);
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let ready = address.contains('@');
            let (ok, cancel) = buttons(ui, t, "Add", ready);
            if (ok || enter) && ready {
                acts.push(Action::Submit("account.addSharedMailbox", json!({"account": account, "address": address.trim()})));
            }
            !cancel
        }
        // Every other kind is shown by its own group (`body`).
        _ => true,
    }
}

/// A field's name, small and strong above it.
fn field_label(ui: &mut Ui, s: &str) {
    ui.label(RichText::new(tl!(s)).strong().small());
}

/// A part of a dialog that opens and closes (`open`), its name after a chevron; closed, `note`
/// says under it what's in it, in brief. Not a checkbox, which reads as turning something on.
fn disclosure(ui: &mut Ui, t: &Tokens, open: &mut bool, label: &str, note: Option<&str>) {
    let label = tl!(label);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    widgets::named(ui, &resp, label, true, Some(*open));
    if resp.clicked() {
        *open = !*open;
    }
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover_row);
    }
    // Laid out from the left, mirrored in a right-to-left interface.
    let m = crate::bidi::Mirror::new(rect);
    let icon = m.rect(egui::Rect::from_center_size(egui::pos2(rect.left() + 10.0, rect.center().y), vec2(14.0, 14.0)));
    crate::icons::paint(ui.painter(), icon, if *open { Icon::ChevronDown } else { m.closed() }, t.dim);
    let name = widgets::one_line(ui, label, theme::bold(13.0), t.text, rect.width() - 28.0);
    let at = m.rect(egui::Rect::from_min_size(egui::pos2(rect.left() + 22.0, rect.center().y - name.size().y / 2.0), name.size()));
    ui.painter().galley(at.min, name, t.text);
    // Lined up with the name, the chevron's room left before it.
    if let Some(note) = note.filter(|_| !*open) {
        // From the side the text starts on (egui's `horizontal` here runs from the left even in
        // Arabic or Hebrew), one line cut short with … past the room.
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 18.0), crate::bidi::row_from_start(egui::Align::Center), |ui| {
            ui.add_space(22.0);
            ui.add(egui::Label::new(RichText::new(note).color(t.faint).small()).truncate());
        });
    }
}

/// What's in an opened part, lined up under its name: set in from the side the text starts on.
fn indented<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(crate::bidi::row_from_start(egui::Align::Min), |ui| {
        ui.add_space(22.0);
        ui.with_layout(crate::bidi::column(), add).inner
    })
    .inner
}

/// A line under a heading saying what it is, faint and wrapped.
fn about(ui: &mut Ui, t: &Tokens, s: &str) {
    ui.add(egui::Label::new(RichText::new(s).color(t.dim).small()).wrap());
}

/// Which server a row of the account dialogs is for.
#[derive(Clone, Copy)]
enum Server {
    Imap,
    Pop,
    Smtp,
}

impl Server {
    /// Its name above the row, its host and port hints, and its security box's id.
    fn says(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Server::Imap => ("Incoming mail (IMAP)", "imap.example.com", "993", "imapsec"),
            Server::Pop => ("Incoming mail (POP)", "pop.example.com", "995", "popsec"),
            Server::Smtp => ("Outgoing mail (SMTP)", "smtp.example.com", "465", "smtpsec"),
        }
    }
}

/// A server's name, then its host, port and security on one row, in a dialog `w` wide (`id`
/// sets its security box apart from the other dialog's). True when the host or port is edited.
/// `plain`: no encryption may be chosen (not when the organisation requires it, issue #245).
#[allow(clippy::too_many_arguments)]
fn server_row(ui: &mut Ui, w: f32, server: Server, id: &str, host: &mut String, port: &mut String, security: &mut Security, plain: bool) -> bool {
    let (name, host_hint, port_hint, security_id) = server.says();
    // Fixed sizes: text boxes inside a Grid can't grow past last frame's column width.
    let host_w = (w - 196.0).max(140.0);
    field_label(ui, name);
    let mut edited = false;
    ui.horizontal(|ui| {
        edited |= ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(host).hint_text(host_hint)).changed();
        edited |= ui.add_sized([56.0, 26.0], egui::TextEdit::singleline(port).hint_text(port_hint).align(egui::Align2::CENTER_TOP)).changed();
        security_combo(ui, &format!("{id}{security_id}"), security, plain);
    });
    edited
}

/// Said under servers that don't encrypt: the password would cross the network readable.
fn clear_text_warning(ui: &mut Ui, t: &Tokens, incoming: Security, outgoing: Security) {
    if incoming == Security::None || outgoing == Security::None {
        ui.label(RichText::new(tl!("Without encryption your password travels in the clear. Use only for local test servers.")).color(t.warn).small());
    }
}

/// The User-Agent an Exchange account's requests name (issue #9): for a server that lets in only
/// the clients it knows. Said in place when it can't be used.
pub(super) fn user_agent_field(ui: &mut Ui, t: &Tokens, w: f32, agent: &mut String) {
    ui.add_space(4.0);
    ui.label(RichText::new(tl!("User-Agent (optional)")).strong().small());
    ui.add_sized([w, 26.0], egui::TextEdit::singleline(agent).hint_text(tl!("Left empty: MailCraft's own")).char_limit(300));
    let note = if !agent.trim().is_empty() && mailcraft_engine::model::user_agent(agent).is_none() {
        RichText::new(tl!("Only letters, digits, spaces and punctuation, up to 256 characters.")).color(t.danger)
    } else {
        RichText::new(tl!("Only if your organisation's server refuses MailCraft: the name of a mail program it lets in.")).color(t.faint)
    };
    ui.add(egui::Label::new(note.small()).wrap());
}

/// Online only (issue #4), as an account is added or changed: none of its mail on this computer.
pub(super) fn online_only(ui: &mut Ui, t: &Tokens, on: &mut bool) {
    ui.add_space(6.0);
    ui.checkbox(on, tl!("Online only: keep none of this account's mail on this computer"));
    if *on {
        ui.add(egui::Label::new(RichText::new(online_only_about()).color(t.faint).small()).wrap());
    }
}

/// What online only means, said where it's chosen.
pub(crate) fn online_only_about() -> &'static str {
    tl!(
        "Messages download when you open them and are held in memory only: nothing is written to this computer. Changes made while offline, mail still waiting to be sent, and calendars, contacts and notes kept only in MailCraft are lost when it closes (you're asked first)."
    )
}

pub(super) fn security_combo(ui: &mut Ui, id: &str, v: &mut Security, plain: bool) {
    let r = egui::ComboBox::from_id_salt(id)
        .selected_text(match v {
            Security::Tls => "SSL/TLS",
            Security::StartTls => "STARTTLS",
            Security::None => tl!("None (unsafe)"),
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(v, Security::Tls, "SSL/TLS");
            ui.selectable_value(v, Security::StartTls, "STARTTLS");
            if plain {
                ui.selectable_value(v, Security::None, tl!("None (unsafe)"));
            }
        })
        .response
        // Nothing beside it says what it is: the server row's name sits above the host.
        .accessible_name(tl!("Security"));
    let choices: &[Security] = if plain { &[Security::Tls, Security::StartTls, Security::None] } else { &[Security::Tls, Security::StartTls] };
    crate::drop_down::keys(ui, &r, v, choices);
}

/// Google takes an app password from mail apps while 2-Step Verification is on (issue #164):
/// said where a Google account's password is typed, as Google's refusal (`refused`) or before it,
/// with the page that makes one a click away.
pub(super) fn google_app_password(ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>, refused: bool) {
    ui.add_space(6.0);
    let (text, colour) = if refused {
        (tl!(mailcraft_engine::model::GOOGLE_APP_PASSWORD_NEEDED), t.danger)
    } else {
        (tl!("With 2-Step Verification on, Google takes an app password here, not your Google password."), t.faint)
    };
    ui.add(egui::Label::new(RichText::new(text).color(colour).small()).wrap());
    if ui.link(RichText::new(tl!("Make an app password")).color(t.accent).small()).clicked() {
        acts.push(Action::OpenUrl(mailcraft_engine::model::GOOGLE_APP_PASSWORDS.into()));
    }
}
