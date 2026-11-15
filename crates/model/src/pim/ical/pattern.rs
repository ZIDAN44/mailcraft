//! A recurrence as the event form edits it (issue #103), Outlook's choices: every so many days,
//! weeks (on some weekdays), months (on a day, or the nth weekday) or years, ending never, after
//! so many occurrences, or by a date. Read from a rule when the rule says no more than that; a
//! rule that says more is kept as it is.

use super::*;
use crate::dates::{civil_from_days, days_from_civil, weekday_index};

/// How often it repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternFreq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// When it ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PatternEnd {
    #[default]
    Never,
    /// After so many occurrences (`COUNT`).
    Count(u32),
    /// On this day, its last occurrence's (`UNTIL`; days since 1970).
    Until(i64),
}

/// A recurrence the form can show and change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub freq: PatternFreq,
    /// Every this many days, weeks, months or years (1 and up).
    pub interval: u32,
    /// A weekly one's weekdays (0 Monday … 6 Sunday; none: the start's).
    pub days: Vec<u32>,
    /// A monthly or yearly one on a weekday of the month (the second Tuesday): which one (1 to
    /// 4, or -1 the last), the weekday being the start's. `None`: on the start's day of the month.
    pub nth: Option<i32>,
    pub end: PatternEnd,
}

/// The most a form's numbers go to: every 999 days, 999 occurrences (a rule's own may be more).
pub const PATTERN_MAX: u32 = 999;

const DAY_CODES: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

impl Pattern {
    /// A pattern of `freq`: every one, on the start's day, no end.
    pub fn new(freq: PatternFreq) -> Pattern {
        Pattern { freq, interval: 1, days: Vec::new(), nth: None, end: PatternEnd::Never }
    }

    /// The pattern a rule says, when it says only what the form offers (`FREQ`, `INTERVAL`,
    /// `BYDAY`, `BYMONTHDAY`, `BYMONTH`, `COUNT`, `UNTIL`, `WKST`), and says it of an event starting
    /// on `start_day` (its day of the month, its month). Else `None`: the rule is kept as it is.
    pub fn read(rule: &str, start_day: i64) -> Option<Pattern> {
        let (_, month, mday) = civil_from_days(start_day);
        let mut p = Pattern::new(PatternFreq::Daily);
        let mut freq = None;
        let (mut by_day, mut by_month_day, mut by_month) = (None, None, None);
        for part in rule.split(';').map(str::trim).filter(|x| !x.is_empty()) {
            let (k, v) = part.split_once('=')?;
            let v = v.trim().to_ascii_uppercase();
            match k.trim().to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match v.as_str() {
                        "DAILY" => PatternFreq::Daily,
                        "WEEKLY" => PatternFreq::Weekly,
                        "MONTHLY" => PatternFreq::Monthly,
                        "YEARLY" => PatternFreq::Yearly,
                        _ => return None,
                    })
                }
                "INTERVAL" => p.interval = v.parse().ok().filter(|n| (1..=PATTERN_MAX).contains(n))?,
                "COUNT" => p.end = PatternEnd::Count(v.parse().ok().filter(|n| (1..=PATTERN_MAX).contains(n))?),
                "UNTIL" => p.end = PatternEnd::Until(until_day(&v)?),
                "BYDAY" => by_day = Some(v),
                "BYMONTHDAY" => by_month_day = Some(v),
                "BYMONTH" => by_month = Some(v),
                "WKST" => {}
                _ => return None,
            }
        }
        p.freq = freq?;
        let start_wd = weekday_index(start_day);
        match p.freq {
            PatternFreq::Daily => (by_day.is_none() && by_month_day.is_none() && by_month.is_none()).then_some(())?,
            PatternFreq::Weekly => {
                (by_month_day.is_none() && by_month.is_none()).then_some(())?;
                if let Some(days) = by_day {
                    for d in days.split(',') {
                        let i = u32::try_from(DAY_CODES.iter().position(|c| *c == d.trim())?).ok()?;
                        if !p.days.contains(&i) {
                            p.days.push(i);
                        }
                    }
                    p.days.sort_unstable();
                }
            }
            PatternFreq::Monthly | PatternFreq::Yearly => {
                // A yearly one is in the start's month; either is on its day, or its weekday.
                if p.freq == PatternFreq::Yearly && by_month.as_deref().is_some_and(|m| m.parse::<u32>().ok() != Some(month)) {
                    return None;
                }
                if p.freq == PatternFreq::Monthly && by_month.is_some() {
                    return None;
                }
                match (by_day, by_month_day) {
                    (None, None) => {}
                    (None, Some(d)) => (d.parse::<u32>().ok() == Some(mday)).then_some(())?,
                    (Some(d), None) => {
                        let at = d.len().checked_sub(2)?;
                        let (n, code) = (d.get(..at)?, d.get(at..)?);
                        (DAY_CODES.get(usize::try_from(start_wd).ok()?) == Some(&code)).then_some(())?;
                        let n: i32 = n.trim_start_matches('+').parse().ok()?;
                        (nth_of(start_day) == n || (n == -1 && last_of_month(start_day))).then_some(())?;
                        p.nth = Some(n);
                    }
                    (Some(_), Some(_)) => return None,
                }
            }
        }
        Some(p)
    }

    /// The rule it makes for an event starting on `start_day`, saying no more than the start
    /// doesn't (RFC 5545 takes a weekly one's weekday, a monthly one's day, from the start): a
    /// plain choice is the plain rule it always was. An end date is a date here
    /// (`UNTIL=20261231`): [`until_where_it_is`] makes it the end of that day where the event is.
    pub fn rule(&self, start_day: i64) -> String {
        let (_, month, _) = civil_from_days(start_day);
        let wd = DAY_CODES.get(usize::try_from(weekday_index(start_day)).unwrap_or(0)).copied().unwrap_or("MO");
        let mut out = match self.freq {
            PatternFreq::Daily => "FREQ=DAILY".to_string(),
            PatternFreq::Weekly => "FREQ=WEEKLY".to_string(),
            PatternFreq::Monthly => "FREQ=MONTHLY".to_string(),
            PatternFreq::Yearly => "FREQ=YEARLY".to_string(),
        };
        if self.interval > 1 {
            out.push_str(&format!(";INTERVAL={}", self.interval.min(PATTERN_MAX)));
        }
        match self.freq {
            PatternFreq::Daily => {}
            PatternFreq::Weekly => {
                let days: Vec<&str> = self.days.iter().filter_map(|d| DAY_CODES.get(usize::try_from(*d).ok()?).copied()).collect();
                if !days.is_empty() && days != [wd] {
                    out.push_str(&format!(";BYDAY={}", days.join(",")));
                }
            }
            PatternFreq::Monthly | PatternFreq::Yearly => {
                if let Some(n) = self.nth {
                    if self.freq == PatternFreq::Yearly {
                        out.push_str(&format!(";BYMONTH={month}"));
                    }
                    out.push_str(&format!(";BYDAY={n}{wd}"));
                }
            }
        }
        match self.end {
            PatternEnd::Never => {}
            PatternEnd::Count(n) => out.push_str(&format!(";COUNT={}", n.clamp(1, PATTERN_MAX))),
            PatternEnd::Until(day) => {
                let (y, m, d) = civil_from_days(day);
                out.push_str(&format!(";UNTIL={y:04}{m:02}{d:02}"));
            }
        }
        out
    }
}

/// Which of its weekday in its month a day is (the 9th: the second).
pub fn nth_of(day: i64) -> i32 {
    let (_, _, d) = civil_from_days(day);
    i32::try_from((d - 1) / 7 + 1).unwrap_or(1)
}

/// Whether a day is the last of its weekday in its month.
pub fn last_of_month(day: i64) -> bool {
    civil_from_days(day).1 != civil_from_days(day.saturating_add(7)).1
}

/// The day an `UNTIL` value names (`20261231`, `20261231T225959Z`), as written.
fn until_day(v: &str) -> Option<i64> {
    let y: i64 = v.get(..4)?.parse().ok()?;
    let m: u32 = v.get(4..6)?.parse().ok()?;
    let d: u32 = v.get(6..8)?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
}

/// An event's rule with a date as its `UNTIL` (`UNTIL=20261231`, as [`Pattern::rule`] writes
/// it) made what RFC 5545 asks of a timed event (§3.3.10): the end of that day where the event
/// is, in UTC. An all-day event's stays a date; any other rule is left as it is.
pub fn until_where_it_is(e: &mut CalEvent) {
    if e.all_day {
        return;
    }
    let Some(rule) = e.rrule.clone() else { return };
    let mut changed = false;
    let parts: Vec<String> = rule
        .split(';')
        .map(|part| match part.split_once('=') {
            Some((k, v)) if k.trim().eq_ignore_ascii_case("UNTIL") && v.trim().len() == 8 => match until_day(v.trim()) {
                Some(day) => {
                    let local = day.saturating_mul(86_400).saturating_add(86_399);
                    let at = zone_of(e).map_or(local, |z| z.to_utc(local));
                    changed = true;
                    format!("UNTIL={}", utc_stamp(at))
                }
                None => part.to_string(),
            },
            _ => part.to_string(),
        })
        .collect();
    if changed {
        e.rrule = Some(parts.join(";"));
    }
}

/// `20261231T225959Z`.
fn utc_stamp(t: i64) -> String {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z", s / 3600, s / 60 % 60, s % 60)
}
