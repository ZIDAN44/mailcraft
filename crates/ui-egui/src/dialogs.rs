//! Modal dialogs: add an account, sign in again, move to a folder, confirmations, settings.

use egui::{Margin, RichText, Ui, vec2};
use mailcraft_engine::Session;
use mailcraft_engine::model::organize::{Condition, Rule, RuleAction};
use mailcraft_engine::model::{AccountConfig, Folder, FolderRole, Security};
use serde_json::json;

use crate::theme::{self, Tokens};
use crate::{Action, widgets};

#[derive(Clone, Debug, Default)]
pub struct AccountForm {
    pub email: String,
    pub name: String,
    pub password: String,
    pub advanced: bool,
    /// Server fields follow the address until the user edits them.
    pub servers_edited: bool,
    pub username: String,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_security: Security,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_security: Security,
    /// Syncs over JMAP instead of IMAP and SMTP, through this session address.
    pub jmap: bool,
    pub jmap_url: String,
    pub error: Option<String>,
    /// The address whose looked-up settings are filled in, and where they came from.
    pub found_for: Option<String>,
    pub found_source: Option<String>,
}

impl AccountForm {
    fn guess(&mut self) {
        let g = AccountConfig::guess(&self.email);
        self.imap_host = g.imap.host;
        self.imap_port = g.imap.port.to_string();
        self.imap_security = g.imap.security;
        self.smtp_host = g.smtp.host;
        self.smtp_port = g.smtp.port.to_string();
        self.smtp_security = g.smtp.security;
    }

    pub(crate) fn params(&self) -> serde_json::Value {
        let port = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
        if self.jmap {
            return json!({
                "email": self.email.trim(),
                "name": self.name.trim(),
                "password": self.password,
                "username": if self.username.trim().is_empty() { self.email.trim() } else { self.username.trim() },
                "jmap": self.jmap_url.trim(),
            });
        }
        json!({
            "email": self.email.trim(),
            "name": self.name.trim(),
            "password": self.password,
            "username": if self.username.trim().is_empty() { self.email.trim() } else { self.username.trim() },
            "imapHost": self.imap_host.trim(),
            "imapPort": port(&self.imap_port),
            "imapSecurity": self.imap_security.code(),
            "smtpHost": self.smtp_host.trim(),
            "smtpPort": port(&self.smtp_port),
            "smtpSecurity": self.smtp_security.code(),
        })
    }
}

/// The server settings of an existing account.
#[derive(Clone, Debug, Default)]
pub struct EditForm {
    pub account: String,
    pub email: String,
    pub name: String,
    pub username: String,
    pub jmap: bool,
    pub jmap_url: String,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_security: Security,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_security: Security,
    pub error: Option<String>,
}

impl EditForm {
    pub(crate) fn params(&self) -> serde_json::Value {
        let port = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
        if self.jmap {
            return json!({"account": self.account, "name": self.name.trim(), "username": self.username.trim(), "jmap": self.jmap_url.trim()});
        }
        json!({
            "account": self.account,
            "name": self.name.trim(),
            "username": self.username.trim(),
            "jmap": null,
            "imapHost": self.imap_host.trim(),
            "imapPort": port(&self.imap_port),
            "imapSecurity": self.imap_security.code(),
            "smtpHost": self.smtp_host.trim(),
            "smtpPort": port(&self.smtp_port),
            "smtpSecurity": self.smtp_security.code(),
        })
    }
}

/// Automatic replies being edited.
#[derive(Clone, Debug, Default)]
pub struct VacationForm {
    pub account: String,
    pub enabled: bool,
    pub subject: String,
    pub text: String,
    /// Only between two days.
    pub dated: bool,
    pub from: String,
    pub until: String,
    pub error: Option<String>,
}

impl VacationForm {
    pub(crate) fn params(&self) -> serde_json::Value {
        let day = |d: &str| if self.dated && !d.trim().is_empty() { json!(d.trim()) } else { serde_json::Value::Null };
        json!({
            "account": self.account,
            "enabled": self.enabled,
            "subject": self.subject.trim(),
            "text": self.text,
            "from": day(&self.from),
            "until": day(&self.until),
        })
    }
}

fn edit_account(ui: &mut Ui, f: &mut EditForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(480.0);
    title(ui, t, "Server settings");
    ui.label(RichText::new(&f.email).color(t.dim));
    ui.add_space(8.0);
    let w = ui.available_width();
    let host_w = (w - 196.0).max(140.0);
    let label = |ui: &mut Ui, s: &str| {
        ui.label(RichText::new(tl!(s)).strong().small());
    };
    label(ui, "Your name");
    ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.name).hint_text(tl!("Shown on mail you send")));
    ui.add_space(4.0);
    label(ui, "User name");
    ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address")));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        label(ui, "Mail server");
        ui.radio_value(&mut f.jmap, false, tl!("IMAP and SMTP"));
        ui.radio_value(&mut f.jmap, true, tl!("JMAP"));
    });
    if f.jmap {
        label(ui, "JMAP session address");
        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.jmap_url).hint_text("https://api.example.com/jmap/session"));
    } else {
        label(ui, "Incoming mail (IMAP)");
        ui.horizontal(|ui| {
            ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(&mut f.imap_host).hint_text("imap.example.com"));
            ui.add_sized([56.0, 26.0], egui::TextEdit::singleline(&mut f.imap_port).hint_text("993").horizontal_align(egui::Align::Center));
            security_combo(ui, "edit-imapsec", &mut f.imap_security);
        });
        ui.add_space(4.0);
        label(ui, "Outgoing mail (SMTP)");
        ui.horizontal(|ui| {
            ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(&mut f.smtp_host).hint_text("smtp.example.com"));
            ui.add_sized([56.0, 26.0], egui::TextEdit::singleline(&mut f.smtp_port).hint_text("465").horizontal_align(egui::Align::Center));
            security_combo(ui, "edit-smtpsec", &mut f.smtp_security);
        });
        if f.imap_security == Security::None || f.smtp_security == Security::None {
            ui.label(
                RichText::new(tl!("Without encryption your password travels in the clear. Use only for local test servers.")).color(t.warn).small(),
            );
        }
    }
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    let (ok, cancel) = buttons(ui, t, "Save", true);
    if ok {
        f.error = None;
        acts.push(Action::Submit("account.update", f.params()));
    }
    !cancel
}

fn vacation(ui: &mut Ui, f: &mut VacationForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(520.0);
    title(ui, t, "Automatic replies");
    ui.label(
        RichText::new(tl!(
            "Each sender gets one reply a day while they're on. They're kept on your mail server, so they work while this computer is off."
        ))
        .color(t.dim)
        .small(),
    );
    ui.add_space(8.0);
    ui.radio_value(&mut f.enabled, false, tl!("Don't send automatic replies"));
    ui.radio_value(&mut f.enabled, true, tl!("Send automatic replies"));
    ui.add_enabled_ui(f.enabled, |ui| {
        ui.add_space(4.0);
        ui.checkbox(&mut f.dated, tl!("Only between these days"));
        if f.dated {
            ui.horizontal(|ui| {
                ui.label(tl!("From"));
                ui.add_sized([110.0, 26.0], egui::TextEdit::singleline(&mut f.from).hint_text("YYYY-MM-DD"));
                ui.label(tl!("Until"));
                ui.add_sized([110.0, 26.0], egui::TextEdit::singleline(&mut f.until).hint_text("YYYY-MM-DD"));
            });
        }
        ui.add_space(4.0);
        ui.add_sized([ui.available_width(), 26.0], egui::TextEdit::singleline(&mut f.subject).hint_text(tl!("Subject (optional)")));
        ui.add_space(4.0);
        ui.add(egui::TextEdit::multiline(&mut f.text).hint_text(tl!("I'm away until…")).desired_rows(6).desired_width(f32::INFINITY));
    });
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    let (ok, cancel) = buttons(ui, t, "Save", !f.enabled || !f.text.trim().is_empty());
    if ok {
        f.error = None;
        acts.push(Action::Submit("account.setVacation", f.params()));
    }
    !cancel
}

#[derive(Clone, Debug)]
pub enum Dialog {
    AddAccount(Box<AccountForm>),
    Password {
        account: String,
        email: String,
        password: String,
    },
    ConfirmDelete {
        count: usize,
    },
    MoveTo {
        filter: String,
    },
    EmptySubject,
    RemoveAccount {
        account: String,
        email: String,
    },
    Signature {
        account: String,
        text: String,
    },
    /// An account's name, user name and servers.
    EditAccount(Box<EditForm>),
    /// An account's automatic replies.
    Vacation(Box<VacationForm>),
    ConfirmEmpty {
        account: String,
        folder: String,
        name: String,
    },
    /// New folder (in `parent`), or rename `rename`.
    FolderName {
        account: String,
        parent: Option<String>,
        rename: Option<String>,
        name: String,
        error: Option<String>,
    },
    MoveFolder {
        account: String,
        folder: String,
        filter: String,
        error: Option<String>,
    },
    DeleteFolder {
        account: String,
        folder: String,
        name: String,
        permanent: bool,
        error: Option<String>,
    },
    NewCategory {
        name: String,
        color: u8,
        error: Option<String>,
    },
    Rules,
    EditRule(Box<RuleForm>),
    /// An attachment shown as text (a file, or an attached email's header and text).
    Preview {
        title: String,
        header: Vec<(String, String)>,
        text: String,
    },
}

/// What the rule editor offers: one test and one action (rules made elsewhere can have more).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleTest {
    From,
    To,
    Subject,
    AnyText,
    HasAttachment,
    Bulk,
}

impl RuleTest {
    const ALL: [RuleTest; 6] = [RuleTest::From, RuleTest::To, RuleTest::Subject, RuleTest::AnyText, RuleTest::HasAttachment, RuleTest::Bulk];
    fn label(self) -> &'static str {
        tl!(match self {
            RuleTest::From => "From contains",
            RuleTest::To => "To or Cc contains",
            RuleTest::Subject => "Subject contains",
            RuleTest::AnyText => "Sender, recipients or subject contain",
            RuleTest::HasAttachment => "Has attachments",
            RuleTest::Bulk => "Is a newsletter or list mail",
        })
    }
    fn needs_text(self) -> bool {
        !matches!(self, RuleTest::HasAttachment | RuleTest::Bulk)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleDo {
    MoveTo,
    Archive,
    Delete,
    Categorize,
    MarkRead,
    Flag,
    Pin,
}

impl RuleDo {
    const ALL: [RuleDo; 7] = [RuleDo::MoveTo, RuleDo::Archive, RuleDo::Delete, RuleDo::Categorize, RuleDo::MarkRead, RuleDo::Flag, RuleDo::Pin];
    fn label(self) -> &'static str {
        tl!(match self {
            RuleDo::MoveTo => "Move to folder",
            RuleDo::Archive => "Archive",
            RuleDo::Delete => "Delete",
            RuleDo::Categorize => "Categorize",
            RuleDo::MarkRead => "Mark as read",
            RuleDo::Flag => "Flag",
            RuleDo::Pin => "Pin",
        })
    }
}

#[derive(Clone, Debug)]
pub struct RuleForm {
    /// The rule being changed (`None`: a new one).
    pub index: Option<usize>,
    pub name: String,
    pub test: RuleTest,
    pub text: String,
    pub action: RuleDo,
    /// A folder path (`MoveTo`) or a category name (`Categorize`).
    pub target: String,
    /// The account whose folders `MoveTo` offers (and the only one it applies to).
    pub account: Option<String>,
    pub stop: bool,
    pub error: Option<String>,
}

impl RuleForm {
    fn to_rule(&self) -> Result<Rule, String> {
        if self.name.trim().is_empty() {
            return Err("Give the rule a name".into());
        }
        if self.test.needs_text() && self.text.trim().is_empty() {
            return Err("Say what to look for".into());
        }
        let t = self.text.trim().to_string();
        let condition = match self.test {
            RuleTest::From => Condition::From(t),
            RuleTest::To => Condition::To(t),
            RuleTest::Subject => Condition::Subject(t),
            RuleTest::AnyText => Condition::AnyText(t),
            RuleTest::HasAttachment => Condition::HasAttachment,
            RuleTest::Bulk => Condition::Bulk,
        };
        let action = match self.action {
            RuleDo::MoveTo if self.target.is_empty() => return Err("Pick a folder".into()),
            RuleDo::MoveTo => RuleAction::MoveTo(self.target.clone()),
            RuleDo::Archive => RuleAction::MoveToRole(FolderRole::Archive),
            RuleDo::Delete => RuleAction::Delete,
            RuleDo::Categorize if self.target.is_empty() => return Err("Pick a category".into()),
            RuleDo::Categorize => RuleAction::Categorize(self.target.clone()),
            RuleDo::MarkRead => RuleAction::MarkRead,
            RuleDo::Flag => RuleAction::Flag,
            RuleDo::Pin => RuleAction::Pin,
        };
        Ok(Rule {
            name: self.name.trim().to_string(),
            enabled: true,
            account: if self.action == RuleDo::MoveTo { self.account.clone() } else { None },
            conditions: vec![condition],
            match_all: true,
            actions: vec![action],
            stop: self.stop,
        })
    }

    fn from_rule(index: usize, r: &Rule) -> RuleForm {
        let (test, text) = match r.conditions.first() {
            Some(Condition::From(t)) => (RuleTest::From, t.clone()),
            Some(Condition::To(t)) => (RuleTest::To, t.clone()),
            Some(Condition::Subject(t)) => (RuleTest::Subject, t.clone()),
            Some(Condition::AnyText(t)) => (RuleTest::AnyText, t.clone()),
            Some(Condition::HasAttachment) => (RuleTest::HasAttachment, String::new()),
            Some(Condition::Bulk) | None => (RuleTest::Bulk, String::new()),
        };
        let (action, target) = match r.actions.first() {
            Some(RuleAction::MoveTo(f)) => (RuleDo::MoveTo, f.clone()),
            Some(RuleAction::MoveToRole(FolderRole::Trash)) | Some(RuleAction::Delete) => (RuleDo::Delete, String::new()),
            Some(RuleAction::MoveToRole(_)) => (RuleDo::Archive, String::new()),
            Some(RuleAction::Categorize(c)) => (RuleDo::Categorize, c.clone()),
            Some(RuleAction::MarkRead) => (RuleDo::MarkRead, String::new()),
            Some(RuleAction::Flag) => (RuleDo::Flag, String::new()),
            Some(RuleAction::Pin) | None => (RuleDo::Pin, String::new()),
        };
        RuleForm { index: Some(index), name: r.name.clone(), test, text, action, target, account: r.account.clone(), stop: r.stop, error: None }
    }
}

/// One line describing a rule.
fn describe(r: &Rule) -> String {
    let cond = r
        .conditions
        .iter()
        .map(|c| match c {
            Condition::From(t) => tf!("from “{text}”", text = t),
            Condition::To(t) => format!("to “{t}”"),
            Condition::Subject(t) => tf!("subject has “{text}”", text = t),
            Condition::AnyText(t) => tf!("mentions “{text}”", text = t),
            Condition::HasAttachment => "has attachments".into(),
            Condition::Bulk => "is list mail".into(),
        })
        .collect::<Vec<_>>()
        .join(if r.match_all { " and " } else { " or " });
    let act = r
        .actions
        .iter()
        .map(|a| match a {
            RuleAction::MoveTo(f) => tf!("move to {folder}", folder = mailcraft_engine::model::utf7::decode(f)),
            RuleAction::MoveToRole(role) => tf!("move to {folder}", folder = tl!(role.label().unwrap_or("folder"))),
            RuleAction::MarkRead => "mark read".into(),
            RuleAction::Flag => "flag".into(),
            RuleAction::Pin => "pin".into(),
            RuleAction::Categorize(c) => tf!("categorize {category}", category = c),
            RuleAction::Delete => "delete".into(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    tf!("If it {condition}: {action}", condition = cond, action = act)
}

impl Dialog {
    pub fn add_account() -> Dialog {
        Dialog::AddAccount(Box::default())
    }

    /// The add-account dialog with the address filled in (servers guessed from it).
    pub fn add_account_with(email: &str, advanced: bool) -> Dialog {
        let mut f = AccountForm { email: email.trim().to_string(), advanced, ..Default::default() };
        if !f.email.is_empty() {
            f.guess();
        }
        Dialog::AddAccount(Box::new(f))
    }

    pub fn move_to() -> Dialog {
        Dialog::MoveTo { filter: String::new() }
    }

    pub fn edit_account(a: &AccountConfig) -> Dialog {
        Dialog::EditAccount(Box::new(EditForm {
            account: a.id.clone(),
            email: a.email.clone(),
            name: a.name.clone(),
            username: a.username.clone(),
            jmap: a.jmap.is_some(),
            jmap_url: a.jmap.clone().unwrap_or_default(),
            imap_host: a.imap.host.clone(),
            imap_port: a.imap.port.to_string(),
            imap_security: a.imap.security,
            smtp_host: a.smtp.host.clone(),
            smtp_port: a.smtp.port.to_string(),
            smtp_security: a.smtp.security,
            error: None,
        }))
    }

    pub fn vacation(account: &str, v: &mailcraft_engine::model::job::Vacation) -> Dialog {
        Dialog::Vacation(Box::new(VacationForm {
            account: account.to_string(),
            enabled: v.enabled,
            subject: v.subject.clone(),
            text: v.text.clone(),
            dated: v.from.is_some() || v.until.is_some(),
            from: v.from.clone().unwrap_or_default(),
            until: v.until.clone().unwrap_or_default(),
            error: None,
        }))
    }

    pub fn password(a: &AccountConfig) -> Dialog {
        Dialog::Password { account: a.id.clone(), email: a.email.clone(), password: String::new() }
    }

    /// Set the error shown in the add-account form.
    pub fn set_error(&mut self, e: String) {
        match self {
            Dialog::AddAccount(f) => f.error = Some(e),
            Dialog::FolderName { error, .. }
            | Dialog::MoveFolder { error, .. }
            | Dialog::DeleteFolder { error, .. }
            | Dialog::NewCategory { error, .. } => *error = Some(e),
            Dialog::EditRule(f) => f.error = Some(e),
            Dialog::EditAccount(f) => f.error = Some(e),
            Dialog::Vacation(f) => f.error = Some(e),
            _ => {}
        }
    }

    /// A new rule for mail like this (from its sender, into Archive).
    pub fn rule_from(sender: &str, _subject: &str) -> Dialog {
        Dialog::EditRule(Box::new(RuleForm {
            index: None,
            name: tf!("Mail from {sender}", sender = sender),
            test: RuleTest::From,
            text: sender.to_string(),
            action: RuleDo::Archive,
            target: String::new(),
            account: None,
            stop: false,
            error: None,
        }))
    }

    /// The preview of an attachment, from `attachment.preview`'s answer.
    pub fn preview(v: &serde_json::Value, clock: &mailcraft_engine::model::dates::Clock) -> Dialog {
        let get = |k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or("").to_string();
        let mut header = Vec::new();
        if get("kind") == "message" {
            header.push((tl!("From").to_string(), get("from")));
            header.push((tl!("To").to_string(), get("to")));
            if let Some(d) = v.get("date").and_then(serde_json::Value::as_i64).filter(|d| *d > 0) {
                header.push((tl!("Date").to_string(), crate::i18n::date(&mailcraft_engine::model::dates::full_time(d, clock))));
            }
        }
        let mut text = get("text");
        if v.get("truncated").and_then(serde_json::Value::as_bool).unwrap_or(false) {
            text.push_str("\n… (the rest isn't shown: open the file to see it all)");
        }
        let title = if get("kind") == "message" && !get("subject").is_empty() { get("subject") } else { get("name") };
        Dialog::Preview { title, header, text }
    }

    pub fn new_folder(account: &str, parent: Option<&str>) -> Dialog {
        Dialog::FolderName { account: account.into(), parent: parent.map(str::to_string), rename: None, name: String::new(), error: None }
    }

    pub fn rename_folder(f: &Folder) -> Dialog {
        Dialog::FolderName { account: f.account.clone(), parent: None, rename: Some(f.path.clone()), name: f.display_name(), error: None }
    }

    pub fn move_folder(f: &Folder) -> Dialog {
        Dialog::MoveFolder { account: f.account.clone(), folder: f.path.clone(), filter: String::new(), error: None }
    }

    /// Delete a folder: into Deleted Items, or for good when it's already there.
    pub fn delete_folder(s: &Session, f: &Folder) -> Dialog {
        let trash = s.account(&f.account).and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Trash)).map(|t| t.path.clone());
        let permanent = trash.is_none_or(|t| mailcraft_engine::model::is_within(&f.path, &t, f.delimiter));
        Dialog::DeleteFolder { account: f.account.clone(), folder: f.path.clone(), name: f.display_name(), permanent, error: None }
    }
}

fn title(ui: &mut Ui, t: &Tokens, s: &str) {
    ui.label(RichText::new(tl!(s)).font(theme::bold(18.0)).color(t.text));
    ui.add_space(6.0);
}

fn security_combo(ui: &mut Ui, id: &str, v: &mut Security) {
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

fn buttons(ui: &mut Ui, t: &Tokens, ok: &str, ok_enabled: bool) -> (bool, bool) {
    let mut res = (false, false);
    ui.add_space(10.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::accent_button(ui, crate::icons::Icon::Send, ok, t, ok_enabled).clicked() {
            res.0 = true;
        }
        if ui.button(tl!("Cancel")).clicked() {
            res.1 = true;
        }
    });
    res
}

/// Show the dialog; returns whether it stays open.
pub fn show(ctx: &egui::Context, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    let frame = egui::Frame::new().fill(t.surface).corner_radius(12).inner_margin(Margin::same(22)).stroke(egui::Stroke::new(1.0, t.border));
    let modal = egui::Modal::new(egui::Id::new("dialog")).frame(frame).show(ctx, |ui| body(ui, d, s, t, acts));
    let keep = modal.inner;
    keep && !modal.should_close()
}

fn body(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::AddAccount(f) => {
            ui.set_width(440.0);
            title(ui, t, "Add an email account");
            ui.label(RichText::new(tl!("Works with any provider that offers IMAP and SMTP. Your password is kept in this computer's keychain and only ever sent to your mail server.")).color(t.dim).small());
            ui.add_space(10.0);
            egui::Grid::new("acct").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                ui.label(tl!("Email address"));
                let r = ui
                    .add(egui::TextEdit::singleline(&mut f.email).hint_text(tl!("you@example.com")).min_size(vec2(300.0, 26.0)).desired_width(300.0));
                if r.changed() && !f.servers_edited {
                    f.guess();
                    f.found_for = None;
                    f.found_source = None;
                }
                // Look the settings up once the address is typed (when the box is left).
                let email = f.email.trim().to_ascii_lowercase();
                if (r.lost_focus() || f.advanced) && mailcraft_engine::model::addr::is_valid_email(&email) && !s.discovered.contains_key(&email) {
                    acts.push(Action::Run("account.discover", json!({"email": email})));
                }
                if !f.servers_edited
                    && f.found_for.as_deref() != Some(email.as_str())
                    && let Some(Some((Some(c), source))) = s.discovered.get(&email)
                {
                    f.imap_host = c.imap.host.clone();
                    f.imap_port = c.imap.port.to_string();
                    f.imap_security = c.imap.security;
                    f.smtp_host = c.smtp.host.clone();
                    f.smtp_port = c.smtp.port.to_string();
                    f.smtp_security = c.smtp.security;
                    f.username = if c.username.eq_ignore_ascii_case(&email) { String::new() } else { c.username.clone() };
                    f.found_for = Some(email.clone());
                    f.found_source = Some(source.clone());
                }
                ui.end_row();
                ui.label(tl!("Your name"));
                ui.add(
                    egui::TextEdit::singleline(&mut f.name).hint_text(tl!("Shown on mail you send")).min_size(vec2(300.0, 26.0)).desired_width(300.0),
                );
                ui.end_row();
                ui.label(tl!("Password"));
                ui.add(egui::TextEdit::singleline(&mut f.password).password(true).min_size(vec2(300.0, 26.0)).desired_width(300.0));
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.checkbox(&mut f.advanced, tl!("Server settings"));
            if f.advanced {
                egui::Frame::new().fill(t.chrome).corner_radius(8).inner_margin(Margin::same(10)).show(ui, |ui| {
                    let mut edited = false;
                    let w = ui.available_width();
                    // Fixed sizes: text boxes inside a Grid can't grow past last frame's column width.
                    let host_w = (w - 196.0).max(140.0);
                    let label = |ui: &mut Ui, s: &str| {
                        ui.label(RichText::new(tl!(s)).strong().small());
                    };
                    ui.horizontal(|ui| {
                        label(ui, "Mail server");
                        ui.radio_value(&mut f.jmap, false, tl!("IMAP and SMTP"));
                        ui.radio_value(&mut f.jmap, true, tl!("JMAP"));
                    });
                    if f.jmap {
                        label(ui, "JMAP session address");
                        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.jmap_url).hint_text("https://api.example.com/jmap/session"));
                        ui.add_space(4.0);
                        label(ui, "User name");
                        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address")));
                        return;
                    }
                    label(ui, "Incoming mail (IMAP)");
                    ui.horizontal(|ui| {
                        edited |= ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(&mut f.imap_host).hint_text("imap.example.com")).changed();
                        edited |= ui
                            .add_sized(
                                [56.0, 26.0],
                                egui::TextEdit::singleline(&mut f.imap_port).hint_text("993").horizontal_align(egui::Align::Center),
                            )
                            .changed();
                        security_combo(ui, "imapsec", &mut f.imap_security);
                    });
                    ui.add_space(4.0);
                    label(ui, "Outgoing mail (SMTP)");
                    ui.horizontal(|ui| {
                        edited |= ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(&mut f.smtp_host).hint_text("smtp.example.com")).changed();
                        edited |= ui
                            .add_sized(
                                [56.0, 26.0],
                                egui::TextEdit::singleline(&mut f.smtp_port).hint_text("465").horizontal_align(egui::Align::Center),
                            )
                            .changed();
                        security_combo(ui, "smtpsec", &mut f.smtp_security);
                    });
                    ui.add_space(4.0);
                    label(ui, "User name");
                    edited |=
                        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address"))).changed();
                    f.servers_edited |= edited;
                    if f.imap_security == Security::None || f.smtp_security == Security::None {
                        ui.label(
                            RichText::new(tl!("Without encryption your password travels in the clear. Use only for local test servers."))
                                .color(t.warn)
                                .small(),
                        );
                    }
                });
            } else if f.jmap && !f.jmap_url.trim().is_empty() {
                ui.label(RichText::new(tf!("Server: {url} (JMAP)", url = f.jmap_url.trim())).color(t.faint).small());
            } else if !f.imap_host.is_empty() {
                ui.label(
                    RichText::new(tf!(
                        "Servers: {imap} (IMAP {imap_port}), {smtp} (SMTP {smtp_port})",
                        imap = f.imap_host,
                        imap_port = f.imap_port,
                        smtp = f.smtp_host,
                        smtp_port = f.smtp_port
                    ))
                    .color(t.faint)
                    .small(),
                );
            }
            let email = f.email.trim().to_ascii_lowercase();
            match (&f.found_source, s.discovered.get(&email)) {
                (Some(src), _) if f.found_for.as_deref() == Some(email.as_str()) => {
                    ui.label(RichText::new(tf!("Settings found: {source}", source = src)).color(t.ok).small());
                }
                (_, Some(None)) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(tl!("Looking up the server settings…")).color(t.faint).small());
                    });
                }
                (_, Some(Some((None, _)))) => {
                    ui.label(
                        RichText::new(tl!("No published settings: the usual server names are filled in. Check them under Server settings."))
                            .color(t.faint)
                            .small(),
                    );
                }
                _ => {}
            }
            if let Some(e) = &f.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(t.danger));
            }
            // Providers that sign in through the browser (OAuth), when this copy can.
            let provider = mailcraft_engine::model::oauth_provider_for(&f.imap_host).filter(|p| s.oauth_available.iter().any(|a| a == p));
            if let Some(p) = provider {
                ui.add_space(6.0);
                let name = if p == "google" { "Google" } else { "Microsoft" };
                ui.horizontal(|ui| {
                    if widgets::accent_button(
                        ui,
                        crate::icons::Icon::Person,
                        &tf!("Sign in with {provider}", provider = name),
                        t,
                        !f.email.trim().is_empty(),
                    )
                    .clicked()
                    {
                        f.error = None;
                        let mut params = f.params();
                        params["oauth"] = json!(p);
                        params["password"] = json!(null);
                        acts.push(Action::Submit("account.add", params));
                    }
                    ui.label(RichText::new(tl!("in your browser: no password stored")).color(t.faint).small());
                });
            }
            let ready = !f.email.trim().is_empty() && !f.password.is_empty() && (!f.jmap || !f.jmap_url.trim().is_empty());
            let (ok, cancel) = buttons(ui, t, "Connect", ready);
            if ok {
                if !f.jmap && f.imap_host.is_empty() {
                    f.guess();
                }
                f.error = None;
                acts.push(Action::Submit("account.add", f.params()));
            }
            !cancel
        }
        Dialog::Password { account, email, password } => {
            ui.set_width(380.0);
            title(ui, t, "Sign in again");
            ui.label(RichText::new(tf!("Enter the password for {email}.", email = email)).color(t.dim));
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(password).password(true).desired_width(f32::INFINITY));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let (ok, cancel) = buttons(ui, t, "Sign in", !password.is_empty());
            if (ok || enter) && !password.is_empty() {
                acts.push(Action::Run("account.setPassword", json!({"account": account, "password": password})));
                return false;
            }
            !cancel
        }
        Dialog::ConfirmDelete { count } => {
            ui.set_width(360.0);
            title(ui, t, "Delete permanently?");
            let what = if *count == 1 {
                tl!("This message will be deleted for good. This can't be undone.").to_string()
            } else {
                tf!("These {n} messages will be deleted for good. This can't be undone.", n = count)
            };
            ui.label(RichText::new(what).color(t.dim));
            let (ok, cancel) = buttons(ui, t, "Delete", true);
            if ok {
                acts.push(Action::Run("mail.delete", json!({"permanent": true})));
                return false;
            }
            !cancel
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
        Dialog::RemoveAccount { account, email } => {
            ui.set_width(380.0);
            title(ui, t, "Remove account?");
            ui.label(
                RichText::new(tf!(
                    "{email} and its downloaded mail will be removed from this computer. Nothing is deleted on the server.",
                    email = email
                ))
                .color(t.dim),
            );
            let (ok, cancel) = buttons(ui, t, "Remove", true);
            if ok {
                acts.push(Action::Run("account.remove", json!({"account": account})));
                return false;
            }
            !cancel
        }
        Dialog::Signature { account, text } => {
            ui.set_width(420.0);
            title(ui, t, "Signature");
            ui.label(RichText::new(tl!("Added to new messages, replies and forwards.")).color(t.dim).small());
            ui.add(egui::TextEdit::multiline(text).desired_rows(5).desired_width(f32::INFINITY));
            let (ok, cancel) = buttons(ui, t, "Save", true);
            if ok {
                acts.push(Action::Run("account.setSignature", json!({"account": account, "signature": text})));
                return false;
            }
            !cancel
        }
        Dialog::MoveTo { filter } => {
            ui.set_width(340.0);
            title(ui, t, "Move to folder");
            let r = ui.add(egui::TextEdit::singleline(filter).hint_text(tl!("Search folders")).desired_width(f32::INFINITY));
            r.request_focus();
            let mut accounts: Vec<String> = s.selected().into_iter().map(|m| m.key.account).collect();
            accounts.sort_unstable();
            accounts.dedup();
            let Some(account) = accounts.first().and_then(|a| s.account(a)) else {
                ui.label(RichText::new(tl!("Select a message first.")).color(t.dim));
                return !buttons(ui, t, "Close", false).1;
            };
            if accounts.len() > 1 {
                ui.label(RichText::new(tl!("Pick messages from one account to move them to a folder.")).color(t.warn).small());
            }
            let needle = filter.to_lowercase();
            let folders: Vec<&Folder> = account
                .folders
                .iter()
                .filter(|f| f.selectable && mailcraft_engine::model::utf7::decode(&f.path).to_lowercase().contains(&needle))
                .collect();
            let mut chosen = None;
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                for f in &folders {
                    let indent = "    ".repeat(f.depth());
                    if ui.selectable_label(false, format!("{indent}{}", f.display_name())).clicked() {
                        chosen = Some(f.path.clone());
                    }
                }
            });
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter && folders.len() == 1 {
                chosen = folders.first().map(|f| f.path.clone());
            }
            if let Some(path) = chosen {
                acts.push(Action::Run("mail.move", json!({"folder": path})));
                return false;
            }
            !buttons(ui, t, "Move", false).1
        }
        Dialog::FolderName { account, parent, rename, name, error } => {
            ui.set_width(360.0);
            title(ui, t, if rename.is_some() { "Rename folder" } else { "New folder" });
            if rename.is_none() {
                let place = parent.as_deref().and_then(|p| s.folder(account, p)).map(Folder::display_name);
                let who = s.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
                let text = match place {
                    Some(p) => tf!("Inside {folder} ({account})", folder = p, account = who),
                    None => tf!("At the top of {account}", account = who),
                };
                ui.label(RichText::new(text).color(t.dim).small());
                ui.add_space(4.0);
            }
            let r = ui.add(egui::TextEdit::singleline(name).hint_text(tl!("Folder name")).desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            // One field: Enter means OK wherever the focus is.
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(e) = error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let ready = !name.trim().is_empty();
            let (ok, cancel) = buttons(ui, t, if rename.is_some() { "Rename" } else { "Create" }, ready);
            if (ok || enter) && ready {
                *error = None;
                match rename {
                    Some(path) => acts.push(Action::Submit("folder.rename", json!({"account": account, "folder": path, "name": name.trim()}))),
                    None => acts.push(Action::Submit("folder.create", json!({"account": account, "parent": parent, "name": name.trim()}))),
                }
            }
            !cancel
        }
        Dialog::MoveFolder { account, folder, filter, error } => {
            ui.set_width(340.0);
            let name = s.folder(account, folder).map(Folder::display_name).unwrap_or_default();
            title(ui, t, &tf!("Move “{folder}” to", folder = name));
            let r = ui.add(egui::TextEdit::singleline(filter).hint_text(tl!("Search folders")).desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            let needle = filter.to_lowercase();
            let Some(acc) = s.account(account) else { return false };
            let d = acc.folders.iter().find_map(|f| f.delimiter);
            let mut chosen: Option<Option<String>> = None;
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                if needle.is_empty() && ui.selectable_label(false, RichText::new(tl!("(Top level)")).italics()).clicked() {
                    chosen = Some(None);
                }
                for f in acc.folders.iter().filter(|f| !mailcraft_engine::model::is_within(&f.path, folder, d)) {
                    if !mailcraft_engine::model::utf7::decode(&f.path).to_lowercase().contains(&needle) {
                        continue;
                    }
                    let indent = "    ".repeat(f.depth());
                    if ui.selectable_label(false, format!("{indent}{}", f.display_name())).clicked() {
                        chosen = Some(Some(f.path.clone()));
                    }
                }
            });
            if let Some(e) = error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            if let Some(parent) = chosen {
                *error = None;
                acts.push(Action::Submit("folder.move", json!({"account": account, "folder": folder, "parent": parent})));
            }
            !buttons(ui, t, "Move", false).1
        }
        Dialog::DeleteFolder { account, folder, name, permanent, error } => {
            ui.set_width(380.0);
            title(ui, t, &tf!("Delete “{folder}”?", folder = name));
            let text = if *permanent {
                tl!("The folder, its subfolders and all the mail in them will be deleted for good. This can't be undone.")
            } else {
                tl!("The folder and everything in it will be moved to Deleted Items.")
            };
            ui.label(RichText::new(text).color(t.dim));
            if let Some(e) = error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let (ok, cancel) = buttons(ui, t, "Delete", true);
            if ok {
                *error = None;
                acts.push(Action::Submit("folder.delete", json!({"account": account, "folder": folder})));
            }
            !cancel
        }
        Dialog::Preview { title: name, header, text } => {
            let w = (ui.ctx().content_rect().width() * 0.7).clamp(360.0, 900.0);
            ui.set_width(w);
            title(ui, t, name);
            for (k, v) in header.iter() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(k.as_str()).color(t.dim).small());
                    ui.label(RichText::new(v.as_str()).small());
                });
            }
            ui.add_space(6.0);
            let h = (ui.ctx().content_rect().height() * 0.6).max(200.0);
            egui::ScrollArea::both().max_height(h).auto_shrink([false, true]).show(ui, |ui| {
                let mut shown = text.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut shown).font(egui::FontId::monospace(13.0)).desired_width(f32::INFINITY).frame(egui::Frame::NONE),
                );
            });
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| !ui.button(tl!("Close")).clicked()).inner
        }
        Dialog::NewCategory { name, color, error } => {
            ui.set_width(340.0);
            title(ui, t, "New category");
            let r = ui.add(egui::TextEdit::singleline(name).hint_text(tl!("Name")).desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.horizontal(|ui| {
                for c in 0..8u8 {
                    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
                    ui.painter().circle_filled(rect.center(), 8.0, theme::category_color(c, t.dark));
                    if *color == c {
                        ui.painter().circle_stroke(rect.center(), 10.0, egui::Stroke::new(2.0, t.text));
                    }
                    if resp.clicked() {
                        *color = c;
                    }
                }
            });
            if let Some(e) = error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ready = !name.trim().is_empty();
            let (ok, cancel) = buttons(ui, t, "Create", ready);
            if (ok || enter) && ready {
                *error = None;
                acts.push(Action::Submit("category.create", json!({"name": name.trim(), "color": color})));
            }
            !cancel
        }
        Dialog::Rules => {
            ui.set_width(520.0);
            title(ui, t, "Rules");
            ui.label(RichText::new(tl!("Rules run on new mail as it arrives in an Inbox, in this order.")).color(t.dim).small());
            ui.add_space(6.0);
            if s.rules.is_empty() {
                ui.label(RichText::new(tl!("No rules yet.")).color(t.faint));
            }
            let mut rules = s.rules.clone();
            let mut changed = false;
            let mut remove = None;
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (i, r) in rules.iter_mut().enumerate() {
                    egui::Frame::new().fill(t.chrome).corner_radius(8).inner_margin(Margin::same(8)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            changed |= ui.checkbox(&mut r.enabled, "").on_hover_text(tl!("On / off")).changed();
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&r.name).strong());
                                ui.label(RichText::new(describe(r)).color(t.dim).small());
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button(tl!("Delete")).clicked() {
                                    remove = Some(i);
                                }
                                if ui.small_button(tl!("Edit")).clicked() {
                                    acts.push(Action::Dialog(Dialog::EditRule(Box::new(RuleForm::from_rule(i, r)))));
                                }
                                if ui.small_button(tl!("Run now")).on_hover_text(tl!("Run on the folder on screen")).clicked() {
                                    acts.push(Action::Run("rules.run", json!({"index": i})));
                                }
                            });
                        });
                    });
                    ui.add_space(4.0);
                }
            });
            if let Some(i) = remove {
                acts.push(Action::Run("rules.delete", json!({"index": i})));
            } else if changed {
                acts.push(Action::Run("rules.set", json!({"rules": rules})));
            }
            ui.add_space(6.0);
            if ui.button(tl!("New rule…")).clicked() {
                acts.push(Action::Dialog(Dialog::rule_from("", "")));
            }
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| !ui.button(tl!("Close")).clicked()).inner
        }
        Dialog::EditRule(f) => {
            ui.set_width(440.0);
            title(ui, t, if f.index.is_some() { "Edit rule" } else { "New rule" });
            egui::Grid::new("rule").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                ui.label(tl!("Name"));
                ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(300.0));
                ui.end_row();
                ui.label(tl!("When mail"));
                egui::ComboBox::from_id_salt("rule-test").width(300.0).selected_text(f.test.label()).show_ui(ui, |ui| {
                    for x in RuleTest::ALL {
                        ui.selectable_value(&mut f.test, x, x.label());
                    }
                });
                ui.end_row();
                if f.test.needs_text() {
                    ui.label("");
                    ui.add(egui::TextEdit::singleline(&mut f.text).hint_text(tl!("Text or address")).desired_width(300.0));
                    ui.end_row();
                }
                ui.label(tl!("Then"));
                egui::ComboBox::from_id_salt("rule-do").width(300.0).selected_text(f.action.label()).show_ui(ui, |ui| {
                    for x in RuleDo::ALL {
                        if ui.selectable_value(&mut f.action, x, x.label()).changed() {
                            f.target.clear();
                        }
                    }
                });
                ui.end_row();
                match f.action {
                    RuleDo::MoveTo => {
                        ui.label(tl!("Folder"));
                        let shown = f
                            .account
                            .as_deref()
                            .and_then(|a| s.folder(a, &f.target))
                            .map(|x| format!("{} ({})", x.display_name(), s.account(&x.account).map(|a| a.config.email.as_str()).unwrap_or_default()))
                            .unwrap_or_else(|| tl!("Choose…").into());
                        egui::ComboBox::from_id_salt("rule-folder").width(300.0).selected_text(shown).show_ui(ui, |ui| {
                            for a in &s.accounts {
                                ui.label(RichText::new(&a.config.email).small().color(t.faint));
                                for x in a.folders.iter().filter(|x| x.selectable) {
                                    let on = f.account.as_deref() == Some(a.config.id.as_str()) && f.target == x.path;
                                    if ui.selectable_label(on, format!("{}{}", "    ".repeat(x.depth()), x.display_name())).clicked() {
                                        f.account = Some(a.config.id.clone());
                                        f.target = x.path.clone();
                                    }
                                }
                            }
                        });
                        ui.end_row();
                    }
                    RuleDo::Categorize => {
                        ui.label(tl!("Category"));
                        let shown = if f.target.is_empty() { tl!("Choose…").to_string() } else { tl!(&f.target).to_string() };
                        egui::ComboBox::from_id_salt("rule-cat").width(300.0).selected_text(shown).show_ui(ui, |ui| {
                            for c in &s.prefs.categories {
                                ui.selectable_value(&mut f.target, c.name.clone(), &c.name);
                            }
                        });
                        ui.end_row();
                    }
                    _ => {}
                }
            });
            ui.checkbox(&mut f.stop, tl!("Don't run later rules on mail this one matches"));
            if let Some(e) = &f.error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let (ok, cancel) = buttons(ui, t, "Save", true);
            if ok {
                match f.to_rule() {
                    Ok(rule) => {
                        let mut rules = s.rules.clone();
                        match f.index.filter(|i| *i < rules.len()) {
                            Some(i) => {
                                let enabled = rules.get(i).is_none_or(|r| r.enabled);
                                if let Some(slot) = rules.get_mut(i) {
                                    *slot = Rule { enabled, ..rule };
                                }
                            }
                            None => rules.push(rule),
                        }
                        f.error = None;
                        acts.push(Action::Submit("rules.set", json!({"rules": rules})));
                    }
                    Err(e) => f.error = Some(e),
                }
            }
            !cancel
        }
        Dialog::EditAccount(f) => edit_account(ui, f, t, acts),
        Dialog::Vacation(f) => vacation(ui, f, t, acts),
        Dialog::ConfirmEmpty { account, folder, name } => {
            ui.set_width(400.0);
            title(ui, t, &tf!("Empty “{folder}”?", folder = name));
            ui.label(
                RichText::new(tl!(
                    "Everything in it will be deleted for good, including mail not downloaded to this computer. This can't be undone."
                ))
                .color(t.dim),
            );
            let (ok, cancel) = buttons(ui, t, "Empty", true);
            if ok {
                acts.push(Action::Run("folder.empty", json!({"account": account, "folder": folder})));
                return false;
            }
            !cancel
        }
    }
}
