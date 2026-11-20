//! A time free for everyone whose calendar is shown (issue #186), as Outlook's AutoPick Next
//! and Previous find one for a meeting: of a length chosen, inside everyone's work hours (#185),
//! where none of their calendars is taken (tentative counting as taken). It's looked for here,
//! over what the calendar has (events, and colleagues' free/busy times), so it works for every
//! kind of account; the time found is marked in the calendar until the calendar is moved.

use mailcraft_model::ical::ShowAs;
use mailcraft_model::pim::work_hours::{merged, outside, overlap};

use super::free_busy::Covered;
use super::*;

/// Times start on the half hour (local), as a meeting's do.
const STEP: i64 = 1800;
/// How far one search looks: six weeks, what a colleague's times are asked for at most.
const LOOK: i64 = 42 * 86_400;
/// The lengths offered, in minutes (the command takes any from 15 minutes to a day).
pub const FREE_LENGTHS: [u16; 8] = [15, 30, 60, 90, 120, 180, 240, 480];

/// What's said when none is found, or can't be looked for yet: whole sentences, translated as
/// they are.
pub const NONE_AHEAD: &str = "No time when everyone is free in the next six weeks";
pub const NONE_YET: &str = "No time when everyone is free as far as their free/busy times reach: Next looks further once they come";
pub const NONE_EARLIER: &str = "No earlier time when everyone is free";
pub const NONE_EARLIER_YET: &str =
    "No earlier time when everyone is free as far as their free/busy times reach: Previous looks further once they come";
pub const NOT_COME: &str = "Their free/busy times haven't come yet: try again in a moment";
pub const UNKNOWN_FREE: &str = "Some people's free/busy times can't be had: they count as free";

impl Session {
    /// The time found free for everyone shown, marked in the calendar.
    pub fn free_slot(&self) -> Option<(i64, i64)> {
        self.free_slot
    }

    /// `t` moved to the half hour at or after it (`up`), or at or before it, in local time.
    fn on_the_half_hour(&self, t: i64, up: bool) -> i64 {
        let past = self.clock.local(t).rem_euclid(STEP);
        if past == 0 {
            t
        } else if up {
            t + STEP - past
        } else {
            t - past
        }
    }

    /// Find the next time (or the one before, `back`) when everyone whose calendar is shown is
    /// free for `minutes` (the length kept when none is given): after the time found last, else
    /// from the days shown on, not before now. The calendar goes to it; nothing found says so.
    pub fn find_free_time(&mut self, minutes: Option<u64>, back: bool) -> CmdResult {
        if let Some(m) = minutes {
            if !(15..=1440).contains(&m) {
                return Err(CmdError::Params(format!("`minutes` is 15 to 1440, not {m}")));
            }
            self.prefs.free_time_minutes = u16::try_from(m).unwrap_or(30);
        }
        let length = i64::from(self.prefs.free_time_minutes.clamp(15, 1440)) * 60;
        let cals = self.shown_calendars();
        if cals.is_empty() {
            return Err(CmdError::Failed("Tick a calendar to show it here".into()));
        }
        let now = self.clock.now;
        let (d0, _) = self.cal_days();
        let anchor = match self.free_slot {
            // The next starts after the one found, the one before before it.
            Some((start, _)) => start + if back { 0 } else { STEP },
            None if back => self.local_midnight(d0),
            None => self.local_midnight(d0).max(now),
        };
        // What can be looked through: as far as each colleague's times reach.
        let (mut lo, mut hi) = if back { ((anchor - LOOK).max(now), anchor + length) } else { (anchor.max(now), anchor + LOOK) };
        let (mut unknown, mut cut) = (Vec::new(), false);
        for (account, id, ..) in &cals {
            match self.free_busy_covered(account, id) {
                Some((_, Covered::Asking)) => return Err(CmdError::Failed(NOT_COME.into())),
                Some((email, Covered::Unknown)) => unknown.push(email),
                Some((_, Covered::Window(from, to))) => {
                    cut |= if back { from > lo } else { to < hi };
                    (lo, hi) = (lo.max(from), hi.min(to));
                }
                None => {}
            }
        }
        let found = if lo < hi { self.free_in(&cals, (lo, hi), length, anchor, back) } else { None };
        let Some(start) = found else {
            self.free_slot = None;
            let edge = if back { lo } else { hi };
            let date = mailcraft_model::dates::day_string(self.clock.local_day(edge));
            // Their times reach no further: the calendar goes there, so they're asked for.
            if cut && edge > now {
                self.cal_day = Some(self.clock.local_day(edge));
            }
            let text = match (back, cut) {
                (false, true) => NONE_YET,
                (false, false) => NONE_AHEAD,
                (true, true) => NONE_EARLIER_YET,
                (true, false) => NONE_EARLIER,
            };
            self.notice(text, false);
            return Ok(serde_json::json!({ "found": false, "to": date, "unknown": unknown }));
        };
        let slot = (start, start + length);
        if !unknown.is_empty() {
            self.notice(UNKNOWN_FREE, false);
        }
        self.free_slot = Some(slot);
        self.cal_day = Some(self.clock.local_day(start));
        Ok(serde_json::json!({
            "found": true,
            "start": slot.0,
            "end": slot.1,
            "date": mailcraft_model::dates::day_string(self.clock.local_day(start)),
            "unknown": unknown,
        }))
    }

    /// The first start (or the last, `back`) of a time `length` long in `window` when the
    /// calendars' owners all work and none of them is taken: after `anchor` (or before it).
    fn free_in(&self, cals: &[(String, String, String, u8)], (lo, hi): (i64, i64), length: i64, anchor: i64, back: bool) -> Option<i64> {
        let work = cals.iter().map(|(account, id, ..)| self.calendar_work_spans(account, id, lo, hi)).reduce(|a, b| overlap(&a, &b))?;
        let taken: Vec<(i64, i64)> = self
            .agenda(lo, hi)
            .into_iter()
            .filter(|it| it.show_as != ShowAs::Free && it.status != "CANCELLED")
            .map(|it| (it.start, it.end.max(it.start + 1)))
            .collect();
        let taken = merged(taken);
        let free = work.iter().flat_map(|(a, b)| outside(&taken, *a, *b));
        if back {
            let free: Vec<(i64, i64)> = free.collect();
            free.iter().rev().find_map(|(a, b)| {
                let start = self.on_the_half_hour((b - length).min(anchor - 1), false);
                (start >= *a && start >= lo).then_some(start)
            })
        } else {
            free.into_iter().find_map(|(a, b)| {
                let start = self.on_the_half_hour(a.max(anchor), true);
                (start + length <= b).then_some(start)
            })
        }
    }
}
