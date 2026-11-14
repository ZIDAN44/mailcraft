//! Dialogs: accounts (adding one, its settings, its password, removing it), and closing MailCraft.

use super::*;

mod add;
mod edit;

pub use add::AccountForm;
pub use edit::EditForm;
pub(super) use edit::edit_account;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::AddAccount(f) if f.calendar_only => super::dav::add_calendar_only(ui, f, t, acts),
        Dialog::AddAccount(f) => add::add_account(ui, f, s, t, acts),
        Dialog::Password { account, email, password } => {
            ui.set_width(380.0);
            title(ui, t, "Sign in again");
            ui.label(RichText::new(tf!("Enter the password for {email}.", email = email)).color(t.dim));
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
        Dialog::EditAccount(f) => edit_account(ui, f, t, acts),
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

/// The outgoing server's own sign-in, as Outlook's "Log on using" for SMTP: for servers whose
/// user name and password aren't the incoming server's (issue #3).
#[derive(Clone, Debug, Default)]
pub struct SmtpSignIn {
    pub own: bool,
    pub username: String,
    pub password: String,
    /// Its password is kept already (an account being changed): left empty, it stays.
    pub kept: bool,
}

impl SmtpSignIn {
    pub(super) fn of(c: &AccountConfig) -> SmtpSignIn {
        SmtpSignIn {
            own: c.smtp_username.is_some(),
            username: c.smtp_username.clone().unwrap_or_default(),
            password: String::new(),
            kept: c.smtp_username.is_some(),
        }
    }

    /// `smtpUsername` (null: the incoming server's sign-in) and `smtpPassword`.
    pub(super) fn params(&self, p: &mut serde_json::Value) {
        if self.own {
            p["smtpUsername"] = json!(self.username.trim());
            if !self.password.is_empty() {
                p["smtpPassword"] = json!(self.password);
            }
        } else {
            p["smtpUsername"] = serde_json::Value::Null;
        }
    }

    pub(super) fn ui(&mut self, ui: &mut Ui, w: f32) {
        ui.checkbox(&mut self.own, tl!("The outgoing server has its own user name and password"));
        if self.own {
            ui.horizontal(|ui| {
                let half = ((w - 30.0) / 2.0).max(100.0);
                ui.add_space(22.0);
                ui.add_sized([half, 26.0], egui::TextEdit::singleline(&mut self.username).hint_text(tl!("Its user name")));
                let hint = if self.kept { tl!("Password (unchanged when empty)") } else { tl!("Its password") };
                ui.add_sized([half, 26.0], egui::TextEdit::singleline(&mut self.password).password(true).hint_text(hint));
            });
        }
    }
}

/// When a POP server's copy of downloaded mail goes: kept (for so many days, or until it's deleted
/// here for good), or removed at once.
#[derive(Clone, Debug)]
pub struct PopKeep {
    pub keep: bool,
    pub for_days: bool,
    pub days: String,
    pub until_deleted: bool,
}

impl Default for PopKeep {
    /// A copy stays on the server for 14 days.
    fn default() -> Self {
        PopKeep { keep: true, for_days: true, days: "14".into(), until_deleted: false }
    }
}

impl PopKeep {
    pub(super) fn of(c: &AccountConfig) -> PopKeep {
        PopKeep {
            keep: !c.pop_delete,
            for_days: c.pop_keep_days.is_some(),
            days: c.pop_keep_days.unwrap_or(14).to_string(),
            until_deleted: c.pop_delete_with_trash,
        }
    }

    /// `popDelete`, `popKeepDays` (0: for good) and `popDeleteWithTrash`.
    pub(super) fn params(&self, p: &mut serde_json::Value) {
        let days = if self.keep && self.for_days { self.days.trim().parse::<u32>().ok().filter(|d| *d > 0).unwrap_or(14) } else { 0 };
        p["popDelete"] = json!(!self.keep);
        p["popKeepDays"] = json!(days);
        p["popDeleteWithTrash"] = json!(self.keep && self.until_deleted);
    }

    pub(super) fn ui(&mut self, ui: &mut Ui) {
        ui.checkbox(&mut self.keep, tl!("Keep a copy of downloaded mail on the server"));
        ui.add_enabled_ui(self.keep, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ui.checkbox(&mut self.for_days, tl!("Remove it from the server after"));
                ui.add_enabled(self.for_days, egui::TextEdit::singleline(&mut self.days).desired_width(36.0).align(egui::Align2::CENTER_TOP));
                ui.label(tl!("days"));
            });
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ui.checkbox(&mut self.until_deleted, tl!("Remove it from the server once it's deleted from Deleted Items"));
            });
        });
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
        "Its mail stays on the server and is held in memory only while MailCraft runs: none of it is written to this computer, and each message downloads when you open it. Mail you send goes straight to the server; only mail that can't go yet (while offline) waits, and it's lost with changes made offline if MailCraft closes first (you're asked). Calendars, contacts and notes on a server stay there; those kept only in MailCraft are lost when it closes."
    )
}

pub(super) fn security_combo(ui: &mut Ui, id: &str, v: &mut Security) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(match v {
            Security::Tls => "SSL/TLS",
            Security::StartTls => "STARTTLS",
            Security::None => tl!("None (unsafe)"),
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(v, Security::Tls, "SSL/TLS");
            ui.selectable_value(v, Security::StartTls, "STARTTLS");
            ui.selectable_value(v, Security::None, tl!("None (unsafe)"));
        });
}
