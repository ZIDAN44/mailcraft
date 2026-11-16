//! Adding an email account. Top to bottom, as it's filled in: the address, name and password
//! (what looking the settings up found, under them); the server settings, opened when they're
//! wanted (closed, a line naming the servers); keeping the mail online only; and a way to add
//! calendars and contacts alone instead. The main button connects, or signs in through the
//! browser where that's the only way.

use super::*;

pub(super) fn add_account(ui: &mut Ui, f: &mut AccountForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(440.0);
    title(ui, t, "Add an email account");
    if s.jmap_only {
        // The browser: JMAP only, and its address is needed.
        (f.kind, f.advanced) = (AccountKind::Jmap, true);
        about(
            ui,
            t,
            tl!(
                "In the browser, MailCraft works with JMAP providers. Your password stays in this tab only and is only ever sent to your mail server."
            ),
        );
    } else {
        about(
            ui,
            t,
            tl!("Works with IMAP, POP, JMAP and Exchange. Your password is kept in this computer's keychain and sent only to your mail server."),
        );
    }
    ui.add_space(10.0);
    basics(ui, f, s, acts);
    // What looking the servers up found: only IMAP and POP have servers to look up.
    if f.kind.imap_or_pop() {
        found_note(ui, f, s, t);
    }
    // Outlook.com, Hotmail and Live take no passwords from mail apps any more.
    let consumer = !matches!(f.kind, AccountKind::Jmap | AccountKind::Ews) && mailcraft_engine::model::microsoft_consumer(&f.email);
    let provider = browser_sign_in(f, s);
    if let Some(p) = browser_note(ui, f, t, consumer, provider) {
        submit_signed_in(f, p, acts);
    }
    ui.add_space(8.0);
    if !s.jmap_only {
        let summary = servers_named(f);
        disclosure(ui, t, &mut f.advanced, "Server settings", summary.as_deref());
    }
    if f.advanced {
        egui::Frame::new().fill(t.chrome).corner_radius(8).inner_margin(Margin::same(12)).show(ui, |ui| server_settings(ui, f, s, t, acts));
    }
    if f.kind != AccountKind::Pop {
        let mut on = f.online_only.unwrap_or(s.portable);
        online_only(ui, t, &mut on);
        f.online_only = Some(on);
    }
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(e).color(t.danger)).wrap());
    }
    if !s.jmap_only {
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tl!("Only calendars and contacts, with no email?")).color(t.dim).small());
            if ui.link(RichText::new(tl!("Add a CalDAV and CardDAV account")).color(t.accent).small()).clicked() {
                f.calendar_only = true;
            }
        });
    }
    // The main button: signing in through the browser where that's the only way (Microsoft 365,
    // or Outlook.com where this copy can), else connecting with the password.
    let browser_only = f.kind == AccountKind::Graph || (consumer && provider.is_some());
    if browser_only {
        let label = tf!("Sign in with {provider}", provider = "Microsoft");
        let (ok, cancel) = buttons(ui, t, &label, provider.is_some() && !f.email.trim().is_empty());
        if ok && let Some(p) = provider {
            submit_signed_in(f, p, acts);
        }
        return !cancel;
    }
    let ready = !consumer && !f.email.trim().is_empty() && !f.password.is_empty() && (f.kind != AccountKind::Jmap || !f.jmap_url.trim().is_empty());
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
    egui::Grid::new("acct").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
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

/// The server settings, opened: the kind of server, what it is, then that kind's own rows.
fn server_settings(ui: &mut Ui, f: &mut AccountForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    let w = ui.available_width();
    if !s.jmap_only {
        let was = f.kind;
        // Five kinds: a list to pick from, on one row, rather than a block of choices.
        ui.horizontal(|ui| {
            field_label(ui, "Mail server");
            egui::ComboBox::from_id_salt("account-kind").width(220.0).selected_text(kind_name(f.kind)).show_ui(ui, |ui| {
                for k in [AccountKind::Imap, AccountKind::Pop, AccountKind::Jmap, AccountKind::Graph, AccountKind::Ews] {
                    ui.selectable_value(&mut f.kind, k, kind_name(k));
                }
            });
        });
        // POP just chosen: its server named after the IMAP one, unless one is in.
        if f.kind == AccountKind::Pop && was != AccountKind::Pop && f.pop_host.is_empty() {
            f.guess_pop();
        }
        ui.add_space(2.0);
        about(ui, t, kind_about(f.kind));
    }
    ui.add_space(8.0);
    let pop = match f.kind {
        AccountKind::Imap => false,
        AccountKind::Pop => true,
        AccountKind::Graph => return,
        AccountKind::Ews => {
            field_label(ui, "Exchange server");
            ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.ews_url).hint_text(tl!("Found automatically when left empty")));
            ui.add_space(6.0);
            field_label(ui, "User name");
            ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("DOMAIN\\user, or the email address")));
            user_agent_field(ui, t, w, &mut f.user_agent);
            return;
        }
        AccountKind::Jmap => {
            field_label(ui, "JMAP session address");
            ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.jmap_url).hint_text("https://api.example.com/jmap/session"));
            ui.add_space(6.0);
            field_label(ui, "User name");
            ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address")));
            return;
        }
    };
    // Who signs in, for both servers.
    field_label(ui, "User name");
    let mut edited = ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address"))).changed();
    ui.add_space(8.0);
    // Each server: where it is, then how it's signed in to.
    edited |= if pop {
        server_row(ui, w, Server::Pop, "", &mut f.pop_host, &mut f.pop_port, &mut f.pop_security)
    } else {
        server_row(ui, w, Server::Imap, "", &mut f.imap_host, &mut f.imap_port, &mut f.imap_security)
    };
    f.sign_in.incoming_ui(ui);
    ui.add_space(8.0);
    edited |= server_row(ui, w, Server::Smtp, "", &mut f.smtp_host, &mut f.smtp_port, &mut f.smtp_security);
    f.sign_in.outgoing_ui(ui);
    // Its own user name only matters when it's signed in to.
    if f.sign_in.outgoing != mailcraft_engine::model::SignInMethod::None {
        f.smtp_sign_in.ui(ui, w);
    }
    f.servers_edited |= edited;
    clear_text_warning(ui, t, if pop { f.pop_security } else { f.imap_security }, f.smtp_security);
    if pop {
        // What most leave as it is, each in a part of its own, closed, saying what it's set to.
        ui.add_space(6.0);
        f.pop_keep.ui(ui, t);
        // Outlook's "use an existing data file": mail kept from before comes in first.
        let hint = tl!("Mail from an old .pst or .mailcraft file comes in first");
        let note = if f.pst.trim().is_empty() { hint } else { f.pst.trim() };
        disclosure(ui, t, &mut f.pst_open, "Existing data file (optional)", Some(note));
        if f.pst_open {
            indented(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized([w - 112.0, 26.0], egui::TextEdit::singleline(&mut f.pst).hint_text(hint));
                    if ui.button(tl!("Browse…")).clicked() {
                        acts.push(Action::PickAccountDataFile);
                    }
                });
            });
        }
    }
}

/// Each kind of mail server's name, as it's chosen.
fn kind_name(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Imap => tl!("IMAP and SMTP"),
        AccountKind::Pop => tl!("POP and SMTP"),
        AccountKind::Jmap => tl!("JMAP"),
        AccountKind::Graph => tl!("Exchange (Microsoft 365)"),
        AccountKind::Ews => tl!("Exchange server (EWS)"),
    }
}

/// What each kind of mail server is, in a line, said under the choice.
fn kind_about(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Imap => tl!("Mail stays on the server and looks the same on every device."),
        AccountKind::Pop => tl!("Mail is downloaded to this computer. The server can keep a copy for a while."),
        AccountKind::Jmap => tl!("A newer way to reach your mail, offered by some providers such as Fastmail."),
        AccountKind::Graph => tl!(
            "Syncs through Microsoft Graph, for work and school accounts where IMAP is switched off. You sign in with Microsoft in the browser: no password is stored."
        ),
        AccountKind::Ews => {
            tl!("For Exchange servers run by your organisation, through Exchange Web Services. Signs in with your network user name and password.")
        }
    }
}

/// The servers in a line, for the server settings while they're closed; none before there are
/// any to name.
fn servers_named(f: &AccountForm) -> Option<String> {
    Some(match f.kind {
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
        _ => return None,
    })
}

/// What looking the address's settings up came to, under the address: found, still looking, or
/// nothing published.
fn found_note(ui: &mut Ui, f: &AccountForm, s: &Session, t: &Tokens) {
    let email = f.email.trim().to_ascii_lowercase();
    match (&f.found_source, s.discovered.get(&email)) {
        (Some(src), _) if f.found_for.as_deref() == Some(email.as_str()) => {
            ui.add_space(4.0);
            ui.label(RichText::new(tf!("Settings found: {source}", source = src)).color(t.ok).small());
        }
        (_, Some(None)) => {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tl!("Looking up the server settings…")).color(t.faint).small());
            });
        }
        (_, Some(Some((None, _)))) => {
            ui.add_space(4.0);
            about(ui, t, tl!("No published settings: the usual server names are filled in. Check them under Server settings."));
        }
        _ => {}
    }
}

/// The provider to sign in with through the browser (OAuth), when this copy can: Microsoft for
/// Exchange (Microsoft 365), else the one the incoming server belongs to.
fn browser_sign_in(f: &AccountForm, s: &Session) -> Option<&'static str> {
    let p = if f.kind == AccountKind::Graph { Some("microsoft") } else { mailcraft_engine::model::oauth_provider_for(&f.imap_host) };
    p.filter(|p| s.oauth_available.iter().any(|a| a == p))
}

/// Under the password: why an account can't be added without browser sign-in this copy lacks;
/// or, for a provider that takes a password and also signs in through the browser, that way
/// (the provider, when it's clicked).
fn browser_note(ui: &mut Ui, f: &AccountForm, t: &Tokens, consumer: bool, provider: Option<&'static str>) -> Option<&'static str> {
    let warn = |ui: &mut Ui, s: &str| {
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(s).color(t.warn).small()).wrap());
    };
    if consumer && provider.is_none() {
        warn(
            ui,
            tl!(
                "Outlook.com, Hotmail and Live addresses only connect with Sign in with Microsoft: Microsoft no longer accepts passwords from mail apps. This copy of MailCraft wasn't built with Microsoft sign-in, so it can't add this account."
            ),
        );
        return None;
    }
    if f.kind == AccountKind::Graph && provider.is_none() {
        warn(ui, tl!("Signing in with Microsoft isn't set up in this copy of MailCraft."));
        return None;
    }
    // The main button signs in through the browser when it's the only way; here it's the other.
    let p = provider.filter(|_| f.kind != AccountKind::Graph && !consumer)?;
    ui.add_space(6.0);
    let name = if p == "google" { "Google" } else { "Microsoft" };
    let mut clicked = false;
    ui.horizontal(|ui| {
        clicked =
            widgets::accent_button(ui, crate::icons::Icon::Person, &tf!("Sign in with {provider}", provider = name), t, !f.email.trim().is_empty())
                .clicked();
        ui.label(RichText::new(tl!("in your browser: no password stored")).color(t.faint).small());
    });
    clicked.then_some(p)
}

/// `account.add` signing in through the browser with `provider`: no password, nor a way to sign
/// in with one.
fn submit_signed_in(f: &mut AccountForm, provider: &str, acts: &mut Vec<Action>) {
    f.error = None;
    let mut params = f.params();
    params["oauth"] = json!(provider);
    if let Some(o) = params.as_object_mut() {
        for key in ["password", "signIn", "smtpSignIn"] {
            o.remove(key);
        }
    }
    acts.push(Action::Submit("account.add", params));
}
