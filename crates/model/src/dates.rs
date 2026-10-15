//! Calendar arithmetic for the message list: date groups (Today, Yesterday, Last week…) and the
//! short and long times shown for a message. Pure integer maths on Unix seconds plus the local UTC
//! offset, so it works the same everywhere, wasm included.

use serde::{Deserialize, Serialize};

/// "Now" and the local time zone, as the app reports them each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clock {
    /// Unix seconds, UTC.
    pub now: i64,
    /// Local offset from UTC in seconds (east positive).
    pub offset: i32,
}

impl Clock {
    pub fn new(now: i64, offset: i32) -> Clock {
        Clock { now, offset: offset.clamp(-14 * 3600, 14 * 3600) }
    }

    fn local_day(&self, t: i64) -> i64 {
        t.saturating_add(i64::from(self.offset)).div_euclid(86_400)
    }
}

/// A date split into its parts (proleptic Gregorian).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// 0 = Monday … 6 = Sunday.
    pub weekday: u32,
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's `civil_from_days`.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month, day) → days since 1970-01-01.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m.clamp(1, 12));
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d.clamp(1, 31)) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Split a time into local calendar parts.
pub fn civil(t: i64, clock: &Clock) -> Civil {
    let local = t.saturating_add(i64::from(clock.offset));
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday (weekday 3 when Monday is 0).
    let weekday = (days + 3).rem_euclid(7) as u32;
    Civil { year, month, day, hour: (secs / 3600) as u32, minute: ((secs % 3600) / 60) as u32, weekday }
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAYS_LONG: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn weekday_name(c: &Civil) -> &'static str {
    WEEKDAYS.get(c.weekday as usize).copied().unwrap_or("")
}

fn month_name(c: &Civil) -> &'static str {
    MONTHS.get(c.month.saturating_sub(1) as usize).copied().unwrap_or("")
}

/// The headings the message list groups by, newest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DateGroup {
    /// Dated in the future (a sender's clock is off): shown on top.
    Future,
    Today,
    Yesterday,
    /// Earlier this week (Monday-based); the weekday 0–6.
    ThisWeek(u32),
    LastWeek,
    TwoWeeksAgo,
    ThreeWeeksAgo,
    LastMonth,
    Older,
}

impl DateGroup {
    pub fn of(t: i64, clock: &Clock) -> DateGroup {
        let today = clock.local_day(clock.now);
        let day = clock.local_day(t);
        let diff = today - day;
        if diff < 0 {
            return DateGroup::Future;
        }
        if diff == 0 {
            return DateGroup::Today;
        }
        if diff == 1 {
            return DateGroup::Yesterday;
        }
        let today_wd = (today + 3).rem_euclid(7);
        let week_start = today - today_wd;
        if day >= week_start {
            return DateGroup::ThisWeek((day + 3).rem_euclid(7) as u32);
        }
        if day >= week_start - 7 {
            return DateGroup::LastWeek;
        }
        if day >= week_start - 14 {
            return DateGroup::TwoWeeksAgo;
        }
        if day >= week_start - 21 {
            return DateGroup::ThreeWeeksAgo;
        }
        let (ty, tm, _) = civil_from_days(today);
        let (y, m, _) = civil_from_days(day);
        let months_back = (ty * 12 + i64::from(tm)) - (y * 12 + i64::from(m));
        if months_back <= 1 { DateGroup::LastMonth } else { DateGroup::Older }
    }

    pub fn label(self) -> &'static str {
        match self {
            DateGroup::Future => "Upcoming",
            DateGroup::Today => "Today",
            DateGroup::Yesterday => "Yesterday",
            DateGroup::ThisWeek(wd) => WEEKDAYS_LONG.get(wd as usize).copied().unwrap_or("This week"),
            DateGroup::LastWeek => "Last week",
            DateGroup::TwoWeeksAgo => "Two weeks ago",
            DateGroup::ThreeWeeksAgo => "Three weeks ago",
            DateGroup::LastMonth => "Last month",
            DateGroup::Older => "Older",
        }
    }
}

/// The compact time in the message list: `14:05` today, `Mon 14:05` this week, `Oct 3` this
/// year, `2025-10-03` before that.
pub fn list_time(t: i64, clock: &Clock) -> String {
    let c = civil(t, clock);
    let now = civil(clock.now, clock);
    let days = clock.local_day(clock.now) - clock.local_day(t);
    if days == 0 {
        format!("{:02}:{:02}", c.hour, c.minute)
    } else if (1..7).contains(&days) {
        format!("{} {:02}:{:02}", weekday_name(&c), c.hour, c.minute)
    } else if c.year == now.year {
        format!("{} {}", month_name(&c), c.day)
    } else {
        format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)
    }
}

/// The full date in the reading pane: `Wed 2026-10-14 14:05`.
pub fn full_time(t: i64, clock: &Clock) -> String {
    let c = civil(t, clock);
    format!("{} {:04}-{:02}-{:02} {:02}:{:02}", weekday_name(&c), c.year, c.month, c.day, c.hour, c.minute)
}

/// When a flagged message is due, as Outlook's Follow Up menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowUp {
    Today,
    Tomorrow,
    /// The last working day of this week (Friday), or today from Friday on.
    ThisWeek,
    /// Friday next week.
    NextWeek,
}

impl FollowUp {
    pub const ALL: [FollowUp; 4] = [FollowUp::Today, FollowUp::Tomorrow, FollowUp::ThisWeek, FollowUp::NextWeek];

    pub fn code(self) -> &'static str {
        match self {
            FollowUp::Today => "today",
            FollowUp::Tomorrow => "tomorrow",
            FollowUp::ThisWeek => "thisWeek",
            FollowUp::NextWeek => "nextWeek",
        }
    }

    pub fn from_code(s: &str) -> Option<FollowUp> {
        FollowUp::ALL.into_iter().find(|f| f.code().eq_ignore_ascii_case(s))
    }

    pub fn label(self) -> &'static str {
        match self {
            FollowUp::Today => "Today",
            FollowUp::Tomorrow => "Tomorrow",
            FollowUp::ThisWeek => "This week",
            FollowUp::NextWeek => "Next week",
        }
    }

    /// The local day it's due (days since 1970-01-01).
    pub fn due(self, clock: &Clock) -> i64 {
        let today = clock.local_day(clock.now);
        // 0 = Monday … 6 = Sunday; Friday is 4.
        let weekday = (today + 3).rem_euclid(7);
        match self {
            FollowUp::Today => today,
            FollowUp::Tomorrow => today + 1,
            FollowUp::ThisWeek => today + (4 - weekday).max(0),
            FollowUp::NextWeek => today + (7 - weekday) + 4,
        }
    }
}

/// Today, as a local day (days since 1970-01-01).
pub fn today(clock: &Clock) -> i64 {
    clock.local_day(clock.now)
}

/// A local day as `YYYY-MM-DD`.
pub fn day_string(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A local day as the reading pane shows it: `Fri 2026-10-16`.
pub fn day_label(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    let weekday = WEEKDAYS.get((day + 3).rem_euclid(7) as usize).copied().unwrap_or("");
    format!("{weekday} {y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_up_days() {
        // NOW is a Wednesday, 2026-10-14.
        let clock = Clock::new(NOW, 0);
        let day = |f: FollowUp| day_string(f.due(&clock));
        assert_eq!(day(FollowUp::Today), "2026-10-14");
        assert_eq!(day(FollowUp::Tomorrow), "2026-10-15");
        assert_eq!(day(FollowUp::ThisWeek), "2026-10-16");
        assert_eq!(day(FollowUp::NextWeek), "2026-10-23");
        // On a Saturday, "this week" is today.
        let sat = Clock::new(NOW + 3 * 86_400, 0);
        assert_eq!(day_string(FollowUp::ThisWeek.due(&sat)), "2026-10-17");
        assert_eq!(day_string(FollowUp::NextWeek.due(&sat)), "2026-10-23");
        assert_eq!(day_label(FollowUp::ThisWeek.due(&clock)), "Fri 2026-10-16");
        assert_eq!(FollowUp::from_code("nextweek"), Some(FollowUp::NextWeek));
    }

    // 2026-10-14 12:00:00 UTC, a Wednesday.
    const NOW: i64 = 1_791_979_200;

    #[test]
    fn civil_round_trip() {
        for d in [-800_000i64, -1, 0, 1, 365, 20_000, 2_000_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        let c = civil(NOW, &Clock::new(NOW, 0));
        assert_eq!((c.year, c.month, c.day, c.hour, c.weekday), (2026, 10, 14, 12, 2));
    }

    #[test]
    fn groups() {
        let clock = Clock::new(NOW, 0);
        let day = 86_400;
        assert_eq!(DateGroup::of(NOW - 60, &clock), DateGroup::Today);
        assert_eq!(DateGroup::of(NOW + day, &clock), DateGroup::Future);
        assert_eq!(DateGroup::of(NOW - day, &clock), DateGroup::Yesterday);
        assert_eq!(DateGroup::of(NOW - 2 * day, &clock), DateGroup::ThisWeek(0));
        assert_eq!(DateGroup::of(NOW - 4 * day, &clock), DateGroup::LastWeek);
        assert_eq!(DateGroup::of(NOW - 12 * day, &clock), DateGroup::TwoWeeksAgo);
        assert_eq!(DateGroup::of(NOW - 30 * day, &clock), DateGroup::LastMonth);
        assert_eq!(DateGroup::of(NOW - 400 * day, &clock), DateGroup::Older);
    }

    #[test]
    fn offsets_move_the_day_boundary() {
        // 23:30 UTC on the 13th is already the 14th in UTC+2.
        let t = NOW - 12 * 3600 - 1800;
        assert_eq!(DateGroup::of(t, &Clock::new(NOW, 0)), DateGroup::Yesterday);
        assert_eq!(DateGroup::of(t, &Clock::new(NOW, 7200)), DateGroup::Today);
    }

    #[test]
    fn formats() {
        let clock = Clock::new(NOW, 0);
        assert_eq!(list_time(NOW - 3600, &clock), "11:00");
        assert_eq!(list_time(NOW - 2 * 86_400, &clock), "Mon 12:00");
        assert_eq!(list_time(NOW - 30 * 86_400, &clock), "Sep 14");
        assert_eq!(list_time(NOW - 400 * 86_400, &clock), "2025-09-09");
        assert_eq!(full_time(NOW, &clock), "Wed 2026-10-14 12:00");
        // Hostile values never panic.
        let _ = full_time(i64::MAX, &clock);
        let _ = list_time(i64::MIN, &clock);
        let _ = DateGroup::of(i64::MIN, &Clock::new(i64::MAX, i32::MAX));
    }
}
