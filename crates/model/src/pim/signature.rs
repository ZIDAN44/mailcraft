//! Contact details found in the signature of someone's message (issue #178): their phone
//! numbers and web addresses, offered to add to their contact, never filled in by themselves (a
//! signature is easily read wrong).

use super::ical::links_in;

/// The most lines of a signature looked at.
const SIGNATURE_LINES: usize = 12;
/// The most lines from the end of a message looked at for one with no `-- ` above it.
const TAIL_LINES: usize = 15;
/// The most of each kind found.
const FOUND_MAX: usize = 5;
/// The most characters of a message read.
const TEXT_MAX: usize = 200_000;

/// What a signature gives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoundDetails {
    pub phones: Vec<String>,
    pub websites: Vec<String>,
}

impl FoundDetails {
    pub fn is_empty(&self) -> bool {
        self.phones.is_empty() && self.websites.is_empty()
    }
}

/// Where a reply's quoted message begins: a quoted line, Outlook's "Original Message" or its
/// From header, or a "… wrote:" line.
fn quote_starts(line: &str) -> bool {
    let l = line.trim();
    let lower = l.to_ascii_lowercase();
    l.starts_with('>')
        || lower.starts_with("-----original message")
        || (lower.starts_with("from:") && lower.contains('@'))
        || (lower.ends_with("wrote:") && lower.len() < 200)
}

/// The details in the signature of a message's text from `name` (`email`): the lines after its
/// `-- ` line; else those from the last line with their name among the message's last lines;
/// else its last lines. Quoted text after is left out.
pub fn signature_details(text: &str, name: &str) -> FoundDetails {
    let text = match text.char_indices().nth(TEXT_MAX) {
        Some((i, _)) => text.get(..i).unwrap_or(text),
        None => text,
    };
    let own: Vec<&str> = text.lines().take_while(|l| !quote_starts(l)).collect();
    let block: Vec<&str> = match own.iter().rposition(|l| l.trim_end() == "--" || l.trim_end() == "-- ") {
        Some(at) => own.iter().skip(at + 1).take(SIGNATURE_LINES).copied().collect(),
        None => {
            let lines: Vec<&str> = own.iter().copied().filter(|l| !l.trim().is_empty()).collect();
            let tail = lines.get(lines.len().saturating_sub(TAIL_LINES)..).unwrap_or(&[]);
            let first = name.split_whitespace().next().unwrap_or("").to_lowercase();
            let named = tail.iter().rposition(|l| first.chars().count() > 1 && l.to_lowercase().contains(&first));
            match named {
                Some(at) => tail.iter().skip(at).take(SIGNATURE_LINES).copied().collect(),
                None => tail.get(tail.len().saturating_sub(6)..).unwrap_or(&[]).to_vec(),
            }
        }
    };
    let mut out = FoundDetails::default();
    for line in block {
        for phone in phones_in(line) {
            let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
            let known = out.phones.iter().any(|p| p.chars().filter(char::is_ascii_digit).collect::<String>() == digits);
            if !known && out.phones.len() < FOUND_MAX {
                out.phones.push(phone);
            }
        }
        for site in websites_in(line) {
            if !out.websites.iter().any(|w| w.eq_ignore_ascii_case(&site)) && out.websites.len() < FOUND_MAX {
                out.websites.push(site);
            }
        }
    }
    out
}

/// The phone numbers in a line: runs of digits, spaces, `+`, `-`, `.`, `/` and brackets with 7 to
/// 15 digits, that aren't a date or a time, nor part of a word (`MC31 (1795385049)`, an
/// invoice's `INV2026-0001234`: their digits run on from letters).
fn phones_in(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let allowed = |c: char| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | '/' | '(' | ')' | ' ' | '\u{a0}');
    let mut run = String::new();
    // The run's first digit, `+` or `(` came straight after a letter (`MC31`), not after a
    // label's dot or a space (`Tel. +44`, `Mob.07700`).
    let mut glued = false;
    let starts = |c: char| c.is_ascii_digit() || matches!(c, '+' | '(');
    let mut prev: Option<char> = None;
    let flush = |run: &mut String, out: &mut Vec<String>, glued: bool| {
        if glued {
            run.clear();
            return;
        }
        let candidate = run.trim().trim_matches(|c: char| matches!(c, '-' | '.' | '/' | ')') || c.is_whitespace()).to_string();
        let candidate = candidate.trim_start_matches(['-', '.', '/', ')']).to_string();
        let digits = candidate.chars().filter(char::is_ascii_digit).count();
        if (7..=15).contains(&digits)
            && !looks_like_date(&candidate)
            && (candidate.starts_with('+') || candidate.starts_with('(') || candidate.starts_with(|c: char| c.is_ascii_digit()))
        {
            out.push(candidate.split_whitespace().collect::<Vec<_>>().join(" "));
        }
        run.clear();
    };
    for c in line.chars() {
        if allowed(c) {
            if starts(c) && !run.chars().any(starts) {
                glued = prev.is_some_and(char::is_alphabetic);
            }
            run.push(c);
        } else {
            flush(&mut run, &mut out, glued);
            glued = false;
        }
        prev = Some(c);
    }
    flush(&mut run, &mut out, glued);
    out
}

/// A date or a time written with digits (2026-11-22, 22.11.2026, 11/22/2026 10:30), not a phone.
fn looks_like_date(s: &str) -> bool {
    let groups: Vec<&str> = s.split(['-', '.', '/', ' ', ':']).filter(|g| !g.is_empty()).collect();
    let year = |g: &&str| g.len() == 4 && g.parse::<u32>().is_ok_and(|y| (1900..=2100).contains(&y));
    let small = |g: &&str| g.len() <= 2;
    groups.len() >= 3 && groups.iter().take(3).any(year) && groups.iter().take(3).filter(|g| small(g)).count() >= 2
}

/// The web addresses in a line: written out (`https://…`) or from `www.`.
fn websites_in(line: &str) -> Vec<String> {
    let mut out = links_in(line);
    for word in line.split_whitespace() {
        let w = word.trim_matches(|c: char| matches!(c, '<' | '>' | '(' | ')' | ',' | ';' | '"' | '\'') || c == '.');
        let lower = w.to_ascii_lowercase();
        if lower.starts_with("www.")
            && w.len() > 6
            && w.contains('.')
            && !lower.contains('@')
            && !out.iter().any(|o| o.to_ascii_lowercase().contains(&lower))
        {
            out.push(w.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_after_its_dashes() {
        let text = "Hi,\n\nThe files are attached.\n\n-- \nAna Silva\nProducer, Studio Example\nTel +1 (555) 010-2030 | Mobile: 555.010.4455\nhttps://studio.example/team\nwww.anasilva.example\n";
        let d = signature_details(text, "Ana Silva");
        assert_eq!(d.phones, ["+1 (555) 010-2030", "555.010.4455"]);
        assert_eq!(d.websites, ["https://studio.example/team", "www.anasilva.example"]);
    }

    #[test]
    fn a_signature_from_their_name_and_nothing_quoted() {
        let text =
            "Thanks, see you Monday.\n\nBest,\nZoe Park\nOffice: 020 7946 0018\n\nOn Mon, 2 Nov 2026 Riley wrote:\n> call me at 0800 123 4567\n";
        let d = signature_details(text, "Zoe Park");
        assert_eq!(d.phones, ["020 7946 0018"], "not the number quoted from the reply");
        assert!(d.websites.is_empty());
    }

    #[test]
    fn dates_times_and_short_numbers_are_no_phones() {
        let text = "-- \nMeeting 2026-11-22 10:30, room 4512\nInvoice 22.11.2026\nRef 12345\n";
        assert!(signature_details(text, "Ann").phones.is_empty());
    }

    #[test]
    fn numbers_in_a_word_are_no_phones() {
        // A reference or a name with a number in it: its digits run on from letters.
        let text = "Ann\nConfidential to MC31 (1795385049).\nInvoice INV2026-0001234\nPhone: +44 20 7946 0958";
        assert_eq!(signature_details(text, "Ann").phones, ["+44 20 7946 0958"]);
        // A label's dot isn't a word the number runs on from.
        let labels = "Ann\nTel. +44 20 7946 0959\nMob.07700 900123\nFax.+44 20 7946 0960";
        assert_eq!(signature_details(labels, "Ann").phones, ["+44 20 7946 0959", "07700 900123", "+44 20 7946 0960"]);
    }

    #[test]
    fn junk_is_harmless() {
        let long = "9".repeat(10_000);
        assert!(signature_details(&long, "x").phones.is_empty());
        assert!(signature_details("", "").is_empty());
        let many = format!("-- \n{}", "+1 555 010 1234\n".repeat(100));
        assert_eq!(signature_details(&many, "x").phones.len(), 1, "the same number once");
    }
}
