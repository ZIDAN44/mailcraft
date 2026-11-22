//! An address book built from mail already in the cache, for To/Cc suggestions.
//!
//! Every sender and recipient seen counts. People you write to (the recipients of your own
//! messages) rank well above people who only write to you, and recent contact counts more than
//! old: score = 4 · sent + received, halved for every year since the last message.

use std::collections::HashMap;

use crate::{EmailAddress, Summary};

/// One person.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contact {
    pub address: EmailAddress,
    /// Messages you sent them.
    pub sent: u32,
    /// Messages from them (or with them in copy).
    pub received: u32,
    /// The newest message involving them (Unix seconds).
    pub last: i64,
}

impl Contact {
    fn score(&self, now: i64) -> f64 {
        let years = (now.saturating_sub(self.last)).max(0) as f64 / (365.0 * 86_400.0);
        (4.0 * f64::from(self.sent) + f64::from(self.received)) * 0.5f64.powf(years)
    }
}

/// Most addresses seen; a mailbox full of newsletters stays bounded.
const MAX_CONTACTS: usize = 20_000;

/// A contact as the book keeps it: with its name and address in lower case, as every search
/// compares them (worked out once, not for each contact on each key typed).
#[derive(Clone, Debug, Default)]
struct Entry {
    contact: Contact,
    name_lower: String,
    email_lower: String,
}

impl Entry {
    /// What the address book's list is in order of: the name, or the address without one.
    fn sort_key(&self) -> &str {
        if self.name_lower.is_empty() { &self.email_lower } else { &self.name_lower }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ContactBook {
    by_email: HashMap<String, Entry>,
}

impl ContactBook {
    pub fn new() -> ContactBook {
        ContactBook::default()
    }

    pub fn len(&self) -> usize {
        self.by_email.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_email.is_empty()
    }

    /// What the book knows about an address.
    pub fn get(&self, email: &str) -> Option<&Contact> {
        self.by_email.get(&email.trim().to_ascii_lowercase()).map(|e| &e.contact)
    }

    /// Count `weight` messages with `a` on `date`: sent to them, or from them.
    fn note(&mut self, a: &EmailAddress, date: i64, sent: bool, weight: u32) {
        let email = a.email.trim().to_ascii_lowercase();
        if !crate::addr::is_valid_email(&email) || email.contains("noreply") || email.contains("no-reply") || email.starts_with("mailer-daemon@") {
            return;
        }
        if self.by_email.len() >= MAX_CONTACTS && !self.by_email.contains_key(&email) {
            return;
        }
        let e = self.by_email.entry(email.clone()).or_insert_with(|| Entry {
            email_lower: email.to_lowercase(),
            contact: Contact { address: EmailAddress::new("", email), ..Default::default() },
            name_lower: String::new(),
        });
        let c = &mut e.contact;
        // The newest non-empty name wins (people fix their display names).
        if !a.name.trim().is_empty() && (c.address.name.is_empty() || date >= c.last) {
            c.address.name = a.name.trim().to_string();
            e.name_lower = c.address.name.to_lowercase();
        }
        if sent {
            c.sent = c.sent.saturating_add(weight);
        } else {
            c.received = c.received.saturating_add(weight);
        }
        c.last = c.last.max(date);
    }

    /// Someone saved in an address book: ranked as if written to often (eight messages).
    pub fn add_saved(&mut self, a: &EmailAddress, now: i64) {
        self.note(a, now, true, 8);
    }

    /// Count a message. `mine` are the user's own addresses: a message from one of them is one
    /// you sent, so its recipients are people you write to.
    pub fn add(&mut self, m: &Summary, mine: &[String]) {
        let from_me = mine.iter().any(|e| e.eq_ignore_ascii_case(m.from.email.trim()));
        if from_me {
            for a in m.to.iter().chain(&m.cc) {
                if !mine.iter().any(|e| e.eq_ignore_ascii_case(a.email.trim())) {
                    self.note(a, m.date, true, 1);
                }
            }
        } else {
            self.note(&m.from, m.date, false, 1);
        }
    }

    /// Count a meeting in the user's calendar on `date` (issue #253): the guests of one the user
    /// organizes as people they write to, the organizer and the other guests of someone else's
    /// as people who write to them. A guest is otherwise known only by the invitation's copy in
    /// Sent, and the guests of a meeting made elsewhere (the server sends its invitations) never.
    pub fn add_meeting(&mut self, organizer: Option<&EmailAddress>, guests: &[EmailAddress], date: i64, mine: &[String]) {
        let is_mine = |a: &EmailAddress| mine.iter().any(|e| e.eq_ignore_ascii_case(a.email.trim()));
        let own = organizer.is_none_or(is_mine);
        for a in organizer.filter(|o| !is_mine(o)).into_iter().chain(guests.iter().filter(|g| !is_mine(g))) {
            self.note(a, date, own, 1);
        }
    }

    /// The domains of the people the user writes to (or saved in an address book): the ones a
    /// lookalike sender would imitate ([`crate::lookalike`]).
    pub fn written_domains(&self) -> impl Iterator<Item = String> + '_ {
        self.written().map(|c| crate::lookalike::domain_of(&c.address.email)).filter(|d| !d.is_empty())
    }

    /// The people the user writes to (or saved in an address book).
    pub fn written(&self) -> impl Iterator<Item = &Contact> + '_ {
        self.by_email.values().map(|e| &e.contact).filter(|c| c.sent > 0)
    }

    /// Up to `limit` contacts matching `query` (the start of the address, of the name, or of any
    /// word of the name; case-insensitive), best first.
    pub fn suggest(&self, query: &str, limit: usize, now: i64) -> Vec<Contact> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<(&Contact, f64)> = self
            .by_email
            .values()
            .filter(|e| {
                let (name, email) = (&e.name_lower, &e.contact.address.email);
                email.starts_with(&q)
                    || name.starts_with(&q)
                    || name.split(|ch: char| ch.is_whitespace() || ch == '-' || ch == '.').any(|w| w.starts_with(&q))
                    || email.split(['.', '_', '-', '@']).any(|w| w.starts_with(&q))
            })
            .map(|e| (&e.contact, e.contact.score(now)))
            .collect();
        hits.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.address.email.cmp(&b.0.address.email)));
        hits.into_iter().take(limit).map(|(c, _)| c.clone()).collect()
    }
}

impl ContactBook {
    /// Everyone known whose name or address contains `query` (everyone for an empty one), by
    /// name (by address without one, and by address among the same names): the address book's
    /// list. Only the `limit` listed are put in order and copied.
    pub fn list(&self, query: &str, limit: usize) -> Vec<Contact> {
        let q = query.trim().to_lowercase();
        let mut v: Vec<&Entry> = self.by_email.values().filter(|e| q.is_empty() || e.name_lower.contains(&q) || e.email_lower.contains(&q)).collect();
        let order = |a: &&Entry, b: &&Entry| a.sort_key().cmp(b.sort_key()).then_with(|| a.email_lower.cmp(&b.email_lower));
        if limit < v.len() {
            v.select_nth_unstable_by(limit, order);
            v.truncate(limit);
        }
        v.sort_unstable_by(order);
        v.into_iter().map(|e| e.contact.clone()).collect()
    }
}

/// The address being typed in a recipient box: the text after the last `,` or `;`.
pub fn last_token(field: &str) -> &str {
    field.rsplit([',', ';']).next().unwrap_or("").trim_start()
}

/// `field` with its last, half-typed address replaced by `a` and a separator for the next one.
pub fn complete(field: &str, a: &EmailAddress) -> String {
    let keep = match field.rfind([',', ';']) {
        Some(i) => format!("{}, ", field.get(..i).unwrap_or("").trim_end()),
        None => String::new(),
    };
    format!("{keep}{a}, ")
}

/// Backspace right after a whole recipient (`Name <address>`, or one ended by `,` or `;`):
/// the field without it and where the cursor goes (characters), as Outlook takes a resolved name
/// away in one go. `None` when the cursor is inside a half-typed address (an ordinary backspace).
pub fn remove_recipient(field: &str, cursor: usize) -> Option<(String, usize)> {
    let chars: Vec<char> = field.chars().collect();
    let cursor = cursor.min(chars.len());
    // Separators outside quotes and angle brackets.
    let mut seps: Vec<usize> = Vec::new();
    let (mut quoted, mut angle) = (false, false);
    for (i, c) in chars.iter().enumerate().take(cursor) {
        match c {
            '"' if !angle => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' if !quoted && !angle => seps.push(i),
            _ => {}
        }
    }
    if quoted || angle {
        return None;
    }
    let mut end = cursor;
    while end > 0 && chars.get(end - 1).is_some_and(|c| c.is_whitespace()) {
        end -= 1;
    }
    let last = *chars.get(end.checked_sub(1)?)?;
    let entry_end = match last {
        ',' | ';' => end - 1,
        '>' => end,
        _ => return None,
    };
    let start = seps.iter().rev().find(|s| **s < entry_end).map_or(0, |s| s + 1);
    let entry: String = chars.get(start..entry_end)?.iter().collect();
    if entry.trim().is_empty() {
        return None;
    }
    // A plain address ended by a separator counts too; a half-typed word before `>` doesn't.
    if last == '>' && !entry.contains('<') {
        return None;
    }
    let before: String = chars.get(..start).unwrap_or(&[]).iter().collect();
    let after: String = chars.get(cursor..).unwrap_or(&[]).iter().collect();
    let before = if after.trim().is_empty() && start > 0 { format!("{} ", before.trim_end()) } else { before };
    let at = before.chars().count();
    Some((format!("{before}{}", after.trim_start()), at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MessageKey;

    /// Issue #1: a wrong address picked from the suggestions goes with one Backspace.
    #[test]
    fn backspace_takes_a_whole_recipient() {
        let f = "Ana Lima <ana@x.example>, Zoe <zoe@x.example>, ";
        let (out, at) = remove_recipient(f, f.chars().count()).unwrap();
        assert_eq!(out, "Ana Lima <ana@x.example>, ");
        assert_eq!(at, out.chars().count());
        let (out, _) = remove_recipient(&out, out.chars().count()).unwrap();
        assert_eq!(out, "");
        let f = "\"Park, Zoe\" <zoe@x.example>";
        assert_eq!(remove_recipient(f, f.chars().count()).unwrap().0, "", "a comma in a quoted name isn't a separator");
        assert_eq!(remove_recipient("ana@x.example; ", 15).unwrap().0, "");
        // Half-typed, or inside an address: an ordinary backspace.
        assert_eq!(remove_recipient("Ana Lima <ana@x.example>, zo", 28), None);
        assert_eq!(remove_recipient("Ana <ana@x.exa", 14), None);
        assert_eq!(remove_recipient("", 0), None);
        assert_eq!(remove_recipient(", ", 2), None);
        // In the middle: the one before the cursor goes, the rest stays.
        let f = "Ana <a@x.example>, Zoe <z@x.example>";
        let (out, at) = remove_recipient(f, 18).unwrap();
        assert_eq!((out.as_str(), at), ("Zoe <z@x.example>", 0));
        assert_eq!(remove_recipient("abc", 99), None);
    }

    fn msg(from: (&str, &str), to: &[(&str, &str)], date: i64) -> Summary {
        Summary {
            key: MessageKey::default(),
            from: EmailAddress::new(from.0, from.1),
            to: to.iter().map(|(n, e)| EmailAddress::new(*n, *e)).collect(),
            date,
            ..Default::default()
        }
    }

    /// Issue #253: a meeting's guests known as its mail's would make them, by whose it is.
    #[test]
    fn a_meetings_guests_are_people_one_writes_with() {
        let me = vec!["me@studio.example".to_string()];
        let mut b = ContactBook::new();
        let now = 2_000_000_000;
        let guests = [EmailAddress::new("Me", "ME@studio.example"), EmailAddress::new("Kim Ito", "kim@studio.example")];
        b.add_meeting(Some(&EmailAddress::new("", "me@studio.example")), &guests, now, &me);
        b.add_meeting(None, &[EmailAddress::new("", "kim@studio.example")], now, &me);
        let kim = b.get("kim@studio.example").expect("known");
        assert_eq!((kim.sent, kim.received, kim.address.name.as_str()), (2, 0, "Kim Ito"), "invited by the user: written to");
        assert!(b.get("me@studio.example").is_none(), "not the user");
        // Someone else's meeting: its organizer and its other guests write to the user.
        let other = [EmailAddress::new("", "me@studio.example"), EmailAddress::new("", "sam@studio.example")];
        b.add_meeting(Some(&EmailAddress::new("Zoe Park", "zoe@studio.example")), &other, now, &me);
        assert_eq!(b.get("zoe@studio.example").map(|c| (c.sent, c.received)), Some((0, 1)));
        assert_eq!(b.get("sam@studio.example").map(|c| (c.sent, c.received)), Some((0, 1)));
        assert_eq!(b.suggest("ki", 5, now)[0].address.email, "kim@studio.example");
    }

    #[test]
    fn people_you_write_to_come_first() {
        let me = vec!["me@studio.example".to_string()];
        let mut b = ContactBook::new();
        let now = 2_000_000_000;
        for _ in 0..3 {
            b.add(&msg(("Zara Newsletter", "zara@news.example"), &[("Me", "me@studio.example")], now - 100), &me);
        }
        b.add(&msg(("Me", "me@studio.example"), &[("Zoe Park", "zoe@studio.example")], now - 100), &me);
        b.add(&msg(("Me", "me@studio.example"), &[("Zoe Park", "ZOE@studio.example")], now - 50), &me);
        b.add(&msg(("Robot", "noreply@x.example"), &[], now), &me);
        assert_eq!(b.len(), 2, "no-reply senders and my own address are left out");
        let s = b.suggest("z", 5, now);
        let emails: Vec<&str> = s.iter().map(|c| c.address.email.as_str()).collect();
        assert_eq!(emails, vec!["zoe@studio.example", "zara@news.example"]);
        assert_eq!(s[0].sent, 2);
        assert_eq!(s[0].address.name, "Zoe Park");
        // Any word of the name, or the domain, matches.
        assert_eq!(b.suggest("park", 5, now).len(), 1);
        assert_eq!(b.suggest("news", 5, now).len(), 1);
        assert!(b.suggest("  ", 5, now).is_empty());
        assert!(b.suggest("q", 5, now).is_empty());
    }

    #[test]
    fn old_contacts_fade() {
        let me = vec!["me@x.example".to_string()];
        let mut b = ContactBook::new();
        let year = 365 * 86_400;
        let now = 10 * year;
        b.add(&msg(("Me", "me@x.example"), &[("Ann Old", "ann.old@x.example")], now - 5 * year), &me);
        b.add(&msg(("Ann New", "ann.new@x.example"), &[], now), &me);
        let s = b.suggest("ann", 5, now);
        assert_eq!(s[0].address.email, "ann.new@x.example");
    }

    /// A full book: what the address book's search and the To box's suggestions cost per key.
    #[test]
    #[ignore = "a measurement: cargo test --release -p mailcraft-model --lib contacts_per_key -- --ignored --nocapture"]
    fn contacts_per_key() {
        let me = vec!["me@studio.example".to_string()];
        let mut b = ContactBook::new();
        let now = 2_000_000_000;
        let started = std::time::Instant::now();
        for i in 0..MAX_CONTACTS {
            let them = EmailAddress::new(format!("Person{i} Family{}", i % 97), format!("p{i}@d{}.example", i % 50));
            if i % 3 == 0 {
                b.add_saved(&them, now);
            } else {
                b.add(&msg((&them.name, &them.email), &[("Me", "me@studio.example")], now - i as i64), &me);
            }
        }
        let built = started.elapsed();
        let started = std::time::Instant::now();
        let mut rows = 0;
        for q in ["", "p", "fam", "person1", "d7", "zz"] {
            rows += b.list(q, 2000).len();
        }
        let listed = started.elapsed() / 6;
        let started = std::time::Instant::now();
        for q in ["p", "fam", "person1", "d7", "zz"] {
            rows += b.suggest(q, 10, now).len();
        }
        let suggested = started.elapsed() / 5;
        println!(
            "MEASURE {} contacts: built {:.2} ms, list {:.2} ms, suggest {:.2} ms a query ({rows} rows)",
            b.len(),
            built.as_secs_f64() * 1000.0,
            listed.as_secs_f64() * 1000.0,
            suggested.as_secs_f64() * 1000.0
        );
    }

    /// The address book's list of the first `limit` (picked out, then put in order) is the
    /// start of the whole list in order, for every size and limit: with names shared by
    /// several, people without a name, and a search.
    #[test]
    fn a_list_cut_short_is_the_start_of_the_whole_list() {
        let names = ["Zoe Park", "ana lima", "", "Ana Lima", "Bo", "zoe park", "", "Élise"];
        for n in 0..40usize {
            let mut b = ContactBook::new();
            for i in 0..n {
                // In no order, the same name for several, and none for some.
                let at = (i * 7 + 3) % 40;
                b.note(&EmailAddress::new(names[at % names.len()], format!("p{at:02}@x.example")), 1000, false, 1);
            }
            assert_eq!(b.len(), n);
            for query in ["", "ana", "p1", "nobody"] {
                let whole = b.list(query, usize::MAX);
                let mut by_hand: Vec<Contact> = whole.clone();
                by_hand.sort_by_key(|c| {
                    let (name, email) = (c.address.name.to_lowercase(), c.address.email.to_lowercase());
                    (if name.is_empty() { email.clone() } else { name }, email)
                });
                assert_eq!(whole, by_hand, "{n} {query:?}");
                for limit in 0..n + 2 {
                    let listed = b.list(query, limit);
                    assert_eq!(listed.as_slice(), &whole[..limit.min(whole.len())], "{n} people, {query:?}, the first {limit}");
                }
            }
        }
    }

    #[test]
    fn completing_the_last_address() {
        assert_eq!(last_token("Ann <a@x>, zo"), "zo");
        assert_eq!(last_token("zo"), "zo");
        assert_eq!(last_token("a@x; "), "");
        let z = EmailAddress::new("Zoe Park", "zoe@x.example");
        assert_eq!(complete("Ann <a@x>, zo", &z), "Ann <a@x>, Zoe Park <zoe@x.example>, ");
        assert_eq!(complete("zo", &z), "Zoe Park <zoe@x.example>, ");
    }
}
