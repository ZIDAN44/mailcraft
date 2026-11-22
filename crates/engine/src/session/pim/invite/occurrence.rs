//! One occurrence of a meeting answered from the calendar (issue #241): as Outlook's "this
//! occurrence", the user accepts a series and declines one date, without needing that date's
//! update message (issue #101 answers one from it). The answer is kept in that occurrence, a
//! changed occurrence of the series, and the organizer is told of it alone; the others keep theirs.

use super::*;

impl Session {
    /// Answer the occurrence `at` of the meeting `uid` (when the series starts it) on its own.
    /// One that doesn't repeat is answered whole.
    pub fn respond_to_occurrence(&mut self, uid: &str, at: i64, answer: &str) -> CmdResult {
        let stat = partstat(answer).ok_or_else(|| CmdError::Params(format!("`{answer}`: answer accepted, tentative or declined")))?;
        let (account, calendar, repeats) = self
            .find_event(uid)
            .map(|(a, e)| (a.to_string(), e.calendar.clone(), e.event.rrule.is_some() || !e.event.rdates.is_empty()))
            .ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        if !repeats {
            return self.respond_to_event(uid, answer);
        }
        let me = self.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
        let mine = self.my_addresses();
        let p = self.pim_mut(&account);
        // In the user's own calendar only (see `respond_to_event`).
        if p.theirs().contains(&calendar) || p.calendars.iter().any(|c| c.id == calendar && c.read_only) {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        let (i, _) =
            super::series::occurrence_part(p, &calendar, uid, at).ok_or_else(|| CmdError::Params(format!("the series has no occurrence at {at}")))?;
        let Some(part) = p.events.get_mut(i) else { return Err(CmdError::Params(format!("no event `{uid}`"))) };
        set_answer(&mut part.event, &mine, &me, stat);
        let remote = calendar != LOCAL;
        part.dirty |= remote;
        let event = part.event.clone();
        let copy: Vec<StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid && e.calendar == calendar).cloned().collect();
        self.after_pim_change(&account, remote);
        self.answer_organizer(&account, &calendar, &copy, &event, stat)?;
        // Its invitations naming that occurrence alone out of the Inbox (issue #31), unless it's
        // the user's own meeting.
        let theirs = event.organizer.as_ref().is_some_and(|o| !is_mine(&o.email, &mine));
        let moved = if theirs { self.tidy_when_stored(&account, uid, Some(&[at]), std::slice::from_ref(&event), None, &[]) } else { 0 };
        Ok(serde_json::json!({"answer": stat, "uid": uid, "occurrence": at, "moved": moved}))
    }
}
