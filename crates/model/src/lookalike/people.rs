//! A known person's name on someone else's address (issue #29). The cheapest impersonation
//! needs no look-alike domain: `"Jane Director" <jane.director1984@gmail.com>` is a real address
//! at a real mail service, and a message list showing only names shows "Jane Director".
//!
//! [`KnownPeople`] holds the people the user really deals with (their contacts, the people they
//! write to, the organisation directory's answers, themselves) by name and by address;
//! [`KnownPeople::check`] finds a sender who wears one of those names on an address that person
//! isn't known by. Names are compared folded: case, accents and letters drawn like Latin ones
//! don't count, nor does the order of the words ("Jane Director", "Director, Jane"), nor a
//! middle initial. Only names that say who someone is count: a single word ("Jane", "Support"),
//! or a name with a word any team uses ("IT Support", "Billing Team"), never warns by itself.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::EmailAddress;

use super::{SenderWarning, domain_of, plain, related};

/// Words that make a name a role or a service rather than a person ("Help Desk", "Sales Team").
const GENERIC: &[&str] = &[
    "account",
    "accounts",
    "accounting",
    "admin",
    "administrator",
    "alert",
    "alerts",
    "automated",
    "billing",
    "bot",
    "calendar",
    "careers",
    "contact",
    "customer",
    "daemon",
    "department",
    "desk",
    "digest",
    "do",
    "finance",
    "hello",
    "help",
    "helpdesk",
    "hr",
    "info",
    "invoice",
    "invoices",
    "it",
    "jobs",
    "list",
    "mail",
    "mailer",
    "marketing",
    "news",
    "newsletter",
    "no",
    "noreply",
    "not",
    "notification",
    "notifications",
    "office",
    "order",
    "orders",
    "payments",
    "payroll",
    "postmaster",
    "reception",
    "reply",
    "root",
    "sales",
    "security",
    "service",
    "services",
    "shop",
    "staff",
    "support",
    "system",
    "team",
    "the",
    "updates",
    "via",
    "webmaster",
];

/// Titles and suffixes worn around a name ("Dr. Jane Director", "Jane Director, CEO"): left out
/// like initials, so a title added to a known name doesn't make it another name.
const TITLES: &[&str] = &[
    "cfo", "ceo", "cio", "coo", "cto", "dr", "esq", "evp", "ii", "iii", "jr", "md", "miss", "mr", "mrs", "ms", "mx", "phd", "prof", "sir", "sr",
    "svp", "vp",
];

/// Names longer than this (in bytes) are sentences or company blurbs, never a person's name.
const LONGEST_NAME: usize = 120;
/// A person's name has at most this many words (initials not counted).
const MOST_WORDS: usize = 5;
/// People kept at most: a huge address book stays bounded.
const MOST_PEOPLE: usize = 50_000;

/// The name as it's compared: folded words, initials left out, in alphabetical order, one space
/// between (`"Director, Jane A."` → `"director jane"`). `None` for a name that doesn't say who
/// someone is: one word, a generic one, digits, too long.
pub fn name_key(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > LONGEST_NAME || name.contains('@') {
        return None;
    }
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    for c in name.chars() {
        // An apostrophe joins (O'Brien), and so do an accent written as a mark of its own and
        // the invisible characters that would part a name unseen; anything else that isn't a
        // letter or digit parts.
        if matches!(c, '\'' | '\u{2019}' | '`') || invisible(c) {
            continue;
        }
        if c.is_alphanumeric() {
            fold_into(c, &mut word);
        } else if !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    // Digits drawn like letters inside a word read as those letters (`Z0e`); any other digit
    // makes it an account or a ticket, not a person. One letter is an initial.
    for w in words.iter_mut() {
        if w.chars().any(char::is_alphabetic) && w.chars().any(|c| c.is_ascii_digit()) {
            *w = w.chars().map(plain).collect();
        }
    }
    if words.iter().any(|w| w.chars().any(char::is_numeric)) {
        return None;
    }
    words.retain(|w| w.chars().count() > 1 && !TITLES.contains(&w.as_str()));
    if words.len() < 2 || words.len() > MOST_WORDS || words.iter().any(|w| GENERIC.contains(&w.as_str())) {
        return None;
    }
    words.sort();
    Some(words.join(" "))
}

/// Combining accents (`e` + `◌́`), zero-width spaces and joiners, the soft hyphen, the byte-order
/// mark: nothing a reader sees between two letters.
fn invisible(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20FF}' | '\u{FE20}'..='\u{FE2F}')
        || matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}' | '\u{034F}')
}

/// `c` lower case, without its accent, and as the Latin letter it's drawn like (full-width and
/// mathematical letters too: `Ｊａｎｅ`, `𝐉𝐚𝐧𝐞`).
fn fold_into(c: char, out: &mut String) {
    for c in wide_or_styled(c).to_lowercase() {
        // `İ` lowers to `i` and a combining dot: the dot isn't seen apart.
        if invisible(c) {
            continue;
        }
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        match c {
            'ß' => out.push_str("ss"),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            'þ' => out.push_str("th"),
            'ð' | 'ď' => out.push('d'),
            'ŕ' | 'ř' => out.push('r'),
            'ğ' | 'ģ' => out.push('g'),
            'ş' | 'ș' => out.push('s'),
            'ţ' | 'ț' | 'ť' => out.push('t'),
            'ň' | 'ņ' => out.push('n'),
            'ů' | 'ű' | 'ų' => out.push('u'),
            'ő' => out.push('o'),
            'ķ' => out.push('k'),
            'ļ' => out.push('l'),
            'ě' => out.push('e'),
            'ỳ' => out.push('y'),
            other => out.push(plain(other)),
        }
    }
}

/// The plain Latin letter or digit a full-width (`Ｊ`) or mathematical (`𝐉`, `𝟎`) one is drawn
/// as; anything else as it is.
fn wide_or_styled(c: char) -> char {
    let n = u32::from(c);
    let shifted = match n {
        // Full-width forms of `!`…`~`.
        0xFF01..=0xFF5E => n.checked_sub(0xFEE0),
        // Mathematical letters: bold, italic, script… each 26 capitals then 26 small letters.
        0x1D400..=0x1D6A3 => {
            let i = (n - 0x1D400) % 52;
            Some(if i < 26 { u32::from(b'A') + i } else { u32::from(b'a') + i - 26 })
        }
        // Mathematical digits: five styles of ten.
        0x1D7CE..=0x1D7FF => Some(u32::from(b'0') + (n - 0x1D7CE) % 10),
        _ => None,
    };
    shifted.and_then(char::from_u32).unwrap_or(c)
}

/// A name's key as it's looked up: [`name_key`] with letters drawn alike made one (`l`, `i` and
/// `1`; `rn` and `m`), as domains are compared, so `Wh1tfield` is `Whitfield`.
fn drawn_key(name: &str) -> Option<String> {
    name_key(name).map(|k| k.replace("rn", "m").replace("vv", "w").replace('i', "l"))
}

/// The people the user deals with, by name and by address.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnownPeople {
    /// [`drawn_key`] → the name that person was first known by, and the addresses they're
    /// known by (lower case).
    by_name: HashMap<String, (String, BTreeSet<String>)>,
    /// Every address known (lower case), named or not.
    addresses: HashSet<String>,
}

impl KnownPeople {
    pub fn new() -> KnownPeople {
        KnownPeople::default()
    }

    /// Someone the user knows (a contact, someone written to, the directory, themselves). The
    /// first name given for a person is the one warnings call them by.
    pub fn add(&mut self, a: &EmailAddress) {
        let email = a.email.trim().to_lowercase();
        if !crate::addr::is_valid_email(&email) || self.addresses.len() >= MOST_PEOPLE {
            return;
        }
        if let Some(key) = drawn_key(&a.name) {
            self.by_name.entry(key).or_insert_with(|| (a.name.trim().to_string(), BTreeSet::new())).1.insert(email.clone());
        }
        self.addresses.insert(email);
    }

    pub fn len(&self) -> usize {
        self.addresses.len()
    }

    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty()
    }

    /// Whether `email` is an address of someone known.
    pub fn knows_address(&self, email: &str) -> bool {
        self.addresses.contains(&email.trim().to_lowercase())
    }

    /// The addresses the person named `name` is known by (none for a name that doesn't say who
    /// someone is).
    pub fn addresses_of(&self, name: &str) -> Option<&BTreeSet<String>> {
        self.by_name.get(&drawn_key(name)?).map(|(_, a)| a)
    }

    /// `from` wears the name of someone known on an address that person isn't known by. Not
    /// when the address is known itself (someone with two names, or a contact under another
    /// name), when it's at the organisation domain of one of that person's addresses (another
    /// address of theirs, or a colleague's at work: a mail service doesn't count, anyone can
    /// sign up there), or when `listed` (a mailing list or someone sending for the author, as
    /// `List-Id` or `Sender` say) and the address isn't at a mail service: lists send their
    /// members' mail under the members' names. An impersonator adding those headers to free
    /// mail is still caught.
    pub fn check(&self, from: &EmailAddress, listed: bool) -> Option<SenderWarning> {
        let real = from.email.trim().to_lowercase();
        let domain = domain_of(&real);
        if domain.is_empty() || self.knows_address(&real) {
            return None;
        }
        let (known_as, theirs) = self.by_name.get(&drawn_key(&from.name)?)?;
        let free = super::is_free_mail(&domain);
        if listed && !free {
            return None;
        }
        if !free && theirs.iter().any(|a| related(&domain_of(a), &domain)) {
            return None;
        }
        let known = theirs.iter().next()?.clone();
        Some(SenderWarning::NameOfKnownPerson { name: known_as.clone(), known, real })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people() -> KnownPeople {
        let mut k = KnownPeople::new();
        for (name, email) in [
            ("Jane Director", "jane@corp.example"),
            ("Jane Director", "jane.director@gmail.com"),
            ("Zoë Ångström", "zoe@studio.example"),
            ("Info", "info@shop.example"),
            ("IT Support", "it@corp.example"),
            ("Bob", "bob@gmail.com"),
            ("", "nameless@corp.example"),
            ("Robert Brown", "rob@corp.example"),
            ("Bobby Brown", "rob@corp.example"),
        ] {
            k.add(&EmailAddress::new(name, email));
        }
        k
    }

    fn warned(k: &KnownPeople, name: &str, email: &str) -> bool {
        k.check(&EmailAddress::new(name, email), false).is_some()
    }

    #[test]
    fn names_fold_case_accents_and_order() {
        assert_eq!(name_key("Jane Director").as_deref(), Some("director jane"));
        assert_eq!(name_key("  DIRECTOR, Jane ").as_deref(), Some("director jane"));
        assert_eq!(name_key("Director Jane A.").as_deref(), Some("director jane"));
        assert_eq!(name_key("\"Jane Director\"").as_deref(), Some("director jane"));
        assert_eq!(name_key("Zoe Angstrom"), name_key("Zoë Ångström"));
        assert_eq!(name_key("Ångström, ZOË"), name_key("zoe angstrom"));
        // A Cyrillic а in the name reads as the Latin one.
        assert_eq!(name_key("J\u{0430}ne Director").as_deref(), Some("director jane"));
        assert_eq!(name_key("Seán O'Brien").as_deref(), Some("obrien sean"));
        assert_eq!(name_key("Straße Hans").as_deref(), Some("hans strasse"));
        assert_eq!(name_key("Anne-Marie Dupont").as_deref(), Some("anne dupont marie"));
        // Accents written as marks of their own, and characters nobody sees, don't part a word.
        assert_eq!(name_key("Jose\u{301} Garci\u{301}a"), name_key("José García"));
        assert_eq!(name_key("Jane Direc\u{200B}tor").as_deref(), Some("director jane"));
        assert_eq!(name_key("\u{202E}Jane Dir\u{2066}ector\u{202C}").as_deref(), Some("director jane"));
        // Letters that only look like the plain ones, and digits standing for letters, read as
        // them; titles worn around the name don't make it another name.
        for name in [
            "J\u{FF41}ne Director",
            "\u{FF2A}\u{FF41}\u{FF4E}\u{FF45} \u{FF24}irector",
            "\u{1D409}\u{1D41A}\u{1D427}\u{1D41E} Director",
            "\u{1D5DD}\u{1D5EE}\u{1D5FB}\u{1D5F2} Director",
            "JANE D\u{130}RECTOR",
            "Jane Direct0r",
            "Dr. Jane Director",
            "Jane Director, CEO",
            "Jane Director Jr.",
        ] {
            assert_eq!(name_key(name).as_deref(), Some("director jane"), "{name:?}");
        }
        assert_eq!(name_key("Dr. Director"), None);
        assert_eq!(name_key("Jane Director \u{1D7CF}"), None);
    }

    #[test]
    fn names_that_dont_say_who_someone_is_dont_count() {
        for name in [
            "",
            "Jane",
            "Info",
            "IT Support",
            "Support Team",
            "Billing",
            "J. D.",
            "Jane D",
            "Jane Director 2",
            "jane@corp.example",
            "Jane Director via Dev List",
            "The Corp Newsletter",
            "a b c d e f g h",
            "one two three four five six",
        ] {
            assert_eq!(name_key(name), None, "{name}");
        }
        let long = "Jane ".repeat(40);
        assert_eq!(name_key(&long), None);
    }

    #[test]
    fn a_known_name_on_a_strange_address_is_caught() {
        let k = people();
        assert_eq!(
            k.check(&EmailAddress::new("Jane Director", "jane.director1984@gmail.com"), false),
            Some(SenderWarning::NameOfKnownPerson {
                name: "Jane Director".into(),
                known: "jane.director@gmail.com".into(),
                real: "jane.director1984@gmail.com".into()
            })
        );
        // Called by the name the user knows her by.
        assert!(matches!(
            k.check(&EmailAddress::new("Director, Jane", "ceo.office@outlook.com"), false),
            Some(SenderWarning::NameOfKnownPerson { name, .. }) if name == "Jane Director"
        ));
        assert!(warned(&k, "JANE DIRECTOR", "jane@corp-payments.example"));
        assert!(warned(&k, "Zoe Angstrom", "zoe.a@proton.me"));
        assert!(warned(&k, "Robert Brown", "robert.brown@yahoo.co.uk"));
        assert!(warned(&k, "Bobby Brown", "rob.brown@hotmail.com"));
        // A digit or letter drawn like another: `1` for `i`, `rn` for `m`.
        assert!(warned(&k, "Jane D1rector", "jd@corp-pay.example"));
        assert!(warned(&k, "Robert Brovvn", "rb@gmail.com"));
        // A list header on free mail doesn't excuse it.
        assert!(k.check(&EmailAddress::new("Jane Director", "jd1984@gmail.com"), true).is_some());
    }

    #[test]
    fn known_addresses_and_the_same_organisation_pass() {
        let k = people();
        // Her own addresses, either name, any case.
        assert!(!warned(&k, "Jane Director", "jane@corp.example"));
        assert!(!warned(&k, "Jane Director", "JANE.DIRECTOR@gmail.com"));
        // Another address at her organisation (an alias, a shared mailbox).
        assert!(!warned(&k, "Jane Director", "j.director@corp.example"));
        assert!(!warned(&k, "Jane Director", "jane@mail.corp.example"));
        // A known address under another name.
        assert!(!warned(&k, "Jane Director", "nameless@corp.example"));
        // Names that don't say who someone is.
        assert!(!warned(&k, "Info", "info@other.example"));
        assert!(!warned(&k, "IT Support", "it-support@gmail.com"));
        assert!(!warned(&k, "Bob", "bob.smith@gmail.com"));
        // A mailing list sending her mail from its own domain.
        assert!(k.check(&EmailAddress::new("Jane Director", "dev@lists.example"), true).is_none());
        // Someone else entirely, or no address.
        assert!(!warned(&k, "John Smith", "john@gmail.com"));
        assert!(!warned(&k, "Jane Director", ""));
        assert!(!warned(&k, "Jane Director", "not an address"));
    }

    #[test]
    fn hostile_names_are_bounded() {
        let mut k = KnownPeople::new();
        let huge = "x".repeat(1 << 20);
        k.add(&EmailAddress::new(huge.clone(), "a@b.example"));
        k.add(&EmailAddress::new("Jane Director", huge.clone()));
        assert!(k.check(&EmailAddress::new(huge, "c@d.example"), false).is_none());
        for name in ["\u{0}\u{0}", "́́́ ́́", "ǅ ǈ", "İstanbul Ünal", "\u{202e}Jane Director", "Ⅻ Ⅻ"] {
            let _ = name_key(name);
            let _ = k.check(&EmailAddress::new(name, "x@y.example"), false);
        }
    }
}
