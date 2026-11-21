//! Proposing a new time for a meeting (issue #238, iTIP's `COUNTER` and `DECLINECOUNTER`,
//! RFC 5546 §3.2.7 and §3.2.8), as Outlook's Propose New Time: a guest sends the organizer the
//! time they'd rather have; the organizer takes it (the meeting moves, and every guest is told)
//! or turns it down (the guest is told). An organizer who doesn't take proposals says so
//! (`X-MICROSOFT-DISALLOW-COUNTER`), and none is sent them.

use super::*;

/// Whether the organizer takes proposals for `e` (Outlook's "Allow new time proposals").
pub fn proposals_allowed(e: &CalEvent) -> bool {
    !e.extra.iter().any(|l| {
        l.split_once(':').is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("X-MICROSOFT-DISALLOW-COUNTER") && v.trim().eq_ignore_ascii_case("TRUE"))
    })
}

impl Session {
    /// Propose `proposal`'s time for the meeting the user is invited to (its uid: the invitation
    /// in the open message, or `uid`'s in the calendar), and, with `answer`, answer it so too
    /// (Outlook proposes with Tentative or Decline).
    pub fn propose_time(&mut self, uid: &str, proposal: &CalEvent, answer: Option<&str>) -> CmdResult {
        let stat = answer.map(|a| partstat(a).ok_or_else(|| CmdError::Params(format!("`{a}`: answer tentative or declined")))).transpose()?;
        let (account, meeting, open) = self.newest_meeting(uid).ok_or_else(|| CmdError::Params(format!("no meeting `{uid}`")))?;
        let mine = self.my_addresses();
        let Some(organizer) = meeting.organizer.clone().filter(|o| addr::is_valid_email(&o.email) && !is_mine(&o.email, &mine)) else {
            return Err(CmdError::Disabled("only a meeting someone else organizes takes a proposal".into()));
        };
        if !proposals_allowed(&meeting) {
            return Err(CmdError::Disabled("the organizer doesn't take proposals for a new time".into()));
        }
        if proposal.end < proposal.start || (proposal.start == meeting.start && proposal.end == meeting.end) {
            return Err(CmdError::Params("propose a time other than the meeting's".into()));
        }
        // Answered first, when asked: the proposal says the answer too (the open invitation's,
        // when it's the version proposed for).
        if let Some(a) = answer {
            if open {
                self.respond_to_invite(a)?;
            } else {
                self.respond_to_event(uid, a)?;
            }
        }
        let config = self.account(&account).map(|a| a.config.clone()).unwrap_or_default();
        // From the address that was invited (issue #81), as an answer is.
        let own_invited = meeting.attendees.iter().any(|a| a.email.eq_ignore_ascii_case(&config.email));
        let invited =
            meeting.attendees.iter().filter(|_| !own_invited).find_map(|a| config.identity(&a.email)).map(|i| i.email.clone()).unwrap_or_default();
        let sender = config.sender(&invited);
        let mut counter = meeting.clone();
        counter.start = proposal.start;
        counter.end = proposal.end;
        counter.all_day = proposal.all_day;
        let current = meeting.attendees.iter().find(|a| is_mine(&a.email, &mine)).map(|a| a.partstat.clone());
        let stat = stat.map(str::to_string).or(current).unwrap_or_else(|| "NEEDS-ACTION".into());
        counter.attendees = vec![Attendee { email: sender.email.clone(), name: sender.name.clone(), partstat: stat, rsvp: false }];
        counter.description.clear();
        ical::set_reminder(&mut counter, None);
        let who = if sender.name.is_empty() { sender.email.clone() } else { sender.name.clone() };
        let when = mailcraft_model::dates::full_time(proposal.start, &self.clock);
        let subject = format!("New time proposed: {}", meeting.summary);
        let body = format!("{who} proposes a new time for {}: {when}.", meeting.summary);
        let to = vec![EmailAddress::new(organizer.name.clone(), organizer.email.clone())];
        self.send_itip(&account, Itip { from: &invited, to, method: "COUNTER", events: &[counter], subject: &subject, body: &body })?;
        Ok(serde_json::json!({"proposed": proposal.start, "uid": uid}))
    }

    /// The meeting `uid` as a proposal or a forward is made of: its newest version, the calendar's copy or
    /// the invitation in the open message when that's newer (an update not answered yet: a
    /// proposal for the version before was out of date to the organizer). The account, the
    /// meeting, and whether it's the open invitation's.
    pub fn newest_meeting(&self, uid: &str) -> Option<(String, CalEvent, bool)> {
        let stored = self.stored_parts(uid).and_then(|(a, parts)| {
            let master = parts.iter().find(|e| e.event.recurrence_id.is_none()).or_else(|| parts.first())?;
            Some((a.to_string(), master.event.clone()))
        });
        let open = self.invite().filter(|i| i.method == "REQUEST" && i.event.uid == uid && i.problem.is_none()).map(|i| i.event);
        let account = || self.reading.as_ref().map(|r| r.key.account.clone()).unwrap_or_default();
        match (stored, open) {
            (Some((a, s)), Some(o)) if o.sequence > s.sequence => Some((a, o, true)),
            (Some((a, s)), _) => Some((a, s, false)),
            (None, Some(o)) => Some((account(), o, true)),
            (None, None) => None,
        }
    }

    /// Take the proposal in the open message: the meeting moves to its time, and every guest is
    /// sent the meeting as it is now.
    pub fn accept_proposal(&mut self) -> CmdResult {
        let (account, uid, proposal, _) = self.open_proposal()?;
        let before = self.find_event(&uid).map(|(_, e)| e.event.sequence).unwrap_or_default();
        self.update_event(&uid, |e| {
            e.start = proposal.start;
            e.end = proposal.end;
            e.all_day = proposal.all_day;
        })?;
        if let Some(stored) = self.find_event(&uid).map(|(_, e)| e.event.clone()).filter(|e| e.sequence != before) {
            let route = self.meeting_route(&account, &uid);
            self.tell_guests(&account, &stored, "REQUEST", route, "")?;
        }
        Ok(serde_json::json!({"uid": uid, "start": proposal.start, "end": proposal.end}))
    }

    /// Turn down the proposal in the open message: its guest is told (`DECLINECOUNTER`), and the
    /// meeting stays as it is.
    pub fn decline_proposal(&mut self) -> CmdResult {
        let (account, uid, _, guest) = self.open_proposal()?;
        let Some(meeting) = self.find_event(&uid).map(|(_, e)| e.event.clone()) else {
            return Err(CmdError::Params(format!("no meeting `{uid}`")));
        };
        let config = self.account(&account).map(|a| a.config.clone()).unwrap_or_default();
        let from = meeting.organizer.as_ref().and_then(|o| config.identity(&o.email)).map(|i| i.email.clone()).unwrap_or_default();
        let mut declined = meeting.clone();
        declined.attendees = vec![guest.clone()];
        declined.description.clear();
        ical::set_reminder(&mut declined, None);
        let subject = format!("New time declined: {}", meeting.summary);
        let body = format!("The proposed new time for {} was declined. The meeting stays as it was.", meeting.summary);
        let to = vec![EmailAddress::new(guest.name.clone(), guest.email.clone())];
        self.send_itip(&account, Itip { from: &from, to, method: "DECLINECOUNTER", events: &[declined], subject: &subject, body: &body })?;
        Ok(serde_json::json!({"uid": uid, "declined": guest.email}))
    }

    /// The proposal in the open message, when it can be taken or turned down: the account, the
    /// meeting, the time proposed and the guest who proposed it.
    fn open_proposal(&self) -> Result<(String, String, CalEvent, Attendee), CmdError> {
        let inv = self.invite().filter(|i| i.method == "COUNTER").ok_or_else(|| CmdError::Disabled("this message proposes no new time".into()))?;
        if let Some(p) = inv.problem {
            return Err(CmdError::Disabled(p.reason().into()));
        }
        let guest = inv.replier.clone().ok_or_else(|| CmdError::Disabled(InviteProblem::NotFromGuest.reason().into()))?;
        // One occurrence of a series: taking it would move the whole series.
        if inv.event.recurrence_id.is_some() {
            return Err(CmdError::Disabled("a new time for one occurrence is set by changing that occurrence in the calendar".into()));
        }
        let account = self
            .stored_parts(&inv.event.uid)
            .map(|(a, _)| a.to_string())
            .ok_or_else(|| CmdError::Disabled("the meeting isn't in your calendar".into()))?;
        Ok((account, inv.event.uid.clone(), inv.event, guest))
    }
}
