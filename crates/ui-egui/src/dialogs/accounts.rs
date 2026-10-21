//! Dialogs: accounts (adding one, its settings, its password, removing it), and closing MailCraft.

use super::*;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::AddAccount(f) => {
            ui.set_width(440.0);
            title(ui, t, "Add an email account");
            if s.jmap_only {
                // The browser: JMAP only, and its address is needed.
                (f.jmap, f.graph, f.ews, f.pop, f.advanced) = (true, false, false, false, true);
                ui.label(
                    RichText::new(tl!(
                        "In the browser, MailCraft works with JMAP providers. Your password stays in this tab only and is only ever sent to your mail server."
                    ))
                    .color(t.dim)
                    .small(),
                );
            } else {
                ui.label(RichText::new(tl!("Works with any provider: IMAP or POP with SMTP, JMAP, or Exchange. Your password is kept in this computer's keychain and only ever sent to your mail server.")).color(t.dim).small());
            }
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
                if !s.jmap_only
                    && (r.lost_focus() || f.advanced)
                    && mailcraft_engine::model::addr::is_valid_email(&email)
                    && !s.discovered.contains_key(&email)
                {
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
                    // The provider's POP server when it named one, else one named after the IMAP server.
                    match &c.pop {
                        Some(p) => {
                            f.pop_host = p.host.clone();
                            f.pop_port = p.port.to_string();
                            f.pop_security = p.security;
                        }
                        None => f.guess_pop(),
                    }
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
                // Exchange signs in with Microsoft in the browser; so do Outlook.com, Hotmail and
                // Live, which take no passwords from mail apps (when this copy can sign in there).
                let browser_only = f.graph
                    || (!f.jmap
                        && !f.ews
                        && mailcraft_engine::model::microsoft_consumer(&f.email)
                        && s.oauth_available.iter().any(|p| p == "microsoft"));
                if !browser_only {
                    ui.label(tl!("Password"));
                    ui.add(egui::TextEdit::singleline(&mut f.password).password(true).min_size(vec2(300.0, 26.0)).desired_width(300.0));
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            if !s.jmap_only {
                ui.checkbox(&mut f.advanced, tl!("Server settings"));
            }
            if f.advanced {
                egui::Frame::new().fill(t.chrome).corner_radius(8).inner_margin(Margin::same(10)).show(ui, |ui| {
                    let mut edited = false;
                    let w = ui.available_width();
                    // Fixed sizes: text boxes inside a Grid can't grow past last frame's column width.
                    let host_w = (w - 196.0).max(140.0);
                    let label = |ui: &mut Ui, s: &str| {
                        ui.label(RichText::new(tl!(s)).strong().small());
                    };
                    if !s.jmap_only {
                        let mut kind = if f.pop {
                            4
                        } else if f.ews {
                            3
                        } else if f.graph {
                            2
                        } else {
                            u8::from(f.jmap)
                        };
                        ui.horizontal_wrapped(|ui| {
                            label(ui, "Mail server");
                            ui.radio_value(&mut kind, 0, tl!("IMAP and SMTP"));
                            ui.radio_value(&mut kind, 4, tl!("POP and SMTP"));
                            ui.radio_value(&mut kind, 1, tl!("JMAP"));
                            ui.radio_value(&mut kind, 2, tl!("Exchange (Microsoft 365)"));
                            ui.radio_value(&mut kind, 3, tl!("Exchange server (EWS)"));
                        });
                        if kind == 4 && !f.pop && f.pop_host.is_empty() {
                            f.guess_pop();
                        }
                        (f.jmap, f.graph, f.ews, f.pop) = (kind == 1, kind == 2, kind == 3, kind == 4);
                    }
                    if f.graph {
                        ui.label(
                            RichText::new(tl!(
                                "Syncs through Microsoft Graph, for work and school accounts where IMAP is switched off. You sign in with Microsoft in the browser: no password is stored."
                            ))
                            .color(t.dim)
                            .small(),
                        );
                        return;
                    }
                    if f.ews {
                        ui.label(
                            RichText::new(tl!(
                                "For Exchange servers run by your organisation, through Exchange Web Services. Signs in with your network user name and password."
                            ))
                            .color(t.dim)
                            .small(),
                        );
                        label(ui, "Exchange server");
                        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.ews_url).hint_text(tl!("Found automatically when left empty")));
                        ui.add_space(4.0);
                        label(ui, "User name");
                        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("DOMAIN\\user, or the email address")));
                        return;
                    }
                    if f.jmap {
                        label(ui, "JMAP session address");
                        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.jmap_url).hint_text("https://api.example.com/jmap/session"));
                        ui.add_space(4.0);
                        label(ui, "User name");
                        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address")));
                        return;
                    }
                    if f.pop {
                        label(ui, "Incoming mail (POP)");
                        ui.horizontal(|ui| {
                            edited |=
                                ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(&mut f.pop_host).hint_text("pop.example.com")).changed();
                            edited |= ui
                                .add_sized(
                                    [56.0, 26.0],
                                    egui::TextEdit::singleline(&mut f.pop_port).hint_text("995").align(egui::Align2::CENTER_TOP),
                                )
                                .changed();
                            security_combo(ui, "popsec", &mut f.pop_security);
                        });
                        f.pop_keep.ui(ui);
                    } else {
                        label(ui, "Incoming mail (IMAP)");
                        ui.horizontal(|ui| {
                            edited |=
                                ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(&mut f.imap_host).hint_text("imap.example.com")).changed();
                            edited |= ui
                                .add_sized(
                                    [56.0, 26.0],
                                    egui::TextEdit::singleline(&mut f.imap_port).hint_text("993").align(egui::Align2::CENTER_TOP),
                                )
                                .changed();
                            security_combo(ui, "imapsec", &mut f.imap_security);
                        });
                    }
                    ui.add_space(4.0);
                    label(ui, "Outgoing mail (SMTP)");
                    ui.horizontal(|ui| {
                        edited |= ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(&mut f.smtp_host).hint_text("smtp.example.com")).changed();
                        edited |= ui
                            .add_sized(
                                [56.0, 26.0],
                                egui::TextEdit::singleline(&mut f.smtp_port).hint_text("465").align(egui::Align2::CENTER_TOP),
                            )
                            .changed();
                        security_combo(ui, "smtpsec", &mut f.smtp_security);
                    });
                    ui.add_space(4.0);
                    label(ui, "User name");
                    edited |=
                        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address"))).changed();
                    f.smtp_sign_in.ui(ui, w);
                    f.servers_edited |= edited;
                    if f.pop {
                        // Outlook's "use an existing data file": mail kept from before comes in first.
                        ui.add_space(4.0);
                        label(ui, "Existing data file (optional)");
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [w - 90.0, 26.0],
                                egui::TextEdit::singleline(&mut f.pst).hint_text(tl!("Mail from an old .pst or .mailcraft file comes in first")),
                            );
                            if ui.button(tl!("Browse…")).clicked() {
                                acts.push(Action::PickAccountDataFile);
                            }
                        });
                    }
                    let incoming = if f.pop { f.pop_security } else { f.imap_security };
                    if incoming == Security::None || f.smtp_security == Security::None {
                        ui.label(
                            RichText::new(tl!("Without encryption your password travels in the clear. Use only for local test servers."))
                                .color(t.warn)
                                .small(),
                        );
                    }
                });
            } else if f.graph {
                ui.label(RichText::new(tl!("Exchange (Microsoft 365)")).color(t.faint).small());
            } else if f.ews && !f.ews_url.trim().is_empty() {
                ui.label(RichText::new(tf!("Server: {url} (EWS)", url = f.ews_url.trim())).color(t.faint).small());
            } else if f.jmap && !f.jmap_url.trim().is_empty() {
                ui.label(RichText::new(tf!("Server: {url} (JMAP)", url = f.jmap_url.trim())).color(t.faint).small());
            } else if f.pop && !f.pop_host.is_empty() {
                ui.label(
                    RichText::new(tf!(
                        "Servers: {pop} (POP {pop_port}), {smtp} (SMTP {smtp_port})",
                        pop = f.pop_host,
                        pop_port = f.pop_port,
                        smtp = f.smtp_host,
                        smtp_port = f.smtp_port
                    ))
                    .color(t.faint)
                    .small(),
                );
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
            if !f.pop {
                let mut on = f.online_only.unwrap_or(s.portable);
                online_only(ui, t, &mut on);
                f.online_only = Some(on);
            }
            if let Some(e) = &f.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(t.danger));
            }
            // Providers that sign in through the browser (OAuth), when this copy can.
            let provider = if f.graph { Some("microsoft") } else { mailcraft_engine::model::oauth_provider_for(&f.imap_host) }
                .filter(|p| s.oauth_available.iter().any(|a| a == p));
            // Outlook.com, Hotmail and Live take no passwords from mail apps any more: without
            // Microsoft sign-in in this copy, say so rather than take a password that can't work.
            let consumer = !f.jmap && !f.ews && mailcraft_engine::model::microsoft_consumer(&f.email);
            let blocked = consumer && provider.is_none();
            if blocked {
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(tl!(
                            "Outlook.com, Hotmail and Live addresses only connect with Sign in with Microsoft: Microsoft no longer accepts passwords from mail apps. This copy of MailCraft wasn't built with Microsoft sign-in, so it can't add this account."
                        ))
                        .color(t.warn)
                        .small(),
                    )
                    .wrap(),
                );
            } else if f.graph && provider.is_none() {
                ui.add_space(6.0);
                ui.label(RichText::new(tl!("Signing in with Microsoft isn't set up in this copy of MailCraft.")).color(t.warn).small());
            }
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
                        if let Some(o) = params.as_object_mut() {
                            o.remove("password");
                        }
                        acts.push(Action::Submit("account.add", params));
                    }
                    ui.label(RichText::new(tl!("in your browser: no password stored")).color(t.faint).small());
                });
            }
            let ready = !f.graph && !consumer && !f.email.trim().is_empty() && !f.password.is_empty() && (!f.jmap || !f.jmap_url.trim().is_empty());
            let (ok, cancel) = buttons(ui, t, "Connect", ready);
            if ok {
                if !f.jmap && !f.ews && f.imap_host.is_empty() {
                    f.guess();
                }
                if f.pop && f.pop_host.trim().is_empty() {
                    f.guess_pop();
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
        Dialog::LostOnClose { unsent, changes } => {
            ui.set_width(420.0);
            title(ui, t, "Close MailCraft?");
            let mut lost = Vec::new();
            if *unsent > 0 {
                lost.push(tf!("Messages waiting to be sent: {n}", n = unsent));
            }
            if *changes > 0 {
                lost.push(tf!("Changes not yet on the server: {n}", n = changes));
            }
            ui.add(
                egui::Label::new(
                    RichText::new(tf!(
                        "These are held in memory only, as their accounts are online only, and are lost if MailCraft closes now: {what}. Connect and send them first (Send / Receive), or close anyway.",
                        what = lost.join("; ")
                    ))
                    .color(t.dim),
                )
                .wrap(),
            );
            let (ok, cancel) = buttons(ui, t, "Close anyway", true);
            if ok {
                acts.push(Action::CloseAnyway);
                return false;
            }
            !cancel
        }
        Dialog::SaveOnClose(n) => {
            ui.set_width(400.0);
            title(ui, t, if *n == 1 { "Save the message you're writing?" } else { "Save the messages you're writing?" });
            let about = if *n == 1 {
                tl!("It's kept in Drafts if you save it.").to_string()
            } else {
                tf!("{n} messages are being written. They're kept in Drafts if you save them.", n = n)
            };
            ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
            ui.add_space(10.0);
            let mut keep = true;
            button_row(ui, |ui| {
                if ui.button(tl!("Cancel")).clicked() {
                    keep = false;
                }
                if ui.button(tl!("Don't save")).clicked() {
                    acts.push(Action::CloseAnyway);
                    keep = false;
                }
                if widgets::accent_button(ui, crate::icons::Icon::Drafts, "Save", t, true).clicked() {
                    acts.push(Action::Run("compose.keepAll", json!({})));
                    acts.push(Action::CloseAnyway);
                    keep = false;
                }
            });
            keep
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
    /// The outgoing server's own user name and password.
    pub smtp_sign_in: SmtpSignIn,
    /// Syncs over JMAP instead of IMAP and SMTP, through this session address.
    pub jmap: bool,
    pub jmap_url: String,
    /// Exchange Online through Microsoft Graph (signs in with Microsoft in the browser).
    pub graph: bool,
    /// An organisation's own Exchange server through EWS, at this address or server name.
    pub ews: bool,
    pub ews_url: String,
    /// Downloads mail over POP3 into folders on this computer (sends over SMTP).
    pub pop: bool,
    pub pop_host: String,
    pub pop_port: String,
    pub pop_security: Security,
    /// POP: when the server's copy of downloaded mail goes.
    pub pop_keep: PopKeep,
    /// POP: an existing data file (Outlook's `.pst` or MailCraft's own `.mailcraft`) whose mail comes in first.
    pub pst: String,
    /// Keep none of its mail on this computer (issue #4; not for POP). Until it's chosen, as
    /// MailCraft runs: on when portable.
    pub online_only: Option<bool>,
    pub error: Option<String>,
    /// The address whose looked-up settings are filled in, and where they came from.
    pub found_for: Option<String>,
    pub found_source: Option<String>,
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

impl AccountForm {
    pub(super) fn guess(&mut self) {
        let g = AccountConfig::guess(&self.email);
        self.imap_host = g.imap.host;
        self.imap_port = g.imap.port.to_string();
        self.imap_security = g.imap.security;
        self.smtp_host = g.smtp.host;
        self.smtp_port = g.smtp.port.to_string();
        self.smtp_security = g.smtp.security;
        self.guess_pop();
    }

    /// The POP server named after the IMAP one (pop.example.com for imap.example.com).
    pub(super) fn guess_pop(&mut self) {
        self.pop_host = match self.imap_host.strip_prefix("imap.") {
            Some(rest) => format!("pop.{rest}"),
            None => self.imap_host.clone(),
        };
        self.pop_port = "995".into();
        self.pop_security = Security::Tls;
    }

    pub(crate) fn params(&self) -> serde_json::Value {
        let mut p = self.server_params();
        if self.online_only == Some(true) && !self.pop {
            p["onlineOnly"] = json!(true);
        }
        p
    }

    pub(super) fn server_params(&self) -> serde_json::Value {
        let port = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
        if self.graph {
            return json!({"email": self.email.trim(), "name": self.name.trim(), "graph": true});
        }
        if self.ews {
            return json!({
                "email": self.email.trim(),
                "name": self.name.trim(),
                "password": self.password,
                "username": if self.username.trim().is_empty() { self.email.trim() } else { self.username.trim() },
                // No server named: Autodiscover finds it.
                "ews": if self.ews_url.trim().is_empty() { json!(true) } else { json!(self.ews_url.trim()) },
            });
        }
        if self.jmap {
            return json!({
                "email": self.email.trim(),
                "name": self.name.trim(),
                "password": self.password,
                "username": if self.username.trim().is_empty() { self.email.trim() } else { self.username.trim() },
                "jmap": self.jmap_url.trim(),
            });
        }
        if self.pop {
            let mut p = json!({
                "email": self.email.trim(),
                "name": self.name.trim(),
                "password": self.password,
                "username": if self.username.trim().is_empty() { self.email.trim() } else { self.username.trim() },
                "pop": true,
                "popHost": self.pop_host.trim(),
                "popPort": port(&self.pop_port),
                "popSecurity": self.pop_security.code(),
                "smtpHost": self.smtp_host.trim(),
                "smtpPort": port(&self.smtp_port),
                "smtpSecurity": self.smtp_security.code(),
            });
            self.pop_keep.params(&mut p);
            self.smtp_sign_in.params(&mut p);
            if !self.pst.trim().is_empty() {
                p["pst"] = json!(self.pst.trim());
            }
            return p;
        }
        let mut p = json!({
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
        });
        self.smtp_sign_in.params(&mut p);
        p
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
    /// An Exchange server's EWS address (EWS accounts only).
    pub ews: Option<String>,
    /// A POP account's POP3 server (host, port, security).
    pub pop: Option<(String, String, Security)>,
    /// POP: when the server's copy of downloaded mail goes.
    pub pop_keep: PopKeep,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_security: Security,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_security: Security,
    /// The outgoing server's own user name and password.
    pub smtp_sign_in: SmtpSignIn,
    /// Keep none of its mail on this computer (issue #4; not for POP).
    pub online_only: bool,
    pub error: Option<String>,
}

impl EditForm {
    pub(crate) fn params(&self) -> serde_json::Value {
        let mut p = self.server_params();
        if self.pop.is_none() {
            p["onlineOnly"] = json!(self.online_only);
        }
        p
    }

    pub(super) fn server_params(&self) -> serde_json::Value {
        let port = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
        if let Some(u) = &self.ews {
            return json!({"account": self.account, "name": self.name.trim(), "username": self.username.trim(), "ews": u.trim()});
        }
        if self.jmap {
            return json!({"account": self.account, "name": self.name.trim(), "username": self.username.trim(), "jmap": self.jmap_url.trim()});
        }
        if let Some((host, p, security)) = &self.pop {
            let mut v = json!({
                "account": self.account,
                "name": self.name.trim(),
                "username": self.username.trim(),
                "popHost": host.trim(),
                "popPort": port(p),
                "popSecurity": security.code(),
                "smtpHost": self.smtp_host.trim(),
                "smtpPort": port(&self.smtp_port),
                "smtpSecurity": self.smtp_security.code(),
            });
            self.pop_keep.params(&mut v);
            self.smtp_sign_in.params(&mut v);
            return v;
        }
        let mut v = json!({
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
        });
        if !self.jmap {
            self.smtp_sign_in.params(&mut v);
        }
        v
    }
}

pub(super) fn edit_account(ui: &mut Ui, f: &mut EditForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
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
    if let Some(url) = f.ews.as_mut() {
        label(ui, "Exchange server");
        ui.add_sized([w, 26.0], egui::TextEdit::singleline(url).hint_text("mail.example.com"));
    } else if let Some((host, port, security)) = f.pop.as_mut() {
        label(ui, "Incoming mail (POP)");
        ui.horizontal(|ui| {
            ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(host).hint_text("pop.example.com"));
            ui.add_sized([56.0, 26.0], egui::TextEdit::singleline(port).hint_text("995").align(egui::Align2::CENTER_TOP));
            security_combo(ui, "edit-popsec", security);
        });
        f.pop_keep.ui(ui);
        ui.add_space(4.0);
        label(ui, "Outgoing mail (SMTP)");
        ui.horizontal(|ui| {
            ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(&mut f.smtp_host).hint_text("smtp.example.com"));
            ui.add_sized([56.0, 26.0], egui::TextEdit::singleline(&mut f.smtp_port).hint_text("465").align(egui::Align2::CENTER_TOP));
            security_combo(ui, "edit-smtpsec", &mut f.smtp_security);
        });
        f.smtp_sign_in.ui(ui, w);
    } else {
        ui.horizontal(|ui| {
            label(ui, "Mail server");
            ui.radio_value(&mut f.jmap, false, tl!("IMAP and SMTP"));
            ui.radio_value(&mut f.jmap, true, tl!("JMAP"));
        });
    }
    // (An Exchange server's address is all EWS needs.)
    if f.jmap && f.ews.is_none() {
        label(ui, "JMAP session address");
        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.jmap_url).hint_text("https://api.example.com/jmap/session"));
    } else if f.ews.is_none() && f.pop.is_none() {
        label(ui, "Incoming mail (IMAP)");
        ui.horizontal(|ui| {
            ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(&mut f.imap_host).hint_text("imap.example.com"));
            ui.add_sized([56.0, 26.0], egui::TextEdit::singleline(&mut f.imap_port).hint_text("993").align(egui::Align2::CENTER_TOP));
            security_combo(ui, "edit-imapsec", &mut f.imap_security);
        });
        ui.add_space(4.0);
        label(ui, "Outgoing mail (SMTP)");
        ui.horizontal(|ui| {
            ui.add_sized([host_w, 26.0], egui::TextEdit::singleline(&mut f.smtp_host).hint_text("smtp.example.com"));
            ui.add_sized([56.0, 26.0], egui::TextEdit::singleline(&mut f.smtp_port).hint_text("465").align(egui::Align2::CENTER_TOP));
            security_combo(ui, "edit-smtpsec", &mut f.smtp_security);
        });
        f.smtp_sign_in.ui(ui, w);
        if f.imap_security == Security::None || f.smtp_security == Security::None {
            ui.label(
                RichText::new(tl!("Without encryption your password travels in the clear. Use only for local test servers.")).color(t.warn).small(),
            );
        }
    }
    if f.pop.is_none() {
        online_only(ui, t, &mut f.online_only);
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
