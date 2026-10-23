//! Meeting invitations by mail (iTIP, RFC 5546, over iMIP, RFC 6047): answering one, taking
//! in the answers and cancellations organizers and guests send, and inviting people to the
//! user's own events. Invitations travel as `text/calendar` parts with a `METHOD`.
//!
//! Mail can come from anyone, so a change or cancellation counts only from the event's
//! organizer, an answer only from the guest it speaks for, and an invitation older than the copy
//! in the calendar (a lower `SEQUENCE`, or the same one stamped earlier) changes nothing. One that
//! names occurrences (`RECURRENCE-ID`) without the series changes those occurrences only.

use mailcraft_model::ical::{self, Attendee, CalEvent, ICalendar};
use mailcraft_model::pim::{LOCAL, Pim, StoredEvent, Tombstone};

use super::*;

/// The most occurrences one message can take out of a series.
const MAX_EXDATES: usize = 1000;

/// An invitation (or an answer to one, or a cancellation) in the open message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    /// `REQUEST`, `REPLY` or `CANCEL`.
    pub method: String,
    pub event: CalEvent,
    /// The user's answer so far (`NEEDS-ACTION`, `ACCEPTED`…), from their calendar when it's there.
    pub answer: String,
    /// The event is in one of the user's calendars.
    pub in_calendar: bool,
    /// For a reply: who answered, and how.
    pub replier: Option<Attendee>,
    /// Why it can't be taken into the calendar, when it can't.
    pub problem: Option<InviteProblem>,
}

/// Why an invitation can't be taken into the calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteProblem {
    /// A change or cancellation sent by someone other than the event's organizer.
    NotFromOrganizer,
    /// An answer sent by someone other than the guest it speaks for.
    NotFromGuest,
    /// Older than the copy in the calendar.
    OutOfDate,
}

impl InviteProblem {
    /// What to tell the user (the interface translates it).
    pub fn reason(self) -> &'static str {
        match self {
            Self::NotFromOrganizer => "This update wasn't sent by the meeting's organizer, so it can't change your calendar.",
            Self::NotFromGuest => "This answer wasn't sent by the guest it's from, so it can't be recorded.",
            Self::OutOfDate => "This invitation is out of date: your calendar has a newer version.",
        }
    }
}

/// The answers a guest can give.
pub fn partstat(answer: &str) -> Option<&'static str> {
    match answer.trim().to_ascii_lowercase().as_str() {
        "accepted" | "accept" => Some("ACCEPTED"),
        "tentative" => Some("TENTATIVE"),
        "declined" | "decline" => Some("DECLINED"),
        _ => None,
    }
}

/// Two addresses are the same person's (`mailto:` and case aside).
fn same_address(one: &str, other: &str) -> bool {
    let one = without_mailto(one);
    if one.is_empty() {
        return false;
    }
    one.eq_ignore_ascii_case(without_mailto(other))
}

/// An attendee or organizer value as a plain address: trimmed, any `mailto:` (in any case) taken
/// off the front.
fn without_mailto(value: &str) -> &str {
    const SCHEME: &str = "mailto:";
    let value = value.trim();
    let has_scheme = value.get(..SCHEME.len()).is_some_and(|start| start.eq_ignore_ascii_case(SCHEME));
    if has_scheme { value.get(SCHEME.len()..).unwrap_or_default().trim() } else { value }
}

/// The message's version is older than the stored one.
fn older(msg: &CalEvent, stored: &CalEvent) -> bool {
    msg.sequence < stored.sequence || (msg.sequence == stored.sequence && msg.dtstamp > 0 && stored.dtstamp > 0 && msg.dtstamp < stored.dtstamp)
}

impl Session {
    /// The user's addresses (lower case).
    fn my_addresses(&self) -> Vec<String> {
        self.accounts
            .iter()
            .flat_map(|a| [a.config.email.trim().to_ascii_lowercase(), a.config.username.trim().to_ascii_lowercase()])
            .filter(|e| e.contains('@'))
            .collect()
    }

    /// The calendar in a message (its `text/calendar` part or `.ics` file), if it has one.
    pub fn message_calendar(&self, key: &MessageKey) -> Option<ICalendar> {
        self.message_invitation(key).map(|(cal, _)| cal)
    }

    /// The calendar in a message, and the address it's from.
    fn message_invitation(&self, key: &MessageKey) -> Option<(ICalendar, String)> {
        let raw = self.store.as_ref()?.read_body(key)?;
        let parsed = mime::parse(&raw)?;
        let att = parsed
            .attachments
            .iter()
            .find(|a| a.mime.to_ascii_lowercase().starts_with("text/calendar") || a.name.to_ascii_lowercase().ends_with(".ics"))?;
        let data = mime::attachment_data(&raw, att.index)?;
        let cal = ical::parse_in(&String::from_utf8_lossy(&data), self.clock.local_zone());
        (!cal.events.is_empty()).then_some((cal, parsed.from.email))
    }

    /// The account holding an event (the series or any occurrence of it), and its parts there.
    fn stored_parts(&self, uid: &str) -> Option<(&str, Vec<&StoredEvent>)> {
        self.pim.iter().find_map(|(a, p)| {
            let parts: Vec<&StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid).collect();
            (!parts.is_empty()).then_some((a.as_str(), parts))
        })
    }

    /// Why the calendar in a message (from `sender`) can't be taken in, if it can't.
    fn invite_problem(&self, cal: &ICalendar, sender: &str) -> Option<InviteProblem> {
        let uid = cal.events.first().map(|e| e.uid.as_str())?;
        let msg: Vec<&CalEvent> = cal.events.iter().filter(|e| e.uid == uid).collect();
        match cal.method.as_deref() {
            Some("REPLY") => (!msg.iter().any(|e| e.attendees.iter().any(|a| same_address(&a.email, sender)))).then_some(InviteProblem::NotFromGuest),
            Some("REQUEST" | "CANCEL") => {
                // A new invitation can come from anyone; what's in the calendar changes only
                // from its organizer, and only for something newer.
                let (_, parts) = self.stored_parts(uid)?;
                let master = parts.iter().find(|e| e.event.recurrence_id.is_none()).or_else(|| parts.first())?;
                if !master.event.organizer.as_ref().is_some_and(|o| same_address(&o.email, sender)) {
                    return Some(InviteProblem::NotFromOrganizer);
                }
                let stale = msg.iter().any(|m| parts.iter().any(|s| s.event.recurrence_id == m.recurrence_id && older(m, &s.event)));
                stale.then_some(InviteProblem::OutOfDate)
            }
            _ => None,
        }
    }

    /// The invitation in the open message.
    pub fn invite(&self) -> Option<Invite> {
        let r = self.reading.as_ref()?;
        let (cal, sender) = self.message_invitation(&r.key)?;
        let method = cal.method.clone().unwrap_or_else(|| "PUBLISH".into());
        if !matches!(method.as_str(), "REQUEST" | "REPLY" | "CANCEL") {
            return None;
        }
        let event = cal.events.iter().find(|e| e.recurrence_id.is_none()).or_else(|| cal.events.first())?.clone();
        let mine = self.my_addresses();
        let parts = self.stored_parts(&event.uid).map(|(_, p)| p).unwrap_or_default();
        let stored = parts
            .iter()
            .find(|e| e.event.recurrence_id == event.recurrence_id)
            .or_else(|| parts.iter().find(|e| e.event.recurrence_id.is_none()))
            .map(|e| e.event.clone());
        let answer = stored
            .as_ref()
            .unwrap_or(&event)
            .attendees
            .iter()
            .find(|a| mine.contains(&a.email.to_ascii_lowercase()))
            .map(|a| a.partstat.clone())
            .unwrap_or_else(|| "NEEDS-ACTION".into());
        let replier = (method == "REPLY")
            .then(|| event.attendees.iter().find(|a| same_address(&a.email, &sender)).or_else(|| event.attendees.first()).cloned())
            .flatten();
        let problem = self.invite_problem(&cal, &sender);
        Some(Invite { method, event, answer, in_calendar: !parts.is_empty(), replier, problem })
    }

    /// Answer the invitation in the open message: the event goes into the calendar (or out of
    /// it, when declined) and the organizer gets the answer. An invitation to some occurrences
    /// of a series changes those occurrences only.
    pub fn respond_to_invite(&mut self, answer: &str) -> CmdResult {
        let stat = partstat(answer).ok_or_else(|| CmdError::Params(format!("`{answer}`: answer accepted, tentative or declined")))?;
        let key = self.reading.as_ref().map(|r| r.key.clone()).ok_or_else(|| CmdError::Disabled("open the invitation first".into()))?;
        let (cal, sender) = self.message_invitation(&key).ok_or_else(|| CmdError::Disabled("this message has no invitation".into()))?;
        if cal.method.as_deref() != Some("REQUEST") {
            return Err(CmdError::Disabled("this message isn't an invitation".into()));
        }
        if let Some(p) = self.invite_problem(&cal, &sender) {
            return Err(CmdError::Disabled(p.reason().into()));
        }
        let account = key.account.clone();
        let me = self.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
        let uid = cal.events.first().map(|e| e.uid.clone()).unwrap_or_default();
        let mut events: Vec<CalEvent> = cal.events.into_iter().filter(|e| e.uid == uid).collect();
        for e in &mut events {
            set_answer(e, &self.my_addresses(), &me, stat);
        }
        let whole = events.iter().any(|e| e.recurrence_id.is_none());
        let p = self.pim_mut(&account);
        let existing: Vec<StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid).cloned().collect();
        let calendar =
            existing.first().map(|e| e.calendar.clone()).or_else(|| p.default_calendar().map(|c| c.id.clone())).unwrap_or_else(|| LOCAL.into());
        let mut remote = false;
        if whole {
            // The series: into the calendar, replacing an earlier copy (an updated
            // invitation); out if declined.
            p.events.retain(|e| e.event.uid != uid);
            if stat == "DECLINED" {
                remote |= delete_items(p, &existing);
            } else {
                for e in &events {
                    let old = existing.iter().find(|x| x.event.recurrence_id == e.recurrence_id);
                    p.events.push(StoredEvent {
                        calendar: calendar.clone(),
                        href: old.map(|o| o.href.clone()).unwrap_or_default(),
                        etag: old.map(|o| o.etag.clone()).unwrap_or_default(),
                        event: e.clone(),
                        dirty: calendar != LOCAL,
                    });
                    remote |= calendar != LOCAL;
                }
            }
        } else {
            // Some occurrences: those change, the series and the others stay.
            for e in &events {
                remote |= match e.recurrence_id {
                    Some(at) if stat == "DECLINED" => drop_occurrence(p, &uid, at),
                    _ => put_occurrence(p, &calendar, e.clone()),
                };
            }
        }
        self.after_pim_change(&account, remote);
        if let Some(first) = events.first().cloned() {
            self.send_reply(&account, &first, stat)?;
        }
        Ok(serde_json::json!({"answer": stat, "uid": uid}))
    }

    /// Answer an invitation already in the calendar (from the event form).
    pub fn respond_to_event(&mut self, uid: &str, answer: &str) -> CmdResult {
        let stat = partstat(answer).ok_or_else(|| CmdError::Params(format!("`{answer}`: answer accepted, tentative or declined")))?;
        let account = self.find_event(uid).map(|(a, _)| a.to_string()).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let me = self.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
        let mine = self.my_addresses();
        let mut master = None;
        let p = self.pim_mut(&account);
        let mut remote = false;
        for e in p.events.iter_mut().filter(|e| e.event.uid == uid) {
            set_answer(&mut e.event, &mine, &me, stat);
            remote |= e.calendar != LOCAL;
            e.dirty |= e.calendar != LOCAL;
            if e.event.recurrence_id.is_none() {
                master = Some(e.event.clone());
            }
        }
        self.after_pim_change(&account, remote);
        if let Some(m) = master {
            self.send_reply(&account, &m, stat)?;
        }
        Ok(serde_json::json!({"answer": stat, "uid": uid}))
    }

    /// Take in an answer from a guest, or a cancellation from the organizer, in the open message.
    pub fn apply_invite(&mut self) -> CmdResult {
        let key = self.reading.as_ref().map(|r| r.key.clone()).ok_or_else(|| CmdError::Disabled("open the invitation first".into()))?;
        let (cal, sender) = self.message_invitation(&key).ok_or_else(|| CmdError::Disabled("this message has no invitation".into()))?;
        let inv = self.invite().ok_or_else(|| CmdError::Disabled("this message has no invitation".into()))?;
        let uid = inv.event.uid.clone();
        let Some(account) = self.stored_parts(&uid).map(|(a, _)| a.to_string()) else {
            return match inv.method.as_str() {
                "CANCEL" => Ok(serde_json::json!({"removed": false})),
                "REPLY" => Err(CmdError::Disabled("the event isn't in your calendar".into())),
                _ => Err(CmdError::Disabled("answer the invitation instead".into())),
            };
        };
        if let Some(p) = self.invite_problem(&cal, &sender) {
            return Err(CmdError::Disabled(p.reason().into()));
        }
        match inv.method.as_str() {
            "CANCEL" => {
                // Cancelled by the organizer: no answer is sent. The whole series, or the
                // occurrences it names.
                let cancelled: Vec<Option<i64>> = cal.events.iter().filter(|e| e.uid == uid).map(|e| e.recurrence_id).collect();
                let p = self.pim_mut(&account);
                let mut remote = false;
                if cancelled.iter().any(Option::is_none) {
                    let gone: Vec<StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid).cloned().collect();
                    remote |= delete_items(p, &gone);
                    p.events.retain(|e| e.event.uid != uid);
                } else {
                    for at in cancelled.into_iter().flatten() {
                        remote |= drop_occurrence(p, &uid, at);
                    }
                }
                self.after_pim_change(&account, remote);
                Ok(serde_json::json!({"removed": true}))
            }
            "REPLY" => {
                let who = inv.replier.ok_or_else(|| CmdError::Disabled("the answer names no one".into()))?;
                let p = self.pim_mut(&account);
                let mut remote = false;
                for e in p.events.iter_mut().filter(|e| e.event.uid == uid && e.event.recurrence_id == inv.event.recurrence_id) {
                    match e.event.attendees.iter_mut().find(|a| a.email.eq_ignore_ascii_case(&who.email)) {
                        Some(a) => a.partstat = who.partstat.clone(),
                        None => e.event.attendees.push(Attendee { rsvp: false, ..who.clone() }),
                    }
                    remote |= e.calendar != LOCAL;
                    e.dirty |= e.calendar != LOCAL;
                }
                self.after_pim_change(&account, remote);
                Ok(serde_json::json!({"recorded": who.partstat}))
            }
            _ => Err(CmdError::Disabled("answer the invitation instead".into())),
        }
    }

    /// Send the organizer the user's answer.
    fn send_reply(&mut self, account: &str, event: &CalEvent, stat: &str) -> Result<(), CmdError> {
        let Some(organizer) = event.organizer.clone().filter(|o| addr::is_valid_email(&o.email)) else { return Ok(()) };
        let me = self.account(account).map(|a| (a.config.name.clone(), a.config.email.clone())).unwrap_or_default();
        if organizer.email.eq_ignore_ascii_case(&me.1) {
            return Ok(());
        }
        let mine = self.my_addresses();
        let mut reply = event.clone();
        reply.attendees.retain(|a| mine.contains(&a.email.to_ascii_lowercase()));
        if reply.attendees.is_empty() {
            reply.attendees.push(Attendee { email: me.1.clone(), name: me.0.clone(), partstat: stat.into(), rsvp: false });
        }
        reply.description.clear();
        reply.extra.retain(|x| !x.to_ascii_uppercase().starts_with("BEGIN:VALARM"));
        let word = match stat {
            "ACCEPTED" => "Accepted",
            "TENTATIVE" => "Tentative",
            _ => "Declined",
        };
        let who = if me.0.is_empty() { me.1.clone() } else { me.0.clone() };
        let body = format!("{who} {} the invitation to {}.", word.to_ascii_lowercase(), event.summary);
        let to = vec![EmailAddress::new(organizer.name.clone(), organizer.email.clone())];
        self.send_itip(account, to, "REPLY", &[reply], &format!("{word}: {}", event.summary), &body)
    }

    /// Invite the guests of one of the user's events (or tell them of a change: `REQUEST`, or
    /// that it's off: `CANCEL`).
    pub fn send_invitations(&mut self, event: &CalEvent, account: &str, method: &str) -> Result<usize, CmdError> {
        let me = self.account(account).map(|a| a.config.email.to_ascii_lowercase()).unwrap_or_default();
        let to: Vec<EmailAddress> = event
            .attendees
            .iter()
            .filter(|a| !a.email.eq_ignore_ascii_case(&me) && addr::is_valid_email(&a.email))
            .map(|a| EmailAddress::new(a.name.clone(), a.email.clone()))
            .collect();
        if to.is_empty() {
            return Ok(0);
        }
        let mut e = event.clone();
        let (subject, body) = if method == "CANCEL" {
            e.status = "CANCELLED".into();
            (format!("Cancelled: {}", event.summary), format!("{} has been cancelled.", event.summary))
        } else {
            let when = mailcraft_model::dates::full_time(event.start, &self.clock);
            let place = if event.location.is_empty() { String::new() } else { format!("\nWhere: {}", event.location) };
            (format!("Invitation: {}", event.summary), format!("{}\nWhen: {when}{place}\n\n{}", event.summary, event.description))
        };
        let n = to.len();
        self.send_itip(account, to, method, &[e], &subject, &body)?;
        Ok(n)
    }

    /// Send a message carrying a calendar (`text/calendar; method=…`, RFC 6047).
    fn send_itip(
        &mut self,
        account: &str,
        to: Vec<EmailAddress>,
        method: &str,
        events: &[CalEvent],
        subject: &str,
        body: &str,
    ) -> Result<(), CmdError> {
        let acc = self.account(account).ok_or_else(|| CmdError::Failed("the account is gone".into()))?;
        let from = EmailAddress::new(acc.config.name.clone(), acc.config.email.clone());
        let local = acc.is_local();
        let sent_folder = acc.folder(FolderRole::Sent).map(|f| f.path.clone());
        let ics = ical::write(events, Some(method), self.clock.now);
        let d = mime::Draft {
            to,
            subject: subject.to_string(),
            body: body.to_string(),
            attachments: vec![mime::OutAttachment {
                name: "invite.ics".into(),
                mime: format!("text/calendar; method={method}"),
                data: ics.into_bytes(),
            }],
            ..Default::default()
        };
        let now = self.clock.now;
        let mid = mime::new_message_id(&from.email, now, self.random());
        let raw = mime::build(&d, &from, now, &mid).map_err(CmdError::Failed)?;
        if local {
            if let Some(folder) = &sent_folder {
                self.file_locally(account, folder, &raw, true);
            }
            return Ok(());
        }
        let id = self.fresh_id();
        self.jobs.push(Job::Send { account: account.to_string(), id, raw, from: from.email.clone(), recipients: d.recipients(), sent_folder });
        Ok(())
    }
}

/// Set the user's answer on an event (adding them as a guest when the invitation came through a
/// list or an alias).
fn set_answer(e: &mut CalEvent, mine: &[String], me: &str, stat: &str) {
    let mut found = false;
    for a in e.attendees.iter_mut().filter(|a| mine.contains(&a.email.to_ascii_lowercase())) {
        a.partstat = stat.to_string();
        a.rsvp = false;
        found = true;
    }
    if !found && !me.is_empty() {
        e.attendees.push(Attendee { email: me.to_string(), name: String::new(), partstat: stat.to_string(), rsvp: false });
    }
}

/// Delete these parts of an event on the server: each item once. True when the server has
/// something to learn.
fn delete_items(p: &mut Pim, gone: &[StoredEvent]) -> bool {
    let mut remote = false;
    for g in gone.iter().filter(|g| !g.href.is_empty()) {
        if !p.deleted.iter().any(|t| t.href == g.href) {
            p.deleted.push(Tombstone { href: g.href.clone(), etag: g.etag.clone(), calendar: g.calendar.clone() });
        }
        remote = true;
    }
    remote
}

/// Take one occurrence out of a series (declined, or cancelled by the organizer): an exception
/// on the series, and a changed copy of it goes. On the server only that copy's own item is
/// deleted; one it shares with the rest of the series (CalDAV keeps a series in one item) is
/// written again without it. True when the server has something to learn.
fn drop_occurrence(p: &mut Pim, uid: &str, at: i64) -> bool {
    let gone: Vec<StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid && e.event.recurrence_id == Some(at)).cloned().collect();
    p.events.retain(|e| !(e.event.uid == uid && e.event.recurrence_id == Some(at)));
    let mut remote = false;
    for g in gone.iter().filter(|g| !g.href.is_empty()) {
        match p.events.iter_mut().find(|e| e.event.uid == uid && e.href == g.href) {
            Some(rest) => rest.dirty = true,
            None => {
                delete_items(p, std::slice::from_ref(g));
            }
        }
        remote = true;
    }
    if let Some(m) = p.events.iter_mut().find(|e| e.event.uid == uid && e.event.recurrence_id.is_none()) {
        if !m.event.exdates.contains(&at) && m.event.exdates.len() < MAX_EXDATES {
            m.event.exdates.push(at);
        }
        let r = m.calendar != LOCAL;
        m.dirty |= r;
        remote |= r;
    }
    remote
}

/// Put one changed occurrence of a series in the calendar, replacing an earlier copy of it (and
/// bringing it back if it had been taken out). True when the server has something to learn.
fn put_occurrence(p: &mut Pim, calendar: &str, e: CalEvent) -> bool {
    let at = e.recurrence_id;
    let old = p.events.iter().find(|x| x.event.uid == e.uid && x.event.recurrence_id == at).cloned();
    p.events.retain(|x| !(x.event.uid == e.uid && x.event.recurrence_id == at));
    if let Some(at) = at
        && let Some(m) = p.events.iter_mut().find(|x| x.event.uid == e.uid && x.event.recurrence_id.is_none())
        && m.event.exdates.contains(&at)
    {
        m.event.exdates.retain(|x| *x != at);
        m.dirty |= m.calendar != LOCAL;
    }
    let calendar = old.as_ref().map_or_else(|| calendar.to_string(), |o| o.calendar.clone());
    let remote = calendar != LOCAL;
    p.events.push(StoredEvent {
        href: old.as_ref().map(|o| o.href.clone()).unwrap_or_default(),
        etag: old.map(|o| o.etag).unwrap_or_default(),
        calendar,
        event: e,
        dirty: remote,
    });
    remote
}
