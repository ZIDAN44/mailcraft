//! Meeting invitations by mail (iTIP, RFC 5546, over iMIP, RFC 6047): answering one, taking
//! in the answers and cancellations organizers and guests send, and inviting people to the
//! user's own events. Invitations travel as `text/calendar` parts with a `METHOD`.

use mailcraft_model::ical::{self, Attendee, CalEvent, ICalendar};
use mailcraft_model::pim::{LOCAL, StoredEvent, Tombstone};

use super::*;

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
        let raw = self.store.as_ref()?.read_body(key)?;
        let parsed = mime::parse(&raw)?;
        let att = parsed
            .attachments
            .iter()
            .find(|a| a.mime.to_ascii_lowercase().starts_with("text/calendar") || a.name.to_ascii_lowercase().ends_with(".ics"))?;
        let data = mime::attachment_data(&raw, att.index)?;
        let cal = ical::parse(&String::from_utf8_lossy(&data), i64::from(self.clock.offset));
        (!cal.events.is_empty()).then_some(cal)
    }

    /// The invitation in the open message.
    pub fn invite(&self) -> Option<Invite> {
        let r = self.reading.as_ref()?;
        let cal = self.message_calendar(&r.key)?;
        let method = cal.method.clone().unwrap_or_else(|| "PUBLISH".into());
        if !matches!(method.as_str(), "REQUEST" | "REPLY" | "CANCEL") {
            return None;
        }
        let event = cal.events.iter().find(|e| e.recurrence_id.is_none()).or_else(|| cal.events.first())?.clone();
        let mine = self.my_addresses();
        let stored = self.find_event(&event.uid).map(|(_, e)| e.event.clone());
        let answer = stored
            .as_ref()
            .unwrap_or(&event)
            .attendees
            .iter()
            .find(|a| mine.contains(&a.email.to_ascii_lowercase()))
            .map(|a| a.partstat.clone())
            .unwrap_or_else(|| "NEEDS-ACTION".into());
        let replier = (method == "REPLY").then(|| event.attendees.first().cloned()).flatten();
        Some(Invite { method, event, answer, in_calendar: stored.is_some(), replier })
    }

    /// Answer the invitation in the open message: the event goes into the calendar (or out of
    /// it, when declined) and the organizer gets the answer.
    pub fn respond_to_invite(&mut self, answer: &str) -> CmdResult {
        let stat = partstat(answer).ok_or_else(|| CmdError::Params(format!("`{answer}`: answer accepted, tentative or declined")))?;
        let key = self.reading.as_ref().map(|r| r.key.clone()).ok_or_else(|| CmdError::Disabled("open the invitation first".into()))?;
        let cal = self.message_calendar(&key).ok_or_else(|| CmdError::Disabled("this message has no invitation".into()))?;
        if cal.method.as_deref() != Some("REQUEST") {
            return Err(CmdError::Disabled("this message isn't an invitation".into()));
        }
        let account = key.account.clone();
        let me = self.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
        let uid = cal.events.first().map(|e| e.uid.clone()).unwrap_or_default();
        let mut events: Vec<CalEvent> = cal.events.into_iter().filter(|e| e.uid == uid).collect();
        for e in &mut events {
            set_answer(e, &self.my_addresses(), &me, stat);
        }
        // Into the calendar, replacing an earlier copy (an updated invitation); out if declined.
        let p = self.pim_mut(&account);
        let existing: Vec<StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid).cloned().collect();
        let calendar =
            existing.first().map(|e| e.calendar.clone()).or_else(|| p.default_calendar().map(|c| c.id.clone())).unwrap_or_else(|| LOCAL.into());
        p.events.retain(|e| e.event.uid != uid);
        let mut remote = false;
        if stat == "DECLINED" {
            for g in existing.iter().filter(|g| !g.href.is_empty()) {
                p.deleted.push(Tombstone { href: g.href.clone(), etag: g.etag.clone(), calendar: g.calendar.clone() });
                remote = true;
            }
        } else {
            for (i, e) in events.iter().enumerate() {
                let old = existing.iter().find(|x| x.event.recurrence_id == e.recurrence_id);
                p.events.push(StoredEvent {
                    calendar: calendar.clone(),
                    href: old.map(|o| o.href.clone()).unwrap_or_default(),
                    etag: old.map(|o| o.etag.clone()).unwrap_or_default(),
                    event: e.clone(),
                    dirty: calendar != LOCAL,
                });
                remote |= calendar != LOCAL && i == 0;
            }
        }
        self.after_pim_change(&account, remote);
        if let Some(master) = events.first().cloned() {
            self.send_reply(&account, &master, stat)?;
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
        let inv = self.invite().ok_or_else(|| CmdError::Disabled("this message has no invitation".into()))?;
        let uid = inv.event.uid.clone();
        match inv.method.as_str() {
            "CANCEL" => {
                if self.find_event(&uid).is_none() {
                    return Ok(serde_json::json!({"removed": false}));
                }
                // Cancelled by the organizer: no answer is sent.
                let account = self.find_event(&uid).map(|(a, _)| a.to_string()).unwrap_or_default();
                let p = self.pim_mut(&account);
                let mut remote = false;
                let cancelled_one = inv.event.recurrence_id;
                match cancelled_one {
                    Some(at) => {
                        if let Some(m) = p.events.iter_mut().find(|e| e.event.uid == uid && e.event.recurrence_id.is_none()) {
                            m.event.exdates.push(at);
                            remote = m.calendar != LOCAL;
                            m.dirty |= remote;
                        }
                    }
                    None => {
                        for g in p.events.iter().filter(|e| e.event.uid == uid && !e.href.is_empty()) {
                            p.deleted.push(Tombstone { href: g.href.clone(), etag: g.etag.clone(), calendar: g.calendar.clone() });
                            remote = true;
                        }
                        p.events.retain(|e| e.event.uid != uid);
                    }
                }
                self.after_pim_change(&account, remote);
                Ok(serde_json::json!({"removed": true}))
            }
            "REPLY" => {
                let who = inv.replier.ok_or_else(|| CmdError::Disabled("the answer names no one".into()))?;
                let account =
                    self.find_event(&uid).map(|(a, _)| a.to_string()).ok_or_else(|| CmdError::Disabled("the event isn't in your calendar".into()))?;
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
