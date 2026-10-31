//! Finding events (issue #33): what's typed in the calendar's search box.
//!
//! As the mail search box reads it: bare words match anywhere (as word prefixes, so `budg` finds
//! "Budget"), `"quoted phrases"` as written, and a field narrows them: `subject:` (`title:`),
//! `location:` (`where:`, `room:`), `notes:` (`body:`, `description:`), `organizer:` (`from:`),
//! `attendee:` (`to:`, `cc:`, `with:`), `category:`. A field takes a list, `organizer:ana,jun`
//! being either. `is:private` and `is:recurring` pick those; `after:` and `before:` a date
//! (`YYYY-MM-DD`), its occurrences starting on or after it, or before it. A leading `-` turns any
//! of them round. The caller places the dates in its zone and finds the occurrences.

use super::*;
use crate::search::words;

/// Where in an event a term looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventField {
    Any,
    Subject,
    Location,
    Notes,
    Organizer,
    Attendees,
    Categories,
}

/// What a term looks for: words (each a word's start), or a phrase as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Needle {
    Words(Vec<String>),
    Phrase(String),
}

/// One part of a query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventTerm {
    /// Any one of these in the field.
    Text(EventField, Vec<Needle>),
    /// Kept private or confidential (`is:private`).
    Private,
    /// A series (`is:recurring`).
    Recurring,
}

/// A parsed calendar search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventQuery {
    /// Each with whether it's turned round (`-`); all must hold.
    pub terms: Vec<(EventTerm, bool)>,
    /// The first day (days since the epoch, in the caller's zone) occurrences may start on.
    pub after: Option<i64>,
    /// The day occurrences must start before.
    pub before: Option<i64>,
}

/// The fields the box knows, and where each looks.
const FIELDS: &[(&str, EventField)] = &[
    ("subject", EventField::Subject),
    ("title", EventField::Subject),
    ("location", EventField::Location),
    ("where", EventField::Location),
    ("room", EventField::Location),
    ("notes", EventField::Notes),
    ("body", EventField::Notes),
    ("description", EventField::Notes),
    ("organizer", EventField::Organizer),
    ("organiser", EventField::Organizer),
    ("from", EventField::Organizer),
    ("attendee", EventField::Attendees),
    ("attendees", EventField::Attendees),
    ("to", EventField::Attendees),
    ("cc", EventField::Attendees),
    ("with", EventField::Attendees),
    ("category", EventField::Categories),
    ("categories", EventField::Categories),
];

/// The most of an event's notes looked through.
const NOTES_SEARCHED: usize = 64 * 1024;

fn is_field(k: &str) -> bool {
    FIELDS.iter().any(|(f, _)| f.eq_ignore_ascii_case(k)) || ["is", "after", "before"].iter().any(|f| f.eq_ignore_ascii_case(k))
}

/// A date: `2026-10-22`, `2026/10/22` or `20261022`, as days since the epoch.
fn date_value(v: &str) -> Option<i64> {
    let digits: String = v.trim().chars().filter(|c| c.is_ascii_digit()).collect();
    let sep_ok = v.trim().chars().all(|c| c.is_ascii_digit() || c == '-' || c == '/');
    (sep_ok && digits.len() == 8).then(|| date_days(&digits)).flatten()
}

fn needle(value: &str) -> Option<Needle> {
    if let Some(inner) = value.strip_prefix('"') {
        let phrase = inner.strip_suffix('"').unwrap_or(inner).trim().to_lowercase();
        return (!phrase.is_empty()).then_some(Needle::Phrase(phrase));
    }
    let w: Vec<String> = words(value).collect();
    if w.is_empty() {
        // Punctuation alone (`@`, `2.14`'s dot) is looked for as written.
        let v = value.trim().to_lowercase();
        return (!v.is_empty()).then_some(Needle::Phrase(v));
    }
    Some(Needle::Words(w))
}

/// A field's value: one needle, or a list at commas outside quotes (any of them).
fn needles(value: &str) -> Vec<Needle> {
    let mut parts = vec![String::new()];
    let mut quoted = false;
    for c in value.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                if let Some(p) = parts.last_mut() {
                    p.push(c);
                }
            }
            ',' if !quoted && parts.len() < 50 => parts.push(String::new()),
            c => {
                if let Some(p) = parts.last_mut() {
                    p.push(c);
                }
            }
        }
    }
    let list: Vec<Needle> = parts.iter().filter_map(|p| needle(p.trim())).collect();
    if list.is_empty() { needle(value).into_iter().collect() } else { list }
}

impl EventQuery {
    pub fn parse(input: &str) -> EventQuery {
        let mut q = EventQuery::default();
        let waiting = |tok: &str| tok.strip_prefix('-').unwrap_or(tok).strip_suffix(':').is_some_and(is_field);
        for tok in crate::search::split_tokens_with(input, waiting).into_iter().take(64) {
            let (negated, tok) = match tok.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest.to_string()),
                _ => (false, tok),
            };
            let field = tok.split_once(':').filter(|(k, v)| !v.is_empty() && !k.starts_with('"'));
            let term = match field {
                Some((k, v)) if k.eq_ignore_ascii_case("after") || k.eq_ignore_ascii_case("before") => {
                    // A date that can't be read is looked for as words.
                    match date_value(v) {
                        Some(d) => {
                            // `-after:x` is before it, `-before:x` from it on.
                            if k.eq_ignore_ascii_case("after") != negated {
                                q.after = Some(q.after.map_or(d, |a| a.max(d)));
                            } else {
                                q.before = Some(q.before.map_or(d, |b| b.min(d)));
                            }
                            continue;
                        }
                        None => Some(EventTerm::Text(EventField::Any, needles(&tok))),
                    }
                }
                Some((k, v)) if k.eq_ignore_ascii_case("is") => match v.to_ascii_lowercase().as_str() {
                    "private" | "confidential" => Some(EventTerm::Private),
                    "recurring" | "series" => Some(EventTerm::Recurring),
                    _ => Some(EventTerm::Text(EventField::Any, needles(&tok))),
                },
                Some((k, v)) => match FIELDS.iter().find(|(f, _)| f.eq_ignore_ascii_case(k)) {
                    Some((_, f)) => Some(EventTerm::Text(*f, needles(v))),
                    None => Some(EventTerm::Text(EventField::Any, needles(&tok))),
                },
                None => Some(EventTerm::Text(EventField::Any, needles(&tok))),
            };
            if let Some(t) = term.filter(|t| !matches!(t, EventTerm::Text(_, n) if n.is_empty())) {
                q.terms.push((t, negated));
            }
        }
        q
    }

    /// Nothing to look for.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.after.is_none() && self.before.is_none()
    }

    /// Whether an event is one looked for (the dates aside: they pick its occurrences).
    pub fn matches(&self, e: &CalEvent) -> bool {
        self.terms.iter().all(|(t, negated)| term_matches(t, e) != *negated)
    }
}

fn people(list: &[&Attendee]) -> String {
    let mut s = String::new();
    for a in list {
        s.push_str(&a.name);
        s.push(' ');
        s.push_str(&a.email);
        s.push(' ');
    }
    s
}

fn field_text(f: EventField, e: &CalEvent) -> String {
    let notes = || {
        let cut = e.description.char_indices().nth(NOTES_SEARCHED).map_or(e.description.len(), |(i, _)| i);
        e.description.get(..cut).unwrap_or_default().to_string()
    };
    match f {
        EventField::Subject => e.summary.clone(),
        // Where it is online too (issue #25): its server's join link, and the `CONFERENCE`
        // links another calendar gave it without putting them in its place or notes.
        EventField::Location => {
            let mut s = e.location.clone();
            let online = std::iter::once(e.join_url.clone()).chain(e.conferences().into_iter().flat_map(|c| [c.uri, c.label]));
            for x in online.filter(|x| !x.trim().is_empty()) {
                if !s.contains(x.as_str()) {
                    s.push_str(" \u{1f} ");
                    s.push_str(&x);
                }
            }
            s
        }
        EventField::Notes => notes(),
        EventField::Organizer => people(&e.organizer.iter().collect::<Vec<_>>()),
        EventField::Attendees => people(&e.attendees.iter().collect::<Vec<_>>()),
        EventField::Categories => e.categories.join(" \u{1f} "),
        EventField::Any => {
            [EventField::Subject, EventField::Location, EventField::Organizer, EventField::Attendees, EventField::Categories, EventField::Notes]
                .into_iter()
                .map(|f| field_text(f, e))
                .collect::<Vec<_>>()
                .join(" \u{1f} ")
        }
    }
}

fn term_matches(t: &EventTerm, e: &CalEvent) -> bool {
    match t {
        EventTerm::Private => privacy(e) != Privacy::Public,
        EventTerm::Recurring => e.rrule.is_some() || e.recurrence_id.is_some(),
        EventTerm::Text(f, list) => {
            let text = field_text(*f, e);
            let lower = text.to_lowercase();
            let have: Vec<String> = words(&text).collect();
            list.iter().any(|n| match n {
                Needle::Phrase(p) => lower.contains(p.as_str()),
                // Each word starts one of the field's (an address, or a room's `2.14`, inside one).
                Needle::Words(ws) => {
                    ws.iter().all(|w| have.iter().any(|h| h.starts_with(w.as_str())) || (w.contains(['@', '.']) && lower.contains(w.as_str())))
                }
            })
        }
    }
}
