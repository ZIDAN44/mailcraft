//! Adding an email account: the address, name and password; its server settings (or a line
//! naming the servers); what looking the settings up found; signing in through the browser.

use super::*;

pub(super) fn add_account(ui: &mut Ui, f: &mut AccountForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(440.0);
    title(ui, t, "Add an email account");
    if s.jmap_only {
        // The browser: JMAP only, and its address is needed.
        (f.kind, f.advanced) = (AccountKind::Jmap, true);
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
    basics(ui, f, s, acts);
    ui.add_space(6.0);
    if !s.jmap_only {
        ui.checkbox(&mut f.advanced, tl!("Server settings"));
    }
    if f.advanced {
        egui::Frame::new().fill(t.chrome).corner_radius(8).inner_margin(Margin::same(10)).show(ui, |ui| server_settings(ui, f, s, t, acts));
    } else {
        servers_named(ui, f, t);
    }
    found_note(ui, f, s, t);
    if f.kind != AccountKind::Pop {
        let mut on = f.online_only.unwrap_or(s.portable);
        online_only(ui, t, &mut on);
        f.online_only = Some(on);
    }
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    // Outlook.com, Hotmail and Live take no passwords from mail apps any more.
    let consumer = !matches!(f.kind, AccountKind::Jmap | AccountKind::Ews) && mailcraft_engine::model::microsoft_consumer(&f.email);
    sign_in(ui, f, s, t, consumer, acts);
    let ready = f.kind != AccountKind::Graph
        && !consumer
        && !f.email.trim().is_empty()
        && !f.password.is_empty()
        && (f.kind != AccountKind::Jmap || !f.jmap_url.trim().is_empty());
    let (ok, cancel) = buttons(ui, t, "Connect", ready);
    if ok {
        if !matches!(f.kind, AccountKind::Jmap | AccountKind::Ews) && f.imap_host.is_empty() {
            f.guess();
        }
        if f.kind == AccountKind::Pop && f.pop_host.trim().is_empty() {
            f.guess_pop();
        }
        f.error = None;
        acts.push(Action::Submit("account.add", f.params()));
    }
    !cancel
}

/// The address (its settings looked up and filled in once it's typed), the name and the password.
fn basics(ui: &mut Ui, f: &mut AccountForm, s: &Session, acts: &mut Vec<Action>) {
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
        // Live, which take no passwords from mail apps (when this copy can sign in there).
        let browser_only = f.kind == AccountKind::Graph
            || (f.kind.imap_or_pop() && mailcraft_engine::model::microsoft_consumer(&f.email) && s.oauth_available.iter().any(|p| p == "microsoft"));
        if !browser_only {
            ui.label(tl!("Password"));
            ui.add(egui::TextEdit::singleline(&mut f.password).password(true).min_size(vec2(300.0, 26.0)).desired_width(300.0));
            ui.end_row();
        }
    });
}

/// The server settings, opened: the kind of server, then that kind's own rows.
fn server_settings(ui: &mut Ui, f: &mut AccountForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    let w = ui.available_width();
    if !s.jmap_only {
        let was = f.kind;
        ui.horizontal_wrapped(|ui| {
            field_label(ui, "Mail server");
            ui.radio_value(&mut f.kind, AccountKind::Imap, tl!("IMAP and SMTP"));
            ui.radio_value(&mut f.kind, AccountKind::Pop, tl!("POP and SMTP"));
            ui.radio_value(&mut f.kind, AccountKind::Jmap, tl!("JMAP"));
            ui.radio_value(&mut f.kind, AccountKind::Graph, tl!("Exchange (Microsoft 365)"));
            ui.radio_value(&mut f.kind, AccountKind::Ews, tl!("Exchange server (EWS)"));
        });
        // POP just chosen: its server named after the IMAP one, unless one is in.
        if f.kind == AccountKind::Pop && was != AccountKind::Pop && f.pop_host.is_empty() {
            f.guess_pop();
        }
    }
    let pop = match f.kind {
        AccountKind::Imap => false,
        AccountKind::Pop => true,
        AccountKind::Graph => {
            ui.label(
                RichText::new(tl!(
                    "Syncs through Microsoft Graph, for work and school accounts where IMAP is switched off. You sign in with Microsoft in the browser: no password is stored."
                ))
                .color(t.dim)
                .small(),
            );
            return;
        }
        AccountKind::Ews => {
            ui.label(
                RichText::new(tl!(
                    "For Exchange servers run by your organisation, through Exchange Web Services. Signs in with your network user name and password."
                ))
                .color(t.dim)
                .small(),
            );
            field_label(ui, "Exchange server");
            ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.ews_url).hint_text(tl!("Found automatically when left empty")));
            ui.add_space(4.0);
            field_label(ui, "User name");
            ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("DOMAIN\\user, or the email address")));
            user_agent_field(ui, t, w, &mut f.user_agent);
            return;
        }
        AccountKind::Jmap => {
            field_label(ui, "JMAP session address");
            ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.jmap_url).hint_text("https://api.example.com/jmap/session"));
            ui.add_space(4.0);
            field_label(ui, "User name");
            ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address")));
            return;
        }
    };
    let mut edited = if pop {
        let edited = server_row(ui, w, Server::Pop, "", &mut f.pop_host, &mut f.pop_port, &mut f.pop_security);
        f.pop_keep.ui(ui);
        edited
    } else {
        server_row(ui, w, Server::Imap, "", &mut f.imap_host, &mut f.imap_port, &mut f.imap_security)
    };
    ui.add_space(4.0);
    edited |= server_row(ui, w, Server::Smtp, "", &mut f.smtp_host, &mut f.smtp_port, &mut f.smtp_security);
    ui.add_space(4.0);
    field_label(ui, "User name");
    edited |= ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address"))).changed();
    f.smtp_sign_in.ui(ui, w);
    f.servers_edited |= edited;
    if pop {
        // Outlook's "use an existing data file": mail kept from before comes in first.
        ui.add_space(4.0);
        field_label(ui, "Existing data file (optional)");
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
    clear_text_warning(ui, t, if pop { f.pop_security } else { f.imap_security }, f.smtp_security);
}

/// The server settings, closed: a line naming the servers, when there are any to name.
fn servers_named(ui: &mut Ui, f: &AccountForm, t: &Tokens) {
    let named = match f.kind {
        AccountKind::Graph => tl!("Exchange (Microsoft 365)").to_string(),
        AccountKind::Ews if !f.ews_url.trim().is_empty() => tf!("Server: {url} (EWS)", url = f.ews_url.trim()),
        AccountKind::Jmap if !f.jmap_url.trim().is_empty() => tf!("Server: {url} (JMAP)", url = f.jmap_url.trim()),
        AccountKind::Pop if !f.pop_host.is_empty() => tf!(
            "Servers: {pop} (POP {pop_port}), {smtp} (SMTP {smtp_port})",
            pop = f.pop_host,
            pop_port = f.pop_port,
            smtp = f.smtp_host,
            smtp_port = f.smtp_port
        ),
        _ if !f.imap_host.is_empty() => tf!(
            "Servers: {imap} (IMAP {imap_port}), {smtp} (SMTP {smtp_port})",
            imap = f.imap_host,
            imap_port = f.imap_port,
            smtp = f.smtp_host,
            smtp_port = f.smtp_port
        ),
        _ => return,
    };
    ui.label(RichText::new(named).color(t.faint).small());
}

/// What looking the address's settings up came to: found, still looking, or nothing published.
fn found_note(ui: &mut Ui, f: &AccountForm, s: &Session, t: &Tokens) {
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
}

/// Signing in through the browser (OAuth) with providers that offer it, when this copy can; for
/// a `consumer` Microsoft address (Outlook.com, Hotmail, Live) without it, why it can't be added.
fn sign_in(ui: &mut Ui, f: &mut AccountForm, s: &Session, t: &Tokens, consumer: bool, acts: &mut Vec<Action>) {
    let provider = if f.kind == AccountKind::Graph { Some("microsoft") } else { mailcraft_engine::model::oauth_provider_for(&f.imap_host) }
        .filter(|p| s.oauth_available.iter().any(|a| a == p));
    // Without Microsoft sign-in in this copy, say so rather than take a password that can't work.
    if consumer && provider.is_none() {
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
    } else if f.kind == AccountKind::Graph && provider.is_none() {
        ui.add_space(6.0);
        ui.label(RichText::new(tl!("Signing in with Microsoft isn't set up in this copy of MailCraft.")).color(t.warn).small());
    }
    let Some(p) = provider else { return };
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
