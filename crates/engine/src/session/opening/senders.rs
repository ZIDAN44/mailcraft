//! Who a message says it's from (issue #29): the sender check the reading pane and the message
//! list share, gathered once from the accounts, the address book, the people written to and the
//! directory's answers, and counted again as those change.

use mailcraft_model::lookalike::{self, KnownPeople, SenderCheck, SenderWarning};

use super::*;

/// How long the sender check is kept while the address book waits to be counted again (mail
/// arriving drops it after every sync), in seconds.
const FRESH_SECS: i64 = 600;

/// The sender check and what it was gathered from.
#[derive(Clone, Debug, Default)]
pub(crate) struct Senders {
    at: i64,
    stamp: Stamp,
    check: SenderCheck,
    /// The organisation's domains, for the External tag (empty: none).
    org: BTreeSet<String>,
}

/// What the check was gathered from: when any of it changes, it's gathered again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Stamp {
    accounts: Vec<(String, String)>,
    pim: u64,
    directory: u64,
    book: u64,
    org: Vec<String>,
}

/// What the message list shows about a sender (issue #29).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SenderMarks {
    /// Show the address after the name.
    pub address: bool,
    /// Something about the sender doesn't add up (the reading pane says what).
    pub warning: bool,
    /// From outside the organisation (with the External tag on).
    pub external: bool,
}

impl Session {
    fn senders_stamp(&self) -> Stamp {
        Stamp {
            accounts: self.accounts.iter().map(|a| (a.config.email.clone(), a.config.name.clone())).collect(),
            pim: self.pim_revision,
            directory: self.directory.rev,
            book: self.contacts_built,
            org: self.prefs.org_domains.clone(),
        }
    }

    /// Gather the sender check again when what it's made of changed, or when it's old and the
    /// address book has been dropped (cheap otherwise: called as the list is refreshed).
    pub fn keep_senders(&mut self) {
        self.keep_senders_when(true);
    }

    /// [`Session::keep_senders`]; with `by_age` false, only when what it's made of changed (each
    /// frame: an idle window doesn't count the address book again every few minutes).
    pub(crate) fn keep_senders_when(&mut self, by_age: bool) {
        let stamp = self.senders_stamp();
        let fresh = self
            .senders
            .as_ref()
            .is_some_and(|s| s.stamp == stamp && (!by_age || self.contacts.is_some() || self.clock.now.saturating_sub(s.at) < FRESH_SECS));
        if fresh {
            return;
        }
        self.ensure_contacts();
        let mut domains: BTreeSet<String> = self.accounts.iter().map(|a| lookalike::domain_of(&a.config.email)).collect();
        let mut people = KnownPeople::new();
        let mut own: Vec<String> = Vec::new();
        for a in &self.accounts {
            for email in [&a.config.email, &a.config.username] {
                if addr::is_valid_email(email.trim()) {
                    people.add(&EmailAddress::new(a.config.name.clone(), email.clone()));
                    own.push(email.clone());
                }
            }
        }
        // Saved contacts by the name on their card first (warnings call people by the first name
        // they're known by; the book keeps the newest name seen).
        for p in self.pim.values() {
            for c in &p.cards {
                for e in &c.card.emails {
                    people.add(&EmailAddress::new(c.card.display(), e.value.clone()));
                }
            }
        }
        // The organisation's directory, as far as it has been asked.
        for answer in self.directory.answers.values() {
            for p in &answer.people {
                people.add(p);
            }
        }
        if let Some(book) = &self.contacts {
            domains.extend(book.written_domains());
            for c in book.written() {
                people.add(&c.address);
            }
        }
        domains.remove("");
        let listed: BTreeSet<String> = self.prefs.org_domains.iter().filter_map(|d| lookalike::clean_domain(d)).collect();
        let org = if listed.is_empty() { lookalike::own_domains(self.accounts.iter().map(|a| a.config.email.as_str())) } else { listed };
        let stamp = self.senders_stamp();
        let check = SenderCheck::new(domains, people, own);
        // What's open says what the check says now (a sender just saved as a contact is fine).
        if let Some(r) = self.reading.as_mut()
            && let Some(p) = &r.parsed
        {
            let listed = self.list.iter().find(|m| m.key == r.key).is_some_and(|m| m.bulk);
            r.warning = check.check(&p.from, listed);
        }
        for w in self.windows.iter_mut() {
            if let Some(p) = &w.parsed {
                let listed = self.list.iter().find(|m| m.key == w.key).is_some_and(|m| m.bulk);
                w.warning = check.check(&p.from, listed);
            }
        }
        self.senders = Some(Senders { at: self.clock.now, stamp, check, org });
    }

    /// The domains the user deals with: their own accounts', and those of the people they write
    /// to or keep in an address book.
    pub fn known_domains(&mut self) -> BTreeSet<String> {
        self.keep_senders();
        self.senders.as_ref().map(|s| s.check.domains().clone()).unwrap_or_default()
    }

    /// The organisation's domains, for the External tag: the ones the user listed, else those of
    /// their accounts (mail services left out). Empty: there's no telling who's external.
    pub fn org_domains(&mut self) -> BTreeSet<String> {
        self.keep_senders();
        self.senders.as_ref().map(|s| s.org.clone()).unwrap_or_default()
    }

    /// What doesn't add up about a sender (nothing for the user's own messages).
    pub fn sender_warning(&mut self, from: &EmailAddress) -> Option<SenderWarning> {
        self.sender_warning_listed(from, false)
    }

    /// [`Session::sender_warning`] for a message that came through a mailing list (or in bulk):
    /// a list sends its members' mail under their names, from its own address.
    pub fn sender_warning_listed(&mut self, from: &EmailAddress, listed: bool) -> Option<SenderWarning> {
        self.keep_senders();
        self.senders.as_ref().and_then(|s| s.check.check(from, listed))
    }

    /// The sender warning for a message in the list (its header says whether it came in bulk).
    pub(crate) fn message_warning(&mut self, key: &MessageKey, from: &EmailAddress) -> Option<SenderWarning> {
        let listed = self.summary(key).is_some_and(|m| m.bulk);
        self.sender_warning_listed(from, listed)
    }

    /// The organisation's domains as the sender check last gathered them (`None` before it's
    /// gathered).
    pub fn organisation(&self) -> Option<&BTreeSet<String>> {
        self.senders.as_ref().map(|s| &s.org)
    }

    /// Whether `email` is from outside the organisation, with the External tag on.
    pub fn is_external(&self, email: &str) -> bool {
        self.prefs.external_tag && self.senders.as_ref().is_some_and(|s| lookalike::is_external(email, &s.org))
    }

    /// What the message list shows about a message's sender, as the check last gathered knows
    /// (nothing before it's gathered, nor for the user's own mail).
    pub fn sender_marks(&self, m: &Summary) -> SenderMarks {
        let Some(s) = &self.senders else { return SenderMarks::default() };
        let email = m.from.email.trim();
        if email.is_empty() || s.check.is_own(email) {
            return SenderMarks::default();
        }
        let named = !m.from.name.trim().is_empty() && !m.from.name.trim().eq_ignore_ascii_case(email);
        let address = named
            && match self.prefs.sender_addresses {
                SenderAddresses::Always => true,
                SenderAddresses::Strangers => !s.check.knows(email),
                SenderAddresses::Never => false,
            };
        SenderMarks { address, warning: s.check.check(&m.from, m.bulk).is_some(), external: self.is_external(email) }
    }
}
