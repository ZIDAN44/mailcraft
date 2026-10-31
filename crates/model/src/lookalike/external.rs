//! Senders from outside the organisation (issue #29): what Exchange's transport rules tag
//! "[EXTERNAL]", done in the client so it works with any server. The organisation is a list of
//! domains: the user's own, or the domains of their accounts when they haven't given one. A mail
//! service is never the organisation by itself: someone whose own address is at gmail.com
//! doesn't work with everyone else there.

use std::collections::BTreeSet;

use super::{domain_of, is_mail_service};

/// Mail services that sign anyone up, by the first label of their domains in each country
/// (`yahoo.co.uk`, `outlook.com.br`, `gmx.ch`).
const MAIL_SERVICE_NAMES: &[&str] = &[
    "aol",
    "gmail",
    "gmx",
    "googlemail",
    "hotmail",
    "icloud",
    "live",
    "mailbox",
    "msn",
    "outlook",
    "posteo",
    "proton",
    "protonmail",
    "rocketmail",
    "tutanota",
    "yahoo",
    "yandex",
    "ymail",
    "zoho",
];

/// Whether anyone can have an address at `domain`: one of the big mail services
/// ([`is_mail_service`]), or one of them in another country (`yahoo.co.uk`, `hotmail.fr`,
/// `outlook.com.br`). `mail.yahoo.example.com` is someone's own domain.
pub fn is_free_mail(domain: &str) -> bool {
    let d = domain.trim().trim_end_matches('.').to_lowercase();
    if is_mail_service(&d) {
        return true;
    }
    let Some((name, rest)) = d.split_once('.') else { return false };
    // What's left is a country's or a generic ending (`fr`, `co.uk`, `com.br`), not a domain.
    let ending = match rest.split_once('.') {
        None => !rest.is_empty(),
        Some((second, tld)) => second.len() <= 3 && !tld.is_empty() && !tld.contains('.'),
    };
    ending && MAIL_SERVICE_NAMES.contains(&name)
}

/// The organisation's domains when the user hasn't listed them: those of their own accounts,
/// mail services left out.
pub fn own_domains<'a>(own: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    own.into_iter().map(domain_of).filter(|d| !d.is_empty() && !is_free_mail(d)).collect()
}

/// A domain as the user lists it (`@Corp.Example.`, ` corp.example `) → `corp.example`; `None`
/// for one that can't be a domain.
pub fn clean_domain(d: &str) -> Option<String> {
    let d = d.trim().trim_start_matches('@').trim_end_matches('.').to_lowercase();
    let ok = !d.is_empty()
        && d.len() <= 253
        && d.contains('.')
        && d.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_alphanumeric() || c == '-'));
    ok.then_some(d)
}

/// Whether `email` is from outside the organisation `org` (its domains, and the domains under
/// them). Never with no organisation, nor for something that isn't an address.
pub fn is_external(email: &str, org: &BTreeSet<String>) -> bool {
    let domain = domain_of(email);
    !org.is_empty() && !domain.is_empty() && !org.iter().any(|o| domain == *o || domain.strip_suffix(o.as_str()).is_some_and(|r| r.ends_with('.')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_services_in_every_country() {
        for d in ["gmail.com", "GMAIL.COM.", "yahoo.co.uk", "hotmail.fr", "outlook.com.br", "gmx.ch", "yahoo.co.jp", "proton.me"] {
            assert!(is_free_mail(d), "{d}");
        }
        for d in ["corp.example", "outlook.corp.example", "mail.yahoo.example.com", "gmail", "", "yahoo.", "mail.example"] {
            assert!(!is_free_mail(d), "{d}");
        }
    }

    #[test]
    fn the_organisation_is_the_users_own_domains() {
        let org = own_domains(["me@corp.example", "me@gmail.com", "me@yahoo.co.uk", "not an address"]);
        assert_eq!(org, BTreeSet::from(["corp.example".to_string()]));
        // Only mail services: no organisation, so nobody is external.
        let none = own_domains(["me@gmail.com"]);
        assert!(none.is_empty());
        assert!(!is_external("anyone@else.example", &none));
    }

    #[test]
    fn outside_senders_are_external() {
        let org = BTreeSet::from(["corp.example".to_string()]);
        assert!(!is_external("ana@corp.example", &org));
        assert!(!is_external("ana@EU.Corp.Example", &org));
        assert!(is_external("ana@corp.example.evil.example", &org));
        assert!(is_external("ana@notcorp.example", &org));
        assert!(is_external("jane@gmail.com", &org));
        assert!(!is_external("", &org));
        assert!(!is_external("no address", &org));
    }

    #[test]
    fn listed_domains_are_cleaned() {
        assert_eq!(clean_domain(" @Corp.Example. ").as_deref(), Some("corp.example"));
        for bad in ["", "corp", "a..b", "@", "a b.example", "x/y.example", &"a.".repeat(200)] {
            assert_eq!(clean_domain(bad), None, "{bad}");
        }
    }
}
