//! Guests' answers to the user's meetings, recorded in the calendar: by Record the answer, or as
//! they arrive (issue #236), as Outlook, Exchange and SOGo record them, the organizer looking only
//! when they want to. One arriving is recorded when it's genuine (from the guest it names), from
//! a guest on the meeting's list, and for the meeting as it is now; any other (someone not on the
//! list, an answer to an earlier version) waits for Record the answer.

use super::*;

impl Session {
    /// The answer of `who` recorded in the user's meeting `uid` in `account`, on the occurrence
    /// it answers (`recurrence_id`, none: the series): their place among the guests says it, or
    /// they're added. Whether anything changed.
    pub(super) fn record_answer(&mut self, account: &str, uid: &str, recurrence_id: Option<i64>, who: &Attendee) -> bool {
        let p = self.pim_mut(account);
        let (mut changed, mut remote) = (false, false);
        for e in p.events.iter_mut().filter(|e| e.event.uid == uid && e.event.recurrence_id == recurrence_id) {
            match e.event.attendees.iter_mut().find(|a| a.email.eq_ignore_ascii_case(&who.email)) {
                Some(a) if a.partstat == who.partstat => continue,
                Some(a) => a.partstat = who.partstat.clone(),
                None => e.event.attendees.push(Attendee { rsvp: false, ..who.clone() }),
            }
            changed = true;
            remote |= e.calendar != LOCAL;
            e.dirty |= e.calendar != LOCAL;
        }
        if changed {
            self.after_pim_change(account, remote);
        }
        changed
    }

    /// An arrived answer (`cal`, a `REPLY` from `sender`, in the message `key`) recorded in the
    /// user's meeting, when the option is on and it's one to record without asking (see the
    /// module); its message then tidied as answered mail is. Whether it was taken.
    pub(super) fn take_answer(&mut self, key: &MessageKey, cal: &ICalendar, sender: &str) -> bool {
        if !self.prefs.record_answers || self.invite_problem(cal, sender).is_some() {
            return false;
        }
        let Some(answer) = cal.events.iter().find(|e| e.recurrence_id.is_none()).or_else(|| cal.events.first()) else { return false };
        let Some(who) = answer.attendees.iter().find(|a| same_address(&a.email, sender)).cloned() else { return false };
        let mine = self.my_addresses();
        let Some((account, meeting)) = self.stored_parts(&answer.uid).and_then(|(a, parts)| {
            let part = parts.iter().find(|e| e.event.recurrence_id == answer.recurrence_id)?;
            Some((a.to_string(), part.event.clone()))
        }) else {
            return false;
        };
        let mine_to_organize = meeting.organizer.as_ref().is_some_and(|o| is_mine(&o.email, &mine));
        let on_the_list = meeting.attendees.iter().any(|a| same_address(&a.email, &who.email));
        if !mine_to_organize || !on_the_list || answer.sequence < meeting.sequence {
            return false;
        }
        self.record_answer(&account, &answer.uid, answer.recurrence_id, &who);
        self.tidy_reply_mail(key);
        true
    }
}
