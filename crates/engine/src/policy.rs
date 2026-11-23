//! An organisation's profile (issue #245): what an IT department sets for everyone who uses
//! MailCraft on its computers, in one place, read as MailCraft starts. Each setting is a default
//! (the person may change it) or locked (they can't); the look, the servers of the
//! organisation's mail domains, a signature and a disclaimer, and policies: updates, encrypted
//! connections only, no certificate exceptions.
//!
//! Where it comes from is the system's business ([`sources`]: Group Policy's registry key on
//! Windows, a configuration profile on macOS, a file only an administrator can write, or one
//! beside a portable copy, whose locks don't count). This module is what a profile says, as
//! JSON (`docs/organisations.md`), and what applies to someone given their accounts' domains:
//!
//! ```json
//! {"organisation": "Contoso", "settings": {"autoPictures": {"value": false, "locked": true}},
//!  "requireEncryption": true, "accounts": {"servers": {"contoso.example": {"imap": "mail.contoso.example:993:tls"}}},
//!  "profiles": [{"domains": ["fabrikam.example"], "organisation": "Fabrikam"}]}
//! ```
//!
//! Several institutions in one file: `profiles`, each for its mail domains. An account's servers,
//! signature and disclaimer come from the profile of its domain (else the top one). What is the
//! window's alone (the look, the settings) comes from the top, then the profile marked `default`;
//! a lock in the profile of any of the person's domains holds too, and of the policies the
//! strictest wins. Nothing here can make MailCraft less safe than it is without a profile: no
//! certificate is trusted by it, and no password is ever in it.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

mod read;
#[cfg(not(target_arch = "wasm32"))]
pub mod sources;

pub use read::{PROFILE_MAX, without_locks};

/// A setting's value from the organisation: a default the person may change, or locked.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Given {
    pub value: Value,
    pub locked: bool,
}

/// A mail server the organisation names for a domain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
    /// `tls`, `starttls` or `none`, as `account.add` takes it.
    pub security: String,
}

/// The servers the organisation names for one mail domain's accounts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DomainServers {
    pub imap: Option<Server>,
    pub pop: Option<Server>,
    pub smtp: Option<Server>,
    pub caldav: Option<String>,
    pub carddav: Option<String>,
    /// On-premises Exchange: its EWS address.
    pub ews: Option<String>,
    /// A JMAP session address.
    pub jmap: Option<String>,
    /// Exchange Online, through Microsoft Graph.
    pub graph: bool,
}

/// The automatic look-ups an address's servers are found by, each on unless turned off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Lookups {
    /// The domain's own autoconfig files (`autoconfig.<domain>`, `/.well-known/`).
    pub domain: bool,
    /// Thunderbird's provider database (`autoconfig.thunderbird.net`).
    pub thunderbird: bool,
    /// The domain's mail exchanger, to find its provider.
    pub mx: bool,
    /// DNS service records (`_imaps._tcp`, `_autodiscover._tcp`…).
    pub dns: bool,
}

impl Default for Lookups {
    fn default() -> Self {
        Lookups { domain: true, thunderbird: true, mx: true, dns: true }
    }
}

/// The organisation's own Microsoft 365 app registration, for signing in with Microsoft.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MicrosoftApp {
    pub client_id: String,
    /// Its directory (tenant) id or domain; `None`: any work or school account.
    pub tenant: Option<String>,
}

/// The organisation's signature: `{name}`, `{title}`… filled from the person's details.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SignatureRule {
    /// Its name among the account's signatures.
    pub name: String,
    pub text: String,
    pub html: String,
    /// It can't be changed or removed, nor another put in its place.
    pub locked: bool,
    /// Put into new messages, and into replies and forwards.
    pub new_messages: bool,
    pub replies: bool,
}

/// What's added under every message sent from the domain's accounts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Disclaimer {
    pub text: String,
    pub html: String,
}

/// What updating MailCraft by itself does (issue #207, and an organisation's, #245).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Updates {
    /// Test builds and releases, as the copy was installed.
    #[default]
    On,
    /// Releases only.
    Stable,
    /// Never.
    Off,
}

impl Updates {
    pub fn parse(s: &str) -> Option<Updates> {
        match s.trim().to_ascii_lowercase().as_str() {
            "on" | "auto" | "automatic" => Some(Updates::On),
            "stable" | "releases" => Some(Updates::Stable),
            "off" | "never" => Some(Updates::Off),
            _ => None,
        }
    }
}

/// One profile: for everyone (the top of the file), or for some mail domains.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    /// The mail domains it is for (lower-case); none at the top.
    pub domains: Vec<String>,
    /// The profile that sets the window's look and settings, with several.
    pub default: bool,
    pub organisation: Option<String>,
    /// A picture file (PNG, JPEG…) shown with the organisation's name.
    pub logo: Option<String>,
    /// Who to ask about a locked setting (a help desk's address or number).
    pub contact: Option<String>,
    /// Settings by their key in MailCraft's settings (`autoPictures`, `accent`…).
    pub settings: BTreeMap<String, Given>,
    /// `light`, `dark` or `system`.
    pub theme: Option<Given>,
    /// The interface language's code (`de`, `pt-br`), or `auto`.
    pub language: Option<Given>,
    pub updates: Option<(Updates, bool)>,
    pub require_encryption: Option<bool>,
    pub certificate_exceptions: Option<bool>,
    /// Accounts only of the organisation's domains.
    pub only_organisation_domains: Option<bool>,
    /// More of the organisation's mail domains, beyond the profiles'.
    pub account_domains: Vec<String>,
    pub lookups: Option<Lookups>,
    /// An autoconfig file's address, asked before anything else.
    pub autoconfig: Option<String>,
    pub servers: BTreeMap<String, DomainServers>,
    pub microsoft: Option<MicrosoftApp>,
    pub signature: Option<SignatureRule>,
    pub disclaimer: Option<Disclaimer>,
}

/// An organisation's profile as read: the top, those for some domains, where it came from and
/// what of it couldn't be read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Organisation {
    pub top: Profile,
    pub profiles: Vec<Profile>,
    /// Where it was read (a registry key, a file), in the order taken: later ones win.
    pub sources: Vec<String>,
    /// What couldn't be read or isn't allowed there, said: never a reason to refuse the rest.
    pub ignored: Vec<String>,
}

/// What applies to someone, given their accounts' domains.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Effective {
    pub organisation: Option<String>,
    pub logo: Option<String>,
    pub contact: Option<String>,
    pub settings: BTreeMap<String, Given>,
    pub theme: Option<Given>,
    pub language: Option<Given>,
    pub updates: Option<(Updates, bool)>,
    pub require_encryption: bool,
    pub no_certificate_exceptions: bool,
    /// Accounts only of these domains (`None`: any).
    pub allowed_domains: Option<Vec<String>>,
    pub lookups: Lookups,
    pub autoconfig: Option<String>,
    pub microsoft: Option<MicrosoftApp>,
    /// What of the settings MailCraft didn't take, said (filled in by the session).
    pub notes: Vec<String>,
    /// The locked settings as the settings write them (absent: not written), to check them
    /// against after each command (filled in by the session).
    #[serde(skip)]
    pub held: Vec<(String, Option<Value>)>,
}

/// The domain of an address, lower-case (`""` for one without).
pub fn domain_of(email: &str) -> String {
    email.rsplit_once('@').map(|(_, d)| d.trim().trim_end_matches('.').to_ascii_lowercase()).unwrap_or_default()
}

impl Organisation {
    /// Whether there is any profile at all: none, and MailCraft is as it always was.
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty() && self.top == Profile::default() && self.profiles.is_empty()
    }

    /// The profile `email`'s account follows: the one for its domain, else the top.
    pub fn profile_for(&self, email: &str) -> &Profile {
        let domain = domain_of(email);
        self.profiles.iter().find(|p| p.domains.contains(&domain)).unwrap_or(&self.top)
    }

    /// The servers the organisation names for `email`'s domain: its profile's, else the top's.
    pub fn servers_for(&self, email: &str) -> Option<&DomainServers> {
        let domain = domain_of(email);
        let own = self.profile_for(email);
        own.servers.get(&domain).or_else(|| self.top.servers.get(&domain))
    }

    /// The signature for `email`'s account: its profile's, else the top's.
    pub fn signature_for(&self, email: &str) -> Option<&SignatureRule> {
        self.profile_for(email).signature.as_ref().or(self.top.signature.as_ref())
    }

    /// The disclaimer under `email`'s mail: its profile's, else the top's.
    pub fn disclaimer_for(&self, email: &str) -> Option<&Disclaimer> {
        self.profile_for(email).disclaimer.as_ref().or(self.top.disclaimer.as_ref())
    }

    /// The organisation's mail domains: its profiles', `accounts.domains` and those it names
    /// servers for.
    pub fn domains(&self) -> Vec<String> {
        let mut all: Vec<String> = std::iter::once(&self.top)
            .chain(&self.profiles)
            .flat_map(|p| p.domains.iter().chain(&p.account_domains).chain(p.servers.keys()))
            .cloned()
            .collect();
        all.sort();
        all.dedup();
        all
    }

    /// Each Microsoft 365 registration the profile gives, with the mail domains whose accounts
    /// sign in through it: a domain's profile's for its domains; the window's (the default
    /// profile's, else the top's) for the organisation's others, and its directory's when that's
    /// a domain. Accounts of any other domain (a personal one) sign in through MailCraft's own.
    pub fn microsoft_apps(&self) -> Vec<(Vec<String>, MicrosoftApp)> {
        let mut out: Vec<(Vec<String>, MicrosoftApp)> = Vec::new();
        let mut claimed: Vec<String> = Vec::new();
        for p in self.profiles.iter().filter(|p| !p.domains.is_empty()) {
            if let Some(app) = &p.microsoft {
                out.push((p.domains.clone(), app.clone()));
                claimed.extend(p.domains.iter().cloned());
            }
        }
        let window = self.profiles.iter().find(|p| p.default).and_then(|p| p.microsoft.clone()).or_else(|| self.top.microsoft.clone());
        if let Some(app) = window {
            let tenant = app.tenant.as_ref().filter(|t| t.contains('.')).map(|t| t.to_ascii_lowercase());
            let mut rest: Vec<String> = self.domains().into_iter().chain(tenant).filter(|d| !claimed.contains(d)).collect();
            rest.sort();
            rest.dedup();
            if !rest.is_empty() {
                out.push((rest, app));
            }
        }
        out
    }

    /// What applies with accounts of these mail domains.
    pub fn effective(&self, domains: &[String]) -> Effective {
        let relevant: Vec<&Profile> = self.profiles.iter().filter(|p| p.default || p.domains.iter().any(|d| domains.contains(d))).collect();
        let default = self.profiles.iter().find(|p| p.default);
        let mut e = Effective { lookups: Lookups::default(), ..Effective::default() };
        // The window's own: the top, then the default profile over it.
        for p in std::iter::once(&self.top).chain(default) {
            if p.organisation.is_some() {
                e.organisation.clone_from(&p.organisation);
            }
            if p.logo.is_some() {
                e.logo.clone_from(&p.logo);
            }
            if p.contact.is_some() {
                e.contact.clone_from(&p.contact);
            }
            for (k, g) in &p.settings {
                e.settings.insert(k.clone(), g.clone());
            }
            for (mine, theirs) in [(&mut e.theme, &p.theme), (&mut e.language, &p.language)] {
                if theirs.is_some() {
                    mine.clone_from(theirs);
                }
            }
            if p.autoconfig.is_some() {
                e.autoconfig.clone_from(&p.autoconfig);
            }
            if p.microsoft.is_some() {
                e.microsoft.clone_from(&p.microsoft);
            }
        }
        // A lock from the profile of any of the person's domains holds, with its value, where
        // the window's own didn't lock that setting.
        for p in &relevant {
            for (k, g) in p.settings.iter().filter(|(_, g)| g.locked) {
                if !e.settings.get(k).is_some_and(|mine| mine.locked) {
                    e.settings.insert(k.clone(), g.clone());
                }
            }
            for (mine, theirs) in [(&mut e.theme, &p.theme), (&mut e.language, &p.language)] {
                if let Some(t) = theirs.as_ref().filter(|t| t.locked)
                    && !mine.as_ref().is_some_and(|m| m.locked)
                {
                    *mine = Some(t.clone());
                }
            }
        }
        // The policies: the strictest of all that apply.
        let all: Vec<&Profile> = std::iter::once(&self.top).chain(relevant.iter().copied()).collect();
        for p in &all {
            if let Some((u, locked)) = p.updates {
                e.updates = Some(match e.updates {
                    Some((mine, mine_locked)) => (mine.max(u), mine_locked || locked),
                    None => (u, locked),
                });
            }
            e.require_encryption |= p.require_encryption == Some(true);
            e.no_certificate_exceptions |= p.certificate_exceptions == Some(false);
            if let Some(l) = p.lookups {
                e.lookups.domain &= l.domain;
                e.lookups.thunderbird &= l.thunderbird;
                e.lookups.mx &= l.mx;
                e.lookups.dns &= l.dns;
            }
        }
        if all.iter().any(|p| p.only_organisation_domains == Some(true)) {
            e.allowed_domains = Some(self.domains());
        }
        e
    }
}

impl Effective {
    /// Whether `key` (a setting's, as `settings` names it) is locked.
    pub fn locked(&self, key: &str) -> bool {
        self.settings.get(key).is_some_and(|g| g.locked)
    }

    /// Whether an account of `email` may be added: any, unless accounts are kept to the
    /// organisation's domains.
    pub fn allows_address(&self, email: &str) -> bool {
        self.allowed_domains.as_ref().is_none_or(|d| d.contains(&domain_of(email)))
    }
}

#[cfg(test)]
mod tests;
