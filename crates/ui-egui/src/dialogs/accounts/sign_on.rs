//! Signing in through a JMAP server's single sign-on (OpenID Connect), as an account is added
//! and in its server settings: the server asked how it's signed in to (`account.probeJmap`),
//! what it said, and the client ID an administrator gives.

use mailcraft_engine::model::{JmapProbe, NO_CLIENT_ID, OIDC, OidcSettings};

use super::*;
use crate::icons::Icon;

/// Whether this copy signs in through a single sign-on (the browser app doesn't).
pub(crate) fn available(s: &Session) -> bool {
    s.oauth_available.iter().any(|p| p == OIDC)
}

/// An issuer as accounts keep it: its trailing `/` dropped.
fn issuer_of(issuer: &str) -> &str {
    issuer.trim().trim_end_matches('/')
}

/// What the JMAP server at `url` was found to say; `None` while it's asked, or before.
pub(super) fn answer<'s>(s: &'s Session, url: &str) -> Option<&'s JmapProbe> {
    s.jmap_probes.get(url.trim()).and_then(Option::as_ref)
}

/// Asks the JMAP server at `url` how it's signed in to, once its address is typed (`typing`: its
/// box has the focus), and once only: an address the command would refuse isn't asked about.
pub(super) fn ask(s: &Session, url: &str, typing: bool, acts: &mut Vec<Action>) {
    let url = url.trim();
    let address = !url.is_empty() && url.chars().count() <= OidcSettings::MAX_ADDRESS && !url.chars().any(|c| c.is_whitespace() || c.is_control());
    if !typing && address && available(s) && !s.jmap_probes.contains_key(url) {
        acts.push(Action::Run("account.probeJmap", json!({"url": url})));
    }
}

/// The single sign-on service as it's named to the user: its host (`sso.example.com`).
fn service(issuer: &str) -> &str {
    let rest = issuer.split_once("://").map_or(issuer, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// What the server at `url` said, under its address: that it's being asked, that it takes no
/// passwords, or why it, or its sign-in service, couldn't be asked, with a way to ask again (it
/// may be passing). Nothing for one that takes a password.
pub(super) fn note(ui: &mut Ui, t: &Tokens, s: &Session, url: &str, acts: &mut Vec<Action>) {
    let say = |ui: &mut Ui, text: RichText| {
        ui.add(egui::Label::new(text.small()).wrap());
    };
    let again = |ui: &mut Ui, acts: &mut Vec<Action>| {
        if ui.small_button(tl!("Try again")).clicked() {
            acts.push(Action::Run("account.probeJmap", json!({"url": url.trim(), "again": true})));
        }
    };
    match s.jmap_probes.get(url.trim()) {
        Some(None) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tl!("Asking the server how it signs you in…")).color(t.faint).small());
            });
        }
        Some(Some(p)) if p.bearer && p.issuer.is_none() => {
            let error = p.error.as_deref().unwrap_or_default();
            say(
                ui,
                RichText::new(tf!("This server signs you in through your browser, but its sign-in service wasn't found: {error}", error = error))
                    .color(t.warn),
            );
            again(ui, acts);
        }
        Some(Some(p)) if p.takes_no_password() => {
            say(ui, RichText::new(tl!("This server takes no passwords: you sign in through your browser, with its single sign-on.")).color(t.dim));
        }
        Some(Some(JmapProbe { error: Some(error), .. })) => {
            say(ui, RichText::new(tf!("The server couldn't be asked how it signs you in: {error}", error = error)).color(t.warn));
            again(ui, acts);
        }
        _ => {}
    }
}

/// The client ID box (`hint`: what it says when empty), as long as `account.add` takes.
fn client_id_box(ui: &mut Ui, w: f32, client_id: &mut String, hint: &str) {
    ui.add_sized([w, 26.0], egui::TextEdit::singleline(client_id).hint_text(hint).char_limit(OidcSettings::MAX_CLIENT_ID));
}

/// The add dialog's row for the client ID, when MailCraft has none for the server's service.
pub(super) fn client_id_row(ui: &mut Ui, f: &mut AccountForm) {
    ui.label(tl!("Client ID"));
    client_id_box(ui, 300.0, &mut f.client_id, tl!("Given by your administrator"));
    ui.end_row();
}

/// The add dialog's way in through the server's single sign-on: a button that adds the account
/// to sign in in the browser, with no password, under the client ID typed when MailCraft has none.
pub(super) fn add_button(ui: &mut Ui, t: &Tokens, f: &mut AccountForm, p: &JmapProbe, acts: &mut Vec<Action>) {
    let Some(issuer) = p.service() else { return };
    let client_id = f.client_id.trim().to_string();
    if p.needs_client_id() && client_id.is_empty() {
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(tl!(NO_CLIENT_ID)).color(t.warn).small()).wrap());
    }
    let enabled = !f.email.trim().is_empty() && p.signs_in_with(&client_id);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if widgets::accent_button(ui, Icon::Person, "Sign in through the browser", t, enabled).clicked() {
            f.error = None;
            let mut params = f.params();
            if let Some(o) = params.as_object_mut() {
                o.remove("password");
                o.insert("oauth".into(), json!(OIDC));
                if !client_id.is_empty() {
                    o.insert("clientId".into(), json!(client_id));
                }
            }
            acts.push(Action::Submit("account.add", params));
        }
        ui.label(RichText::new(tf!("with {service}: no password stored", service = service(issuer))).color(t.faint).small());
    });
}

/// How an account signs in, in its server settings, for a JMAP account that can through its
/// server's single sign-on: with a password, or through the browser under a client ID.
#[derive(Clone, Debug, Default)]
pub struct SignOn {
    /// Through the browser.
    pub browser: bool,
    /// As it did when its settings were opened: one going back to a password needs it.
    pub was_browser: bool,
    /// The client ID its administrator gave; empty: the one MailCraft was given.
    pub client_id: String,
    /// The single sign-on it had, and the JMAP server it had it for.
    pub kept: OidcSettings,
    pub kept_url: String,
    /// The service the client ID box is for (an issuer as accounts keep it): one the server
    /// names in place of it empties the box, as what was typed is the other service's.
    pub issuer: String,
    /// The password it signs in with again, going back to one.
    pub password: String,
    /// It signs in with another provider (Google or Microsoft): nothing to choose.
    pub other_provider: bool,
}

impl SignOn {
    pub(crate) fn of(c: &AccountConfig) -> SignOn {
        let browser = c.signs_on();
        let kept = c.oidc.clone().unwrap_or_default();
        SignOn {
            browser,
            was_browser: browser,
            client_id: kept.client_id.clone(),
            issuer: issuer_of(&kept.issuer).to_string(),
            kept,
            kept_url: c.jmap.as_deref().unwrap_or_default().trim().to_string(),
            password: String::new(),
            other_provider: c.oauth.is_some() && !browser,
        }
    }

    /// Whether its server's address is another than the one it had: it signs in through the
    /// single sign-on that one names.
    fn moved(&self, url: &str) -> bool {
        url.trim() != self.kept_url
    }

    /// The service saving signs it in through: the one the server at `url` named, starting to
    /// sign in through the browser or moved to it; else the one it had.
    fn service<'s>(&'s self, s: &'s Session, url: &str) -> Option<&'s str> {
        if self.was_browser && !self.moved(url) {
            return Some(issuer_of(&self.kept.issuer));
        }
        answer(s, url).and_then(JmapProbe::service).map(issuer_of)
    }

    /// The client ID it has for the service the box is for: its own, kept for that service.
    fn kept_client_id(&self) -> &str {
        if self.issuer == issuer_of(&self.kept.issuer) { self.kept.client_id.trim() } else { "" }
    }

    /// Whether saving signs it in again through the browser: it starts to, or another service,
    /// or another client ID.
    fn signs_in_again(&self) -> bool {
        !self.was_browser || self.issuer != issuer_of(&self.kept.issuer) || self.client_id.trim() != self.kept.client_id.trim()
    }

    /// Its choice in the server settings of the JMAP account at `url` (`typing`: its address is
    /// being typed), and what each way needs.
    pub(super) fn ui(&mut self, ui: &mut Ui, t: &Tokens, s: &Session, url: &str, typing: bool, acts: &mut Vec<Action>) {
        if self.other_provider || !(available(s) || self.was_browser) {
            return;
        }
        let w = ui.available_width();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Sign-in")).strong().small());
            ui.radio_value(&mut self.browser, false, tl!("Password"));
            ui.radio_value(&mut self.browser, true, tl!("In the browser"));
        });
        if self.browser {
            ask(s, url, typing, acts);
            note(ui, t, s, url, acts);
            // What was typed for another service isn't this one's; with none before, it is.
            if let Some(service) = self.service(s, url).map(str::to_string)
                && service != self.issuer
            {
                let another = !self.issuer.is_empty();
                self.issuer = service;
                if another {
                    self.client_id = self.kept_client_id().to_string();
                }
            }
            let found = answer(s, url).filter(|p| p.bearer);
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("Client ID")).strong().small());
            let hint = found.and_then(|p| p.client_id.as_deref()).unwrap_or(tl!("Given by your administrator"));
            client_id_box(ui, w, &mut self.client_id, hint);
            if found.is_some_and(JmapProbe::needs_client_id) && self.client_id.trim().is_empty() {
                ui.add(egui::Label::new(RichText::new(tl!(NO_CLIENT_ID)).color(t.warn).small()).wrap());
            }
            if self.signs_in_again() {
                ui.label(RichText::new(tl!("Saving opens the sign-in page in your browser.")).color(t.faint).small());
            }
        } else if self.was_browser {
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("Password")).strong().small());
            ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut self.password).password(true));
        }
    }

    /// Whether what's chosen can be saved: going through the browser, or moved to another
    /// server, needs its sign-in service found and a client ID; going back to a password needs it.
    pub(super) fn ready(&self, s: &Session, url: &str) -> bool {
        if !self.browser {
            return !self.was_browser || !self.password.is_empty();
        }
        if self.was_browser && !self.moved(url) {
            return true;
        }
        answer(s, url).is_some_and(|p| p.signs_in_with(&self.client_id))
    }

    /// `oauth`, with the service the server named (when it starts signing in through one, or
    /// moved to it) and the client ID typed (when it isn't the one it had for that service); or
    /// the password going back to one.
    pub(super) fn params(&self, p: &mut serde_json::Value, s: &Session, url: &str) {
        let Some(o) = p.as_object_mut().filter(|_| !self.other_provider) else { return };
        if self.browser {
            o.insert("oauth".into(), json!(OIDC));
            if (!self.was_browser || self.moved(url))
                && let Some(issuer) = self.service(s, url)
            {
                o.insert("issuer".into(), json!(issuer));
            }
            // Emptied: the one MailCraft was given.
            let typed = self.client_id.trim();
            if typed != self.kept_client_id() {
                o.insert("clientId".into(), json!(typed));
            }
        } else if self.was_browser {
            o.insert("oauth".into(), serde_json::Value::Null);
            o.insert("password".into(), json!(self.password));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_is_named_by_its_host() {
        assert_eq!(service("https://sso.example.com"), "sso.example.com");
        assert_eq!(service("https://kc.example.org/realms/mail"), "kc.example.org");
        assert_eq!(service("http://127.0.0.1:8080/"), "127.0.0.1:8080");
        assert_eq!(service("sso.example.com?x#y"), "sso.example.com");
        assert_eq!(service(""), "");
    }
}
