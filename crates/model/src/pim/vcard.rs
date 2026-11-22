//! vCard (RFC 6350; 3.0 files read too): the people in an address book.
//!
//! The fields MailCraft shows and edits are read into a [`Card`]; every other property is kept
//! as written so editing a card from a server loses nothing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The longest input read: as much as a file opened is (an address book exported with its pictures and files reads
/// back whole).
const MAX_INPUT: usize = 16 << 20;

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

/// Placeholder year Exchange stores for a birthday entered as day and month only.
const NO_YEAR: &str = "1604";

/// A birthday as written (`YYYY-MM-DD`, or `--MM-DD` without the year), from vCard's `BDAY`
/// (`19900401`, `1990-04-01`, `--0401`, a date and time) or from Exchange, whose placeholder
/// year means "no year": `None` when it isn't a date.
pub fn birthday_of(v: &str) -> Option<String> {
    let v = v.trim();
    let v = v.split('T').next().unwrap_or(v);
    let digits: String = v.chars().filter(char::is_ascii_digit).collect();
    let (year, rest) = if v.starts_with("--") { (None, digits.as_str()) } else { (Some(digits.get(..4)?), digits.get(4..)?) };
    let year = year.filter(|y| *y != NO_YEAR);
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

/// The `YYYY-MM-DD` to send to Exchange: a day-and-month birthday gets the placeholder year.
pub fn birthday_for_exchange(v: &str) -> Option<String> {
    let b = birthday_of(v)?;
    Some(match b.strip_prefix("--") {
        Some(day) => format!("{NO_YEAR}-{day}"),
        None => b,
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
    /// The rest of `N` as written (additional names; prefixes; suffixes: `Marie;Dr.;Jr.`),
    /// written back after the family and given names.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name_rest: String,
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
    /// The addresses' and numbers' lines as written (`item1.TEL;TYPE=WORK,FAX;PREF=1:…`), by
    /// property and value (`TEL:+1 555 0100`): written back so, with all their types, their
    /// preference and their group, while they're of the kind read from them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub as_written: BTreeMap<String, String>,
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

    /// The name a list sorted by last name files someone under (issue #244): the family name
    /// and then the given one, from `N`; else the name shown read as "Given Family" (its last
    /// word first) or "Family, Given" (as it is). A company's card (its name is its company's)
    /// and an address alone are filed as they are.
    pub fn last_name_first(&self) -> String {
        let family = self.family.trim();
        if !family.is_empty() {
            return format!("{family} {}", self.given.trim()).trim_end().to_string();
        }
        let shown = self.display();
        let org = self.org.trim();
        if (self.name.trim().is_empty() && self.given.trim().is_empty()) || (!org.is_empty() && shown.eq_ignore_ascii_case(org)) {
            return shown;
        }
        if let Some((family, given)) = shown.split_once(',') {
            return format!("{} {}", family.trim(), given.trim()).trim().to_string();
        }
        match shown.rsplit_once(char::is_whitespace) {
            Some((given, family)) => format!("{} {}", family.trim(), given.trim()),
            None => shown,
        }
    }

    /// Whether the card matches a search (name, organisation or any address, case-insensitive).
    pub fn matches(&self, q: &str) -> bool {
        self.matches_search(&CardSearch::new(q))
    }

    /// [`Card::matches`], the search made ready once for many cards.
    pub fn matches_search(&self, q: &CardSearch) -> bool {
        let lower = q.lower.as_str();
        lower.is_empty()
            || self.display().to_lowercase().contains(lower)
            || self.org.to_lowercase().contains(lower)
            || self.emails.iter().any(|e| e.value.to_lowercase().contains(lower))
            || self.phones.iter().any(|p| p.value.replace(' ', "").contains(q.digits.as_str()))
    }
}

/// A search of the address book, made ready once: lower-cased and trimmed, and without its
/// spaces for phone numbers.
pub struct CardSearch {
    lower: String,
    digits: String,
}

impl CardSearch {
    pub fn new(q: &str) -> CardSearch {
        let lower = q.trim().to_lowercase();
        let digits = lower.replace(' ', "");
        CardSearch { lower, digits }
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

/// Where a structured value's components are parted: its unescaped `;`s.
fn separators(v: &str) -> Vec<usize> {
    let mut at = Vec::new();
    let mut escaped = false;
    for (i, c) in v.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            ';' => at.push(i),
            _ => {}
        }
    }
    at
}

/// What follows the first `n` components of a structured value, as written (`None` when it has
/// no more).
fn after_components(v: &str, n: usize) -> Option<&str> {
    let i = *separators(v).get(n.checked_sub(1)?)?;
    v.get(i + 1..)
}

/// A content line's head and value, split at its first colon outside quotes.
fn split_line(raw: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    let colon = raw.char_indices().find(|(_, c)| {
        if *c == '"' {
            quoted = !quoted;
        }
        *c == ':' && !quoted
    });
    crate::work::steps(colon.map_or(raw.len(), |(at, _)| at + 1));
    let colon = colon?;
    Some((raw.get(..colon.0).unwrap_or(""), raw.get(colon.0 + 1..).unwrap_or("")))
}

/// A head's parameters, keys upper-cased (a bare one, 3.0's `WORK`, with an empty value).
fn params_of(head: &str) -> Vec<(String, String)> {
    head.split(';')
        .skip(1)
        .map(|p| p.split_once('=').map_or((p.trim().to_ascii_uppercase(), String::new()), |(k, v)| (k.trim().to_ascii_uppercase(), v.to_string())))
        .collect()
}

/// Where an address's or number's line as written is kept ([`Card::as_written`]).
fn written_key(prop: &str, value: &str) -> String {
    format!("{prop}:{value}")
}

/// Keep an address's or number's line as written, when it can go into the 4.0 card written
/// back as it is: not 2.1's quoted-printable or character set, nor 3.0's bare `WORK`. Of two
/// with one value (a number both work and home), the first.
fn keep_written(c: &mut Card, prop: &str, value: &str, raw: &str, params: &[(String, String)]) {
    let fits = params.iter().all(|(k, v)| !v.is_empty() && k != "ENCODING" && k != "CHARSET");
    if fits {
        c.as_written.entry(written_key(prop, value)).or_insert_with(|| raw.to_string());
    }
}

/// An address's or number's line as it goes back: as written while it's of the kind read from
/// it, else with its group and other parameters and the new kind as its type.
fn written_again(line: &str, kind_now: &str) -> Option<String> {
    let (head, value) = split_line(line)?;
    if kind(&params_of(head)) == kind_now {
        return Some(line.replace(['\r', '\n'], ""));
    }
    let mut parts = head.split(';');
    let mut out = parts.next().unwrap_or("").to_string();
    for p in parts.filter(|p| p.split_once('=').is_some_and(|(k, _)| !k.trim().eq_ignore_ascii_case("TYPE"))) {
        out.push(';');
        out.push_str(p);
    }
    if !kind_now.is_empty() {
        out.push_str(";TYPE=");
        out.push_str(kind_now);
    }
    Some(format!("{out}:{value}").replace(['\r', '\n'], ""))
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

/// The longest property head read: past it a line's head is taken as settled.
const MAX_HEAD: usize = 4096;

/// A line's (quoted-printable, head still open) after it grew: its head is read again only
/// while it has no colon and is short, so each line costs at most [`MAX_HEAD`] bytes.
fn reread_head(line: &str, qp: bool, open: bool) -> (bool, bool) {
    if !open {
        return (qp, false);
    }
    let mut end = line.len().min(MAX_HEAD);
    while !line.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let head = line.get(..end).unwrap_or_default();
    crate::work::steps(head.len());
    (quoted_printable_line(head), !head.contains(':') && line.len() < MAX_HEAD)
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
    // At a character's start (a cut inside one would read the whole text).
    let mut end = MAX_INPUT.min(text.len());
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let text = text.get(..end).unwrap_or_default();
    let mut unfolded: Vec<String> = Vec::new();
    // Whether the last line's head says quoted-printable, and whether that head is still open
    // (no colon yet, and short enough to be one): kept beside the line, so a line growing by
    // continuations isn't rescanned at each one (16 MiB of `=` lines would take minutes).
    let (mut qp, mut head_open) = (false, false);
    for l in text.split('\n') {
        crate::work::steps(l.len() + 1);
        let l = l.strip_suffix('\r').unwrap_or(l);
        match (l.strip_prefix(' ').or_else(|| l.strip_prefix('\t')), unfolded.last_mut()) {
            // 2.1's quoted-printable values break lines with a trailing `=`, the next line as it is.
            (_, Some(prev)) if prev.ends_with('=') && qp => {
                prev.pop();
                prev.push_str(l);
                (qp, head_open) = reread_head(prev, qp, head_open);
            }
            (Some(rest), Some(prev)) => {
                prev.push_str(rest);
                (qp, head_open) = reread_head(prev, qp, head_open);
            }
            _ => {
                (qp, head_open) = reread_head(l, false, true);
                unfolded.push(l.to_string());
            }
        }
    }
    let mut cards = Vec::new();
    let mut cur: Option<Card> = None;
    for raw in unfolded {
        let Some((head, value)) = split_line(&raw) else { continue };
        let decoded;
        let value = if quoted_printable_line(head) {
            decoded = quoted_printable(value, charset_of(head));
            decoded.as_str()
        } else {
            value
        };
        // Properties may carry a group prefix (`item1.EMAIL`).
        let name = head.split(';').next().unwrap_or("").rsplit('.').next().unwrap_or("").trim().to_ascii_uppercase();
        let params = params_of(head);
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
                    "UID" => c.uid = crate::ical::stored_uid(value),
                    "FN" => c.name = text(),
                    "N" => {
                        let n = components(value);
                        c.family = n.first().cloned().unwrap_or_default();
                        c.given = n.get(1).cloned().unwrap_or_default();
                        let rest = after_components(value, 2).unwrap_or("");
                        c.name_rest = if rest.chars().all(|ch| ch == ';') { String::new() } else { rest.to_string() };
                    }
                    "EMAIL" if c.emails.len() < 50 => {
                        let t = Typed { value: value.trim().to_string(), kind: kind(&params) };
                        keep_written(c, "EMAIL", &t.value, &raw, &params);
                        c.emails.push(t);
                    }
                    "TEL" if c.phones.len() < 50 => {
                        let v = value.trim();
                        let t = Typed { value: v.strip_prefix("tel:").unwrap_or(v).to_string(), kind: kind(&params) };
                        keep_written(c, "TEL", &t.value, &raw, &params);
                        c.phones.push(t);
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

/// Fold a content line at 75 octets, on character boundaries; a character it may not hold
/// ([`crate::ical::allowed`]) is left out.
fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for c in line.chars().filter(|c| crate::ical::allowed(*c)) {
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
    // Five components: the rest as written, made up to three.
    let rest = c.name_rest.replace(['\r', '\n'], "");
    let pad = ";".repeat(2usize.saturating_sub(separators(&rest).len()));
    lines.push(format!("N:{};{};{rest}{pad}", esc(&c.family), esc(&c.given)));
    lines.extend(property_lines(c));
    lines.push("END:VCARD".into());
    let mut out = String::new();
    for l in lines {
        fold(&l, &mut out);
    }
    out
}

/// A card's properties past its uid and names, as unfolded content lines: its addresses, phones,
/// organization, title, note, postal addresses, birthday and the rest, those it has.
pub fn property_lines(c: &Card) -> Vec<String> {
    let mut lines = Vec::new();
    let typed = |prop: &str, t: &Typed| {
        let kind = t.kind.replace(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-', "");
        if let Some(line) = c.as_written.get(&written_key(prop, &t.value)).and_then(|l| written_again(l, &kind)) {
            line
        } else if kind.is_empty() {
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
    // Without the characters a line can't hold (line breaks too), as they'll be written; one
    // that would then begin or end a card when read back (`END:VCARD` and a control
    // character, kept as no end) is left out.
    let line = |x: &String| x.chars().filter(|ch| crate::ical::allowed(*ch)).collect::<String>();
    lines.extend(c.extra.iter().map(line).filter(|x| !begins_or_ends_a_card(x)));
    lines
}

/// Whether a line is read as a card's `BEGIN` or `END`.
fn begins_or_ends_a_card(line: &str) -> bool {
    let Some((head, value)) = split_line(line) else { return false };
    let name = head.split(';').next().unwrap_or("").rsplit('.').next().unwrap_or("").trim();
    (name.eq_ignore_ascii_case("BEGIN") || name.eq_ignore_ascii_case("END")) && value.trim().eq_ignore_ascii_case("VCARD")
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
        // Exchange's placeholder year in, `--MM-DD` out, and back again.
        assert_eq!(birthday_of("1604-02-20T00:00:00Z").as_deref(), Some("--02-20"));
        assert_eq!(birthday_for_exchange("--02-20").as_deref(), Some("1604-02-20"));
        assert_eq!(birthday_for_exchange("1990-04-01").as_deref(), Some("1990-04-01"));
        assert_eq!(birthday_for_exchange("soon"), None);
        assert!(z.extra.iter().any(|x| x.contains("AAAABBBB")), "the rest is kept");
        assert_eq!(cards[1].display(), "ann@x.example");
        let back = parse(&write(z));
        assert_eq!(back.len(), 1);
        let unkept = |c: &Card| Card { as_written: BTreeMap::new(), ..c.clone() };
        assert_eq!(unkept(&back[0]), Card { org: "Studio Lantern\\, Story".replace("\\,", ","), ..unkept(z) });
        assert!(z.as_written.iter().all(|(k, v)| back[0].as_written.get(k) == Some(v)), "lines kept as they were read: {:?}", back[0].as_written);
        assert!(z.matches("lantern") && z.matches("ZOE@home") && z.matches("+49151") && !z.matches("ann"));
    }

    /// Found in the audit: a card's uid or name with a control character was written raw (RFC
    /// 6350 allows none but HTAB); it's left out of what's written and of a uid read, so the
    /// uid kept is the one written.
    #[test]
    fn a_control_character_is_neither_written_nor_kept_in_a_uid() {
        let c = Card { uid: "zoe\u{1}-1".into(), name: "Zoe\u{1b} Park".into(), ..Default::default() };
        let text = write(&c);
        assert!(!text.contains(|c: char| c.is_ascii_control() && !matches!(c, '\t' | '\r' | '\n')), "{text:?}");
        let back = parse(&text).remove(0);
        assert_eq!((back.uid.as_str(), back.name.as_str()), ("zoe-1", "Zoe Park"));
        let kept = parse(&text.replace("UID:zoe-1", "UID:zoe\u{1}-1")).remove(0);
        assert_eq!(kept.uid, "zoe-1");
        // A line kept as written that would end the card once its control is left out.
        let odd = "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:z\r\nFN:Zoe\r\nEND:VCARD\u{1}\r\nX-AFTER:kept\r\nEND:VCARD\r\n";
        let z = parse(odd).remove(0);
        let back = parse(&write(&z));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].extra, ["X-AFTER:kept"]);
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

    /// An address book is read to its first 16 MiB, cut at a character's start: a cut that falls
    /// inside one doesn't read the whole text (the card past the cut isn't read).
    #[test]
    fn read_to_the_limit_at_a_character_s_start() {
        let mut text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ann\r\nEND:VCARD\r\nX-PAD:".to_string();
        if (MAX_INPUT - text.len()).is_multiple_of(2) {
            text.push('x');
        }
        text.push_str(&"ü".repeat((MAX_INPUT - text.len()) / 2 + 1));
        assert!(!text.is_char_boundary(MAX_INPUT), "the limit falls inside a character");
        text.push_str("\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ben\r\nEND:VCARD\r\n");
        let names: Vec<String> = parse(&text).into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["Ann"]);
    }

    /// A quoted-printable line with no colon that goes on for 16 MiB of `=` breaks is read in
    /// one pass (its head was read again at each break: about 15 minutes), and a head folded
    /// before its encoding still reads. The bytes looked through are counted, held in proportion
    /// to the card.
    #[test]
    fn endless_quoted_printable_breaks_read_quickly() {
        let text = endless_quoted_printable_breaks();
        assert!(crate::work::linear(text.len(), || parse(&text)).is_empty());
        let folded = parse("BEGIN:VCARD\nNOTE;ENCODING=QUOTED-\n PRINTABLE:caf=\n=C3=A9\nEND:VCARD");
        assert_eq!(folded[0].note, "café");
    }

    /// A quoted-printable line with no colon, its `=` breaks as long as is read.
    fn endless_quoted_printable_breaks() -> String {
        let mut text = "BEGIN:VCARD\nX;ENCODING=QUOTED-PRINTABLE;Y=".to_string();
        text.push_str(&"\nz=".repeat((MAX_INPUT - text.len()) / 3));
        text
    }

    /// How long that card takes: `cargo test --release -p mailcraft-model --lib
    /// how_long_endless_breaks_take -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement: run in release"]
    fn how_long_endless_breaks_take() {
        let text = endless_quoted_printable_breaks();
        let started = std::time::Instant::now();
        let _ = parse(&text);
        println!("MEASURE {} MiB of quoted-printable breaks: {} ms", text.len() >> 20, started.elapsed().as_millis());
    }

    #[test]
    fn junk_is_harmless() {
        for junk in ["", "BEGIN:VCARD", "END:VCARD", ":::", "BEGIN:VCARD\nFN\nEMAIL;TYPE=\"a:b\":x@y\nEND:VCARD", &"BEGIN:VCARD\n".repeat(1000)] {
            let _ = parse(junk);
        }
        assert_eq!(parse("BEGIN:VCARD\nEMAIL;TYPE=\"a:b\":x@y\nEND:VCARD")[0].emails[0].value, "x@y", "colons in quoted parameters");
    }

    /// A card from a server written back as it was read: all of its name (a middle name, a
    /// prefix and a suffix were dropped), and its addresses' and numbers' lines whole, with their
    /// group (an Apple label beside one was left naming nothing), every type and their preference.
    /// One whose kind was changed is written with the new kind, its group and preference kept.
    #[test]
    fn a_server_card_is_written_back_whole() {
        let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:zoe\r\nFN:Dr. Zoe Marie Park Jr.\r\nN:Park;Zoe;Marie;Dr.;Jr.\r\n\
item1.TEL;TYPE=WORK,FAX;PREF=1:+1 555 0100\r\nitem1.X-ABLabel:Studio fax\r\n\
EMAIL;TYPE=INTERNET,WORK;PREF=1:zoe@studio.example\r\nTEL;VALUE=uri;TYPE=cell:tel:+49-151-234\r\nEND:VCARD\r\n";
        let c = parse(card).remove(0);
        assert_eq!((c.given.as_str(), c.family.as_str()), ("Zoe", "Park"));
        assert_eq!(c.phones[1], Typed { value: "+49-151-234".into(), kind: "cell".into() });
        let out = write(&c);
        for line in [
            "N:Park;Zoe;Marie;Dr.;Jr.",
            "item1.TEL;TYPE=WORK,FAX;PREF=1:+1 555 0100",
            "item1.X-ABLabel:Studio fax",
            "EMAIL;TYPE=INTERNET,WORK;PREF=1:zoe@studio.example",
            "TEL;VALUE=uri;TYPE=cell:tel:+49-151-234",
        ] {
            assert!(out.lines().any(|l| l == line), "{line}:\n{out}");
        }
        let mut changed = c.clone();
        changed.phones[0].kind = "home".into();
        changed.given = "Zoë".into();
        let out = write(&changed);
        assert!(out.lines().any(|l| l == "item1.TEL;PREF=1;TYPE=home:+1 555 0100"), "{out}");
        assert!(out.lines().any(|l| l == "N:Park;Zoë;Marie;Dr.;Jr."), "{out}");
        // A card made here is written as before.
        let mine = Card { name: "Ann Lee".into(), given: "Ann".into(), family: "Lee".into(), ..Default::default() };
        assert!(write(&mine).lines().any(|l| l == "N:Lee;Ann;;;"));
    }

    /// Filed by last name (issue #244): the family name from `N`, else the name shown's last word
    /// (or what's before its comma); a company and an address alone as they are.
    #[test]
    fn people_are_filed_by_last_name() {
        let card =
            |name: &str, given: &str, family: &str| Card { name: name.into(), given: given.into(), family: family.into(), ..Default::default() };
        assert_eq!(card("Zoe Park", "Zoe", "Park").last_name_first(), "Park Zoe");
        // N says which word is the family name, where the name shown doesn't.
        assert_eq!(card("Ana María García López", "Ana María", "García López").last_name_first(), "García López Ana María");
        assert_eq!(card("", "", "Okafor").last_name_first(), "Okafor");
        assert_eq!(card("Sam Okafor", "", "").last_name_first(), "Okafor Sam");
        assert_eq!(card("Mira van Dijk", "", "").last_name_first(), "Dijk Mira van");
        assert_eq!(card("Park, Zoe", "", "").last_name_first(), "Park Zoe");
        assert_eq!(card("Cher", "", "").last_name_first(), "Cher");
        assert_eq!(card("", "Ann", "").last_name_first(), "Ann");
        let shop = Card { name: "Print Shop".into(), org: "Print Shop".into(), ..Default::default() };
        assert_eq!(shop.last_name_first(), "Print Shop");
        let address = Card { emails: vec![Typed { value: "ann@x.example".into(), ..Default::default() }], ..Default::default() };
        assert_eq!(address.last_name_first(), "ann@x.example");
        assert_eq!(Card::default().last_name_first(), "");
    }
}
