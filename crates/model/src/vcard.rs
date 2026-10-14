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

/// Read every card in a text (one `.vcf` can hold many).
pub fn parse(text: &str) -> Vec<Card> {
    let text = text.get(..MAX_INPUT).unwrap_or(text);
    let mut unfolded: Vec<String> = Vec::new();
    for l in text.split('\n') {
        let l = l.strip_suffix('\r').unwrap_or(l);
        match (l.strip_prefix(' ').or_else(|| l.strip_prefix('\t')), unfolded.last_mut()) {
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
        assert!(z.extra.iter().any(|x| x.starts_with("BDAY")) && z.extra.iter().any(|x| x.contains("AAAABBBB")), "the rest is kept");
        assert_eq!(cards[1].display(), "ann@x.example");
        let back = parse(&write(z));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], Card { org: "Studio Lantern\\, Story".replace("\\,", ","), ..z.clone() });
        assert!(z.matches("lantern") && z.matches("ZOE@home") && z.matches("+49151") && !z.matches("ann"));
    }

    #[test]
    fn junk_is_harmless() {
        for junk in ["", "BEGIN:VCARD", "END:VCARD", ":::", "BEGIN:VCARD\nFN\nEMAIL;TYPE=\"a:b\":x@y\nEND:VCARD", &"BEGIN:VCARD\n".repeat(1000)] {
            let _ = parse(junk);
        }
        assert_eq!(parse("BEGIN:VCARD\nEMAIL;TYPE=\"a:b\":x@y\nEND:VCARD")[0].emails[0].value, "x@y", "colons in quoted parameters");
    }
}
