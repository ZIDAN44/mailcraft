//! Account setup form state (`AccountForm`, `SmtpSignIn`, `PopKeep`).

use super::*;

/// Form state for adding a new mail or calendar-only account.
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
    /// `jmap` came from guessing or discovering the address, not from choosing JMAP by hand.
    pub jmap_guessed: bool,
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
    /// Build `SmtpSignIn` form fields from an existing [`AccountConfig`].
    pub(crate) fn of(c: &AccountConfig) -> SmtpSignIn {
        SmtpSignIn {
            own: c.smtp_username.is_some(),
            username: c.smtp_username.clone().unwrap_or_default(),
            password: String::new(),
            kept: c.smtp_username.is_some(),
        }
    }

    /// `smtpUsername` (null: the incoming server's sign-in) and `smtpPassword`.
    pub(crate) fn params(&self, p: &mut serde_json::Value) {
        if self.own {
            p["smtpUsername"] = json!(self.username.trim());
            if !self.password.is_empty() {
                p["smtpPassword"] = json!(self.password);
            }
        } else {
            p["smtpUsername"] = serde_json::Value::Null;
        }
    }

    /// Draw the outgoing server's own sign-in checkbox and fields.
    pub(crate) fn ui(&mut self, ui: &mut Ui, w: f32) {
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
    /// Build `PopKeep` form fields from an existing [`AccountConfig`].
    pub(crate) fn of(c: &AccountConfig) -> PopKeep {
        PopKeep {
            keep: !c.pop_delete,
            for_days: c.pop_keep_days.is_some(),
            days: c.pop_keep_days.unwrap_or(14).to_string(),
            until_deleted: c.pop_delete_with_trash,
        }
    }

    /// `popDelete`, `popKeepDays` (0: for good) and `popDeleteWithTrash`.
    pub(crate) fn params(&self, p: &mut serde_json::Value) {
        let days = if self.keep && self.for_days { self.days.trim().parse::<u32>().ok().filter(|d| *d > 0).unwrap_or(14) } else { 0 };
        p["popDelete"] = json!(!self.keep);
        p["popKeepDays"] = json!(days);
        p["popDeleteWithTrash"] = json!(self.keep && self.until_deleted);
    }

    /// Draw POP retention checkboxes and day-count field.
    pub(crate) fn ui(&mut self, ui: &mut Ui) {
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
    /// Fill in guessed server names and ports from `self.email`.
    pub(crate) fn guess(&mut self) {
        let g = AccountConfig::guess(&self.email);
        self.imap_host = g.imap.host;
        self.imap_port = g.imap.port.to_string();
        self.imap_security = g.imap.security;
        self.smtp_host = g.smtp.host;
        self.smtp_port = g.smtp.port.to_string();
        self.smtp_security = g.smtp.security;
        if let Some(u) = g.jmap {
            (self.jmap, self.graph, self.ews, self.pop) = (true, false, false, false);
            self.jmap_url = u;
            self.jmap_guessed = true;
        } else if self.jmap_guessed {
            self.jmap = false;
            self.jmap_url.clear();
            self.jmap_guessed = false;
        }
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

    /// Command parameters for `account.add`.
    pub(crate) fn params(&self) -> serde_json::Value {
        let mut p = self.server_params();
        if self.online_only == Some(true) && !self.pop {
            p["onlineOnly"] = json!(true);
        }
        p
    }

    /// Transport-specific command parameters for `account.add`.
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
