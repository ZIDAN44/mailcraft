//! Modal dialogs: add an account, sign in again, move to a folder, confirmations, settings.

use egui::{Margin, RichText, Ui, vec2};
use mailcraft_engine::Session;
use mailcraft_engine::model::{AccountConfig, Folder, Security};
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
    pub error: Option<String>,
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

    fn params(&self) -> serde_json::Value {
        let port = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
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

#[derive(Clone, Debug)]
pub enum Dialog {
    AddAccount(Box<AccountForm>),
    Password { account: String, email: String, password: String },
    ConfirmDelete { count: usize },
    MoveTo { filter: String },
    EmptySubject,
    RemoveAccount { account: String, email: String },
    Signature { account: String, text: String },
    Settings,
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

    pub fn password(a: &AccountConfig) -> Dialog {
        Dialog::Password { account: a.id.clone(), email: a.email.clone(), password: String::new() }
    }

    /// Set the error shown in the add-account form.
    pub fn set_error(&mut self, e: String) {
        if let Dialog::AddAccount(f) = self {
            f.error = Some(e);
        }
    }
}

fn title(ui: &mut Ui, t: &Tokens, s: &str) {
    ui.label(RichText::new(s).font(theme::bold(18.0)).color(t.text));
    ui.add_space(6.0);
}

fn security_combo(ui: &mut Ui, id: &str, v: &mut Security) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(match v {
            Security::Tls => "SSL/TLS",
            Security::StartTls => "STARTTLS",
            Security::None => "None (unsafe)",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(v, Security::Tls, "SSL/TLS");
            ui.selectable_value(v, Security::StartTls, "STARTTLS");
            ui.selectable_value(v, Security::None, "None (unsafe)");
        });
}

fn buttons(ui: &mut Ui, t: &Tokens, ok: &str, ok_enabled: bool) -> (bool, bool) {
    let mut res = (false, false);
    ui.add_space(10.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::accent_button(ui, crate::icons::Icon::Send, ok, t, ok_enabled).clicked() {
            res.0 = true;
        }
        if ui.button("Cancel").clicked() {
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
            ui.label(RichText::new("Works with any provider that offers IMAP and SMTP. Your password is kept in this computer's keychain and only ever sent to your mail server.").color(t.dim).small());
            ui.add_space(10.0);
            egui::Grid::new("acct").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                ui.label("Email address");
                let r =
                    ui.add(egui::TextEdit::singleline(&mut f.email).hint_text("you@example.com").min_size(vec2(300.0, 26.0)).desired_width(300.0));
                if r.changed() && !f.servers_edited {
                    f.guess();
                }
                ui.end_row();
                ui.label("Your name");
                ui.add(egui::TextEdit::singleline(&mut f.name).hint_text("Shown on mail you send").min_size(vec2(300.0, 26.0)).desired_width(300.0));
                ui.end_row();
                ui.label("Password");
                ui.add(egui::TextEdit::singleline(&mut f.password).password(true).min_size(vec2(300.0, 26.0)).desired_width(300.0));
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.checkbox(&mut f.advanced, "Server settings");
            if f.advanced {
                egui::Frame::new().fill(t.chrome).corner_radius(8).inner_margin(Margin::same(10)).show(ui, |ui| {
                    let mut edited = false;
                    let w = ui.available_width();
                    // Fixed sizes: text boxes inside a Grid can't grow past last frame's column width.
                    let host_w = (w - 196.0).max(140.0);
                    let label = |ui: &mut Ui, s: &str| {
                        ui.label(RichText::new(s).strong().small());
                    };
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
                    edited |= ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text("Usually the email address")).changed();
                    f.servers_edited |= edited;
                    if f.imap_security == Security::None || f.smtp_security == Security::None {
                        ui.label(
                            RichText::new("Without encryption your password travels in the clear. Use only for local test servers.")
                                .color(t.warn)
                                .small(),
                        );
                    }
                });
            } else if !f.imap_host.is_empty() {
                ui.label(
                    RichText::new(format!("Servers: {} (IMAP {}), {} (SMTP {})", f.imap_host, f.imap_port, f.smtp_host, f.smtp_port))
                        .color(t.faint)
                        .small(),
                );
            }
            if let Some(e) = &f.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(t.danger));
            }
            let ready = !f.email.trim().is_empty() && !f.password.is_empty();
            let (ok, cancel) = buttons(ui, t, "Connect", ready);
            if ok {
                if f.imap_host.is_empty() {
                    f.guess();
                }
                f.error = None;
                acts.push(Action::SubmitAccount(f.params()));
            }
            !cancel
        }
        Dialog::Password { account, email, password } => {
            ui.set_width(380.0);
            title(ui, t, "Sign in again");
            ui.label(RichText::new(format!("Enter the password for {email}.")).color(t.dim));
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
            let what = if *count == 1 { "This message".to_string() } else { format!("These {count} messages") };
            ui.label(RichText::new(format!("{what} will be deleted for good. This can't be undone.")).color(t.dim));
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
            ui.label(RichText::new("Messages without a subject are easy to overlook.").color(t.dim));
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
                RichText::new(format!("{email} and its downloaded mail will be removed from this computer. Nothing is deleted on the server."))
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
            ui.label(RichText::new("Added to new messages, replies and forwards.").color(t.dim).small());
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
            let r = ui.add(egui::TextEdit::singleline(filter).hint_text("Search folders").desired_width(f32::INFINITY));
            r.request_focus();
            let mut accounts: Vec<String> = s.selected().into_iter().map(|m| m.key.account).collect();
            accounts.sort_unstable();
            accounts.dedup();
            let Some(account) = accounts.first().and_then(|a| s.account(a)) else {
                ui.label(RichText::new("Select a message first.").color(t.dim));
                return !buttons(ui, t, "Close", false).1;
            };
            if accounts.len() > 1 {
                ui.label(RichText::new("Pick messages from one account to move them to a folder.").color(t.warn).small());
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
        Dialog::Settings => {
            ui.set_width(460.0);
            title(ui, t, "Accounts");
            for a in &s.accounts {
                egui::Frame::new().fill(t.chrome).corner_radius(8).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&a.config.email).strong());
                            let servers = if a.is_local() {
                                "Sample mailbox, kept on this computer only".to_string()
                            } else {
                                format!("{}:{} · {}:{}", a.config.imap.host, a.config.imap.port, a.config.smtp.host, a.config.smtp.port)
                            };
                            ui.label(RichText::new(servers).color(t.faint).small());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Remove…").clicked() {
                                acts.push(Action::Dialog(Dialog::RemoveAccount { account: a.config.id.clone(), email: a.config.email.clone() }));
                            }
                            if ui.button("Signature…").clicked() {
                                acts.push(Action::Dialog(Dialog::Signature { account: a.config.id.clone(), text: a.config.signature.clone() }));
                            }
                            if !a.is_local() && ui.button("Password…").clicked() {
                                acts.push(Action::Dialog(Dialog::password(&a.config)));
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
            ui.add_space(6.0);
            if ui.button("Add account…").clicked() {
                acts.push(Action::Dialog(Dialog::add_account()));
            }
            ui.add_space(12.0);
            ui.separator();
            ui.label(RichText::new(format!("MailCraft {} · an ArtCraft app · MIT OR Apache-2.0", env!("CARGO_PKG_VERSION"))).color(t.faint).small());
            ui.label(
                RichText::new("Local-first: mail is cached on this computer for fast offline search. Passwords stay in the system keychain.")
                    .color(t.faint)
                    .small(),
            );
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| !ui.button("Close").clicked()).inner
        }
    }
}
