//! An event moved to another time or day, as it's dragged in the calendar (issue #192): all of
//! it, as long as it was. One that repeats isn't: nothing changes one occurrence on its own yet,
//! and the series moved for one occurrence went wrong on its days (Monday's moved to Tuesday,
//! the rule still said Mondays).

use super::*;

/// Furthest an event is moved at once: ten years.
const MOVE_MOST: i64 = 10 * 366 * 86_400;

impl Session {
    /// Why the event `uid` can't be moved, if it can't: one that repeats, one in a calendar that
    /// can't be changed, one whose details are hidden, or a meeting someone else organized (its
    /// time is theirs to change).
    pub fn why_not_movable(&self, uid: &str) -> Option<&'static str> {
        let Some((account, stored)) = self.find_event(uid) else { return Some("no such event") };
        let e = &stored.event;
        if e.rrule.is_some()
            || !e.rdates.is_empty()
            || self.pim.get(account).is_some_and(|p| p.events.iter().any(|x| x.event.uid == uid && x.event.recurrence_id.is_some()))
        {
            return Some("a repeating event is moved in its form");
        }
        if self.pim.get(account).is_some_and(|p| p.calendars.iter().any(|c| c.id == stored.calendar && c.read_only)) {
            return Some("this calendar can't be changed");
        }
        if ical::details_hidden(e) {
            return Some("only this event's time can be seen: it can't be changed");
        }
        if !e.attendees.is_empty() && !self.organizes(e) {
            return Some("only its organizer moves a meeting");
        }
        None
    }

    /// Whether a calendar item can be dragged to another time: an event (not a task, nor a
    /// colleague's time taken) that [`Session::why_not_movable`] finds nothing against.
    pub fn movable(&self, it: &AgendaItem) -> bool {
        !it.task && !it.busy_only && !it.recurring && self.why_not_movable(&it.uid).is_none()
    }

    /// Whether the user organizes the meeting `e`: one of their accounts is its organizer.
    pub fn organizes(&self, e: &CalEvent) -> bool {
        e.organizer.as_ref().is_some_and(|o| {
            self.accounts.iter().any(|a| a.config.email.eq_ignore_ascii_case(&o.email) || a.config.username.eq_ignore_ascii_case(&o.email))
        })
    }

    /// Move the event `uid` by `by` seconds, all of it: an all-day one by whole days.
    pub fn move_event(&mut self, uid: &str, by: i64) -> CmdResult {
        if let Some(why) = self.why_not_movable(uid) {
            return Err(CmdError::Params(why.into()));
        }
        if by.abs() > MOVE_MOST {
            return Err(CmdError::Params("an event is moved ten years at most".into()));
        }
        let all_day = self.find_event(uid).is_some_and(|(_, e)| e.event.all_day);
        if all_day && by % 86_400 != 0 {
            return Err(CmdError::Params("an all-day event is moved by whole days".into()));
        }
        self.update_event(uid, |e| {
            e.start = e.start.saturating_add(by);
            e.end = e.end.saturating_add(by);
        })
    }
}
