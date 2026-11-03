//! A rule (RFC 5545 `RRULE`) as Outlook's RecurrencePattern (MS-OXOCAL 2.2.1.44.1), the form a
//! task's PidLidTaskRecurrence takes and `pst_pim::recur` reads: daily every so many days, weekly
//! on some days, monthly on a day of the month, its last day or the nth of a weekday, yearly on
//! the first occurrence's day or the nth of a weekday in its month; ending on a day, after so many
//! times, or never. A rule it can't say (hourly, `BYSETPOS`, several days of the month…) has no
//! pattern, and one it says differently from the rule is checked by reading it back ([`says`]).

use mailcraft_model::dates::{civil_from_days, days_from_civil};

/// Minutes from 1601-01-01 to 1970-01-01.
const EPOCH_MINUTES: i64 = 11_644_473_600 / 60;
/// A pattern's version (ReaderVersion, WriterVersion).
const VERSION: u16 = 0x3004;
/// EndDate of a pattern that doesn't end (31 December 4500, 23:59).
const NO_END: u32 = 0x5AE9_80DF;
/// OccurrenceCount of a pattern that doesn't end, as Outlook writes it.
const NO_END_COUNT: u32 = 10;
/// Weekdays as BYDAY names them, Sunday first (the pattern's bit 0).
const DAYS: [&str; 7] = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];
/// The most occurrences counted to find where a pattern ends; COUNT and INTERVAL Outlook takes.
const MAX_COUNT: u32 = 999;
const MAX_INTERVAL: u32 = 999;

/// A rule's parts the pattern can say.
#[derive(Default)]
struct Rule {
    freq: String,
    interval: u32,
    /// (ordinal, weekday: 0 Sunday); ordinal 0 is every one.
    days: Vec<(i32, u32)>,
    month_day: Option<i32>,
    month: Option<u32>,
    count: Option<u32>,
    /// The last day (days since 1970).
    until: Option<i64>,
}

fn parse(rule: &str) -> Option<Rule> {
    let mut r = Rule { interval: 1, ..Default::default() };
    for part in rule.split(';').filter(|p| !p.trim().is_empty()) {
        let (k, v) = part.split_once('=')?;
        let v = v.trim();
        match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" => r.freq = v.to_ascii_uppercase(),
            "INTERVAL" => r.interval = v.parse().ok().filter(|n| (1..=MAX_INTERVAL).contains(n))?,
            "COUNT" => r.count = Some(v.parse().ok().filter(|n| (1..=MAX_COUNT).contains(n))?),
            "UNTIL" => {
                let d = |a: usize, b: usize| v.get(a..b).and_then(|s| s.parse::<u32>().ok());
                let (y, m, day) = (d(0, 4)?, d(4, 6)?, d(6, 8)?);
                r.until = ((1..=12).contains(&m) && (1..=31).contains(&day)).then(|| days_from_civil(i64::from(y), m, day));
                r.until?;
            }
            "BYDAY" => {
                for d in v.split(',') {
                    let d = d.trim().to_ascii_uppercase();
                    let split = d.len().checked_sub(2)?;
                    let (n, w) = (d.get(..split)?, d.get(split..)?);
                    let nth = if n.is_empty() { 0 } else { n.trim_start_matches('+').parse::<i32>().ok()? };
                    let day = DAYS.iter().position(|x| *x == w).and_then(|i| u32::try_from(i).ok())?;
                    r.days.push((nth, day));
                }
            }
            "BYMONTHDAY" => r.month_day = Some(v.parse().ok()?),
            "BYMONTH" => r.month = Some(v.parse().ok().filter(|m| (1..=12).contains(m))?),
            "WKST" if v.eq_ignore_ascii_case("SU") || r.interval == 1 => {}
            _ => return None,
        }
    }
    (r.count.is_none() || r.until.is_none()).then_some(r)
}

/// The pattern of `rule` for a task whose first day is `from` (Unix seconds, its midnight in
/// UTC); `None` when the pattern can't say it.
pub(super) fn write(rule: &str, from: i64) -> Option<Vec<u8>> {
    let r = parse(rule)?;
    let day = from.div_euclid(86_400);
    let (y, m, d) = civil_from_days(day);
    let start = minutes(day)?;
    let weekday = u32::try_from((day + 4).rem_euclid(7)).ok()?;
    let mask = |days: &[(i32, u32)]| days.iter().fold(0u32, |m, (_, d)| m | (1 << d));
    // The ordinal all the days share (-1, the last, is 5), when they share one.
    let nth = |days: &[(i32, u32)]| match days.first()?.0 {
        n @ 1..=4 if days.iter().all(|(o, _)| *o == n) => u32::try_from(n).ok(),
        -1 if days.iter().all(|(o, _)| *o == -1) => Some(5),
        _ => None,
    };
    // The months since 1601 the pattern's periods start from (FirstDateTime for months and years).
    let months_first = |period: u32| -> Option<u32> {
        let months = (y - 1601).checked_mul(12)?.checked_add(i64::from(m) - 1)?;
        let k = months.rem_euclid(i64::from(period));
        minutes(days_from_civil(1601 + k / 12, u32::try_from(k % 12 + 1).ok()?, 1))
    };
    let (frequency, kind, period, specific, first): (u16, u16, u32, Vec<u32>, u32) = match r.freq.as_str() {
        "DAILY" if r.days.is_empty() && r.month_day.is_none() && r.month.is_none() => {
            let period = r.interval.checked_mul(1440)?;
            (0x200A, 0x0000, period, Vec::new(), start % period)
        }
        "WEEKLY" if r.days.iter().all(|(n, _)| *n == 0) && r.month_day.is_none() && r.month.is_none() => {
            let mask = if r.days.is_empty() { 1 << weekday } else { mask(&r.days) };
            let week = minutes(day - i64::from(weekday))?;
            (0x200B, 0x0001, r.interval, vec![mask], week % r.interval.checked_mul(10_080)?)
        }
        "MONTHLY" if r.month.is_none() => {
            let (kind, specific) = match (r.month_day, r.days.is_empty()) {
                (Some(-1), true) => (0x0004, vec![31]),
                (Some(n @ 1..=31), true) => (0x0002, vec![u32::try_from(n).ok()?]),
                (None, true) => (0x0002, vec![d]),
                (None, false) => (0x0003, vec![mask(&r.days), nth(&r.days)?]),
                _ => return None,
            };
            (0x200C, kind, r.interval, specific, months_first(r.interval)?)
        }
        "YEARLY" if r.month.is_none_or(|x| x == m) => {
            let period = r.interval.checked_mul(12)?;
            let (kind, specific) = match (r.month_day, r.days.is_empty()) {
                (None, true) => (0x0002, vec![d]),
                (Some(n), true) if i64::from(n) == i64::from(d) => (0x0002, vec![d]),
                (None, false) if r.month.is_some() => (0x0003, vec![mask(&r.days), nth(&r.days)?]),
                _ => return None,
            };
            (0x200D, kind, period, specific, months_first(period)?)
        }
        _ => return None,
    };
    // Where it ends: the last day said, or that of its last time.
    let times = || {
        let e = mailcraft_model::ical::CalEvent { start: from, end: from, all_day: true, rrule: Some(rule.to_string()), ..Default::default() };
        mailcraft_model::ical::starts(&e, i64::MAX)
    };
    let (end_type, count, end) = match (r.until, r.count) {
        (Some(until), _) => (0x2021, u32::try_from(times().len()).ok()?, minutes(until)?),
        (None, Some(count)) => (0x2022, count, minutes(times().last()?.div_euclid(86_400))?),
        (None, None) => (0x2023, NO_END_COUNT, NO_END),
    };
    let mut out = Vec::new();
    for x in [VERSION, VERSION, frequency, kind, 0] {
        out.extend(x.to_le_bytes());
    }
    let mut words = vec![first, period, 0];
    words.extend(specific);
    // FirstDOW Sunday; no deleted or changed occurrences.
    words.extend([end_type, count, 0, 0, 0, start, end]);
    for w in words {
        out.extend(w.to_le_bytes());
    }
    Some(out)
}

/// Minutes from 1601 to the start of `day` (days since 1970), as a pattern keeps a day.
fn minutes(day: i64) -> Option<u32> {
    u32::try_from(day.checked_mul(1440)?.checked_add(EPOCH_MINUTES)?).ok()
}

/// Whether the pattern `p`, read back as the import reads it from the task's first day `from`,
/// says what `rule` says (the same parts, `INTERVAL=1` said or not, the days in any order, an
/// end on the same day).
pub(super) fn says(p: &[u8], from: i64, rule: &str) -> bool {
    let parts = |r: &str| {
        let mut out: Vec<(String, String)> = r
            .split(';')
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| {
                let (k, v) = (k.trim().to_ascii_uppercase(), v.trim().to_ascii_uppercase());
                let v = match k.as_str() {
                    "BYDAY" => {
                        let mut days: Vec<&str> = v.split(',').map(str::trim).collect();
                        days.sort_unstable();
                        days.join(",")
                    }
                    "UNTIL" => v.get(..8).unwrap_or(&v).to_string(),
                    _ => v,
                };
                (k, v)
            })
            .filter(|(k, v)| !(k == "INTERVAL" && v == "1") && k != "WKST")
            .collect();
        out.sort();
        out
    };
    crate::pst_pim::recurrence(p, from).is_some_and(|(back, _)| parts(&back) == parts(rule))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each rule the pattern can say is read back the same, and those it can't have no pattern.
    #[test]
    fn rules_as_patterns_and_back() {
        let from = days_from_civil(2026, 11, 3) * 86_400; // a Tuesday
        for rule in [
            "FREQ=DAILY;INTERVAL=2",
            "FREQ=DAILY;INTERVAL=1;COUNT=5",
            "FREQ=WEEKLY;INTERVAL=1;BYDAY=TU,TH",
            "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR",
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU;UNTIL=20270131T235959Z",
            "FREQ=MONTHLY;INTERVAL=1;BYMONTHDAY=3",
            "FREQ=MONTHLY;INTERVAL=3;BYMONTHDAY=-1",
            "FREQ=MONTHLY;INTERVAL=1;BYDAY=1TU",
            "FREQ=MONTHLY;INTERVAL=1;BYDAY=-1FR;COUNT=12",
            "FREQ=YEARLY;INTERVAL=1",
            "FREQ=YEARLY;INTERVAL=1;BYMONTH=11;BYDAY=1TU",
        ] {
            let p = write(rule, from).unwrap_or_else(|| panic!("{rule}"));
            assert!(says(&p, from, rule), "{rule}: {:?}", crate::pst_pim::recurrence(&p, from));
        }
        for rule in [
            "FREQ=HOURLY",
            "FREQ=MONTHLY;BYMONTHDAY=1,15",
            "FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2",
            "FREQ=YEARLY;BYMONTH=3",
            "FREQ=DAILY;COUNT=0",
            "junk",
            "FREQ=DAILY;COUNT=2;UNTIL=20270101",
        ] {
            assert!(write(rule, from).is_none(), "{rule}");
        }
        // Said another way than the rule: read back, it isn't the rule (the task keeps the rule).
        assert!(!says(&write("FREQ=WEEKLY", from).unwrap(), from, "FREQ=WEEKLY"));
        let p = write("FREQ=DAILY;UNTIL=20261110", from).unwrap();
        assert!(says(&p, from, "FREQ=DAILY;UNTIL=20261110"));
        assert!(!says(&p, from, "FREQ=DAILY;UNTIL=20261111"));
        // Ten times, the last on the 12th: its end day.
        let p = write("FREQ=DAILY;COUNT=10", from).unwrap();
        let end = u32::from_le_bytes(p[p.len() - 4..].try_into().unwrap());
        assert_eq!(Some(end), minutes(days_from_civil(2026, 11, 12)));
        // Hostile days and numbers make no pattern, and no panic.
        for rule in ["FREQ=DAILY;INTERVAL=99999999999", "FREQ=WEEKLY;BYDAY=9XX", "FREQ=WEEKLY;BYDAY=X", "FREQ=DAILY;UNTIL=2026"] {
            assert!(write(rule, from).is_none(), "{rule}");
        }
        assert!(write("FREQ=DAILY", i64::MAX).is_none());
    }
}
