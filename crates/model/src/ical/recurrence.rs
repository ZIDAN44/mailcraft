//! Recurring events: RRULE read, and a series' occurrences worked out over a window.

use super::*;

/// When a rule stops: an instant (`UNTIL` in UTC), or a local time where the event is (a date,
/// to the end of that day, or a floating time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Until {
    Utc(i64),
    Local(i64),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Rule {
    pub(super) freq: String,
    pub(super) interval: i64,
    pub(super) count: Option<usize>,
    pub(super) until: Option<Until>,
    /// (ordinal, weekday 0 = Monday); ordinal 0 = every.
    pub(super) by_day: Vec<(i32, u32)>,
    pub(super) by_month_day: Vec<i32>,
    pub(super) by_month: Vec<u32>,
    /// Which of each period's days count (1-based; negative from the end).
    pub(super) by_set_pos: Vec<i32>,
}

impl Rule {
    pub(super) fn parse(s: &str) -> Rule {
        let mut r = Rule { interval: 1, ..Default::default() };
        for part in s.split(';') {
            let Some((k, v)) = part.split_once('=') else { continue };
            match k.trim().to_ascii_uppercase().as_str() {
                "FREQ" => r.freq = v.trim().to_ascii_uppercase(),
                "INTERVAL" => r.interval = v.trim().parse::<i64>().unwrap_or(1).clamp(1, 10_000),
                "COUNT" => r.count = v.trim().parse::<usize>().ok().map(|c| c.min(MAX_OCCURRENCES * 10)),
                "UNTIL" => {
                    r.until = date_time(v).map(|(t, all_day, utc)| match (utc, all_day) {
                        (true, _) => Until::Utc(t),
                        (false, true) => Until::Local(t + 86_399),
                        (false, false) => Until::Local(t),
                    })
                }
                "BYDAY" => {
                    r.by_day = v
                        .split(',')
                        .filter_map(|d| {
                            let d = d.trim();
                            let split = d.len().checked_sub(2)?;
                            let (n, w) = (d.get(..split)?, d.get(split..)?);
                            let nth = if n.is_empty() || n == "+" { 0 } else { n.trim_start_matches('+').parse().ok()? };
                            Some((nth, weekday_code(w)?))
                        })
                        .take(31)
                        .collect();
                }
                "BYMONTHDAY" => {
                    r.by_month_day = v.split(',').filter_map(|d| d.trim().parse().ok()).filter(|d: &i32| *d != 0 && d.abs() <= 31).take(31).collect()
                }
                "BYMONTH" => r.by_month = v.split(',').filter_map(|m| m.trim().parse().ok()).filter(|m| (1..=12).contains(m)).take(12).collect(),
                "BYSETPOS" => {
                    r.by_set_pos = v.split(',').filter_map(|p| p.trim().parse().ok()).filter(|p: &i32| *p != 0 && p.abs() <= 366).take(31).collect()
                }
                _ => {}
            }
        }
        r
    }

    /// Whether a day is one of `BYDAY`'s weekdays (any ordinal), when it limits.
    fn weekday_ok(&self, day: i64) -> bool {
        let wd = (day + 3).rem_euclid(7) as u32;
        self.by_day.is_empty() || self.by_day.iter().any(|(_, w)| *w == wd)
    }

    /// Whether a day is one of `BYMONTHDAY`'s, when it limits.
    fn month_day_ok(&self, day: i64) -> bool {
        let (y, m, _) = civil_from_days(day);
        self.by_month_day.is_empty() || self.by_month_day.iter().any(|md| month_day(y, m, *md) == Some(day))
    }

    /// The days of month `m` that `BYMONTHDAY` and `BYDAY` give (both: the days in both, as
    /// Friday the 13th); neither: day `fallback` of the month.
    fn days_in_month(&self, y: i64, m: u32, fallback: u32) -> Vec<i64> {
        let by_month_day: Vec<i64> = self.by_month_day.iter().filter_map(|md| month_day(y, m, *md)).collect();
        let mut by_day: Vec<i64> = Vec::new();
        for (nth, w) in &self.by_day {
            if *nth == 0 {
                by_day.extend(every_weekday(y, m, *w));
            } else {
                by_day.extend(nth_weekday(y, m, *nth, *w));
            }
        }
        match (self.by_month_day.is_empty(), self.by_day.is_empty()) {
            (false, false) => by_month_day.into_iter().filter(|d| by_day.contains(d)).collect(),
            (false, true) => by_month_day,
            (true, false) => by_day,
            (true, true) => month_day(y, m, i32::try_from(fallback).unwrap_or(1)).into_iter().collect(),
        }
    }

    /// The candidate days (since the epoch) of the period containing `day`, in order: what the
    /// rule's parts make of it, limited by the others, then picked by `BYSETPOS`.
    pub(super) fn days_in_period(&self, day: i64, start_day: i64) -> Vec<i64> {
        let (y, m, _) = civil_from_days(day);
        let (_, sm, sd) = civil_from_days(start_day);
        let wd = |x: i64| (x + 3).rem_euclid(7) as u32;
        let mut v: Vec<i64> = match self.freq.as_str() {
            "DAILY" => [day].into_iter().filter(|d| self.weekday_ok(*d) && self.month_day_ok(*d)).collect(),
            "WEEKLY" if self.by_day.is_empty() => vec![day],
            "WEEKLY" => {
                // The week (Monday first) containing `day`.
                let monday = day - i64::from(wd(day));
                self.by_day.iter().map(|(_, w)| monday + i64::from(*w)).filter(|d| self.month_day_ok(*d)).collect()
            }
            "MONTHLY" => self.days_in_month(y, m, sd),
            "YEARLY" => {
                let months: Vec<u32> = if self.by_month.is_empty() { vec![sm] } else { self.by_month.clone() };
                months.into_iter().flat_map(|mo| self.days_in_month(y, mo, sd)).collect()
            }
            _ => vec![],
        };
        if !self.by_month.is_empty() {
            v.retain(|d| self.by_month.contains(&civil_from_days(*d).1));
        }
        v.sort_unstable();
        v.dedup();
        if self.by_set_pos.is_empty() {
            return v;
        }
        // BYSETPOS counts within the period's candidate days: 1 is the first, -1 the last.
        let nth = |pos: i32| -> Option<i64> {
            let from_end = usize::try_from(pos.unsigned_abs()).ok()?;
            let index = if pos > 0 { from_end - 1 } else { v.len().checked_sub(from_end)? };
            v.get(index).copied()
        };
        let mut chosen = std::collections::BTreeSet::new();
        for &pos in &self.by_set_pos {
            if let Some(day) = nth(pos) {
                chosen.insert(day);
            }
        }
        chosen.into_iter().collect()
    }

    /// The first day of the period `k` periods after the one containing `start_day`.
    pub(super) fn period(&self, start_day: i64, k: i64) -> Option<i64> {
        let (y, m, _) = civil_from_days(start_day);
        let step = k.checked_mul(self.interval)?;
        match self.freq.as_str() {
            "DAILY" => start_day.checked_add(step),
            "WEEKLY" => start_day.checked_add(step.checked_mul(7)?),
            "MONTHLY" => {
                let total = i64::from(m) - 1 + step;
                let yy = y.checked_add(total.div_euclid(12))?;
                (yy <= 9999).then(|| days_from_civil(yy, u32::try_from(total.rem_euclid(12) + 1).unwrap_or(1), 1))
            }
            "YEARLY" => {
                let yy = y.checked_add(step)?;
                (yy <= 9999).then(|| days_from_civil(yy, 1, 1))
            }
            _ => None,
        }
    }
}

/// Every `weekday` (0 Monday) of a month, as days since the epoch.
fn every_weekday(y: i64, m: u32, weekday: u32) -> Vec<i64> {
    let Some(first) = nth_weekday(y, m, 1, weekday) else { return Vec::new() };
    (0..5).map(|n| first + 7 * n).filter(|d| civil_from_days(*d).1 == m).collect()
}

/// The day of a month (`md` 1-based, negative from the end), if the month has it.
pub(super) fn month_day(y: i64, m: u32, md: i32) -> Option<i64> {
    let first = days_from_civil(y, m, 1);
    let next = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
    let len = next - first;
    let d = if md > 0 { i64::from(md) } else { len + 1 + i64::from(md) };
    (1..=len).contains(&d).then_some(first + d - 1)
}

/// Every start of a (possibly recurring) event, from its first, up to `before`. A rule repeats at
/// the same wall-clock time in the event's zone (09:00 stays 09:00 there when summer time
/// starts), on that zone's days; each occurrence is then turned into UTC on its own date.
pub fn starts(e: &CalEvent, before: i64) -> Vec<i64> {
    let Some(rule) = e.rrule.as_deref().map(Rule::parse).filter(|r| !r.freq.is_empty()) else {
        return vec![e.start];
    };
    let zone = zone_of(e);
    let local_start = zone.as_ref().map_or(e.start, |z| z.local(e.start));
    let start_day = local_start.div_euclid(86_400);
    let time = local_start.rem_euclid(86_400);
    let mut out = Vec::new();
    let mut produced = 0usize;
    let mut steps = 0usize;
    let mut k = 0i64;
    'periods: while let Some(p) = rule.period(start_day, k) {
        k += 1;
        for day in rule.days_in_period(if k == 1 { start_day } else { p }, start_day) {
            steps += 1;
            if steps > MAX_STEPS {
                break 'periods;
            }
            let local = day.saturating_mul(86_400).saturating_add(time);
            if local < local_start {
                continue;
            }
            // The first is the event's own start, whatever its zone makes of it.
            let t = if local == local_start { e.start } else { zone.as_ref().map_or(local, |z| z.to_utc(local)) };
            let past_until = match rule.until {
                Some(Until::Utc(u)) => t > u,
                Some(Until::Local(u)) => local > u,
                None => false,
            };
            if past_until || t >= before {
                break 'periods;
            }
            produced += 1;
            if rule.count.is_some_and(|c| produced > c) {
                break 'periods;
            }
            out.push(t);
            if out.len() >= MAX_OCCURRENCES * 10 {
                break 'periods;
            }
        }
        if steps > MAX_STEPS {
            break;
        }
    }
    out
}

/// The occurrences of `events` overlapping `[from, to)`, by start. Instances with a
/// `RECURRENCE-ID` replace the occurrence of the same event they name; cancelled ones are left out.
pub fn occurrences(events: &[CalEvent], from: i64, to: i64) -> Vec<Occurrence> {
    let mut out = Vec::new();
    // The moved or changed occurrences of each series, gathered once (not per event: a big
    // calendar would make that quadratic).
    let mut moved: std::collections::BTreeMap<&str, Vec<i64>> = std::collections::BTreeMap::new();
    for e in events {
        if let Some(r) = e.recurrence_id {
            moved.entry(e.uid.as_str()).or_default().push(r);
        }
    }
    let none = Vec::new();
    for (i, e) in events.iter().enumerate() {
        if e.recurrence_id.is_some() || e.status == "CANCELLED" {
            continue;
        }
        let len = (e.end - e.start).max(0);
        let overrides = moved.get(e.uid.as_str()).unwrap_or(&none);
        let mut n = 0;
        for s in starts(e, to) {
            if s >= to {
                break;
            }
            if s + len.max(1) <= from || e.exdates.contains(&s) || overrides.contains(&s) {
                continue;
            }
            out.push(Occurrence { event: i, start: s, end: s + len });
            n += 1;
            if n >= MAX_OCCURRENCES {
                break;
            }
        }
    }
    for (i, e) in events.iter().enumerate() {
        if e.recurrence_id.is_some() && e.status != "CANCELLED" && e.start < to && e.end.max(e.start + 1) > from {
            out.push(Occurrence { event: i, start: e.start, end: e.end });
        }
    }
    out.sort_by_key(|o| (o.start, o.event));
    out
}
