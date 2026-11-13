//! Who is given the password while a calendar or contacts server is being looked for. The
//! places asked come from the account's address, a DNS answer and redirects, not from the user:
//! the mail password went with the first request to each of them (the domain's web site, a host
//! a forged DNS answer named, wherever a redirect led), every day for an account whose provider
//! has no calendars. While looking, a request goes without it; it's given only to a server
//! that asks for one (401 with a Basic or Digest challenge) and is the account's own.

use super::digest;

/// The account a server is being looked for, and the hosts that asked for its password.
#[derive(Debug, Default)]
pub(crate) struct Guard {
    /// The domain of the account's address, lower case.
    domain: String,
    /// The account's mail server, lower case.
    mail_host: String,
    /// Hosts that asked for the password and may have it: signed in to from their first request.
    asked: Vec<String>,
}

impl Guard {
    pub(crate) fn new(email: &str, mail_host: &str) -> Guard {
        let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).unwrap_or_default();
        Guard { domain, mail_host: mail_host.trim().trim_end_matches('.').to_ascii_lowercase(), asked: Vec::new() }
    }

    /// Whether `host` is the account's own: its address's domain or a host under it, its mail
    /// server, or a host beside the mail server (`caldav.provider.example` for
    /// `imap.provider.example`, where an address at one's own domain is hosted).
    pub(crate) fn own(&self, host: &str) -> bool {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        let under = |parent: &str| !parent.is_empty() && (host == parent || host.strip_suffix(parent).is_some_and(|h| h.ends_with('.')));
        if host.is_empty() {
            return false;
        }
        if under(&self.domain) || host == self.mail_host {
            return true;
        }
        // Beside the mail server, or under the name it's under, when that isn't a registry's
        // (`imap.example.com` has `example.com`; `example.co.uk` shares `co.uk` with no one).
        if crate::http::same_site(&host, &self.mail_host) {
            return true;
        }
        match self.mail_host.split_once('.') {
            Some((_, parent)) if self.mail_host.parse::<std::net::IpAddr>().is_err() && !crate::http::registry(parent) => under(parent),
            _ => false,
        }
    }

    /// Whether `host` has asked for the password before, and is signed in to straight away.
    pub(super) fn asked(&self, host: &str) -> bool {
        self.asked.iter().any(|h| h.eq_ignore_ascii_case(host))
    }

    pub(super) fn note_asked(&mut self, host: &str) {
        if !self.asked(host) && self.asked.len() < 32 {
            self.asked.push(host.to_ascii_lowercase());
        }
    }
}

/// Whether a refusal asks for a password MailCraft can give (Basic or Digest).
pub(super) fn asks_for_password(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("WWW-Authenticate"))
        .flat_map(|(_, v)| digest::challenges(v))
        .any(|(scheme, _)| scheme == "basic" || scheme == "digest")
}
