//! The server settings of an account already added.

use super::*;

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
    /// The User-Agent its EWS requests name (issue #9); empty for MailCraft's.
    pub user_agent: String,
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
    /// A JMAP account's sign-in: with a password, or through its server's single sign-on.
    pub sign_on: SignOn,
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

    pub(crate) fn server_params(&self) -> serde_json::Value {
        let port = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
        if let Some(u) = &self.ews {
            return json!({
                "account": self.account,
                "name": self.name.trim(),
                "username": self.username.trim(),
                "ews": u.trim(),
                "userAgent": self.user_agent.trim(),
            });
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

pub(crate) fn edit_account(ui: &mut Ui, f: &mut EditForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(480.0);
    title(ui, t, "Server settings");
    ui.label(RichText::new(&f.email).color(t.dim));
    ui.add_space(8.0);
    let w = ui.available_width();
    let host_w = (w - 196.0).max(140.0);
    let label = |ui: &mut Ui, s: &str| {
        ui.label(RichText::new(tl!(s)).strong().small());
    };
    // A JMAP account, which can sign in through its server's single sign-on.
    let jmap = f.jmap && f.ews.is_none() && f.pop.is_none();
    label(ui, "Your name");
    ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.name).hint_text(tl!("Shown on mail you send")));
    // A single sign-on knows who signs in: no user name.
    if !(jmap && f.sign_on.browser) {
        ui.add_space(4.0);
        label(ui, "User name");
        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address")));
    }
    ui.add_space(6.0);
    if let Some(url) = f.ews.as_mut() {
        label(ui, "Exchange server");
        ui.add_sized([w, 26.0], egui::TextEdit::singleline(url).hint_text("mail.example.com"));
        user_agent_field(ui, t, w, &mut f.user_agent);
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
        let r = ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.jmap_url).hint_text("https://api.example.com/jmap/session"));
        f.sign_on.ui(ui, t, s, &f.jmap_url, r.has_focus(), acts);
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
    let ready = !jmap || f.sign_on.ready(s, &f.jmap_url);
    let (ok, cancel) = buttons(ui, t, "Save", ready);
    if ok {
        f.error = None;
        let mut params = f.params();
        if jmap {
            f.sign_on.params(&mut params, s, &f.jmap_url);
        }
        acts.push(Action::Submit("account.update", params));
    }
    !cancel
}
