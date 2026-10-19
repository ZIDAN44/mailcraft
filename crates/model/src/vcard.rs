//! vCard (RFC 6350; 3.0 files read too): the people in an address book.
//!
//! The fields MailCraft shows and edits are read into a [`Card`]; every other property is kept
//! as written so editing a card from a server loses nothing.

use serde::{Deserialize, Serialize};

/// The longest input read.
const MAX_INPUT: usize = 1 << 20;

/// A typed value (an address or number with its kind: `work`, `home`, `cell`…).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Typed {
    pub value: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
}

/// A postal address with its kind (`work`, `home`, or empty for another), as vCard's `ADR`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostalAddress {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// Street and number (lines kept with `\n`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub street: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub city: String,
    /// State or province.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub region: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub postcode: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub country: String,
}

impl PostalAddress {
    pub fn is_empty(&self) -> bool {
        [&self.street, &self.city, &self.region, &self.postcode, &self.country].iter().all(|x| x.trim().is_empty())
    }

    /// On one line: street, postcode and city, region, country.
    pub fn one_line(&self) -> String {
        let town = format!("{} {}", self.postcode.trim(), self.city.trim()).trim().to_string();
        [self.street.replace('\n', ", ").trim().to_string(), town, self.region.trim().to_string(), self.country.trim().to_string()]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A birthday as written (`YYYY-MM-DD`, or `--MM-DD` without the year), from vCard's `BDAY`
/// (`19900401`, `1990-04-01`, `--0401`, a date and time): `None` when it isn't a date.
pub fn birthday_of(v: &str) -> Option<String> {
    let v = v.trim();
    let v = v.split('T').next().unwrap_or(v);
    let digits: String = v.chars().filter(char::is_ascii_digit).collect();
    let (year, rest) = if v.starts_with("--") { (None, digits.as_str()) } else { (Some(digits.get(..4)?), digits.get(4..)?) };
    let month: u32 = rest.get(..2)?.parse().ok()?;
    let day: u32 = rest.get(2..4)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(match year {
        Some(y) => format!("{y}-{month:02}-{day:02}"),
        None => format!("--{month:02}-{day:02}"),
    })
}

/// A person or organisation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub uid: String,
    /// The name as shown (`FN`).
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub given: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub family: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<Typed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phones: Vec<Typed>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub org: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Postal addresses: work, home, other.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<PostalAddress>,
    /// `YYYY-MM-DD` (or `--MM-DD`); empty when not known.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub birthday: String,
    /// Other properties, as written (unfolded content lines).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<String>,
}

impl Card {
    /// The name to show: `FN`, else the parts of `N`, else the first address.
    pub fn display(&self) -> String {
        if !self.name.trim().is_empty() {
            return self.name.trim().to_string();
        }
        let n = format!("{} {}", self.given.trim(), self.family.trim()).trim().to_string();
        if !n.is_empty() {
            return n;
        }
        self.emails.first().map(|e| e.value.clone()).unwrap_or_default()
    }

    /// Whether the card matches a search (name, organisation or any address, case-insensitive).
    pub fn matches(&self, q: &str) -> bool {
        let q = q.trim().to_lowercase();
        q.is_empty()
            || self.display().to_lowercase().contains(&q)
            || self.org.to_lowercase().contains(&q)
            || self.emails.iter().any(|e| e.value.to_lowercase().contains(&q))
            || self.phones.iter().any(|p| p.value.replace(' ', "").contains(&q.replace(' ', "")))
    }
}

/// A component of a structured value (`N`, `ORG`): split on unescaped `;`, unescaped.
fn components(v: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut it = v.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('n' | 'N') => out.last_mut().into_iter().for_each(|s| s.push('\n')),
                Some(o) => out.last_mut().into_iter().for_each(|s| s.push(o)),
                None => {}
            },
            ';' => out.push(String::new()),
            c => out.last_mut().into_iter().for_each(|s| s.push(c)),
        }
    }
    out
}

/// The kind from `TYPE` parameters (`TYPE=work`, `TYPE=WORK,VOICE`, 3.0's bare `WORK`).
fn kind(params: &[(String, String)]) -> String {
    let mut kinds: Vec<String> = Vec::new();
    for (k, v) in params {
        let v = v.trim_matches('"');
        if k == "TYPE" {
            kinds.extend(v.split(',').map(|x| x.trim().to_ascii_lowercase()));
        } else if v.is_empty() {
            kinds.push(k.to_ascii_lowercase());
        }
    }
    kinds.retain(|k| !matches!(k.as_str(), "internet" | "voice" | "pref" | "x400"));
    kinds.into_iter().next().unwrap_or_default()
}

/// Whether a property's head (`NOTE;ENCODING=QUOTED-PRINTABLE`, 2.1's bare
/// `NOTE;QUOTED-PRINTABLE`) says its value is quoted-printable.
fn quoted_printable_line(line: &str) -> bool {
    let head = line.split(':').next().unwrap_or("");
    head.split(';').skip(1).any(|p| {
        let v = p.split_once('=').map_or(p, |(k, v)| if k.trim().eq_ignore_ascii_case("ENCODING") { v } else { "" });
        v.trim().trim_matches('"').eq_ignore_ascii_case("QUOTED-PRINTABLE")
    })
}

/// Upper-cased value of `CHARSET=` in a property head; empty means UTF-8.
fn charset_of(head: &str) -> String {
    head.split(';')
        .skip(1)
        .find_map(|p| p.split_once('=').filter(|(k, _)| k.trim().eq_ignore_ascii_case("CHARSET")))
        .map(|(_, v)| v.trim().trim_matches('"').to_ascii_uppercase())
        .unwrap_or_default()
}

/// A quoted-printable value as text: `=XX` bytes read in its character set (UTF-8, or the
/// single-byte Western sets older phones and address books wrote).
fn quoted_printable(v: &str, charset: String) -> String {
    let mut bytes = Vec::with_capacity(v.len());
    let b = v.as_bytes();
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |j: usize| b.get(j).and_then(|h| (*h as char).to_digit(16));
        match (c, hex(i + 1), hex(i + 2)) {
            (b'=', Some(hi), Some(lo)) => {
                bytes.push(u8::try_from(hi * 16 + lo).unwrap_or(b'?'));
                i += 3;
            }
            _ => {
                bytes.push(c);
                i += 1;
            }
        }
    }
    let single_byte = charset.starts_with("ISO-8859")
        || charset.starts_with("WINDOWS-125")
        || charset.starts_with("CP125")
        || charset == "LATIN1"
        || charset == "US-ASCII";
    match String::from_utf8(bytes) {
        Ok(s) if !single_byte => s,
        Ok(s) if s.is_ascii() => s,
        Ok(s) => s.bytes().map(western).collect(),
        Err(e) => e.into_bytes().into_iter().map(western).collect(),
    }
}

/// A byte of Windows-1252 (a superset of Latin-1 for text) as a character.
fn western(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—',
        '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or('?'),
        _ => char::from(b),
    }
}

/// Read every card in a text (one `.vcf` can hold many).
pub fn parse(text: &str) -> Vec<Card> {
    let text = text.get(..MAX_INPUT).unwrap_or(text);
    let mut unfolded: Vec<String> = Vec::new();
    for l in text.split('\n') {
        let l = l.strip_suffix('\r').unwrap_or(l);
        match (l.strip_prefix(' ').or_else(|| l.strip_prefix('\t')), unfolded.last_mut()) {
            // 2.1's quoted-printable values break lines with a trailing `=`, the next line as it is.
            (_, Some(prev)) if prev.ends_with('=') && quoted_printable_line(prev) => {
                prev.pop();
                prev.push_str(l);
            }
            (Some(rest), Some(prev)) => prev.push_str(rest),
            _ => unfolded.push(l.to_string()),
        }
    }
    let mut cards = Vec::new();
    let mut cur: Option<Card> = None;
    for raw in unfolded {
        let mut quoted = false;
        let Some(colon) = raw.char_indices().find(|(_, c)| {
            if *c == '"' {
                quoted = !quoted;
            }
            *c == ':' && !quoted
        }) else {
            continue;
        };
        let (head, value) = (raw.get(..colon.0).unwrap_or(""), raw.get(colon.0 + 1..).unwrap_or(""));
        let decoded;
        let value = if quoted_printable_line(head) {
            decoded = quoted_printable(value, charset_of(head));
            decoded.as_str()
        } else {
            value
        };
        let mut parts = head.split(';');
        // Properties may carry a group prefix (`item1.EMAIL`).
        let name = parts.next().unwrap_or("").rsplit('.').next().unwrap_or("").trim().to_ascii_uppercase();
        let params: Vec<(String, String)> = parts
            .map(|p| {
                p.split_once('=').map_or((p.trim().to_ascii_uppercase(), String::new()), |(k, v)| (k.trim().to_ascii_uppercase(), v.to_string()))
            })
            .collect();
        match name.as_str() {
            "BEGIN" if value.trim().eq_ignore_ascii_case("VCARD") => cur = Some(Card::default()),
            "END" if value.trim().eq_ignore_ascii_case("VCARD") => {
                if let Some(c) = cur.take()
                    && cards.len() < 50_000
                {
                    cards.push(c);
                }
            }
            _ => {
                let Some(c) = cur.as_mut() else { continue };
                let text = || components(value).join(" ").trim().to_string();
                match name.as_str() {
                    "VERSION" | "PRODID" | "REV" => {}
                    "UID" => c.uid = value.trim().to_string(),
                    "FN" => c.name = text(),
                    "N" => {
                        let n = components(value);
                        c.family = n.first().cloned().unwrap_or_default();
                        c.given = n.get(1).cloned().unwrap_or_default();
                    }
                    "EMAIL" if c.emails.len() < 50 => c.emails.push(Typed { value: value.trim().to_string(), kind: kind(&params) }),
                    "TEL" if c.phones.len() < 50 => {
                        let v = value.trim();
                        let v = v.strip_prefix("tel:").unwrap_or(v);
                        c.phones.push(Typed { value: v.to_string(), kind: kind(&params) });
                    }
                    "ORG" => c.org = components(value).into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(", "),
                    "TITLE" => c.title = text(),
                    "NOTE" => c.note = components(value).join(";"),
                    "ADR" if c.addresses.len() < 20 => {
                        let a = components(value);
                        let part = |i: usize| a.get(i).map(|x| x.trim().to_string()).unwrap_or_default();
                        let street = [part(0), part(1), part(2)].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n");
                        let adr = PostalAddress { kind: kind(&params), street, city: part(3), region: part(4), postcode: part(5), country: part(6) };
                        if !adr.is_empty() {
                            c.addresses.push(adr);
                        }
                    }
                    "BDAY" if birthday_of(value).is_some() => c.birthday = birthday_of(value).unwrap_or_default(),
                    _ => {
                        if c.extra.len() < 200 {
                            c.extra.push(raw.clone());
                        }
                    }
                }
            }
        }
    }
    cards
}

fn esc(v: &str) -> String {
    v.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace("\r\n", "\\n").replace(['\n', '\r'], "\\n")
}

fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += n;
    }
    out.push_str("\r\n");
}

/// A card as vCard 4.0 text.
pub fn write(c: &Card) -> String {
    let mut lines: Vec<String> = vec!["BEGIN:VCARD".into(), "VERSION:4.0".into(), "PRODID:-//MailCraft//MailCraft//EN".into()];
    lines.push(format!("UID:{}", c.uid.replace(['\r', '\n'], "")));
    lines.push(format!("FN:{}", esc(&c.display())));
    lines.push(format!("N:{};{};;;", esc(&c.family), esc(&c.given)));
    let typed = |prop: &str, t: &Typed| {
        let kind = t.kind.replace(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-', "");
        if kind.is_empty() {
            format!("{prop}:{}", t.value.replace(['\r', '\n'], ""))
        } else {
            format!("{prop};TYPE={kind}:{}", t.value.replace(['\r', '\n'], ""))
        }
    };
    lines.extend(c.emails.iter().map(|e| typed("EMAIL", e)));
    lines.extend(c.phones.iter().map(|p| typed("TEL", p)));
    if !c.org.is_empty() {
        lines.push(format!("ORG:{}", esc(&c.org)));
    }
    if !c.title.is_empty() {
        lines.push(format!("TITLE:{}", esc(&c.title)));
    }
    if !c.note.is_empty() {
        lines.push(format!("NOTE:{}", esc(&c.note)));
    }
    for a in c.addresses.iter().filter(|a| !a.is_empty()) {
        let kind = a.kind.replace(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-', "");
        let head = if kind.is_empty() { "ADR".to_string() } else { format!("ADR;TYPE={kind}") };
        lines.push(format!("{head}:;;{};{};{};{};{}", esc(&a.street), esc(&a.city), esc(&a.region), esc(&a.postcode), esc(&a.country)));
    }
    if let Some(b) = birthday_of(&c.birthday) {
        lines.push(match b.strip_prefix("--") {
            Some(md) => format!("BDAY:--{}", md.replace('-', "")),
            None => format!("BDAY:{}", b.replace('-', "")),
        });
    }
    lines.extend(c.extra.iter().map(|x| x.replace(['\r', '\n'], "")));
    lines.push("END:VCARD".into());
    let mut out = String::new();
    for l in lines {
        fold(&l, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_versions_and_writes_back() {
        let v3 = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:zoe-1\r\nFN:Zoe Park\r\nN:Park;Zoe;;;\r\n\
item1.EMAIL;TYPE=INTERNET,WORK:zoe@studio.example\r\nEMAIL;HOME:zoe@home.example\r\n\
TEL;TYPE=CELL,VOICE:+49 151 234\r\nORG:Studio Lantern;Story\r\nTITLE:Director\r\n\
NOTE:Prefers mornings\\, not Mondays.\\nCall first.\r\nBDAY:1990-04-01\r\n\
PHOTO;ENCODING=b;TYPE=JPEG:AAAA\r\n BBBB\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:ann\r\nEMAIL:ann@x.example\r\nEND:VCARD\r\n";
        let cards = parse(v3);
        assert_eq!(cards.len(), 2);
        let z = &cards[0];
        assert_eq!((z.name.as_str(), z.given.as_str(), z.family.as_str()), ("Zoe Park", "Zoe", "Park"));
        assert_eq!(
            z.emails,
            vec![Typed { value: "zoe@studio.example".into(), kind: "work".into() }, Typed { value: "zoe@home.example".into(), kind: "home".into() }]
        );
        assert_eq!(z.phones[0], Typed { value: "+49 151 234".into(), kind: "cell".into() });
        assert_eq!((z.org.as_str(), z.title.as_str()), ("Studio Lantern, Story", "Director"));
        assert_eq!(z.note, "Prefers mornings, not Mondays.\nCall first.");
        assert_eq!(z.birthday, "1990-04-01");
        assert!(z.extra.iter().any(|x| x.contains("AAAABBBB")), "the rest is kept");
        assert_eq!(cards[1].display(), "ann@x.example");
        let back = parse(&write(z));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], Card { org: "Studio Lantern\\, Story".replace("\\,", ","), ..z.clone() });
        assert!(z.matches("lantern") && z.matches("ZOE@home") && z.matches("+49151") && !z.matches("ann"));
    }

    /// Issue #1: addresses and birthdays are fields of their own (Outlook's Business, Home and
    /// Other addresses, and Birthday).
    #[test]
    fn addresses_and_birthdays() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Jos\r\nADR;TYPE=WORK:;;An street 1;Brussel;;1000;Belgium\r\n\
ADR;TYPE=home:;Flat 2;Rue Haute 9;Gent;Oost-Vlaanderen;9000;Belgium\r\nBDAY:--0712\r\nTEL;TYPE=WORK:+32 2 123\r\n\
TEL;TYPE=HOME:+32 9 456\r\nTEL;TYPE=CELL:+32 490\r\nEND:VCARD\r\n";
        let c = parse(text).remove(0);
        assert_eq!(c.addresses.len(), 2);
        assert_eq!((c.addresses[0].kind.as_str(), c.addresses[0].street.as_str(), c.addresses[0].postcode.as_str()), ("work", "An street 1", "1000"));
        assert_eq!(c.addresses[1].street, "Flat 2\nRue Haute 9");
        assert_eq!(c.addresses[0].one_line(), "An street 1, 1000 Brussel, Belgium");
        assert_eq!(c.birthday, "--07-12");
        let kinds: Vec<&str> = c.phones.iter().map(|p| p.kind.as_str()).collect();
        assert_eq!(kinds, ["work", "home", "cell"]);
        let back = parse(&write(&c)).remove(0);
        assert_eq!((back.addresses.clone(), back.birthday.clone(), back.phones.clone()), (c.addresses.clone(), c.birthday.clone(), c.phones.clone()));
        assert!(write(&Card { birthday: "1990-04-01".into(), ..Default::default() }).contains("BDAY:19900401"));
        for (v, want) in [
            ("19900401", Some("1990-04-01")),
            ("1990-04-01T00:00:00Z", Some("1990-04-01")),
            ("--0401", Some("--04-01")),
            ("1990-13-01", None),
            ("x", None),
            ("", None),
        ] {
            assert_eq!(birthday_of(v).as_deref(), want, "{v}");
        }
    }

    /// Version 2.1 cards (older phones, exported address books) write accented text as
    /// quoted-printable, with soft line breaks and a character set.
    #[test]
    fn quoted_printable_values() {
        let v = "BEGIN:VCARD\r\nVERSION:2.1\r\n\
N;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:M=C3=BCller;J=C3=BCrgen\r\n\
FN;CHARSET=ISO-8859-1;QUOTED-PRINTABLE:J=FCrgen M=FCller\r\n\
NOTE;ENCODING=QUOTED-PRINTABLE:Caf=C3=A9 au lait,=0D=0Athen a very long line that has been bro=\r\n\
ken in two\r\n\
ORG;CHARSET=WINDOWS-1252;ENCODING=QUOTED-PRINTABLE:=93Studio=94 =3D 50=80\r\n\
TITLE:Price=5 not encoded\r\nEND:VCARD\r\n";
        let c = parse(v).remove(0);
        assert_eq!((c.family.as_str(), c.given.as_str()), ("Müller", "Jürgen"));
        assert_eq!(c.name, "Jürgen Müller");
        assert_eq!(c.note, "Café au lait,\r\nthen a very long line that has been broken in two");
        assert_eq!(c.org, "“Studio” = 50€");
        assert_eq!(c.title, "Price=5 not encoded");
        // Broken escapes stay as written.
        assert_eq!(quoted_printable("a=Zb=4", String::new()), "a=Zb=4");
        assert_eq!(quoted_printable("=FF=FE", String::new()), "ÿþ");
    }

    #[test]
    fn junk_is_harmless() {
        for junk in ["", "BEGIN:VCARD", "END:VCARD", ":::", "BEGIN:VCARD\nFN\nEMAIL;TYPE=\"a:b\":x@y\nEND:VCARD", &"BEGIN:VCARD\n".repeat(1000)] {
            let _ = parse(junk);
        }
        assert_eq!(parse("BEGIN:VCARD\nEMAIL;TYPE=\"a:b\":x@y\nEND:VCARD")[0].emails[0].value, "x@y", "colons in quoted parameters");
    }
}
