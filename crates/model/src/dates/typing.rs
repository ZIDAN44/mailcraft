//! A time of day as people type it into a box: `13:30` on a 24-hour clock, or on a 12-hour one
//! `1:30 PM`, `1:30pm`, `1:30 p.m.`, `1 PM`, and each language's own way (`午後1:30`,
//! `1:30 a. m.`), whichever clock File › Options shows times on.

/// Longer than any time typed: refused before it's looked at.
const MAX_TYPED: usize = 64;

/// The words that say which half of the day a 12-hour time is in: `AM` and `PM`, and those of
/// each language added. They're compared folded: without case, dots, spaces, quote marks or
/// the marks that order right-to-left text, so `p.m.`, `P M` and `pm` are the same word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayHalves {
    /// (a word folded, whether it says after noon).
    marks: Vec<(String, bool)>,
}

impl Default for DayHalves {
    fn default() -> DayHalves {
        DayHalves { marks: vec![("am".into(), false), ("pm".into(), true)] }
    }
}

impl DayHalves {
    /// A language's words for before and after noon: what's left of its 12-hour time with the
    /// time taken out, wherever it stood (`午後{time}` leaves `午後`, `午後{hour}時` `午後時`).
    /// A word already known keeps the half it was first given; one that folds to nothing is
    /// left out, as a time with no word is read on the 24-hour clock.
    pub fn add(&mut self, am: &str, pm: &str) {
        for (word, after_noon) in [(am, false), (pm, true)] {
            let word = fold(word);
            if !word.is_empty() && !self.marks.iter().any(|(w, _)| *w == word) {
                self.marks.push((word, after_noon));
            }
        }
    }

    /// Whether a folded word says after noon (`None`: it says neither).
    fn after_noon(&self, word: &str) -> Option<bool> {
        self.marks.iter().find(|(w, _)| w == word).map(|(_, pm)| *pm)
    }
}

/// A typed time of day as (hour 0 to 23, minute), or `None` when it isn't one.
///
/// `HH:MM` (or `H:MM`) on the 24-hour clock; or an hour from 1 to 12, with or without its
/// minutes, and a word of `halves` before or after it (`1 PM`, `12:30 am`, `午後1:30`):
/// `12 AM` is midnight, `12 PM` noon. Digits typed full-width or in Arabic-Indic figures, and a
/// full-width colon, count as their ASCII selves. `13 PM`, `25:00`, `1:5` and words it doesn't
/// know are refused.
pub fn read_time(text: &str, halves: &DayHalves) -> Option<(u32, u32)> {
    if text.len() > MAX_TYPED {
        return None;
    }
    let typed: String = text.chars().map(ascii).collect();
    let start = typed.find(|c: char| c.is_ascii_digit())?;
    let after = typed.get(start..)?;
    let end = start.saturating_add(after.find(|c: char| !(c.is_ascii_digit() || c == ':')).unwrap_or(after.len()));
    let number = typed.get(start..end)?;
    let word = fold(&format!("{}{}", typed.get(..start)?, typed.get(end..)?));
    let (hour, minute) = match number.split_once(':') {
        Some((h, m)) if m.len() == 2 => (h, Some(m)),
        Some(_) => return None,
        None => (number, None),
    };
    if hour.is_empty() || hour.len() > 2 {
        return None;
    }
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = match minute {
        Some(m) => m.parse().ok()?,
        None => 0,
    };
    if minute > 59 {
        return None;
    }
    if word.is_empty() {
        // The 24-hour clock, minutes and all: a bare `9` could be either half of the day.
        return (number.contains(':') && hour < 24).then_some((hour, minute));
    }
    let pm = halves.after_noon(&word)?;
    (1..=12).contains(&hour).then_some((hour % 12 + if pm { 12 } else { 0 }, minute))
}

/// A digit or colon typed full-width (as East Asian input methods do) or in Arabic-Indic
/// figures, as ASCII; anything else as it is.
fn ascii(c: char) -> char {
    let digit = |zero: char| char::from_digit(u32::from(c).saturating_sub(u32::from(zero)), 10).unwrap_or(c);
    match c {
        '０'..='９' => digit('０'),
        '\u{660}'..='\u{669}' => digit('\u{660}'),
        '\u{6f0}'..='\u{6f9}' => digit('\u{6f0}'),
        '：' => ':',
        c => c,
    }
}

/// A word as [`DayHalves`] compares it: lower case, without dots, spaces, quote marks (Hebrew
/// writes its abbreviations with gershayim, typed as `"` as often) or the invisible marks that
/// order right-to-left text.
fn fold(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !(c.is_whitespace()
                || matches!(
                    c,
                    '.' | '．'
                        | '"'
                        | '\''
                        | '\u{5f3}'
                        | '\u{5f4}'
                        | '\u{200b}'..='\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{61c}'
                        | '\u{feff}'
                ))
        })
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests;
