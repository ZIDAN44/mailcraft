//! People's photos (issue #223), shown where their initials were: a contact's own, as its card
//! has it (vCard's `PHOTO`, inline), else the one the organisation has, asked of an on-premises
//! Exchange account's own server as the person is shown ([`Job::FetchPhoto`]). A photo at a web
//! address isn't fetched: only the user's own servers are asked anything.

use std::collections::HashMap;
use std::sync::Arc;

use super::*;

/// The most people whose photo, or that they have none, is kept.
const MAX_KNOWN: usize = 2_000;
/// The most asked of servers at once.
const MAX_ASKING: usize = 8;
/// How long a server's "none", or a failure to ask, stands before it's asked again.
const NONE_FOR: i64 = 6 * 3600;
/// How long a question unanswered stands (a worker gone) before it's asked again.
const ASKING_FOR: i64 = 120;
/// The largest photo kept, and the longest `PHOTO` value read.
const MAX_BYTES: usize = 512 * 1024;

/// What's known of each person's photo, by address in lower case.
#[derive(Debug, Default)]
pub struct Photos {
    known: HashMap<String, Known>,
    /// The contacts' photos, by address, as read at `cards_at` ([`Session::pim_revision`]).
    cards: HashMap<String, Arc<[u8]>>,
    cards_at: Option<u64>,
    /// Bumped whenever a photo comes or goes: what the window drew from may have changed.
    pub rev: u64,
}

#[derive(Debug)]
enum Known {
    Asking { since: i64 },
    Have(Arc<[u8]>),
    None { at: i64 },
}

/// Lower case, trimmed: how photos are kept.
fn key(email: &str) -> String {
    email.trim().to_lowercase()
}

/// A card's own photo, when its `PHOTO` is in it (vCard 3's `ENCODING=b`, vCard 4's `data:`
/// URI); `None` for one at a web address, or none.
pub fn card_photo(card: &mailcraft_model::vcard::Card) -> Option<Vec<u8>> {
    for line in &card.extra {
        let Some((head, value)) = line.split_once(':') else { continue };
        let mut parts = head.split(';');
        let name = parts.next().unwrap_or_default();
        // A group before the name (`item1.PHOTO`).
        let name = name.rsplit('.').next().unwrap_or(name);
        if !name.eq_ignore_ascii_case("PHOTO") || value.len() > MAX_BYTES * 2 {
            continue;
        }
        let base64 = parts.any(|p| {
            let p = p.trim().to_ascii_lowercase();
            p == "encoding=b" || p == "encoding=base64"
        });
        let data = if base64 {
            value
        } else {
            match value.trim().split_once(',') {
                Some((kind, data)) if kind.to_ascii_lowercase().starts_with("data:image/") && kind.to_ascii_lowercase().ends_with(";base64") => data,
                _ => continue,
            }
        };
        if let Some(bytes) = mailcraft_mime::decode_base64(data).filter(|b| !b.is_empty() && b.len() <= MAX_BYTES) {
            return Some(bytes);
        }
    }
    None
}

impl Session {
    /// The photo of the person at `email`, when one's known: their contact's, else their
    /// organisation's.
    pub fn photo(&self, email: &str) -> Option<Arc<[u8]>> {
        let k = key(email);
        if let Some(p) = self.photos.cards.get(&k) {
            return Some(p.clone());
        }
        match self.photos.known.get(&k) {
            Some(Known::Have(p)) => Some(p.clone()),
            _ => None,
        }
    }

    /// The people shown now: their contacts' photos read (when the address books changed), and
    /// the organisation's server asked for those it hasn't been yet.
    pub fn want_photos(&mut self, emails: &[String]) {
        if self.photos.cards_at != Some(self.pim_revision) {
            self.read_card_photos();
        }
        let Some(account) = self.photo_account() else { return };
        let now = self.clock.now;
        let mut asking = self.photos.known.values().filter(|k| matches!(k, Known::Asking { since } if now - since < ASKING_FOR)).count();
        for email in emails {
            let k = key(email);
            if asking >= MAX_ASKING || !k.contains('@') || k.len() > 320 || self.photos.cards.contains_key(&k) {
                continue;
            }
            let stale = match self.photos.known.get(&k) {
                None => true,
                Some(Known::Have(_)) => false,
                Some(Known::Asking { since }) => now - since >= ASKING_FOR,
                Some(Known::None { at }) => now - at >= NONE_FOR,
            };
            if !stale {
                continue;
            }
            if self.photos.known.len() >= MAX_KNOWN && !self.photos.known.contains_key(&k) {
                // The answers with no photo go first; those with one if that's not enough.
                self.photos.known.retain(|_, v| !matches!(v, Known::None { .. }));
                if self.photos.known.len() >= MAX_KNOWN {
                    self.photos.known.clear();
                    self.photos.rev = self.photos.rev.wrapping_add(1);
                }
            }
            self.photos.known.insert(k.clone(), Known::Asking { since: now });
            self.jobs.push(Job::FetchPhoto { account: account.clone(), email: k });
            asking += 1;
        }
    }

    /// What a server said of someone's photo.
    pub(crate) fn photo_came(&mut self, email: &str, photo: Option<Vec<u8>>, error: Option<String>) {
        let k = key(email);
        // Only an answer asked for is kept (a server can't put photos here unasked).
        if !matches!(self.photos.known.get(&k), Some(Known::Asking { .. })) {
            return;
        }
        if let Some(e) = &error {
            log::info!("photo: {e}");
        }
        let now = self.clock.now;
        let known = match photo.filter(|p| !p.is_empty() && p.len() <= MAX_BYTES) {
            Some(p) => Known::Have(p.into()),
            None => Known::None { at: now },
        };
        self.photos.known.insert(k, known);
        self.photos.rev = self.photos.rev.wrapping_add(1);
    }

    /// The account the organisation's photos are asked of: an on-premises Exchange one.
    fn photo_account(&self) -> Option<String> {
        self.accounts.iter().find(|a| a.config.ews.is_some() && !a.is_local()).map(|a| a.config.id.clone())
    }

    /// Every contact's photo, by each of its addresses (the first card with one wins).
    fn read_card_photos(&mut self) {
        let mut cards: HashMap<String, Arc<[u8]>> = HashMap::new();
        for p in self.pim.values() {
            for c in &p.cards {
                let emails: Vec<String> = c.card.emails.iter().map(|e| key(&e.value)).filter(|e| !cards.contains_key(e)).collect();
                if emails.is_empty() {
                    continue;
                }
                let Some(photo) = card_photo(&c.card) else { continue };
                let photo: Arc<[u8]> = photo.into();
                for e in emails {
                    if cards.len() < MAX_KNOWN {
                        cards.insert(e, photo.clone());
                    }
                }
            }
        }
        self.photos.rev = self.photos.rev.wrapping_add(1);
        self.photos.cards = cards;
        self.photos.cards_at = Some(self.pim_revision);
    }
}
