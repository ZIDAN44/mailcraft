//! Dates in a search: `received:` (also `sent:`, `date:`), `after:` and `before:`.
//!
//! - `received:2024`, `received:2024-03`, `received:2024-03-15` (or `/` between, or `20240315`)
//!   are that whole year, month or day; so are `today`, `yesterday`, `"this week"`, `"last week"`
//!   (also `lastweek`, `last-week`), and the same for months and years. Weeks start on Monday.
//! - `received:2024-01-01..2024-03-31` runs from the first day's start to the last one's end (one
//!   side may be left out); `received:>=2024-03-01`, `>`, `<` and `<=` compare with a period's
//!   start (`>=`, `<`) or end (`>`, `<=`).
//! - `after:X` is X's start on, and `before:X` everything before X starts (as Gmail reads them).
//!
//! Years run 1970 to 9999 and days are checked against their month; anything else is no date,
//! and the term is left out with a [`Problem`](super::Problem) rather than searched as words.
//! The days are the user's: [`Query::resolve`] places them in the local zone (each on the offset
//! in force that day), from the clock the caller passes, as the model never reads the time.

use crate::dates::{Clock, civil_from_days, days_from_civil, month_len};

use super::{Query, Term};

/// A period named in a search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatePoint {
    Day { y: i64, m: u32, d: u32 },
    Month { y: i64, m: u32 },
    Year { y: i64 },
    Rel(Relative),
}

/// A period named from today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relative {
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    ThisMonth,
    LastMonth,
    ThisYear,
    LastYear,
}

/// Which end of a period a bound is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// The period's first instant.
    Start,
    /// The instant after it (the next period's start).
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateBound {
    pub point: DatePoint,
    pub edge: Edge,
}

/// Mail dated from `from` (on) to `to` (before), either open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateSpec {
    pub from: Option<DateBound>,
    pub to: Option<DateBound>,
    /// `[from, to)` in Unix seconds, once [resolved](Query::resolve).
    pub range: Option<(i64, i64)>,
    /// Whether it reads when the mail was sent, its Date: (`sent:`, `date:`), rather than when
    /// it came ([`Summary::when`](crate::Summary::when)).
    pub sent: bool,
}

const DAY: i64 = 86_400;

/// The term of `received:` (`sent:`, `date:`) with this value, or `None` when it's no date.
pub(super) fn period(value: &str) -> Option<DateSpec> {
    let v = unquote(value);
    let bound = |p: DatePoint, edge| Some(DateBound { point: p, edge });
    let spec = |from, to| Some(DateSpec { from, to, range: None, sent: false });
    for (op, from_edge, to_edge) in
        [(">=", Some(Edge::Start), None), ("<=", None, Some(Edge::End)), (">", Some(Edge::End), None), ("<", None, Some(Edge::Start))]
    {
        if let Some(rest) = v.strip_prefix(op) {
            let p = point(rest)?;
            return spec(from_edge.and_then(|e| bound(p, e)), to_edge.and_then(|e| bound(p, e)));
        }
    }
    if let Some((a, b)) = v.split_once("..") {
        let (a, b) = (a.trim(), b.trim());
        if a.is_empty() && b.is_empty() {
            return None;
        }
        let from = if a.is_empty() { None } else { bound(point(a)?, Edge::Start) };
        let to = if b.is_empty() { None } else { bound(point(b)?, Edge::End) };
        return spec(from, to);
    }
    let p = point(&v)?;
    spec(bound(p, Edge::Start), bound(p, Edge::End))
}

/// `after:X`: from X's start on.
pub(super) fn after(value: &str) -> Option<DateSpec> {
    let p = point(&unquote(value))?;
    Some(DateSpec { from: Some(DateBound { point: p, edge: Edge::Start }), to: None, range: None, sent: false })
}

/// `before:X`: before X starts.
pub(super) fn before(value: &str) -> Option<DateSpec> {
    let p = point(&unquote(value))?;
    Some(DateSpec { from: None, to: Some(DateBound { point: p, edge: Edge::Start }), range: None, sent: false })
}

/// Whether a value is one day written out (`2024-03-15`, `2024/3/15`, `20240315`): what the
/// Advanced form's date fields hold.
pub fn is_day(value: &str) -> bool {
    matches!(point(value.trim()), Some(DatePoint::Day { .. }))
}

/// Whether a value is meant as a date, if not a good one: it starts with a digit or a comparison
/// (`2024-13`, `>=`, `..`) or one of the names from today. Anything else (`before: launch`,
/// `date: friday`) is words that happen to follow the field's name.
pub(super) fn looks_like_date(value: &str) -> bool {
    let v = unquote(value);
    let named = squashed(&v);
    v.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '<' | '>' | '=' | '.')) || RELATIVE.iter().any(|(n, _)| named.starts_with(n))
}

/// The periods named from today, as [`squashed`].
const RELATIVE: [(&str, Relative); 8] = [
    ("today", Relative::Today),
    ("yesterday", Relative::Yesterday),
    ("thisweek", Relative::ThisWeek),
    ("lastweek", Relative::LastWeek),
    ("thismonth", Relative::ThisMonth),
    ("lastmonth", Relative::LastMonth),
    ("thisyear", Relative::ThisYear),
    ("lastyear", Relative::LastYear),
];

/// A value as a name from today is read: lower-cased, without spaces, `-` or `_`
/// (`"Last week"`, `last-week` and `last_week` are `lastweek`).
fn squashed(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).collect::<String>().to_ascii_lowercase()
}

fn unquote(value: &str) -> String {
    let v = value.trim();
    let v = v.strip_prefix('"').unwrap_or(v);
    v.strip_suffix('"').unwrap_or(v).trim().to_ascii_lowercase()
}

/// A period as written: a year, month or day, or one named from today.
fn point(s: &str) -> Option<DatePoint> {
    let named = squashed(s);
    if let Some((_, r)) = RELATIVE.iter().find(|(n, _)| *n == named) {
        return Some(DatePoint::Rel(*r));
    }
    let s = s.trim();
    // `20240315`, as the calendar's search reads it too.
    if s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit()) {
        let (y, md) = s.split_at(4);
        let (m, d) = md.split_at(2);
        return day(y, m, d);
    }
    let parts: Vec<&str> = s.split(['-', '/']).collect();
    match parts.as_slice() {
        [y] => year(y).map(|y| DatePoint::Year { y }),
        [y, m] => Some(DatePoint::Month { y: year(y)?, m: month(m)? }),
        [y, m, d] => day(y, m, d),
        _ => None,
    }
}

fn number(s: &str, max_len: usize) -> Option<u32> {
    (!s.is_empty() && s.len() <= max_len && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

fn year(s: &str) -> Option<i64> {
    number(s, 4).filter(|y| s.len() == 4 && (1970..=9999).contains(y)).map(i64::from)
}

fn month(s: &str) -> Option<u32> {
    number(s, 2).filter(|m| (1..=12).contains(m))
}

fn day(y: &str, m: &str, d: &str) -> Option<DatePoint> {
    let (y, m, d) = (year(y)?, month(m)?, number(d, 2)?);
    // A day its month has: not 2023-02-29.
    (1..=month_len(y, m)).contains(&i64::from(d)).then_some(DatePoint::Day { y, m, d })
}

fn next_month(y: i64, m: u32) -> (i64, u32) {
    if m >= 12 { (y.saturating_add(1), 1) } else { (y, m.saturating_add(1)) }
}

fn previous_month(y: i64, m: u32) -> (i64, u32) {
    if m <= 1 { (y.saturating_sub(1), 12) } else { (y, m - 1) }
}

impl DatePoint {
    /// The period's days (since the epoch, local): its first, and the one after its last.
    fn days(self, clock: &Clock) -> (i64, i64) {
        let month = |y: i64, m: u32| {
            let (ny, nm) = next_month(y, m);
            (days_from_civil(y, m, 1), days_from_civil(ny, nm, 1))
        };
        let year = |y: i64| (days_from_civil(y, 1, 1), days_from_civil(y.saturating_add(1), 1, 1));
        match self {
            DatePoint::Day { y, m, d } => {
                let at = days_from_civil(y, m, d);
                (at, at.saturating_add(1))
            }
            DatePoint::Month { y, m } => month(y, m),
            DatePoint::Year { y } => year(y),
            DatePoint::Rel(r) => {
                let today = clock.local(clock.now).div_euclid(DAY);
                let monday = today.saturating_sub(i64::from(crate::dates::weekday_index(today)));
                let (y, m, _) = civil_from_days(today);
                match r {
                    Relative::Today => (today, today.saturating_add(1)),
                    Relative::Yesterday => (today.saturating_sub(1), today),
                    Relative::ThisWeek => (monday, monday.saturating_add(7)),
                    Relative::LastWeek => (monday.saturating_sub(7), monday),
                    Relative::ThisMonth => month(y, m),
                    Relative::LastMonth => {
                        let (py, pm) = previous_month(y, m);
                        month(py, pm)
                    }
                    Relative::ThisYear => year(y),
                    Relative::LastYear => year(y.saturating_sub(1)),
                }
            }
        }
    }
}

impl DateBound {
    /// The instant this bound is, in the clock's zone.
    fn at(self, clock: &Clock) -> i64 {
        let (first, after) = self.point.days(clock);
        clock.midnight(if self.edge == Edge::Start { first } else { after })
    }
}

impl DateSpec {
    /// Place the period in the clock's zone, from its day.
    pub fn resolve(&mut self, clock: &Clock) {
        let from = self.from.map_or(i64::MIN, |b| b.at(clock));
        let to = self.to.map_or(i64::MAX, |b| b.at(clock));
        self.range = Some((from, to));
    }

    /// Whether a time falls in the period: `None` until resolved.
    pub fn holds(&self, t: i64) -> Option<bool> {
        self.range.map(|(from, to)| from <= t && t < to)
    }
}

impl Query {
    /// Place the query's dates (`received:2024`, `after:yesterday`) in the clock's zone, from
    /// its day: needed before they match anything.
    pub fn resolve(&mut self, clock: &Clock) {
        fn term(t: &mut Term, clock: &Clock) {
            match t {
                Term::Date(spec) => spec.resolve(clock),
                Term::AnyOf(any) => any.iter_mut().for_each(|t| term(t, clock)),
                _ => {}
            }
        }
        for c in &mut self.clauses {
            term(&mut c.term, clock);
        }
    }

    /// The period a server is asked for (issue #179): from (on) and to (before) in Unix seconds,
    /// either open, where every date the query names (not left out with `-`) holds; `None` when
    /// it names none. A day wider each side than the clock's zone gives: a server reads days in
    /// its own zone, and what it finds is checked here as any search is.
    pub fn server_period(&self, clock: &Clock) -> Option<(Option<i64>, Option<i64>)> {
        let mut found = None;
        for c in self.clauses.iter().filter(|c| !c.negated) {
            let Term::Date(spec) = &c.term else { continue };
            let mut spec = spec.clone();
            spec.resolve(clock);
            let Some((from, to)) = spec.range else { continue };
            let (f, t): &mut (Option<i64>, Option<i64>) = found.get_or_insert((None, None));
            if from != i64::MIN {
                *f = Some(f.map_or(from, |x| x.max(from)));
            }
            if to != i64::MAX {
                *t = Some(t.map_or(to, |x| x.min(to)));
            }
        }
        found.map(|(f, t)| (f.map(|f| f.saturating_sub(DAY)), t.map(|t| t.saturating_add(DAY))))
    }
}
