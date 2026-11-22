//! An event moved to another time or day, as it's dragged in the calendar (issue #192): all of
//! it, as long as it was. One that repeats moves the occurrence dragged, on its own, as Outlook
//! does (issue #241): the series moved for one occurrence went wrong on its days (Monday's moved
//! to Tuesday, the rule still said Mondays).

use super::*;

/// Furthest an event is moved at once: ten years.
const MOVE_MOST: i64 = 10 * 366 * 86_400;

impl Session {
    /// Why the event `uid` can't be moved, if it can't: one that repeats, one in a calendar that
    /// can't be changed, one whose details are hidden, or a meeting someone else organized (its
    /// time is theirs to change).
    pub fn why_not_movable(&self, uid: &str) -> Option<&'static str> {
        self.why_not_movable_at(uid, None)
    }

    /// [`Session::why_not_movable`] for the occurrence `occurrence` of it on its own, when given:
    /// a repeating event's occurrences move one at a time.
    pub fn why_not_movable_at(&self, uid: &str, occurrence: Option<i64>) -> Option<&'static str> {
        let Some((account, stored)) = self.find_event(uid) else { return Some("no such event") };
        let e = &stored.event;
        if occurrence.is_none()
            && (e.rrule.is_some()
                || !e.rdates.is_empty()
                || self.pim.get(account).is_some_and(|p| p.events.iter().any(|x| x.event.uid == uid && x.event.recurrence_id.is_some())))
        {
            return Some("a repeating event is moved an occurrence at a time, or in its form");
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
        !it.task && !it.busy_only && self.why_not_movable_at(&it.uid, it.recurring.then_some(it.occurrence)).is_none()
    }

    /// Whether the user organizes the meeting `e`: one of their accounts is its organizer.
    pub fn organizes(&self, e: &CalEvent) -> bool {
        e.organizer.as_ref().is_some_and(|o| {
            self.accounts.iter().any(|a| a.config.email.eq_ignore_ascii_case(&o.email) || a.config.username.eq_ignore_ascii_case(&o.email))
        })
    }

    /// Move the event `uid` by `by` seconds, all of it: an all-day one by whole days. With
    /// `occurrence`, that occurrence of a repeating event alone (issue #241), from where it is now.
    pub fn move_event(&mut self, uid: &str, by: i64, occurrence: Option<i64>) -> CmdResult {
        if let Some(why) = self.why_not_movable_at(uid, occurrence) {
            return Err(CmdError::Params(why.into()));
        }
        if by.abs() > MOVE_MOST {
            return Err(CmdError::Params("an event is moved ten years at most".into()));
        }
        let all_day = self.find_event(uid).is_some_and(|(_, e)| e.event.all_day);
        if all_day && by % 86_400 != 0 {
            return Err(CmdError::Params("an all-day event is moved by whole days".into()));
        }
        let shift = |e: &mut CalEvent| {
            e.start = e.start.saturating_add(by);
            e.end = e.end.saturating_add(by);
        };
        match occurrence {
            Some(at) => self.update_occurrence(uid, at, shift),
            None => self.update_event(uid, shift),
        }
    }
}
