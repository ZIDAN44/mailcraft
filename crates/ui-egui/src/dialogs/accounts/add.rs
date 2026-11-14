//! Adding an account: its address and sign-in (a password, or the browser), and its servers.

use super::*;

pub(super) fn add_account(ui: &mut Ui, f: &mut AccountForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
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
        ui.add_space(6.0);
        ui.checkbox(&mut f.calendar_only, tl!("Calendars and contacts only, with no email (CalDAV and CardDAV)"));
    }
    ui.add_space(10.0);
    // What the JMAP server said about how it's signed in to, once asked (below, as its address
    // is typed).
    let probe = if f.jmap { sign_on::answer(s, &f.jmap_url).cloned() } else { None };
    let single_sign_on_only = probe.as_ref().is_some_and(|p| p.bearer && !p.password);
    egui::Grid::new("acct").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
        ui.label(tl!("Email address"));
        let r = ui.add(egui::TextEdit::singleline(&mut f.email).hint_text(tl!("you@example.com")).min_size(vec2(300.0, 26.0)).desired_width(300.0));
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
        ui.add(egui::TextEdit::singleline(&mut f.name).hint_text(tl!("Shown on mail you send")).min_size(vec2(300.0, 26.0)).desired_width(300.0));
        ui.end_row();
        // Exchange signs in with Microsoft in the browser; so do Outlook.com, Hotmail and
        // Live, which take no passwords from mail apps (when this copy can sign in there), and
        // a JMAP server behind a single sign-on that takes none.
        let browser_only = f.graph
            || single_sign_on_only
            || (!f.jmap && !f.ews && mailcraft_engine::model::microsoft_consumer(&f.email) && s.oauth_available.iter().any(|p| p == "microsoft"));
        if !browser_only {
            ui.label(tl!("Password"));
            ui.add(egui::TextEdit::singleline(&mut f.password).password(true).min_size(vec2(300.0, 26.0)).desired_width(300.0));
            ui.end_row();
        }
        if probe.as_ref().is_some_and(sign_on::needs_client_id) {
            sign_on::client_id_row(ui, f);
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
                        user_agent_field(ui, t, w, &mut f.user_agent);
                        return;
                    }
                    if f.jmap {
                        label(ui, "JMAP session address");
                        let r = ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.jmap_url).hint_text("https://api.example.com/jmap/session"));
                        sign_on::ask(s, &f.jmap_url, r.has_focus(), acts);
                        sign_on::note(ui, t, s, &f.jmap_url, acts);
                        // A single sign-on knows who signs in: no user name.
                        if !single_sign_on_only {
                            ui.add_space(4.0);
                            label(ui, "User name");
                            ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address")));
                        }
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
        sign_on::ask(s, &f.jmap_url, false, acts);
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
            if widgets::accent_button(ui, crate::icons::Icon::Person, &tf!("Sign in with {provider}", provider = name), t, !f.email.trim().is_empty())
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
    // A JMAP server's own single sign-on.
    if let Some(p) = &probe {
        sign_on::add_button(ui, t, f, p, acts);
    }
    let ready = !f.graph
        && !consumer
        && !single_sign_on_only
        && !f.email.trim().is_empty()
        && !f.password.is_empty()
        && (!f.jmap || !f.jmap_url.trim().is_empty());
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
    /// The client ID its administrator gave for the JMAP server's single sign-on, when MailCraft
    /// has none for it.
    pub client_id: String,
    /// Exchange Online through Microsoft Graph (signs in with Microsoft in the browser).
    pub graph: bool,
    /// An organisation's own Exchange server through EWS, at this address or server name.
    pub ews: bool,
    pub ews_url: String,
    /// The User-Agent EWS requests name (issue #9); empty for MailCraft's.
    pub user_agent: String,
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
    /// Calendars and contacts only, with no email (issue #61), at this CalDAV and CardDAV server.
    pub calendar_only: bool,
    pub dav_url: String,
}

impl AccountForm {
    pub(crate) fn guess(&mut self) {
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
    pub(crate) fn guess_pop(&mut self) {
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

    pub(crate) fn server_params(&self) -> serde_json::Value {
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
                "userAgent": self.user_agent.trim(),
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
