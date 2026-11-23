//! An organisation's rules for accounts (issue #245): the servers it names for its mail domains
//! (an address and a password are then all an account needs), the ways an address's servers are
//! looked for, the domains accounts may be of, encrypted connections only, and no certificate
//! trusted by hand. They hold whichever way an account comes (the window, `mailcraft-cli`, an
//! AI agent): a job leaves the engine as they allow, or not at all.

use mailcraft_model::{DavSettings, DiscoverRules, Security, ServerConfig};
use serde_json::{Value, json};

use super::*;

/// Said of a connection an organisation that requires encryption doesn't allow.
pub const ENCRYPTION_REQUIRED: &str = "your organisation requires encrypted connections: this account's server settings have one without";
/// Said of a certificate trusted by hand, which the organisation doesn't allow.
pub const NO_EXCEPTIONS: &str = "your organisation doesn't allow trusting a certificate by hand";

fn server_of(s: &crate::policy::Server) -> ServerConfig {
    ServerConfig { host: s.host.clone(), port: s.port, security: Security::from_code(&s.security).unwrap_or(Security::Tls) }
}

impl Session {
    /// How addresses' servers are looked for, as the organisation allows (every way without one).
    pub fn discover_rules(&self) -> DiscoverRules {
        let l = self.org.lookups;
        DiscoverRules { autoconfig: self.org.autoconfig.clone(), domain: l.domain, thunderbird: l.thunderbird, mx: l.mx, dns: l.dns }
    }

    /// The servers the organisation names for `email`'s domain, as an account's settings (the
    /// account form fills itself in from them); `None` when it names none.
    pub fn org_account_config(&self, email: &str) -> Option<AccountConfig> {
        let s = self.organisation.servers_for(email)?;
        let mut c = AccountConfig::guess(email);
        if let Some(i) = &s.imap {
            c.imap = server_of(i);
        }
        if let Some(o) = &s.smtp {
            c.smtp = server_of(o);
        }
        c.pop = s.pop.as_ref().map(server_of);
        if s.caldav.is_some() || s.carddav.is_some() {
            c.dav =
                Some(DavSettings { caldav: s.caldav.clone().unwrap_or_default(), carddav: s.carddav.clone().unwrap_or_default(), username: None });
        }
        c.ews.clone_from(&s.ews);
        c.jmap.clone_from(&s.jmap);
        if s.graph {
            c.graph = Some(mailcraft_model::GRAPH_URL.to_string());
        }
        Some(c)
    }

    /// `account.add`'s params, with the organisation's servers for the address put in where the
    /// params name no servers of their own.
    pub(crate) fn with_org_servers(&self, v: &Value) -> Value {
        let mut v = v.clone();
        let Some(email) = v.get("email").and_then(Value::as_str).map(str::to_string) else { return v };
        let Some(s) = self.organisation.servers_for(&email).cloned() else { return v };
        let Some(o) = v.as_object_mut() else { return v };
        let named = ["imapHost", "smtpHost", "popHost", "jmap", "ews", "graph", "pop", "dav", "local"].iter().any(|k| o.contains_key(*k));
        if named {
            return v;
        }
        let mut put = |prefix: &str, server: &crate::policy::Server| {
            o.insert(format!("{prefix}Host"), json!(server.host));
            o.insert(format!("{prefix}Port"), json!(server.port));
            o.insert(format!("{prefix}Security"), json!(server.security));
        };
        if let Some(i) = &s.imap {
            put("imap", i);
        }
        if let Some(p) = &s.pop {
            put("pop", p);
        }
        if let Some(sm) = &s.smtp {
            put("smtp", sm);
        }
        if s.pop.is_some() && s.imap.is_none() {
            o.insert("pop".into(), json!(true));
        }
        if let Some(e) = &s.ews {
            o.insert("ews".into(), json!(e));
        } else if let Some(j) = &s.jmap {
            o.insert("jmap".into(), json!(j));
        } else if s.graph {
            o.insert("graph".into(), json!(true));
        }
        for (k, val) in [("caldav", &s.caldav), ("carddav", &s.carddav)] {
            if let Some(u) = val
                && !o.contains_key(k)
            {
                o.insert(k.into(), json!(u));
            }
        }
        v
    }

    /// Why the organisation doesn't allow an account with `c`'s servers: a connection without
    /// encryption, where encryption is required. `None`: allowed.
    pub fn org_refuses_servers(&self, c: &AccountConfig) -> Option<&'static str> {
        if !self.org.require_encryption || c.is_local() && !c.is_calendar_only() {
            return None;
        }
        let plain_web = |u: &str| u.trim().to_ascii_lowercase().starts_with("http://");
        let mail = !c.is_api() && !c.is_local();
        let unencrypted = mail && c.pop.is_none() && c.imap.security == Security::None
            || mail && c.smtp.security == Security::None
            || c.pop.as_ref().is_some_and(|p| p.security == Security::None)
            || [&c.ews, &c.jmap, &c.graph].into_iter().flatten().any(|u| plain_web(u))
            || c.dav.as_ref().is_some_and(|d| plain_web(&d.caldav) || plain_web(&d.carddav));
        unencrypted.then_some(ENCRYPTION_REQUIRED)
    }

    /// A job as the organisation allows it, on its way out of the engine: an account's
    /// certificate exceptions taken out when none are allowed, and its start held back (its
    /// status saying why) when its connection isn't encrypted and encryption is required.
    pub(crate) fn org_job(&mut self, mut job: Job) -> Option<Job> {
        // A calendar subscribed to before the profile, in the clear: not read.
        if let Job::FetchCalendar { url, .. } = &job
            && self.org.require_encryption
            && url.trim().to_ascii_lowercase().starts_with("http://")
        {
            return None;
        }
        let config = match &mut job {
            Job::Start { account, .. }
            | Job::SmtpPassword { account, .. }
            | Job::DavPassword { account, .. }
            | Job::SignIn { account }
            | Job::AllowDirectory { account } => Some(account),
            _ => None,
        };
        let Some(config) = config else { return Some(job) };
        if self.org.no_certificate_exceptions {
            config.trusted_certificates.clear();
            config.trusted_authorities.clear();
        }
        if let Some(why) = self.org_refuses_servers(config) {
            let id = config.id.clone();
            if let Some(a) = self.accounts.iter_mut().find(|a| a.config.id == id) {
                a.status = mailcraft_model::SyncStatus::error(why);
            }
            return None;
        }
        Some(job)
    }
}
