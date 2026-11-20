//! Work time (issue #185), as Outlook's Calendar options keep it: the days of the week worked and
//! the hours between, which the work week shows and the day and week views shade outside of.

use mailcraft_model::pim::work_hours::{WorkPeriod, work_spans};

use super::*;

/// The weekdays worked (0 Monday … 6 Sunday), from the one that starts the working week: the
/// first worked after one that isn't (Sunday for Sunday to Thursday), Monday when all are.
fn week_from(days: u8) -> Vec<u32> {
    let worked = |w: u32| days & (1 << (w % 7)) != 0;
    let start = (0..7).find(|w| worked(*w) && !worked(w + 6)).unwrap_or(0);
    (start..start + 7).filter(|w| worked(*w)).map(|w| w % 7).collect()
}

impl Session {
    /// The work time, as set: Monday to Friday, 08:00 to 17:00, when none is.
    pub fn work_time(&self) -> WorkPeriod {
        let w = self.prefs.work_time;
        if w.days & 0x7f == 0 || w.end <= w.start { WorkPeriod::default() } else { w }
    }

    /// The local days the work week shows, in order: the week's worked days, the week starting
    /// on the first of them on or before the day shown.
    pub fn work_week_days(&self) -> Vec<i64> {
        let d = self.cal_day();
        let week = week_from(self.work_time().days);
        let Some(first) = week.first().copied() else { return Vec::new() };
        let start = d - (i64::from(weekday_index(d)) - i64::from(first)).rem_euclid(7);
        week.iter().map(|w| start + (i64::from(*w) - i64::from(first)).rem_euclid(7)).collect()
    }

    /// The local days the day and week views show, in order (the work week's may skip days).
    pub fn cal_day_list(&self) -> Vec<i64> {
        if self.cal_mode == CalMode::WorkWeek {
            return self.work_week_days();
        }
        let (d0, d1) = self.cal_days();
        (d0..d1).collect()
    }

    /// When the user works between `from` and `to` (Unix seconds), in this computer's time.
    pub fn work_spans(&self, from: i64, to: i64) -> Vec<(i64, i64)> {
        work_spans(&[self.work_time()], from, to, |t| self.clock.offset_at(t))
    }

    /// When a calendar's owner works between `from` and `to`: a colleague's hours when their
    /// server gave them with their free/busy time, else the user's own.
    pub fn calendar_work_spans(&self, account: &str, calendar: &str, from: i64, to: i64) -> Vec<(i64, i64)> {
        match self.calendar_work(account, calendar) {
            Some(hours) => hours.spans(from, to),
            None => self.work_spans(from, to),
        }
    }

    /// Set the work time: the days worked (0 Monday … 6 Sunday), and the hours between (minutes
    /// past midnight), what isn't given kept.
    pub fn set_work_time(&mut self, days: Option<&[u32]>, start: Option<u16>, end: Option<u16>) -> CmdResult {
        let mut w = self.work_time();
        if let Some(days) = days {
            if let Some(bad) = days.iter().find(|d| **d > 6) {
                return Err(CmdError::Params(format!("a day is 0 (Monday) to 6 (Sunday), not {bad}")));
            }
            w.days = days.iter().fold(0, |b, d| b | (1 << d));
            if w.days == 0 {
                return Err(CmdError::Params("at least one day is worked".into()));
            }
        }
        w.start = start.unwrap_or(w.start);
        w.end = end.unwrap_or(w.end);
        if w.start >= w.end || w.end > 24 * 60 {
            return Err(CmdError::Params("the work day ends after it starts, by midnight".into()));
        }
        self.prefs.work_time = w;
        Ok(work_json(w))
    }
}

/// A work time as the command gives it back.
fn work_json(w: WorkPeriod) -> serde_json::Value {
    let hm = |m: u16| format!("{:02}:{:02}", m / 60, m % 60);
    let days: Vec<u32> = (0..7).filter(|d| w.works_on(*d)).collect();
    serde_json::json!({ "days": days, "start": hm(w.start), "end": hm(w.end) })
}

#[cfg(test)]
mod tests {
    use super::week_from;

    #[test]
    fn the_working_week_starts_after_the_days_off() {
        assert_eq!(week_from(0b001_1111), [0, 1, 2, 3, 4], "Monday to Friday");
        assert_eq!(week_from(0b100_1111), [6, 0, 1, 2, 3], "Sunday to Thursday");
        assert_eq!(week_from(0b111_1111), [0, 1, 2, 3, 4, 5, 6], "every day: from Monday");
        assert_eq!(week_from(0b001_0101), [0, 2, 4], "Monday, Wednesday, Friday");
        assert!(week_from(0).is_empty());
    }
}
