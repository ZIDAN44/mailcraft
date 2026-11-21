//! Forwarding a meeting the user is invited to (issue #234), as Outlook's Forward: the
//! organizer's invitation goes to someone else as an invitation (same meeting, same version,
//! the organizer's still), with them among its guests, so their program shows it with its
//! buttons. The user's own answer and copy stay as they are; the new guest's answer goes to the
//! organizer, whose copy offers to add them (issue #236). An organizer who doesn't want it
//! forwarded says so (`X-MICROSOFT-DONOTFORWARDMEETING`), and it isn't.

use super::*;

/// The most people a meeting is forwarded to at once.
const MAX_FORWARD: usize = 100;

/// Whether the organizer lets `e` be forwarded (Outlook's "Allow forwarding").
pub fn forwarding_allowed(e: &CalEvent) -> bool {
    !e.extra.iter().any(|l| {
        l.split_once(':')
            .is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("X-MICROSOFT-DONOTFORWARDMEETING") && v.trim().eq_ignore_ascii_case("TRUE"))
    })
}

impl Session {
    /// Forward the meeting `uid` (the invitation in the open message, or one in the calendar) to
    /// `to` (addresses, as typed in a recipient box).
    pub fn forward_meeting(&mut self, uid: &str, to: &str) -> CmdResult {
        let people = addr::parse_list(to).map_err(|bad| CmdError::Params(format!("`{bad}` isn't an address")))?;
        if people.is_empty() {
            return Err(CmdError::Params("say who to forward it to".into()));
        }
        if people.len() > MAX_FORWARD {
            return Err(CmdError::Params(format!("a meeting is forwarded to {MAX_FORWARD} people at most")));
        }
        let (account, master, _) = self.newest_meeting(uid).ok_or_else(|| CmdError::Params(format!("no meeting `{uid}`")))?;
        let mine = self.my_addresses();
        if master.organizer.as_ref().is_none_or(|o| is_mine(&o.email, &mine)) {
            return Err(CmdError::Disabled("one's own meeting takes more guests in its form, not forwarded".into()));
        }
        if !forwarding_allowed(&master) {
            return Err(CmdError::Disabled("the organizer doesn't let this meeting be forwarded".into()));
        }
        // The meeting as the organizer sent it: its changed occurrences with it, from the
        // calendar when it's there, else the invitation's.
        let mut events: Vec<CalEvent> = match self.stored_parts(uid) {
            Some((_, parts)) if parts.iter().any(|p| p.event.recurrence_id.is_none() && p.event.sequence >= master.sequence) => {
                parts.iter().map(|p| p.event.clone()).collect()
            }
            _ => vec![master.clone()],
        };
        let new: Vec<&EmailAddress> = people.iter().filter(|p| !is_mine(&p.email, &mine)).collect();
        for e in &mut events {
            for p in &new {
                if !e.attendees.iter().any(|a| same_address(&a.email, &p.email)) {
                    e.attendees.push(Attendee { email: p.email.clone(), name: p.name.clone(), partstat: "NEEDS-ACTION".into(), rsvp: true });
                }
            }
            // The user's reminders are their own.
            ical::set_reminder(e, None);
        }
        let config = self.account(&account).map(|a| a.config.clone()).unwrap_or_default();
        let own_invited = master.attendees.iter().any(|a| a.email.eq_ignore_ascii_case(&config.email));
        let invited =
            master.attendees.iter().filter(|_| !own_invited).find_map(|a| config.identity(&a.email)).map(|i| i.email.clone()).unwrap_or_default();
        let sender = config.sender(&invited);
        let who = if sender.name.is_empty() { sender.email.clone() } else { sender.name.clone() };
        let when = mailcraft_model::dates::full_time(master.start, &self.clock);
        let place = if master.location.is_empty() { String::new() } else { format!("\nWhere: {}", master.location) };
        let organizer = master.organizer.as_ref().map(|o| if o.name.is_empty() { o.email.clone() } else { o.name.clone() }).unwrap_or_default();
        let body = format!(
            "{who} forwarded you this meeting, organized by {organizer}.\n\n{}\nWhen: {when}{place}\n\n{}",
            master.summary, master.description
        );
        let subject = format!("Fwd: {}", master.summary);
        let to: Vec<EmailAddress> = new.into_iter().cloned().collect();
        let n = to.len();
        if n == 0 {
            return Err(CmdError::Params("say who to forward it to (not yourself)".into()));
        }
        self.send_itip(&account, Itip { from: &invited, to, method: "REQUEST", events: &events, subject: &subject, body: &body })?;
        Ok(serde_json::json!({"uid": uid, "forwarded": n}))
    }
}
