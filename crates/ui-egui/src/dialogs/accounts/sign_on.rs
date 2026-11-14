//! Signing in through a JMAP server's single sign-on (OpenID Connect), as an account is added
//! and in its server settings: the server asked how it's signed in to (`account.probeJmap`),
//! what it said, and the client ID an administrator gives.

use mailcraft_engine::model::{JmapProbe, NO_CLIENT_ID, OIDC};

use super::*;
use crate::icons::Icon;

/// The longest server address asked about, as `account.probeJmap` takes them.
const MAX_ADDRESS: usize = 2048;
/// The longest client ID typed, as `account.add` takes them.
const MAX_CLIENT_ID: usize = 256;

/// Whether this copy signs in through a single sign-on (the browser app doesn't).
pub(super) fn available(s: &Session) -> bool {
    s.oauth_available.iter().any(|p| p == OIDC)
}

/// What the JMAP server at `url` was found to say; `None` while it's asked, or before.
pub(super) fn answer<'s>(s: &'s Session, url: &str) -> Option<&'s JmapProbe> {
    s.jmap_probes.get(url.trim()).and_then(Option::as_ref)
}

/// Asks the JMAP server at `url` how it's signed in to, once its address is typed (`typing`: its
/// box has the focus), and once only: an address the command would refuse isn't asked about.
pub(super) fn ask(s: &Session, url: &str, typing: bool, acts: &mut Vec<Action>) {
    let url = url.trim();
    let address = !url.is_empty() && url.len() <= MAX_ADDRESS && !url.chars().any(|c| c.is_whitespace() || c.is_control());
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
/// passwords, or that its sign-in service wasn't found. Nothing for one that takes a password.
pub(super) fn note(ui: &mut Ui, t: &Tokens, s: &Session, url: &str) {
    let say = |ui: &mut Ui, text: RichText| {
        ui.add(egui::Label::new(text.small()).wrap());
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
        }
        Some(Some(p)) if p.bearer && !p.password => {
            say(ui, RichText::new(tl!("This server takes no passwords: you sign in through your browser, with its single sign-on.")).color(t.dim));
        }
        _ => {}
    }
}

/// Whether MailCraft has no client ID for the single sign-on the server named: one is typed.
pub(super) fn needs_client_id(p: &JmapProbe) -> bool {
    p.bearer && p.issuer.is_some() && p.client_id.is_none()
}

/// The client ID box (`hint`: what it says when empty).
fn client_id_box(ui: &mut Ui, w: f32, client_id: &mut String, hint: &str) {
    ui.add_sized([w, 26.0], egui::TextEdit::singleline(client_id).hint_text(hint).char_limit(MAX_CLIENT_ID));
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
    let Some(issuer) = p.issuer.as_deref().filter(|_| p.bearer) else { return };
    let client_id = f.client_id.trim().to_string();
    if needs_client_id(p) && client_id.is_empty() {
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(tl!(NO_CLIENT_ID)).color(t.warn).small()).wrap());
    }
    let enabled = !f.email.trim().is_empty() && (p.client_id.is_some() || !client_id.is_empty());
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
    /// The one it had.
    pub kept_client_id: String,
    /// The password it signs in with again, going back to one.
    pub password: String,
    /// It signs in with Google or Microsoft: nothing to choose.
    pub other: bool,
}

impl SignOn {
    pub(crate) fn of(c: &AccountConfig) -> SignOn {
        let browser = c.oauth.as_deref() == Some(OIDC);
        let client_id = c.oidc.as_ref().map(|o| o.client_id.clone()).unwrap_or_default();
        SignOn {
            browser,
            was_browser: browser,
            kept_client_id: client_id.clone(),
            client_id,
            password: String::new(),
            other: c.oauth.is_some() && !browser,
        }
    }

    /// Its choice in the server settings of the JMAP account at `url` (`typing`: its address is
    /// being typed), and what each way needs.
    pub(super) fn ui(&mut self, ui: &mut Ui, t: &Tokens, s: &Session, url: &str, typing: bool, acts: &mut Vec<Action>) {
        if self.other || !(available(s) || self.was_browser) {
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
            note(ui, t, s, url);
            let found = answer(s, url).filter(|p| p.bearer);
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("Client ID")).strong().small());
            let hint = found.and_then(|p| p.client_id.as_deref()).unwrap_or(tl!("Given by your administrator"));
            client_id_box(ui, w, &mut self.client_id, hint);
            if found.is_some_and(needs_client_id) && self.client_id.trim().is_empty() {
                ui.add(egui::Label::new(RichText::new(tl!(NO_CLIENT_ID)).color(t.warn).small()).wrap());
            }
            if !self.was_browser || self.client_id.trim() != self.kept_client_id {
                ui.label(RichText::new(tl!("Saving opens the sign-in page in your browser.")).color(t.faint).small());
            }
        } else if self.was_browser {
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("Password")).strong().small());
            ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut self.password).password(true));
        }
    }

    /// Whether what's chosen can be saved: going through the browser needs the server's sign-in
    /// service found and a client ID; going back to a password needs it.
    pub(super) fn ready(&self, s: &Session, url: &str) -> bool {
        match (self.browser, self.was_browser) {
            (true, false) => {
                answer(s, url).is_some_and(|p| p.bearer && p.issuer.is_some() && (p.client_id.is_some() || !self.client_id.trim().is_empty()))
            }
            (false, true) => !self.password.is_empty(),
            _ => true,
        }
    }

    /// `oauth`, with the service the server named (when it starts signing in through one) and
    /// the client ID typed; or the password going back to one.
    pub(super) fn params(&self, p: &mut serde_json::Value, s: &Session, url: &str) {
        let Some(o) = p.as_object_mut().filter(|_| !self.other) else { return };
        match (self.browser, self.was_browser) {
            (true, was) => {
                o.insert("oauth".into(), json!(OIDC));
                if !was && let Some(issuer) = answer(s, url).filter(|f| f.bearer).and_then(|f| f.issuer.as_deref()) {
                    o.insert("issuer".into(), json!(issuer));
                }
                // Emptied: the one MailCraft was given.
                let typed = self.client_id.trim();
                if !typed.is_empty() || !self.kept_client_id.is_empty() {
                    o.insert("clientId".into(), json!(typed));
                }
            }
            (false, true) => {
                o.insert("oauth".into(), serde_json::Value::Null);
                o.insert("password".into(), json!(self.password));
            }
            (false, false) => {}
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
