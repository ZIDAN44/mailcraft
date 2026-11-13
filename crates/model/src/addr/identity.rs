//! An account's other addresses (issue #81): a mailbox many providers give several addresses
//! (iCloud's aliases, Gmail's "Send mail as", Exchange's secondary addresses, Fastmail's
//! identities). The account stays one mailbox, read once and kept once; a message is written
//! from any of its addresses, each with a name, a place for replies and a signature of its own
//! when it has them.

use std::ops::Not;

use serde::{Deserialize, Serialize};

use crate::{AccountConfig, EmailAddress, addr};

/// One more address an account's mail is sent from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub email: String,
    /// The name shown on mail sent from it; empty: the account's.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Where replies to mail sent from it go, when not to it; empty: to it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reply_to: String,
    /// Which of the account's signatures mail written from it gets, new or a reply; `None`:
    /// the account's own for each.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// The most other addresses an account has.
pub const MAX_IDENTITIES: usize = 50;
/// The longest name an address has, in characters (as an account's).
pub const MAX_IDENTITY_NAME: usize = 200;

impl AccountConfig {
    /// One of its other addresses, by address (told apart without regard to case).
    pub fn identity(&self, email: &str) -> Option<&Identity> {
        let email = email.trim();
        self.identities.iter().find(|i| i.email.eq_ignore_ascii_case(email))
    }

    /// Every address its mail is sent from: its own first, then the others.
    pub fn addresses(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.email.as_str()).chain(self.identities.iter().map(|i| i.email.as_str()))
    }

    /// Whether `email` is one of its addresses.
    pub fn has_address(&self, email: &str) -> bool {
        let email = email.trim();
        !email.is_empty() && self.addresses().any(|a| a.eq_ignore_ascii_case(email))
    }

    /// Who mail written from `email` is from: that address with its name, or the account's
    /// name when it has none. An address that isn't the account's (or none) is the account's
    /// own.
    pub fn sender(&self, email: &str) -> EmailAddress {
        match self.identity(email) {
            Some(i) if i.name.trim().is_empty() => EmailAddress::new(self.name.clone(), i.email.clone()),
            Some(i) => EmailAddress::new(i.name.clone(), i.email.clone()),
            None => EmailAddress::new(self.name.clone(), self.email.clone()),
        }
    }

    /// Add `identity` to its other addresses, or change the one of that address. Refused with
    /// why: an address that isn't one, the account's own, a name or a place for replies that
    /// can't be, a signature it doesn't have, or too many.
    pub fn set_identity(&mut self, identity: Identity) -> Result<(), String> {
        let email = identity.email.trim().to_string();
        if !addr::is_valid_email(&email) || email.chars().count() > 254 || email.chars().any(char::is_control) {
            return Err(format!("`{}` isn't an email address", identity.email.trim()));
        }
        if email.eq_ignore_ascii_case(self.email.trim()) {
            return Err("that's the account's own address".into());
        }
        let name = identity.name.trim().to_string();
        if name.chars().count() > MAX_IDENTITY_NAME || name.chars().any(char::is_control) {
            return Err("that name can't be used".into());
        }
        let reply_to = identity.reply_to.trim().to_string();
        if !reply_to.is_empty()
            && (addr::parse_list(&reply_to).is_ok_and(|l| !l.is_empty()).not()
                || reply_to.chars().count() > 1000
                || reply_to.chars().any(char::is_control))
        {
            return Err("replies need an email address to go to".into());
        }
        let signature = match identity.signature.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(n) => Some(self.signature_named(n).map(|s| s.name.clone()).ok_or_else(|| format!("no signature `{n}`"))?),
            None => None,
        };
        let new = Identity { email, name, reply_to, signature };
        match self.identities.iter().position(|i| i.email.eq_ignore_ascii_case(&new.email)) {
            Some(at) => {
                if let Some(old) = self.identities.get_mut(at) {
                    *old = new;
                }
            }
            None if self.identities.len() >= MAX_IDENTITIES => return Err(format!("an account has at most {MAX_IDENTITIES} other addresses")),
            None => self.identities.push(new),
        }
        Ok(())
    }

    /// Take `email` out of its other addresses; whether it was one.
    pub fn remove_identity(&mut self, email: &str) -> bool {
        let before = self.identities.len();
        let email = email.trim();
        self.identities.retain(|i| !i.email.eq_ignore_ascii_case(email));
        self.identities.len() < before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Signature;

    fn account() -> AccountConfig {
        AccountConfig {
            id: "a1".into(),
            name: "Riley Moreno".into(),
            email: "riley@studio.example".into(),
            signatures: vec![Signature { name: "Short".into(), text: "R.".into(), ..Default::default() }],
            ..Default::default()
        }
    }

    #[test]
    fn an_account_keeps_other_addresses_to_send_from() {
        let mut a = account();
        a.set_identity(Identity {
            email: " Shop@Studio.example ".into(),
            name: " The Shop ".into(),
            reply_to: "help@studio.example".into(),
            signature: Some("short".into()),
        })
        .unwrap();
        a.set_identity(Identity { email: "plain@studio.example".into(), ..Default::default() }).unwrap();
        assert_eq!(a.addresses().collect::<Vec<_>>(), ["riley@studio.example", "Shop@Studio.example", "plain@studio.example"]);
        assert!(a.has_address("SHOP@studio.example") && a.has_address("riley@studio.example") && !a.has_address("") && !a.has_address("x@y.example"));
        assert_eq!(a.identity("shop@studio.example").unwrap().signature.as_deref(), Some("Short"), "the signature's own name");
        // Who mail is from: the address's name, else the account's; the account's own otherwise.
        assert_eq!(a.sender("shop@studio.example"), EmailAddress::new("The Shop", "Shop@Studio.example"));
        assert_eq!(a.sender("plain@studio.example"), EmailAddress::new("Riley Moreno", "plain@studio.example"));
        assert_eq!(a.sender(""), EmailAddress::new("Riley Moreno", "riley@studio.example"));
        assert_eq!(a.sender("else@where.example"), EmailAddress::new("Riley Moreno", "riley@studio.example"));
        // Changed in place, by its address; taken out.
        a.set_identity(Identity { email: "shop@studio.example".into(), ..Default::default() }).unwrap();
        assert_eq!(a.identities.len(), 2);
        assert_eq!(a.identity("shop@studio.example").unwrap(), &Identity { email: "shop@studio.example".into(), ..Default::default() });
        assert!(a.remove_identity("PLAIN@studio.example") && !a.remove_identity("plain@studio.example"));
        assert_eq!(a.identities.len(), 1);
    }

    #[test]
    fn what_cant_be_an_address_is_refused() {
        let mut a = account();
        let id = |email: &str| Identity { email: email.into(), ..Default::default() };
        for bad in ["", "nobody", "a@", "RILEY@studio.example", "x y@z.example", "line\n@z.example"] {
            assert!(a.set_identity(id(bad)).is_err(), "{bad:?}");
        }
        assert!(a.set_identity(Identity { name: "x".repeat(MAX_IDENTITY_NAME + 1), ..id("a@z.example") }).is_err());
        assert!(a.set_identity(Identity { name: "two\nlines".into(), ..id("a@z.example") }).is_err());
        assert!(a.set_identity(Identity { reply_to: "not an address".into(), ..id("a@z.example") }).is_err());
        assert!(a.set_identity(Identity { signature: Some("Missing".into()), ..id("a@z.example") }).is_err());
        assert!(a.identities.is_empty());
        for n in 0..MAX_IDENTITIES {
            a.set_identity(id(&format!("a{n}@z.example"))).unwrap();
        }
        assert!(a.set_identity(id("one-more@z.example")).is_err());
        a.set_identity(Identity { name: "Still changed".into(), ..id("a0@z.example") }).unwrap();
        // Saved without what it doesn't set, and read back the same; an account without any
        // is saved as it always was.
        let one = Identity { email: "a@z.example".into(), ..Default::default() };
        assert_eq!(serde_json::to_string(&one).unwrap(), r#"{"email":"a@z.example"}"#);
        assert!(!serde_json::to_string(&account()).unwrap().contains("identities"));
        let back: AccountConfig = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }
}
