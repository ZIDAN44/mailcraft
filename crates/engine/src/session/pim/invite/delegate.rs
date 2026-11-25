//! Delegating a meeting the user is invited to (issue #254; iTIP's delegation, RFC 5546
//! §3.2.2.3), as SOGo's Delegate: someone else goes in the user's place. They're sent the
//! organizer's invitation (same meeting, same version) with them among its guests, handed the
//! place by the user (`DELEGATED-FROM`), asked to answer; the organizer is answered `DELEGATED`,
//! to whom (`DELEGATED-TO`), with the new guest. The user's copy stays in the calendar, marked so
//! and shown as free. The whole series, or one occurrence of it on its own (issue #241).
//!
//! The organizer's copy takes the new guest as it records the answer, and the new guest's own
//! answer then as any guest's ([`Session::record_answer`]). The new guest's invitation, sent by
//! a guest, can be answered though a server put the meeting in their calendar first
//! ([`Session::answer_problem`]).

use super::*;

impl Session {
    /// Why the invitation in a message can't be answered, if it can't: [`Session::invite_problem`],
    /// but one passed on by one of the meeting's guests (forwarded, issue #234; the user made
    /// their delegate, issue #254), no newer than the calendar has, can: answering it changes
    /// nothing but the user's answer in the calendar's copy (`respond_to_invite`). A server that
    /// schedules may have put the meeting there first: SOGo puts a delegate's copy in their
    /// calendar as the guest hands it on. Never anything more: what's in the calendar changes
    /// only from the organizer (taking an arrived invitation in asks `invite_problem` itself).
    pub(super) fn answer_problem(&self, cal: &ICalendar, sender: &str) -> Option<InviteProblem> {
        let problem = self.invite_problem(cal, sender);
        if problem != Some(InviteProblem::NotFromOrganizer) || cal.method.as_deref() != Some("REQUEST") {
            return problem;
        }
        let uid = cal.events.first().map(|e| e.uid.as_str())?;
        let (_, parts) = self.stored_parts(uid)?;
        let master = parts.iter().find(|e| e.event.recurrence_id.is_none()).or_else(|| parts.first())?;
        let passed_on = master.event.attendees.iter().any(|a| same_address(&a.email, sender))
            && cal
                .events
                .iter()
                .filter(|m| m.uid == uid)
                .all(|m| parts.iter().any(|s| s.event.recurrence_id == m.recurrence_id && m.sequence <= s.event.sequence));
        (!passed_on).then_some(InviteProblem::NotFromOrganizer)
    }

    /// For the card of an invitation (`method`, its `event`) to the user's account `account`
    /// (`current`: the calendar's copy, else the invitation's): whether the user can hand their
    /// place on, who they handed it to (their answer `DELEGATED`), and who handed theirs to them.
    pub(super) fn handed(&self, method: &str, event: &CalEvent, current: &CalEvent, account: &str) -> (bool, Option<Attendee>, Option<String>) {
        let mine = self.my_addresses();
        let me = current.attendees.iter().find(|a| is_mine(&a.email, &mine));
        let delegable = method == "REQUEST"
            && me.is_some()
            && event.organizer.as_ref().is_some_and(|o| !is_mine(&o.email, &mine))
            && self.delegates_from(account);
        let handed_to = me.filter(|a| a.partstat == "DELEGATED").map(|a| {
            current
                .attendees
                .iter()
                .find(|d| same_address(&d.email, &a.delegated_to))
                .cloned()
                .unwrap_or_else(|| Attendee { email: a.delegated_to.clone(), ..Default::default() })
        });
        let handed_by = me
            .map(|a| a.delegated_from.clone())
            .filter(|d| !d.is_empty())
            .map(|d| current.attendees.iter().find(|x| same_address(&x.email, &d)).map(|x| x.name.clone()).filter(|n| !n.is_empty()).unwrap_or(d));
        (delegable, handed_to, handed_by)
    }

    /// Whether the user can hand their place in a meeting of `account` on: the account mails the
    /// new guest their invitation itself. One with calendars only has no mail, and its server
    /// doesn't always send it (Nextcloud and Stalwart don't, and Nextcloud tells the organizer
    /// only that the place was handed on, not to whom); Microsoft Graph sends a meeting's mail
    /// itself.
    pub fn delegates_from(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| !a.config.is_calendar_only() && a.config.graph.is_none())
    }

    /// Delegate the meeting `uid` (the invitation in the open message, or one in the calendar) to
    /// `to` (one address, as typed in a recipient box): all of it, or its occurrence `at` alone
    /// (when it repeats; it must be in the calendar).
    pub fn delegate_meeting(&mut self, uid: &str, to: &str, at: Option<i64>) -> CmdResult {
        let people = addr::parse_list(to).map_err(|bad| CmdError::Params(format!("`{bad}` isn't an address")))?;
        let [person] = people.as_slice() else {
            return Err(CmdError::Params("say the one person to go in your place".into()));
        };
        let (account, master, open) = self.newest_meeting(uid).ok_or_else(|| CmdError::Params(format!("no meeting `{uid}`")))?;
        if !self.delegates_from(&account) {
            return Err(CmdError::Disabled("this account can't send the invitation to the one who goes in your place".into()));
        }
        let mine = self.my_addresses();
        let Some(organizer) = master.organizer.clone().filter(|o| !is_mine(&o.email, &mine)) else {
            return Err(CmdError::Disabled("one's own meeting takes more guests in its form, not delegated".into()));
        };
        let invited = master
            .attendees
            .iter()
            .find(|a| is_mine(&a.email, &mine))
            .map(|a| a.email.clone())
            .ok_or_else(|| CmdError::Disabled("you aren't among this meeting's guests: forward it instead".into()))?;
        if is_mine(&person.email, &mine) || same_address(&person.email, &organizer.email) {
            return Err(CmdError::Params("delegate it to someone else".into()));
        }
        // Invited already, but not as the user's own delegate (delegating to them again finishes
        // what a failed send left undone).
        if master.attendees.iter().any(|a| same_address(&a.email, &person.email) && !is_mine(&a.delegated_from, &mine)) {
            return Err(CmdError::Params(format!("{} is invited already", person.email)));
        }
        let delegate = Attendee {
            email: person.email.clone(),
            name: person.name.clone(),
            partstat: "NEEDS-ACTION".into(),
            rsvp: true,
            delegated_from: invited.clone(),
            ..Default::default()
        };
        let hand_over = |e: &mut CalEvent| {
            // Handed on again: the one before isn't the user's delegate any more.
            e.attendees.retain(|a| !(is_mine(&a.delegated_from, &mine) && !same_address(&a.email, &delegate.email)));
            for a in e.attendees.iter_mut().filter(|a| is_mine(&a.email, &mine)) {
                a.partstat = "DELEGATED".into();
                a.rsvp = false;
                a.delegated_to = delegate.email.clone();
            }
            if !e.attendees.iter().any(|a| same_address(&a.email, &delegate.email)) {
                e.attendees.push(delegate.clone());
            }
            ical::show_as_answered(e, "DELEGATED");
        };
        let repeats = master.rrule.is_some() || !master.rdates.is_empty();
        let stored = self.stored_parts(uid).map(|(a, parts)| (a.to_string(), parts.into_iter().cloned().collect::<Vec<StoredEvent>>()));
        // The parts handed over, and the copy as it was (to answer the organizer with).
        let (calendar, copy, handed): (String, Vec<StoredEvent>, Vec<CalEvent>) = match (at.filter(|_| repeats), stored) {
            // One occurrence: its own part, made from the series when it has none yet.
            (Some(at), Some((account, copy))) => {
                let calendar = calendar_of(&copy);
                let p = self.pim_mut(&account);
                if p.theirs().contains(&calendar) || p.calendars.iter().any(|c| c.id == calendar && c.read_only) {
                    return Err(CmdError::Params("this calendar can't be changed".into()));
                }
                let (i, _) = super::super::series::occurrence_part(p, &calendar, uid, at)
                    .ok_or_else(|| CmdError::Params(format!("the series has no occurrence at {at}")))?;
                let Some(part) = p.events.get_mut(i) else { return Err(CmdError::Params(format!("no event `{uid}`"))) };
                hand_over(&mut part.event);
                part.dirty |= calendar != LOCAL;
                let handed = vec![part.event.clone()];
                (calendar, copy, handed)
            }
            (Some(_), None) => return Err(CmdError::Disabled("put the meeting in your calendar first".into())),
            // All of it, in the calendar: each part there, when it's as new as the meeting (or
            // when the calendar has some occurrences of it alone, invited to those).
            (None, Some((account_there, copy)))
                if copy.iter().any(|p| p.event.recurrence_id.is_none() && p.event.sequence >= master.sequence)
                    || !copy.iter().any(|p| p.event.recurrence_id.is_none()) =>
            {
                let calendar = calendar_of(&copy);
                let p = self.pim_mut(&account_there);
                let theirs = p.theirs();
                let mut handed = Vec::new();
                for e in p.events.iter_mut().filter(|e| e.event.uid == uid && !theirs.contains(&e.calendar)) {
                    hand_over(&mut e.event);
                    e.dirty |= e.calendar != LOCAL;
                    handed.push(e.event.clone());
                }
                (calendar, copy, handed)
            }
            // All of it, from the invitation: into the calendar as delegated, replacing an older
            // copy, as an answer puts it there.
            (None, stored) => {
                let copy = stored.map(|(_, c)| c).unwrap_or_default();
                let mut e = master.clone();
                hand_over(&mut e);
                let p = self.pim_mut(&account);
                let calendar = copy
                    .iter()
                    .find(|e| e.event.recurrence_id.is_none())
                    .map(|e| e.calendar.clone())
                    .or_else(|| p.default_calendar().map(|c| c.id.clone()))
                    .unwrap_or_else(|| LOCAL.into());
                let old = copy.iter().find(|x| x.event.recurrence_id.is_none());
                p.events.retain(|x| !(x.event.uid == uid && x.event.recurrence_id.is_none()));
                p.events.push(StoredEvent {
                    calendar: calendar.clone(),
                    href: old.map(|o| o.href.clone()).unwrap_or_default(),
                    etag: old.map(|o| o.etag.clone()).unwrap_or_default(),
                    event: e.clone(),
                    dirty: calendar != LOCAL,
                });
                // The older version's changed occurrences handed on too (the invitation brings
                // the series alone).
                let theirs = p.theirs();
                for x in p.events.iter_mut().filter(|x| x.event.uid == uid && x.event.recurrence_id.is_some() && !theirs.contains(&x.calendar)) {
                    hand_over(&mut x.event);
                    x.dirty |= x.calendar != LOCAL;
                }
                if copy.is_empty() {
                    self.placed_here(&account, &calendar, uid);
                }
                self.forget_held(&account, uid, std::slice::from_ref(&e));
                (calendar, copy, vec![e])
            }
        };
        let Some(first) = handed.iter().find(|e| e.recurrence_id.is_none()).or_else(|| handed.first()).cloned() else {
            return Err(CmdError::Params(format!("no meeting `{uid}`")));
        };
        self.after_pim_change(&account, calendar != LOCAL);
        // The new guest gets the organizer's invitation, from the address that was invited (issue
        // #81), as a forward does; the user's reminders stay theirs.
        let mut sent = handed.clone();
        for e in &mut sent {
            ical::set_reminder(e, None);
        }
        let config = self.account(&account).map(|a| a.config.clone()).unwrap_or_default();
        let from = config.identity(&invited).map(|i| i.email.clone()).unwrap_or_default();
        let sender = config.sender(&from);
        let who = if sender.name.is_empty() { sender.email.clone() } else { sender.name.clone() };
        // In the window's language (issue #235).
        let w = &self.mail_words;
        let lines = w.when_where(&w.time(first.start, &self.clock), &first.location);
        let by = if organizer.name.is_empty() { organizer.email.clone() } else { organizer.name.clone() };
        let said = MailWords::fill(&w.delegated_body, &[("who", &who), ("organizer", &by)]);
        let body = format!("{said}\n\n{}\n{lines}\n\n{}", first.summary, first.description);
        let subject = MailWords::fill(&w.invitation, &[("summary", &first.summary)]);
        let to = vec![EmailAddress::new(person.name.clone(), person.email.clone())];
        self.send_itip(&account, Itip { from: &from, to, method: "REQUEST", events: &sent, subject: &subject, body: &body })?;
        // The organizer told, as an answer is (now, or once a server that schedules has it).
        self.answer_organizer(&account, &calendar, &copy, &first, "DELEGATED")?;
        let only: Option<Vec<i64>> = at.filter(|_| repeats).map(|t| vec![t]);
        // The invitation, and its earlier versions in the Inbox, out of the way (issue #31).
        let key = self.reading.as_ref().filter(|_| open).map(|r| r.key.clone());
        let moved = self.tidy_when_stored(&account, uid, only.as_deref(), &handed, key.as_ref(), &[]);
        Ok(serde_json::json!({"uid": uid, "delegated": person.email, "occurrence": at.filter(|_| repeats), "moved": moved}))
    }
}

/// The calendar a meeting's copy is in: its series', or the first part's when the calendar has
/// some occurrences of it alone.
fn calendar_of(copy: &[StoredEvent]) -> String {
    copy.iter().find(|e| e.event.recurrence_id.is_none()).or_else(|| copy.first()).map(|e| e.calendar.clone()).unwrap_or_default()
}
