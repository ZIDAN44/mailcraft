//! When someone works (issue #185): their working days and hours, in their own time zone, as
//! Outlook's Work time keeps them and Exchange's availability service gives them for a colleague;
//! and the spans of a window they work in, to shade the hours they don't.

use serde::{Deserialize, Serialize};

use crate::dates::{civil_from_days, days_from_civil, month_len, weekday_index};

const DAY: i64 = 86_400;
/// The most days a window is looked through.
const DAYS_MAX: i64 = 400;
/// The most working periods kept of someone's week.
pub const PERIODS_MAX: usize = 14;

/// The days of the week, Monday first, as `WorkPeriod::days` holds them (bit 0 Monday … bit 6
/// Sunday).
pub const MONDAY_TO_FRIDAY: u8 = 0b001_1111;

/// Some days of the week worked between two times of the day (minutes since midnight, in the
/// worker's zone).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkPeriod {
    /// Bit 0 Monday … bit 6 Sunday.
    pub days: u8,
    pub start: u16,
    pub end: u16,
}

impl Default for WorkPeriod {
    /// Monday to Friday, 08:00 to 17:00, as Outlook starts.
    fn default() -> WorkPeriod {
        WorkPeriod { days: MONDAY_TO_FRIDAY, start: 8 * 60, end: 17 * 60 }
    }
}

impl WorkPeriod {
    /// Whether it's worked on a weekday (0 Monday … 6 Sunday).
    pub fn works_on(&self, weekday: u32) -> bool {
        weekday < 7 && self.days & (1 << weekday) != 0
    }

    /// Whether it's worked on the local `day` (days since 1970-01-01).
    pub fn works(&self, day: i64) -> bool {
        self.works_on(weekday_index(day))
    }
}

/// Where `periods` are worked between `from` and `to` (Unix seconds), each day's span apart, in
/// order: a local day's start and end placed by `offset_at` (seconds east of UTC at an instant).
/// A period that ends before it starts is none; one window is looked through for a year at most.
pub fn work_spans(periods: &[WorkPeriod], from: i64, to: i64, offset_at: impl Fn(i64) -> i64) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    if to <= from {
        return out;
    }
    let first = (from + offset_at(from)).div_euclid(DAY) - 1;
    let last = (to + offset_at(to)).div_euclid(DAY) + 1;
    for day in first..=last.min(first.saturating_add(DAYS_MAX)) {
        for p in periods.iter().filter(|p| p.works(day) && p.end > p.start) {
            let local = |m: u16| day.saturating_mul(DAY).saturating_add(i64::from(m) * 60);
            let (a, b) = (local(p.start), local(p.end));
            let (a, b) = (a - offset_at(a - offset_at(a)), b - offset_at(b - offset_at(b)));
            let (a, b) = (a.max(from), b.min(to));
            if a < b {
                out.push((a, b));
            }
        }
    }
    out.sort_unstable();
    out
}

/// Where two lists of spans (each in order, apart) both are: when two people both work.
pub fn overlap(a: &[(i64, i64)], b: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while let (Some(x), Some(y)) = (a.get(i), b.get(j)) {
        let (from, to) = (x.0.max(y.0), x.1.min(y.1));
        if from < to {
            out.push((from, to));
        }
        if x.1 < y.1 { i += 1 } else { j += 1 }
    }
    out
}

/// Spans in order, those that overlap or touch made one: when anyone is busy.
pub fn merged(mut spans: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    spans.retain(|(a, b)| a < b);
    spans.sort_unstable();
    let mut out: Vec<(i64, i64)> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// What of `from` to `to` lies outside the spans (in order, apart): the hours not worked.
pub fn outside(spans: &[(i64, i64)], from: i64, to: i64) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut at = from;
    for (a, b) in spans.iter().copied() {
        if a > at {
            out.push((at, a.min(to)));
        }
        at = at.max(b);
        if at >= to {
            break;
        }
    }
    if at < to {
        out.push((at, to));
    }
    out.retain(|(a, b)| a < b);
    out
}

/// A change of clocks in a zone, as Exchange writes one (`SerializableTimeZone`'s `StandardTime`
/// or `DaylightTime`): the bias it adds, and when it comes each year, in local time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockChange {
    /// Minutes added to the zone's bias while it holds.
    pub bias: i32,
    /// 1 to 12.
    pub month: u32,
    /// Which of the month's `weekday`s: 1 to 4, or 5 for its last.
    pub order: u32,
    /// 0 Monday … 6 Sunday.
    pub weekday: u32,
    /// Minutes past local midnight.
    pub minute: u32,
}

impl ClockChange {
    /// The local time (seconds since 1970, as if UTC) it comes in `year`.
    fn at(&self, year: i64) -> Option<i64> {
        if !(1..=12).contains(&self.month) || self.weekday > 6 {
            return None;
        }
        let first = days_from_civil(year, self.month, 1);
        let to_weekday = (i64::from(self.weekday) - i64::from(weekday_index(first))).rem_euclid(7);
        let mut day = first + to_weekday + 7 * i64::from(self.order.clamp(1, 5) - 1);
        let past = first + month_len(year, self.month);
        while day >= past {
            day -= 7;
        }
        Some(day * DAY + i64::from(self.minute.min(24 * 60)) * 60)
    }
}

/// A zone as Exchange's availability service writes one: minutes west of UTC (UTC = local +
/// `bias`), and its two changes of clocks when it has them; or (Microsoft Graph) its name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkZone {
    pub bias: i32,
    #[serde(default)]
    pub standard: Option<ClockChange>,
    #[serde(default)]
    pub daylight: Option<ClockChange>,
    /// A zone's name (Windows' or IANA's), which says it all when it's known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl WorkZone {
    /// Seconds east of UTC at the instant `t`.
    pub fn offset_at(&self, t: i64) -> i64 {
        if let Some(zone) = self.name.as_deref().and_then(crate::dates::zone::Zone::named) {
            return zone.offset_at(t);
        }
        let standard = -(i64::from(self.bias) + i64::from(self.standard.map_or(0, |c| c.bias))) * 60;
        let (Some(std), Some(dst)) = (self.standard, self.daylight) else { return standard };
        if std.month == 0 || dst.month == 0 {
            return standard;
        }
        let local = t + standard;
        let (year, _, _) = civil_from_days(local.div_euclid(DAY));
        let (Some(on), Some(off)) = (dst.at(year), std.at(year)) else { return standard };
        // Northern (on before off in the year) or southern (on after off, across the new year).
        let summer = if on < off { local >= on && local < off } else { local >= on || local < off };
        if summer { -(i64::from(self.bias) + i64::from(dst.bias)) * 60 } else { standard }
    }
}

/// Someone's working week, in their zone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkHours {
    pub periods: Vec<WorkPeriod>,
    pub zone: WorkZone,
}

impl WorkHours {
    /// Where they work between `from` and `to` (Unix seconds).
    pub fn spans(&self, from: i64, to: i64) -> Vec<(i64, i64)> {
        work_spans(&self.periods, from, to, |t| self.zone.offset_at(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-16 was a Monday; 00:00 UTC.
    const MONDAY: i64 = 1_794_787_200;

    #[test]
    fn work_spans_follow_the_days_and_hours() {
        let p = WorkPeriod::default();
        let week = work_spans(&[p], MONDAY, MONDAY + 7 * DAY, |_| 0);
        assert_eq!(week.len(), 5, "Monday to Friday: {week:?}");
        assert_eq!(week[0], (MONDAY + 8 * 3600, MONDAY + 17 * 3600));
        // An hour east of UTC: an hour earlier in UTC; cut to the window.
        let east = work_spans(&[p], MONDAY + 10 * 3600, MONDAY + DAY, |_| 3600);
        assert_eq!(east, [(MONDAY + 10 * 3600, MONDAY + 16 * 3600)]);
        let none = work_spans(&[WorkPeriod { start: 600, end: 500, ..p }], MONDAY, MONDAY + DAY, |_| 0);
        assert!(none.is_empty(), "an end before its start: none");
        assert!(WorkPeriod { days: 1 << 6, ..p }.works(MONDAY / DAY + 6), "Sunday");
    }

    #[test]
    fn spans_meet_and_leave_gaps() {
        let a = [(0, 10), (20, 30)];
        let b = [(5, 25)];
        assert_eq!(overlap(&a, &b), [(5, 10), (20, 25)]);
        assert_eq!(overlap(&a, &[]), []);
        assert_eq!(outside(&a, 0, 40), [(10, 20), (30, 40)]);
        assert_eq!(outside(&a, 5, 25), [(10, 20)]);
        assert_eq!(outside(&[], 5, 25), [(5, 25)]);
        assert_eq!(merged(vec![(20, 30), (0, 10), (5, 12), (12, 14), (40, 40)]), [(0, 14), (20, 30)]);
    }

    #[test]
    fn a_zone_s_clocks_change() {
        // Central Europe, as Exchange writes it: UTC+1, +2 from March's last Sunday 02:00 to
        // October's last Sunday 03:00.
        let cet = WorkZone {
            bias: -60,
            standard: Some(ClockChange { bias: 0, month: 10, order: 5, weekday: 6, minute: 180 }),
            daylight: Some(ClockChange { bias: -60, month: 3, order: 5, weekday: 6, minute: 120 }),
            name: None,
        };
        let named = WorkZone { name: Some("W. Europe Standard Time".into()), ..Default::default() };
        assert_eq!((named.offset_at(MONDAY), named.offset_at(MONDAY - 140 * DAY)), (3600, 7200), "by its Windows name");
        assert_eq!(cet.offset_at(MONDAY), 3600, "November: winter");
        assert_eq!(cet.offset_at(MONDAY - 140 * DAY), 7200, "July: summer");
        let hours = WorkHours { periods: vec![WorkPeriod::default()], zone: cet.clone() };
        assert_eq!(hours.spans(MONDAY, MONDAY + DAY), [(MONDAY + 7 * 3600, MONDAY + 16 * 3600)], "08:00 to 17:00 there is 07:00 to 16:00 UTC");
        // The south: on in October, off in April.
        let south = WorkZone {
            bias: -600,
            standard: Some(ClockChange { bias: 0, month: 4, order: 1, weekday: 6, minute: 180 }),
            daylight: Some(ClockChange { bias: -60, month: 10, order: 1, weekday: 6, minute: 120 }),
            name: None,
        };
        assert_eq!(south.offset_at(MONDAY), 11 * 3600, "November there: summer");
        assert_eq!(WorkZone { bias: 300, ..Default::default() }.offset_at(MONDAY), -5 * 3600, "no changes");
    }
}
