//! Calendar arithmetic for the message list: date groups (Today, Yesterday, Last week…) and the
//! short and long times shown for a message. Pure integer maths on Unix seconds plus the local
//! time zone (its offset on the date shown), so it works the same everywhere, wasm included.

use serde::{Deserialize, Serialize};

pub mod typing;
pub mod zone;

pub use typing::{DayHalves, read_day, read_time, typed_day};

use zone::Zone;

/// "Now" and the local time zone, as the app reports them each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clock {
    /// Unix seconds, UTC.
    pub now: i64,
    /// Local offset from UTC in seconds (east positive), now.
    pub offset: i32,
    /// The local time zone, when known: then each time is shown with the offset in force on its
    /// own date (summer or winter), not today's.
    #[serde(skip)]
    pub zone: Option<Zone>,
    /// How dates and times are written (issue #46).
    #[serde(skip)]
    pub style: DateStyle,
}

/// How dates are written (issue #46): the order of a date's day, month and year, the clock, and
/// whether the message list gives every message its whole date and time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DateStyle {
    #[serde(deserialize_with = "order_or_iso")]
    pub order: DateOrder,
    /// `1:53 PM` instead of `13:53`.
    pub hours12: bool,
    /// `28/10/2026 1:53 PM` on every message in the list, instead of the time today, the day
    /// this week and the date before that.
    pub full_in_list: bool,
}

/// An order saved by a later MailCraft and unknown here: MailCraft's own, rather than every
/// setting lost with it.
fn order_or_iso<'de, D: serde::Deserializer<'de>>(d: D) -> Result<DateOrder, D::Error> {
    Ok(serde_json::Value::deserialize(d).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
}

/// The order of a date's parts, and what joins them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DateOrder {
    /// `2026-10-28`: MailCraft's own, read the same everywhere.
    #[default]
    Iso,
    /// `28/10/2026`, as most of the world writes it.
    DayMonthYear,
    /// `10/28/2026`, as the United States writes it.
    MonthDayYear,
    /// `28.10.2026`, as much of Europe writes it.
    DayMonthYearDots,
    /// `2026.10.28`, the year first, as Hungary writes it.
    YearMonthDayDots,
}

impl DateOrder {
    pub const ALL: [DateOrder; 5] =
        [DateOrder::Iso, DateOrder::DayMonthYear, DateOrder::MonthDayYear, DateOrder::DayMonthYearDots, DateOrder::YearMonthDayDots];

    pub fn code(self) -> &'static str {
        match self {
            DateOrder::Iso => "iso",
            DateOrder::DayMonthYear => "dmy",
            DateOrder::MonthDayYear => "mdy",
            DateOrder::DayMonthYearDots => "dmy.",
            DateOrder::YearMonthDayDots => "ymd.",
        }
    }

    pub fn from_code(s: &str) -> Option<DateOrder> {
        DateOrder::ALL.into_iter().find(|o| o.code() == s)
    }

    /// A day written in this order.
    pub fn day(self, y: i64, m: u32, d: u32) -> String {
        match self {
            DateOrder::Iso => format!("{y:04}-{m:02}-{d:02}"),
            DateOrder::DayMonthYear => format!("{d:02}/{m:02}/{y:04}"),
            DateOrder::MonthDayYear => format!("{m:02}/{d:02}/{y:04}"),
            DateOrder::DayMonthYearDots => format!("{d:02}.{m:02}.{y:04}"),
            DateOrder::YearMonthDayDots => format!("{y:04}.{m:02}.{d:02}"),
        }
    }

    /// Whether the day comes before the month (`3 Oct` rather than `Oct 3`).
    pub fn day_first(self) -> bool {
        matches!(self, DateOrder::DayMonthYear | DateOrder::DayMonthYearDots)
    }
}

impl Clock {
    pub fn new(now: i64, offset: i32) -> Clock {
        Clock { now, offset: offset.clamp(-14 * 3600, 14 * 3600), zone: None, style: DateStyle::default() }
    }

    /// The same clock in a time zone.
    pub fn in_zone(self, zone: Option<Zone>) -> Clock {
        Clock { zone, ..self }
    }

    /// The same clock, writing dates and times so.
    pub fn styled(self, style: DateStyle) -> Clock {
        Clock { style, ..self }
    }

    /// The local zone: the one known, else today's offset all year.
    pub fn local_zone(&self) -> Zone {
        self.zone.unwrap_or(Zone::Fixed(self.offset))
    }

    /// The local offset (seconds east of UTC) at the instant `t`.
    pub fn offset_at(&self, t: i64) -> i64 {
        self.local_zone().offset_at(t)
    }

    /// The local wall-clock time of the instant `t` (seconds since the epoch, as if UTC).
    pub fn local(&self, t: i64) -> i64 {
        t.saturating_add(self.offset_at(t))
    }

    /// The instant of a local wall-clock time.
    pub fn utc(&self, local: i64) -> i64 {
        self.local_zone().to_utc(local)
    }

    /// The instant a local day (days since the epoch) starts.
    pub fn midnight(&self, day: i64) -> i64 {
        self.utc(day.saturating_mul(86_400))
    }

    /// The local day (days since the epoch) of the instant `t`.
    pub fn local_day(&self, t: i64) -> i64 {
        self.local(t).div_euclid(86_400)
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

/// The days in month `m` (1 to 12) of year `y`, proleptic Gregorian as [`days_from_civil`]
/// counts them: 28 to 31; 0 for a month no calendar has.
pub fn month_len(y: i64, m: u32) -> i64 {
    let leap = y.rem_euclid(4) == 0 && (y.rem_euclid(100) != 0 || y.rem_euclid(400) == 0);
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    }
}

/// (year, month, day) → days since 1970-01-01. A year too far out for an `i64` of days gives the
/// nearest end (`i64::MIN` or `i64::MAX`), never a wrapped day: callers that read a year from
/// a server or a file bound it themselves.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y.saturating_sub(1) } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m.clamp(1, 12));
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d.clamp(1, 31)) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let far = if era < 0 { i64::MIN } else { i64::MAX };
    era.checked_mul(146_097).and_then(|e| e.checked_add(doe - 719_468)).unwrap_or(far)
}

/// Split a time into local calendar parts.
pub fn civil(t: i64, clock: &Clock) -> Civil {
    let local = clock.local(t);
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Civil { year, month, day, hour: (secs / 3600) as u32, minute: ((secs % 3600) / 60) as u32, weekday: weekday_index(days) }
}

/// The weekdays' short English names, Monday first (the interface translates them).
pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
/// The weekdays' whole English names, Monday first.
pub const WEEKDAYS_LONG: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The weekday of a day (days since 1970-01-01): 0 = Monday … 6 = Sunday. 1970-01-01 was a
/// Thursday; any day, however far off (it may come from disk), has one.
pub fn weekday_index(day: i64) -> u32 {
    (day.rem_euclid(7) + 3).rem_euclid(7) as u32
}

/// A day's weekday, short and in English: `Mon`.
pub fn weekday_short(day: i64) -> &'static str {
    WEEKDAYS.get(weekday_index(day) as usize).copied().unwrap_or("")
}

/// A day's weekday, whole and in English: `Monday`.
pub fn weekday_long(day: i64) -> &'static str {
    WEEKDAYS_LONG.get(weekday_index(day) as usize).copied().unwrap_or("")
}

/// A month's short English name (1 = `Jan`); nothing for a month no calendar has.
pub fn month_short(month: u32) -> &'static str {
    month.checked_sub(1).and_then(|i| MONTHS.get(i as usize)).copied().unwrap_or("")
}

fn weekday_name(c: &Civil) -> &'static str {
    WEEKDAYS.get(c.weekday as usize).copied().unwrap_or("")
}

fn month_name(c: &Civil) -> &'static str {
    month_short(c.month)
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
    /// This month, before the weeks above.
    ThisMonth,
    LastMonth,
    Older,
}

impl DateGroup {
    pub fn of(t: i64, clock: &Clock) -> DateGroup {
        DateGroup::of_day(clock.local_day(t), today(clock))
    }

    /// The group of the local day `day` ([`Clock::local_day`]), `today` being today's.
    pub fn of_day(day: i64, today: i64) -> DateGroup {
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
        let week_start = today - i64::from(weekday_index(today));
        if day >= week_start {
            return DateGroup::ThisWeek(weekday_index(day));
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
        match months_back {
            // The first days of a month, from its end: they were under "Last month".
            0 => DateGroup::ThisMonth,
            1 => DateGroup::LastMonth,
            _ => DateGroup::Older,
        }
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
            DateGroup::ThisMonth => "Earlier this month",
            DateGroup::LastMonth => "Last month",
            DateGroup::Older => "Older",
        }
    }
}

/// The compact time in the message list: `14:05` today, `Mon 14:05` this week, `Oct 3` (`3 Oct`
/// with the day first) this year, the date before that (`2025-10-03`, or as [`DateStyle`] says);
/// or the whole date and time each time, when it says so (issue #46).
pub fn list_time(t: i64, clock: &Clock) -> String {
    let c = civil(t, clock);
    let style = clock.style;
    if style.full_in_list {
        return format!("{} {}", date_of(&c, style), time_of(&c, style));
    }
    let now = civil(clock.now, clock);
    let days = today(clock) - clock.local_day(t);
    if days == 0 {
        time_of(&c, style)
    } else if (1..7).contains(&days) {
        format!("{} {}", weekday_name(&c), time_of(&c, style))
    } else if c.year == now.year {
        if style.order.day_first() { format!("{} {}", c.day, month_name(&c)) } else { format!("{} {}", month_name(&c), c.day) }
    } else {
        date_of(&c, style)
    }
}

/// The full date in the reading pane: `Wed 2026-10-14 14:05` (as [`DateStyle`] says).
pub fn full_time(t: i64, clock: &Clock) -> String {
    let c = civil(t, clock);
    format!("{} {} {}", weekday_name(&c), date_of(&c, clock.style), time_of(&c, clock.style))
}

/// The time of day alone: `14:05`, or `2:05 PM` on a 12-hour clock.
pub fn hm(t: i64, clock: &Clock) -> String {
    time_of(&civil(t, clock), clock.style)
}

/// An hour as the calendar's day and week label it: `13:00`, or `1 PM` on a 12-hour clock.
pub fn hour_label(hour: u32, style: DateStyle) -> String {
    let hour = hour % 24;
    if style.hours12 { format!("{} {}", twelve(hour), if hour < 12 { "AM" } else { "PM" }) } else { format!("{hour:02}:00") }
}

/// An hour of the day (0 to 23) on a 12-hour clock: 12, 1, … 11.
fn twelve(hour: u32) -> u32 {
    match hour % 12 {
        0 => 12,
        h => h,
    }
}

/// A day written as [`DateStyle`] says.
fn date_of(c: &Civil, style: DateStyle) -> String {
    style.order.day(c.year, c.month, c.day)
}

/// A time of day written as [`DateStyle`] says.
fn time_of(c: &Civil, style: DateStyle) -> String {
    time_text(c.hour, c.minute, style)
}

/// An hour (0 to 23) and minute written as [`DateStyle`] says: `13:30`, or `1:30 PM` on a
/// 12-hour clock. [`read_time`] reads it back.
pub fn time_text(hour: u32, minute: u32, style: DateStyle) -> String {
    let hour = hour % 24;
    if style.hours12 { format!("{}:{:02} {}", twelve(hour), minute, if hour < 12 { "AM" } else { "PM" }) } else { format!("{hour:02}:{minute:02}") }
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
        let today = today(clock);
        // 0 = Monday … 6 = Sunday; Friday is 4.
        let weekday = i64::from(weekday_index(today));
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
    DateOrder::Iso.day(y, m, d)
}

/// A local day as the reading pane shows it, written as the clock's [`DateStyle`] says:
/// `Fri 2026-10-16`, `Fri 16/10/2026`…
pub fn day_text(day: i64, clock: &Clock) -> String {
    weekday_and_day(day, clock.style.order)
}

/// A local day with its weekday, its date in `order`.
fn weekday_and_day(day: i64, order: DateOrder) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{} {}", weekday_short(day), order.day(y, m, d))
}

/// A local day as the lists name one: `Mon Oct 26` in the viewer's year (`Mon 26 Oct` with the
/// day first, as [`list_time`] writes a day), else the whole date as [`day_text`] writes it.
pub fn day_short(day: i64, clock: &Clock) -> String {
    let (y, m, d) = civil_from_days(day);
    if y != civil_from_days(today(clock)).0 {
        return day_text(day, clock);
    }
    let (weekday, month) = (weekday_short(day), month_short(m));
    if clock.style.order.day_first() { format!("{weekday} {d} {month}") } else { format!("{weekday} {month} {d}") }
}

/// A local day as [`day_text`] writes it in MailCraft's own order, whatever the Dates option
/// says: `Fri 2026-10-16`. For text written where no clock is at hand (an error message).
pub fn day_label(day: i64) -> String {
    weekday_and_day(day, DateOrder::Iso)
}

/// A moment (Unix seconds) as its date and time in UTC, without a zone: `2026-11-03T10:00:00`
/// (what Microsoft Graph takes with `"timeZone": "UTC"` beside it).
pub fn utc_date_time(t: i64) -> String {
    let s = t.rem_euclid(86_400);
    format!("{}T{:02}:{:02}:{:02}", day_string(t.div_euclid(86_400)), s / 3600, s / 60 % 60, s % 60)
}

/// A moment (Unix seconds) as an RFC 3339 time in UTC: `2026-11-03T10:00:00Z`.
pub fn utc_stamp(t: i64) -> String {
    format!("{}Z", utc_date_time(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UTC times written as each of the network layer's own copies wrote them.
    #[test]
    fn utc_times_as_written_before() {
        let old = |t: i64| {
            let (y, m, d) = civil_from_days(t.div_euclid(86_400));
            let s = t.rem_euclid(86_400);
            format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
        };
        let mut t: i64 = -70_000_000_000;
        while t < 70_000_000_000 {
            assert_eq!(utc_stamp(t), old(t), "{t}");
            assert_eq!(format!("{}Z", utc_date_time(t)), utc_stamp(t));
            t += 7_919_993;
        }
        assert_eq!(utc_stamp(1_793_700_000), "2026-11-03T10:00:00Z");
        assert_eq!(utc_date_time(-1), "1969-12-31T23:59:59");
        assert!(!utc_stamp(i64::MAX).is_empty() && !utc_stamp(i64::MIN).is_empty());
    }

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
        // A day from a damaged file, far off either way: a label all the same.
        assert!(!day_label(i64::MAX).is_empty() && !day_label(i64::MIN).is_empty());
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
        // A year no calendar has (from a hostile server or file): the nearest end, no overflow.
        assert_eq!(days_from_civil(i64::MAX, 12, 31), i64::MAX);
        assert_eq!(days_from_civil(i64::MIN, 1, 1), i64::MIN);
        assert_eq!(days_from_civil(9_000_000_000_000_000_000, 1, 1), i64::MAX);
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
        // From Friday 30 October 2026: the 2nd is this month, not last; 28 September is.
        let (today, second, september) = (20756, 20728, 20724);
        assert_eq!(DateGroup::of_day(second, today), DateGroup::ThisMonth);
        assert_eq!(DateGroup::of_day(today - 22, today), DateGroup::ThreeWeeksAgo);
        assert_eq!(DateGroup::of_day(september, today), DateGroup::LastMonth);
        assert_eq!(DateGroup::of_day(september - 40, today), DateGroup::Older);
    }

    /// Issue #46: dates written day first, month first or with dots, on a 12-hour clock, and the
    /// whole date and time on every message in the list, as Evolution can show them.
    #[test]
    fn dates_and_times_as_chosen() {
        let style = |order, hours12, full_in_list| Clock::new(NOW, 0).styled(DateStyle { order, hours12, full_in_list });
        let day = 86_400;
        let dmy = style(DateOrder::DayMonthYear, true, false);
        assert_eq!(list_time(NOW - 3600, &dmy), "11:00 AM");
        assert_eq!(list_time(NOW - 2 * day, &dmy), "Mon 12:00 PM");
        assert_eq!(list_time(NOW - 30 * day, &dmy), "14 Sep");
        assert_eq!(list_time(NOW - 400 * day, &dmy), "09/09/2025");
        assert_eq!(full_time(NOW - 12 * 3600, &dmy), "Wed 14/10/2026 12:00 AM");
        assert_eq!(hm(NOW + 3600 + 300, &dmy), "1:05 PM");
        let mdy = style(DateOrder::MonthDayYear, false, false);
        assert_eq!(list_time(NOW - 30 * day, &mdy), "Sep 14");
        assert_eq!(full_time(NOW, &mdy), "Wed 10/14/2026 12:00");
        assert_eq!(full_time(NOW, &style(DateOrder::DayMonthYearDots, false, false)), "Wed 14.10.2026 12:00");
        // The whole date and time on every message, today's too.
        let full = style(DateOrder::DayMonthYear, true, true);
        assert_eq!(list_time(NOW - 3600, &full), "14/10/2026 11:00 AM");
        assert_eq!(list_time(NOW - 400 * day, &full), "09/09/2025 12:00 PM");
        // MailCraft's own, as before.
        assert_eq!(full_time(NOW, &Clock::new(NOW, 0)), "Wed 2026-10-14 12:00");
        for o in DateOrder::ALL {
            assert_eq!(DateOrder::from_code(o.code()), Some(o));
        }
        assert_eq!(DateOrder::from_code("ymd"), None);
        // The calendar's hours.
        assert_eq!(hour_label(13, DateStyle::default()), "13:00");
        assert_eq!((hour_label(0, dmy.style), hour_label(12, dmy.style), hour_label(13, dmy.style)), ("12 AM".into(), "12 PM".into(), "1 PM".into()));
        // An order a later MailCraft saved: MailCraft's own, the rest of the style kept.
        let later: DateStyle = serde_json::from_str(r#"{"order": "ymdDots", "hours12": true}"#).unwrap();
        assert_eq!(later, DateStyle { order: DateOrder::Iso, hours12: true, full_in_list: false });
    }

    /// A day the lists name: its weekday, month and day in the viewer's year, the whole date in
    /// another, in the order the Dates option says.
    #[test]
    fn short_days() {
        let clock = Clock::new(NOW, 0);
        let dmy = clock.styled(DateStyle { order: DateOrder::DayMonthYear, ..DateStyle::default() });
        assert_eq!(day_short(days_from_civil(2026, 10, 26), &clock), "Mon Oct 26");
        assert_eq!(day_short(days_from_civil(2026, 1, 5), &dmy), "Mon 5 Jan");
        assert_eq!(day_short(days_from_civil(2025, 10, 27), &clock), "Mon 2025-10-27");
        assert_eq!(day_short(days_from_civil(2027, 3, 1), &dmy), "Mon 01/03/2027");
        // The viewer's year: at 23:30 on 31 December in UTC, it's already 2027 in UTC+1.
        let eve = Clock::new(days_from_civil(2027, 1, 1) * 86_400 - 1800, 3600);
        assert_eq!(day_short(days_from_civil(2027, 1, 5), &eve), "Tue Jan 5");
        assert_eq!(day_short(days_from_civil(2026, 12, 20), &eve), "Sun 2026-12-20");
        for far in [i64::MIN, i64::MAX] {
            assert!(!day_short(far, &clock).is_empty());
        }
    }

    /// The weekday, the day string and the names, as every copy wrote them by hand before: the
    /// same for every day from year 0 to 10000, before 1970 and long after, and for days far off
    /// either way (where `day + 3` would overflow, the weekday is still the week's).
    #[test]
    fn weekdays_and_day_strings_as_written_by_hand() {
        const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        const LONG: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
        const SHORT: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let (first, last) = (days_from_civil(0, 1, 1), days_from_civil(10_000, 12, 31));
        assert_eq!((day_string(days_from_civil(1, 1, 1)), day_string(days_from_civil(9999, 12, 31))), ("0001-01-01".into(), "9999-12-31".into()));
        let far = [i64::MIN, i64::MIN + 1, i64::MIN / 86_400, -1_000_000_000, i64::MAX / 86_400, i64::MAX - 3, i64::MAX];
        let mut strings = 0;
        for day in (first..=last).chain(far) {
            let wd = weekday_index(day);
            // `(day + 3).rem_euclid(7)` (calendar, tasks, follow-ups, recurrences) and the To-Do
            // Bar's `(day.rem_euclid(7) + 3) % 7`.
            if let Some(d3) = day.checked_add(3) {
                assert_eq!(i64::from(wd), d3.rem_euclid(7), "{day}");
            }
            assert_eq!(i64::from(wd), (day.rem_euclid(7) + 3) % 7, "{day}");
            assert_eq!((weekday_short(day), weekday_long(day)), (DAYS[wd as usize], LONG[wd as usize]));
            // Every day of years 0 to 2, 1969 to 1971 and 9998 to 10000; a day in eleven between.
            let (y, m, d) = civil_from_days(day);
            if matches!(y, 0..=2 | 1969..=1971 | 9998..) || y < 0 || day.rem_euclid(11) == 0 {
                strings += 1;
                assert_eq!(day_string(day), format!("{y:04}-{m:02}-{d:02}"));
                assert_eq!(day_label(day), format!("{} {y:04}-{m:02}-{d:02}", DAYS[wd as usize]));
                for order in DateOrder::ALL {
                    let mut clock = Clock::new(NOW, 0);
                    clock.style.order = order;
                    assert_eq!(day_text(day, &clock), format!("{} {}", DAYS[wd as usize], order.day(y, m, d)));
                }
                assert_eq!(month_short(m), SHORT[m as usize - 1]);
            }
        }
        assert!(strings > 300_000, "{strings}");
        assert_eq!((month_short(0), month_short(13)), ("", ""));
        // A time's local day, as the reading pane and the forms wrote it from `civil`.
        for zone in
            [Clock::new(NOW, 0), Clock::new(NOW, -14 * 3600), Clock::new(NOW, 14 * 3600), Clock::new(NOW, 0).in_zone(Zone::named("America/New_York"))]
        {
            for t in (-200_000_000_000i64..300_000_000_000).step_by(7_777_777) {
                let c = civil(t, &zone);
                assert_eq!(day_string(zone.local_day(t)), format!("{:04}-{:02}-{:02}", c.year, c.month, c.day), "{t}");
            }
        }
    }

    /// A month's length, as the copies worked it out before: the first of the next month less
    /// the first of this one (recurrences, time zones), and the Out of Office form's table.
    #[test]
    fn month_lengths_as_worked_out_by_hand() {
        for y in -1_000..=11_000i64 {
            for m in 1..=12u32 {
                let next = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
                assert_eq!(month_len(y, m), next - days_from_civil(y, m, 1), "{y}-{m}");
                let table = match m {
                    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                    4 | 6 | 9 | 11 => 30,
                    2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
                    _ => 28,
                };
                if y >= 0 {
                    assert_eq!(month_len(y, m), table, "{y}-{m}");
                }
            }
            assert_eq!((month_len(y, 0), month_len(y, 13)), (0, 0));
        }
        assert_eq!((month_len(i64::MIN, 2), month_len(i64::MAX, 2)), (29, 28));
    }

    #[test]
    fn offsets_move_the_day_boundary() {
        // 23:30 UTC on the 13th is already the 14th in UTC+2.
        let t = NOW - 12 * 3600 - 1800;
        assert_eq!(DateGroup::of(t, &Clock::new(NOW, 0)), DateGroup::Yesterday);
        assert_eq!(DateGroup::of(t, &Clock::new(NOW, 7200)), DateGroup::Today);
    }

    /// In a zone, each time is shown with the offset of its own date: a meeting at 09:00 in New
    /// York reads 09:00 in January and in July, whichever season it is now.
    #[test]
    fn times_in_the_viewers_zone() {
        let ny = Zone::named("America/New_York");
        let clock = Clock::new(NOW, -4 * 3600).in_zone(ny);
        let jan = days_from_civil(2026, 1, 5) * 86_400 + 14 * 3600;
        let jul = days_from_civil(2026, 7, 6) * 86_400 + 13 * 3600;
        assert_eq!((civil(jan, &clock).hour, civil(jul, &clock).hour), (9, 9));
        assert_eq!(full_time(jan, &clock), "Mon 2026-01-05 09:00");
        assert_eq!(clock.midnight(days_from_civil(2026, 1, 5)), days_from_civil(2026, 1, 5) * 86_400 + 5 * 3600);
        assert_eq!(clock.midnight(days_from_civil(2026, 7, 6)), days_from_civil(2026, 7, 6) * 86_400 + 4 * 3600);
        // Without a zone, today's offset all year (as before).
        let fixed = Clock::new(NOW, -4 * 3600);
        assert_eq!((civil(jan, &fixed).hour, civil(jul, &fixed).hour), (10, 9));
        assert_eq!(fixed.local_zone(), Zone::Fixed(-4 * 3600));
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
