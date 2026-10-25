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
    /// A password is kept for the user name already: none needs typing again.
    pub kept: bool,
    pub error: Option<String>,
}

impl DavForm {
    pub fn of(c: &mailcraft_engine::model::AccountConfig) -> DavForm {
        let d = c.dav.clone().unwrap_or_default();
        DavForm {
            account: c.id.clone(),
            caldav: d.caldav,
            carddav: d.carddav,
            own: d.username.is_some(),
            username: d.username.clone().unwrap_or_else(|| c.email.clone()),
            password: String::new(),
            kept: d.username.is_some(),
            error: None,
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

pub(super) fn draw(ui: &mut Ui, f: &mut DavForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(480.0);
    title(ui, t, "Calendars and contacts");
    let about = tl!(
        "MailCraft looks for your calendars and contacts on your mail provider's server. If it can't find them, give the addresses from your provider's help pages; leave one empty to keep looking for it."
    );
    ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
    ui.add_space(10.0);
    let w = ui.available_width();
    ui.label(RichText::new(tl!("Calendar server (CalDAV)")).color(t.text));
    ui.add(egui::TextEdit::singleline(&mut f.caldav).hint_text("https://dav.example.com/calendars/").desired_width(w));
    ui.add_space(6.0);
    ui.label(RichText::new(tl!("Contacts server (CardDAV)")).color(t.text));
    ui.add(egui::TextEdit::singleline(&mut f.carddav).hint_text("https://dav.example.com/addressbooks/").desired_width(w));
    ui.add_space(8.0);
    ui.checkbox(&mut f.own, tl!("The calendar server has its own user name and password"));
    if f.own {
        ui.add(egui::TextEdit::singleline(&mut f.username).hint_text(tl!("User name")).desired_width(w));
        let hint = if f.kept { tl!("Password (kept: type to change it)") } else { tl!("Password") };
        ui.add(egui::TextEdit::singleline(&mut f.password).password(true).hint_text(hint).desired_width(w));
        ui.label(
            RichText::new(tl!("Some providers want an app password for calendars: make one in your account's settings there."))
                .color(t.faint)
                .small(),
        );
    }
    if let Some(e) = &f.error {
        ui.add_space(4.0);
        ui.label(RichText::new(e.as_str()).color(t.danger).small());
    }
    let ready = !f.own || (!f.username.trim().is_empty() && (f.kept || !f.password.is_empty()));
    let (ok, cancel) = buttons(ui, t, "Save", ready);
    if ok && ready {
        acts.push(Action::Submit("account.dav", f.params()));
    }
    !cancel
}
