//! The keywords behind categories, made and read the way other mail programs do (issue #28), so
//! that a category and a Thunderbird tag of the same name mark mail with the same keyword, and
//! a keyword another program set shows here even when it isn't in the category list.
//!
//! Thunderbird (since 2026) makes a tag's keyword from its name: ASCII letters and digits kept,
//! other printable ASCII kept unless an IMAP atom can't hold it (or it is `=`, the escape),
//! everything else written `=xx` for each UTF-8 byte, the whole lowercased; a key longer than 50
//! characters (Dovecot's limit) is `&x-moz-` and the first 16 bytes of the name's SHA-256 in hex;
//! a key another tag has already gets `_1`, `_2`… This is MailCraft's own code for that rule.

use sha2::{Digest, Sha256};

use super::Category;
use crate::is_keyword;

/// The longest keyword made or accepted for a category: Dovecot refuses longer ones
/// (`mail_max_keyword_length`), and Thunderbird keeps to it too.
pub const MAX_KEYWORD: usize = 50;

/// The prefix of a keyword made from a hash of a long name.
const HASHED: &str = "&x-moz-";

/// Keywords mail programs and servers set for their own purposes, not categories: the IANA
/// "IMAP and JMAP Keywords" registry, those Thunderbird, Apple Mail, KMail and Dovecot set, and
/// MailCraft's own. Compared without case.
const SYSTEM: &[&str] = &[
    // IANA registry.
    "$Forwarded",
    "$MDNSent",
    "$SubmitPending",
    "$Submitted",
    "$Junk",
    "$NotJunk",
    "$Phishing",
    "$Important",
    "$AutoSent",
    "$CanUnsubscribe",
    "$Followed",
    "$HasAttachment",
    "$HasMemo",
    "$HasNoAttachment",
    "$Imported",
    "$IsTrusted",
    "$MaskedEmail",
    "$Memo",
    "$Muted",
    "$New",
    "$Notify",
    "$Unsubscribed",
    "$Recent",
    // Junk marks without the `$`, and others in common use.
    "Junk",
    "NonJunk",
    "NotJunk",
    "$NotPhishing",
    "Forwarded",
    "Redirected",
    "$Redirected",
    // Apple Mail's note that it has trained its junk filter on a message.
    "JunkRecorded",
    // KMail's message states.
    "$Attachment",
    "$Signed",
    "$Encrypted",
    "$Invitation",
    "$Watched",
    "$Ignored",
    "$Replied",
    "$Queued",
    "$Sent",
    "$Error",
    "$ToDo",
    // MailCraft's own.
    crate::PINNED,
    crate::FOLLOW_UP_COMPLETE,
];

/// Whether `k` is a keyword some program sets for its own purposes (junk marks, `$Forwarded`,
/// Apple's flag colours `$MailFlagBit0…`), which isn't shown as a category.
pub fn is_system_keyword(k: &str) -> bool {
    k.starts_with('\\') || SYSTEM.iter().any(|s| s.eq_ignore_ascii_case(k)) || k.get(..12).is_some_and(|p| p.eq_ignore_ascii_case("$MailFlagBit"))
}

/// Whether `k` can be a category's keyword: an IMAP atom of at most [`MAX_KEYWORD`] characters
/// that isn't one of the [system keywords](is_system_keyword). `Err` says what's wrong.
pub fn check_keyword(k: &str) -> Result<(), &'static str> {
    if k.is_empty() {
        Err("a keyword can't be empty")
    } else if k.len() > MAX_KEYWORD {
        Err("a keyword can have at most 50 characters")
    } else if !is_keyword(k) {
        Err("a keyword can only have letters, digits and symbols other than ( ) { } % * \" \\ ], no spaces")
    } else if is_system_keyword(k) {
        Err("that keyword has another use")
    } else {
        Ok(())
    }
}

/// The keyword Thunderbird would make for a tag called `name`, before any `_1` suffix:
/// `To do` → `to=20do`, `Rückfrage` → `r=c3=bcckfrage`, a long name → `&x-moz-<hash>`.
pub fn tag_key(name: &str) -> String {
    let mut key = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || (b > 0x20 && b < 0x7f && !b"=()[]{}%*\"\\<>;&".contains(&b)) {
            key.push(char::from(b.to_ascii_lowercase()));
        } else {
            key.push_str(&format!("={b:02x}"));
        }
    }
    if key.is_empty() {
        key.push_str("tag");
    }
    if key.len() > MAX_KEYWORD { hashed_key(name) } else { key }
}

/// `&x-moz-` and the first 16 bytes of the name's SHA-256, in hex: 39 characters.
fn hashed_key(name: &str) -> String {
    let digest = Sha256::digest(name.as_bytes());
    let mut key = String::from(HASHED);
    for b in digest.iter().take(16) {
        key.push_str(&format!("{b:02x}"));
    }
    key
}

/// The keyword for a new category called `name`: [`tag_key`], with `_1`, `_2`… when a category
/// in `existing` (or a system keyword) has it already, as Thunderbird numbers them.
pub fn new_keyword(name: &str, existing: &[Category]) -> String {
    let name = name.trim();
    let taken = |k: &str| is_system_keyword(k) || existing.iter().any(|c| c.keyword.eq_ignore_ascii_case(k) && c.name != name);
    let mut base = tag_key(name);
    let mut key = base.clone();
    let mut n = 1u32;
    // Bounded: the list is short, but the loop must end whatever it holds.
    while n < 10_000 {
        if key.len() > MAX_KEYWORD && !base.starts_with(HASHED) {
            base = hashed_key(name);
            key = base.clone();
            n = 1;
        }
        if !taken(&key) {
            return key;
        }
        key = format!("{base}_{n}");
        n += 1;
    }
    key
}

/// A readable name for a keyword that isn't in the category list: a Thunderbird-style key
/// decoded (`r=c3=bcckfrage` → `rückfrage`, `to=20do` → `to do`), anything else as written
/// (`$label6`, `work`, a hashed `&x-moz-…`).
pub fn readable(keyword: &str) -> String {
    if !keyword.contains('=') || keyword.starts_with(HASHED) {
        return keyword.to_string();
    }
    let b = keyword.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'=' {
            let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            match hex {
                Some(v) => out.push(v),
                // Not one of Thunderbird's escapes: leave the keyword as it is.
                None => return keyword.to_string(),
            }
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    match String::from_utf8(out) {
        Ok(s) if !s.trim().is_empty() && !s.chars().any(char::is_control) => s,
        _ => keyword.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_as_thunderbird_makes_them() {
        assert_eq!(tag_key("Vertraulich"), "vertraulich");
        assert_eq!(tag_key("To do"), "to=20do");
        assert_eq!(tag_key("Rückfrage"), "r=c3=bcckfrage");
        assert_eq!(tag_key("$label6"), "$label6");
        // Atom specials, the escape itself and Thunderbird's extra unsafe ones are escaped;
        // other symbols kept.
        assert_eq!(tag_key("a=b (c)"), "a=3db=20=28c=29");
        assert_eq!(tag_key("x&y;<z>[w]"), "x=26y=3b=3cz=3e=5bw=5d");
        assert_eq!(tag_key("Projects: Film!"), "projects:=20film!");
        assert_eq!(tag_key("50%*"), "50=25=2a");
        assert_eq!(tag_key(""), "tag");
        assert_eq!(tag_key("Ж"), "=d0=96");
        // A name whose key would be longer than 50 characters: a hash, still an atom.
        let long = tag_key("Ein ziemlich langer Name für eine Kategorie, wirklich");
        assert_eq!(long, "&x-moz-ef030ced524f062305d88085ecf3b8fb");
        assert!(long.len() == 39 && is_keyword(&long), "{long}");
        assert_ne!(long, tag_key("Ein ziemlich langer Name für eine Kategorie, wirklich!"));
        assert_eq!(tag_key(&"a".repeat(50)), "a".repeat(50));
        assert!(tag_key(&"a".repeat(51)).starts_with("&x-moz-"));
        for name in ["To do", "Rückfrage", "日本語のタグ", "a\"b\\c]d", "\u{0}\t\n", "x y z", "€"] {
            let k = tag_key(name);
            assert!(check_keyword(&k).is_ok(), "{name:?} → {k:?}");
        }
    }

    #[test]
    fn collisions_are_numbered() {
        let cat = |n: &str, k: &str| Category { name: n.into(), keyword: k.into(), color: 0 };
        assert_eq!(new_keyword("To do", &[]), "to=20do");
        assert_eq!(new_keyword(" To do ", &[cat("To-do list", "to=20do")]), "to=20do_1");
        assert_eq!(new_keyword("To do", &[cat("x", "TO=20DO"), cat("y", "to=20do_1")]), "to=20do_2");
        // The same name keeps its key.
        assert_eq!(new_keyword("To do", &[cat("To do", "to=20do")]), "to=20do");
        // A system keyword is never handed out.
        assert_eq!(new_keyword("Junk", &[]), "junk_1");
        assert_eq!(new_keyword("$Forwarded", &[]), "$forwarded_1");
        // A 49-character key with a suffix would pass 50: hashed instead.
        let k49 = "b".repeat(49);
        let k = new_keyword(&k49, &[cat("other", &k49)]);
        assert!(k.starts_with("&x-moz-") && k.len() <= MAX_KEYWORD, "{k}");
    }

    #[test]
    fn keywords_read_back() {
        assert_eq!(readable("r=c3=bcckfrage"), "rückfrage");
        assert_eq!(readable("to=20do"), "to do");
        assert_eq!(readable("$label6"), "$label6");
        assert_eq!(readable("work"), "work");
        for name in ["Rückfrage", "To do", "a=b", "日本語", "x&y;"] {
            assert_eq!(readable(&tag_key(name)), name.to_lowercase());
        }
        // Not Thunderbird's escapes, or not text once decoded: as written.
        for k in ["a=zz", "a=", "a=4", "=ff=fe", "=00", "=20", "&x-moz-3d00"] {
            assert_eq!(readable(k), k);
        }
    }

    #[test]
    fn system_keywords_and_checks() {
        for k in [
            "$Forwarded",
            "$mdnsent",
            "$Junk",
            "junk",
            "NonJunk",
            "NotJunk",
            "$SubmitPending",
            "$MailFlagBit2",
            "$Pinned",
            "$FollowUpComplete",
            "\\Seen",
            "JunkRecorded",
        ] {
            assert!(is_system_keyword(k), "{k}");
        }
        for k in ["$label1", "$label6", "to=20do", "work", "$Mail"] {
            assert!(!is_system_keyword(k), "{k}");
        }
        assert!(check_keyword("$label6").is_ok());
        assert!(check_keyword("").is_err());
        assert!(check_keyword("two words").is_err());
        assert!(check_keyword("a]").is_err());
        assert!(check_keyword(&"x".repeat(51)).is_err());
        assert!(check_keyword("$Junk").is_err());
        assert!(check_keyword("ü").is_err());
    }
}
