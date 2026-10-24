//! Keys held where MailCraft can't read them: a smart card's (through its PKCS #11 module), or
//! the macOS Keychain's. The network layer looks for them ([`Job::FindHeldKeys`]); one chosen
//! becomes the account's certificate ([`Job::UseHeldKey`]), its key staying where it is.

use mailcraft_model::HeldKey;

use super::*;

/// The last look for held keys, and the modules this computer has (from the app).
#[derive(Clone, Debug, Default)]
pub struct CardSearch {
    /// The usual PKCS #11 modules installed here, the likeliest first.
    pub modules: Vec<String>,
    pub searching: bool,
    pub keys: Vec<HeldKey>,
    pub error: Option<String>,
    /// The PIN the search logged in with, for the key chosen from it (memory only).
    pin: Option<String>,
}

impl Session {
    /// The PKCS #11 modules installed on this computer (the app finds them).
    pub fn set_card_modules(&mut self, modules: Vec<String>) {
        self.cards.modules = modules;
    }

    pub fn card_search(&self) -> &CardSearch {
        &self.cards
    }
}

impl CardSearch {
    /// The search logged in to a card with a PIN.
    pub fn pin_given(&self) -> bool {
        self.pin.is_some()
    }
}

impl Session {
    /// Look for certificates with their keys: on the cards `module` sees (`pin` logs in), or
    /// with none, in the system's keychain.
    pub fn find_held_keys(&mut self, module: Option<String>, pin: Option<String>) -> CmdResult {
        let module = module.map(|m| m.trim().to_string());
        if module.as_deref() == Some("") {
            return Err(CmdError::Params("which module? (the .dll, .so or .dylib the card came with)".into()));
        }
        let pin = pin.filter(|p| !p.is_empty());
        self.cards = CardSearch { modules: std::mem::take(&mut self.cards.modules), searching: true, pin: pin.clone(), ..Default::default() };
        self.jobs.push(Job::FindHeldKeys { module, pin: pin.map(mailcraft_model::Secret) });
        Ok(serde_json::json!({ "searching": true }))
    }

    /// Use the key found at `index` for `account`'s mail.
    pub fn use_held_key(&mut self, account: &str, index: usize) -> CmdResult {
        let config = self.account(account).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        let key = self.cards.keys.get(index).cloned().ok_or_else(|| CmdError::Params(format!("no certificate {index} was found")))?;
        let pin = self.cards.pin.clone().map(mailcraft_model::Secret);
        self.jobs.push(Job::UseHeldKey { account: config, at: key.at, cert: key.cert, pin });
        Ok(serde_json::json!({ "using": key.label }))
    }

    pub(super) fn heard_held_keys(&mut self, keys: Vec<HeldKey>, error: Option<String>) {
        self.cards.searching = false;
        self.cards.error = error.or_else(|| keys.is_empty().then(|| "no certificate with its key was found".to_string()));
        self.cards.keys = keys;
    }
}
