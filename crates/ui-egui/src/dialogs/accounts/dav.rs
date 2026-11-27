//! Calendars and contacts given by hand (issue #7): the CalDAV and CardDAV addresses of a server
//! MailCraft can't find from the account's address, and its own user name and password (an app
//! password, as some providers want for calendars).

use super::*;

/// What the dialog holds while it's open.
#[derive(Clone, Debug, Default)]
pub struct DavForm {
    pub account: String,
    pub caldav: String,
    pub carddav: String,
    /// Signs in with its own user name and password, not the mail server's.
    pub own: bool,
    pub username: String,
    pub password: String,
    /// A password is kept for this user name already (the one it had when opened): none needs
    /// typing again while the user name stays.
    pub kept: Option<String>,
    pub error: Option<String>,
    /// Where MailCraft found the calendars and contacts itself (issue #176), shown under a field
    /// left empty: an empty field looked as if nothing was set.
    pub found_caldav: Option<String>,
    pub found_carddav: Option<String>,
}

impl DavForm {
    /// The dialog for an account; `found`: where its calendars and contacts were found.
    pub fn of(c: &mailcraft_engine::model::AccountConfig, found: (Option<String>, Option<String>)) -> DavForm {
        let d = c.dav.clone().unwrap_or_default();
        let given = |h: Option<String>| h.filter(|h| !h.trim().is_empty());
        DavForm {
            account: c.id.clone(),
            caldav: d.caldav,
            carddav: d.carddav,
            own: d.username.is_some(),
            username: d.username.clone().unwrap_or_else(|| c.email.clone()),
            password: String::new(),
            kept: d.username.clone(),
            error: None,
            found_caldav: given(found.0),
            found_carddav: given(found.1),
        }
    }

    fn params(&self) -> serde_json::Value {
        let mut p = json!({"account": self.account, "caldav": self.caldav.trim(), "carddav": self.carddav.trim()});
        if self.own {
            p["username"] = json!(self.username.trim());
            if !self.password.is_empty() {
                p["password"] = json!(self.password);
            }
        } else {
            p["username"] = serde_json::Value::Null;
        }
        p
    }
}

pub(crate) fn draw(ui: &mut Ui, f: &mut DavForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(crate::sizes::DIALOG_M);
    title(ui, t, "Calendars and contacts");
    let about = tl!(
        "MailCraft looks for your calendars and contacts on your mail provider's server. If it can't find them, give the addresses from your provider's help pages; leave one empty to keep looking for it."
    );
    ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
    ui.add_space(10.0);
    let w = ui.available_width();
    ui.label(RichText::new(tl!("Calendar server (CalDAV)")).color(t.text));
    ui.add(crate::widgets::line_edit(&mut f.caldav).hint_text("https://dav.example.com/calendars/").desired_width(w));
    found(ui, t, &f.caldav, f.found_caldav.as_deref());
    ui.add_space(6.0);
    ui.label(RichText::new(tl!("Contacts server (CardDAV)")).color(t.text));
    ui.add(crate::widgets::line_edit(&mut f.carddav).hint_text("https://dav.example.com/addressbooks/").desired_width(w));
    found(ui, t, &f.carddav, f.found_carddav.as_deref());
    ui.add_space(8.0);
    ui.checkbox(&mut f.own, tl!("The calendar server has its own user name and password"));
    if f.own {
        label_over(ui, &Tokens::of(ui), "User name");
        ui.add(crate::widgets::line_edit(&mut f.username).desired_width(w));
        let kept = f.kept.as_deref().is_some_and(|k| k == f.username.trim());
        // The hint says one is kept, "Saved", as the other password boxes do.
        let hint = if kept { tl!("Saved: type to replace it") } else { "" };
        label_over(ui, &Tokens::of(ui), "Password");
        widgets::password_field(ui, &mut f.password, hint);
        ui.label(
            RichText::new(tl!("Some providers want an app password for calendars: make one in your account's settings there."))
                .color(t.faint)
                .small(),
        );
    }
    if let Some(e) = &f.error {
        ui.add_space(4.0);
        crate::dialogs::field_error(ui, t, e);
    }
    let kept = f.kept.as_deref().is_some_and(|k| k == f.username.trim());
    let ready = !f.own || (!f.username.trim().is_empty() && (kept || !f.password.is_empty()));
    let (ok, cancel) = buttons(ui, t, "Save", ready);
    if ok && ready {
        acts.push(Action::Submit("account.dav", f.params()));
    }
    !cancel
}

/// Under a field left empty, where MailCraft found that server itself (issue #176).
fn found(ui: &mut Ui, t: &Tokens, typed: &str, found: Option<&str>) {
    if let Some(url) = found.filter(|_| typed.trim().is_empty()) {
        ui.add(egui::Label::new(RichText::new(tf!("Found: {url}", url = url)).color(t.dim).small()).truncate()).on_hover_text(url);
    }
}

/// Adding a calendar-only account (issue #61): a CalDAV and CardDAV server with no email. Its
/// address names the account, as every account's does, and the meetings it organises.
pub(crate) fn add_calendar_only(ui: &mut Ui, f: &mut super::AccountForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(crate::sizes::DIALOG_M);
    title(ui, t, "Add calendars and contacts");
    super::about(
        ui,
        t,
        tl!(
            "For a CalDAV and CardDAV server with no email, such as Nextcloud, Baïkal, SOGo or Radicale. Your password is kept in this computer's keychain and sent only to that server."
        ),
    );
    ui.add_space(10.0);
    // Who you are, as when an email account is added; then the server, and who signs in to it.
    let (w, h) = (ui.available_width(), crate::sizes::CONTROL_H);
    {
        super::field_label(ui, "Email address");
        ui.add_sized([w, h], crate::widgets::line_edit(&mut f.email).hint_text(tl!("you@example.com")));
        ui.add_space(6.0);
        super::field_label(ui, "Your name");
        ui.add_sized([w, h], crate::widgets::line_edit(&mut f.name).hint_text(tl!("Shown on mail you send")));
        ui.add_space(6.0);
        super::field_label(ui, "Password");
        widgets::password_field(ui, &mut f.password, "");
    }
    ui.add_space(10.0);
    let w = ui.available_width();
    super::field_label(ui, "Calendar and contacts server");
    ui.add_sized([w, crate::sizes::CONTROL_H], crate::widgets::line_edit(&mut f.dav_url).hint_text("https://cloud.example.com/remote.php/dav"));
    ui.add_space(6.0);
    super::field_label(ui, "User name");
    ui.add_sized([w, crate::sizes::CONTROL_H], crate::widgets::line_edit(&mut f.username).hint_text(tl!("Usually the email address")));
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        crate::dialogs::field_error(ui, t, e);
    }
    ui.add_space(8.0);
    if ui.link(RichText::new(tl!("Add an email account instead")).color(t.accent).small()).clicked() {
        f.calendar_only = false;
    }
    let ready = !f.dav_url.trim().is_empty() && !f.email.trim().is_empty() && !f.password.is_empty();
    let (ok, cancel) = buttons(ui, t, "Connect", ready);
    if ok && ready {
        f.error = None;
        acts.push(Action::Submit("account.add", f.calendar_params()));
    }
    !cancel
}

impl super::AccountForm {
    /// `account.add` for a calendar-only account (issue #61): an empty user name is the address.
    pub(crate) fn calendar_params(&self) -> serde_json::Value {
        json!({
            "email": self.email.trim(),
            "name": self.name.trim(),
            "username": self.username.trim(),
            "password": self.password,
            "dav": self.dav_url.trim(),
        })
    }
}
