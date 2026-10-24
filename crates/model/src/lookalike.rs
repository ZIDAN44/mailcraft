//! Addresses made to pass for someone else's. A phishing message rarely comes from the bank's own
//! domain: it comes from one a glance mistakes for it (`paypa1.com`, `rnicrosoft.com`, a Cyrillic
//! `а` in `аpple.com`, `paypal.com.account-check.ru`), or it writes a trusted address into the
//! sender's *name*, where most mail clients show it in place of the real one.
//!
//! [`imitates`] compares a domain with the ones the user really deals with (their own, and the
//! people they write to) and names the one it imitates; [`check_sender`] does that for a message's
//! sender and also catches an address hidden in the name. Both only compare text: nothing is
//! looked up.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::EmailAddress;

/// What's wrong with who a message says it's from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SenderWarning {
    /// The sender's domain looks like `like`, one the user deals with, but isn't it.
    Lookalike { domain: String, like: String },
    /// The sender's name shows an address (`shown`) that isn't the one it came from.
    NameShowsOther { shown: String, real: String },
}

/// Big mail services whose names are close to each other (`mail.com`, `gmail.com`): neither is
/// ever taken for an imitation of another.
const MAIL_SERVICES: &[&str] = &[
    "126.com",
    "163.com",
    "aol.com",
    "daum.net",
    "fastmail.com",
    "free.fr",
    "gmail.com",
    "gmx.at",
    "gmx.com",
    "gmx.de",
    "gmx.net",
    "googlemail.com",
    "hotmail.com",
    "icloud.com",
    "libero.it",
    "live.com",
    "mac.com",
    "mail.com",
    "mail.ru",
    "me.com",
    "msn.com",
    "naver.com",
    "orange.fr",
    "outlook.com",
    "pm.me",
    "proton.me",
    "protonmail.com",
    "qq.com",
    "t-online.de",
    "web.de",
    "ya.ru",
    "yahoo.com",
    "yandex.ru",
    "ymail.com",
    "zoho.com",
];

/// Whether `domain` is one of the big mail services, where everyone has an address of their own
/// (so two people there needn't know each other).
pub fn is_mail_service(domain: &str) -> bool {
    MAIL_SERVICES.contains(&domain.trim().to_lowercase().as_str())
}

/// Shorter than this, two domains one letter apart are as likely to be two real ones.
const SHORTEST_COMPARED: usize = 6;

/// The domain of an address, lower-case (`""` without one).
pub fn domain_of(email: &str) -> String {
    email.trim().rsplit_once('@').map(|(_, d)| d.trim_end_matches('.').to_lowercase()).unwrap_or_default()
}

/// The domain among `known` that `domain` is made to look like, if any. `domain` itself known,
/// one of the big mail services, or a part of a known one (`mail.example.com` and
/// `example.com`) is never an imitation.
pub fn imitates<'a>(domain: &str, known: &'a BTreeSet<String>) -> Option<&'a str> {
    let d = domain.trim().trim_end_matches('.').to_lowercase();
    if d.is_empty() || d.len() > 253 || known.contains(&d) || MAIL_SERVICES.contains(&d.as_str()) {
        return None;
    }
    let shape = skeleton(&d);
    known.iter().map(String::as_str).find(|k| {
        if k.is_empty() || related(&d, k) {
            return false;
        }
        // Drawn the same: look-alike letters, digits for letters, or the same name in punycode.
        shape == skeleton(k)
            // One slip of the keyboard away.
            || (d.len().min(k.len()) >= SHORTEST_COMPARED && one_edit_apart(&d, k))
            // The real domain worn as a label: `paypal.com.secure.example`, `secure-paypal.com`.
            || worn_inside(&d, k)
    })
}

/// What's wrong with `from`, as a sender, given the domains the user deals with.
pub fn check_sender(from: &EmailAddress, known: &BTreeSet<String>) -> Option<SenderWarning> {
    let real = from.email.trim().to_lowercase();
    let domain = domain_of(&real);
    if domain.is_empty() {
        return None;
    }
    // An address in the name that isn't the sender's own.
    for word in from.name.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '(' | ')' | '"' | '\'' | ',' | ';' | '[' | ']')) {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        if word.contains('@') && crate::addr::is_valid_email(&word) && word != real {
            let shown = domain_of(&word);
            if !related(&shown, &domain) {
                return Some(SenderWarning::NameShowsOther { shown: word, real });
            }
        }
    }
    imitates(&domain, known).map(|like| SenderWarning::Lookalike { domain: domain.clone(), like: like.to_string() })
}

/// One domain is the other or under it (`a.example.com`, `example.com`).
fn related(a: &str, b: &str) -> bool {
    a == b || a.strip_suffix(b).is_some_and(|r| r.ends_with('.')) || b.strip_suffix(a).is_some_and(|r| r.ends_with('.'))
}

/// `known` (at least two labels and [`SHORTEST_COMPARED`] long) appears in `d` with a separator
/// on each side, while `d` belongs to someone else.
fn worn_inside(d: &str, known: &str) -> bool {
    if known.len() < SHORTEST_COMPARED || !known.contains('.') {
        return false;
    }
    d.match_indices(known).any(|(at, _)| {
        let before = d.get(..at).and_then(|s| s.chars().next_back());
        let after = d.get(at + known.len()..).and_then(|s| s.chars().next());
        let edge = |c: Option<char>| c.is_none_or(|c| c == '.' || c == '-');
        // Not where it ends `d` after a dot: that's the domain itself or one under it.
        edge(before) && edge(after) && !(after.is_none() && before == Some('.'))
    })
}

/// Whether one letter added, dropped, changed or two swapped turns `a` into `b`.
fn one_edit_apart(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a == b || a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let start = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let (ra, rb) = (a.get(start..).unwrap_or_default(), b.get(start..).unwrap_or_default());
    match ra.len().cmp(&rb.len()) {
        std::cmp::Ordering::Equal => {
            ra.get(1..) == rb.get(1..) || (ra.len() >= 2 && ra.first() == rb.get(1) && ra.get(1) == rb.first() && ra.get(2..) == rb.get(2..))
        }
        std::cmp::Ordering::Greater => ra.get(1..) == Some(rb),
        std::cmp::Ordering::Less => rb.get(1..) == Some(ra),
    }
}

/// How a domain looks: punycode labels read as the letters they stand for, every letter drawn
/// like a Latin one written as that one (Cyrillic `а`, Greek `ο`, `é`), digits and pairs a glance
/// takes for letters (`0` `o`, `1` `l`, `rn` `m`, `vv` `w`) as those letters.
pub fn skeleton(domain: &str) -> String {
    let mut out = String::with_capacity(domain.len());
    for (i, label) in domain.split('.').enumerate() {
        if i > 0 {
            out.push('.');
        }
        let decoded = label.strip_prefix("xn--").and_then(punycode_decode);
        for c in decoded.as_deref().unwrap_or(label).chars().flat_map(char::to_lowercase) {
            out.push(plain(c));
        }
    }
    out.replace("rn", "m").replace("vv", "w").replace(['i', '|'], "l")
}

/// The Latin letter `c` is drawn like (itself when none).
fn plain(c: char) -> char {
    match c {
        '0' | 'о' | 'ο' | 'ö' | 'ó' | 'ò' | 'ô' | 'õ' | 'ø' | 'ō' | 'σ' => 'o',
        '1' | 'ӏ' | 'ł' | 'ĺ' | 'ľ' => 'l',
        'а' | 'α' | 'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ą' => 'a',
        'е' | 'ё' | 'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ε' => 'e',
        'і' | 'ї' | 'ι' | 'ı' | 'ɩ' | 'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
        'с' | 'ϲ' | 'ç' | 'ć' | 'č' => 'c',
        'р' | 'ρ' => 'p',
        'у' | 'ý' | 'ÿ' | 'γ' => 'y',
        'х' | 'χ' => 'x',
        'ј' => 'j',
        'ѕ' | 'ś' | 'š' | '5' => 's',
        'һ' => 'h',
        'ԁ' | 'đ' => 'd',
        'ԛ' => 'q',
        'ԝ' | 'ѡ' => 'w',
        'к' | 'κ' => 'k',
        'ν' => 'v',
        'υ' | 'ù' | 'ú' | 'û' | 'ü' | 'ū' => 'u',
        'ñ' | 'ń' | 'η' => 'n',
        'ɡ' => 'g',
        'τ' => 't',
        'ž' | 'ź' | 'ż' => 'z',
        _ => c,
    }
}

/// A punycode label's letters (RFC 3492, section 6.2), without its `xn--`; `None` when it isn't
/// one.
fn punycode_decode(input: &str) -> Option<String> {
    const BASE: u32 = 36;
    const T_MIN: u32 = 1;
    const T_MAX: u32 = 26;
    if input.is_empty() || input.len() > 63 || !input.is_ascii() {
        return None;
    }
    let (basic, rest) = match input.rfind('-') {
        Some(at) => (input.get(..at)?, input.get(at + 1..)?),
        None => ("", input),
    };
    let mut out: Vec<char> = basic.chars().collect();
    let (mut n, mut i, mut bias) = (128u32, 0u32, 72u32);
    let mut digits = rest.bytes().peekable();
    while digits.peek().is_some() {
        let old_i = i;
        let mut w = 1u32;
        let mut k = BASE;
        loop {
            let d = match digits.next()? {
                b @ b'a'..=b'z' => u32::from(b - b'a'),
                b @ b'A'..=b'Z' => u32::from(b - b'A'),
                b @ b'0'..=b'9' => u32::from(b - b'0') + 26,
                _ => return None,
            };
            i = i.checked_add(d.checked_mul(w)?)?;
            let t = k.saturating_sub(bias).clamp(T_MIN, T_MAX);
            if d < t {
                break;
            }
            w = w.checked_mul(BASE - t)?;
            k = k.checked_add(BASE)?;
        }
        let len = u32::try_from(out.len()).ok()?.checked_add(1)?;
        bias = adapt(i - old_i, len, old_i == 0);
        n = n.checked_add(i / len)?;
        i %= len;
        out.insert(usize::try_from(i).ok()?, char::from_u32(n)?);
        i += 1;
        if out.len() > 63 {
            return None;
        }
    }
    Some(out.into_iter().collect())
}

fn adapt(delta: u32, points: u32, first: bool) -> u32 {
    let mut delta = if first { delta / 700 } else { delta / 2 };
    delta += delta / points;
    let mut k = 0;
    while delta > ((36 - 1) * 26) / 2 {
        delta /= 36 - 1;
        k += 36;
    }
    k + (36 * delta) / (delta + 38)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> BTreeSet<String> {
        ["paypal.com", "studio-lantern.example", "apple.com", "microsoft.com", "example.org"].into_iter().map(String::from).collect()
    }

    #[test]
    fn imitations_are_caught() {
        let k = known();
        for (domain, like) in [
            ("paypa1.com", "paypal.com"),
            ("paypai.com", "paypal.com"),
            ("paypall.com", "paypal.com"),
            ("payapl.com", "paypal.com"),
            ("rnicrosoft.com", "microsoft.com"),
            ("micros0ft.com", "microsoft.com"),
            ("xn--pple-43d.com", "apple.com"), // а (Cyrillic) pple
            ("аpple.com", "apple.com"),
            ("studio-lanterns.example", "studio-lantern.example"),
            ("paypal.co", "paypal.com"),
            ("paypal.com.account-check.example", "paypal.com"),
            ("secure-paypal.com", "paypal.com"),
            ("PAYPA1.COM.", "paypal.com"),
        ] {
            assert_eq!(imitates(domain, &k), Some(like), "{domain}");
        }
    }

    #[test]
    fn real_and_unrelated_domains_pass() {
        let k = known();
        for domain in [
            "paypal.com",
            "mail.paypal.com",
            "email.apple.com",
            "github.com",
            "gitlab.com",
            "gmail.com",
            "mail.com",
            "example.net",
            "orchestra.example",
            "",
            "xn--",
            "xn--zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        ] {
            assert_eq!(imitates(domain, &k), None, "{domain}");
        }
        // Two short domains one letter apart are just two domains.
        let short: BTreeSet<String> = ["ab.io".to_string()].into();
        assert_eq!(imitates("ac.io", &short), None);
    }

    #[test]
    fn punycode_reads_as_its_letters() {
        assert_eq!(punycode_decode("bcher-kva").as_deref(), Some("bücher"));
        assert_eq!(punycode_decode("pple-43d").as_deref(), Some("аpple"));
        assert_eq!(skeleton("xn--bcher-kva.example"), "bucher.example");
        for junk in ["", "-", "99999999999", "a-b-c-!", "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"] {
            let _ = punycode_decode(junk);
        }
    }

    #[test]
    fn an_address_in_the_name_is_caught() {
        let k = known();
        let from = EmailAddress::new("service@paypal.com", "alerts@billing-notice.example");
        assert_eq!(
            check_sender(&from, &k),
            Some(SenderWarning::NameShowsOther { shown: "service@paypal.com".into(), real: "alerts@billing-notice.example".into() })
        );
        // The sender's own address, or one at its own domain, in the name is fine.
        assert_eq!(check_sender(&EmailAddress::new("Ana (ana@paypal.com)", "ana@paypal.com"), &k), None);
        assert_eq!(check_sender(&EmailAddress::new("help@mail.paypal.com", "ana@paypal.com"), &k), None);
        assert_eq!(
            check_sender(&EmailAddress::new("PayPal", "service@paypa1.com"), &k),
            Some(SenderWarning::Lookalike { domain: "paypa1.com".into(), like: "paypal.com".into() })
        );
        assert_eq!(check_sender(&EmailAddress::new("Ana", "not an address"), &k), None);
    }
}
