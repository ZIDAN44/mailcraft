//! Meeting messages and a calendar server that schedules (RFC 6638; see `ical::scheduling`).
//!
//! MailCraft stays the one that mails invitations, changes, cancellations and answers, so each
//! guest gets one and the organizer one answer wherever they are; the server is told to send
//! none (`SCHEDULE-AGENT=CLIENT`). On such a server they wait for it ([`Notice`]): an
//! invitation or change goes with the version the server stored (SOGo raises its `SEQUENCE`, and
//! the copy it puts in a colleague's calendar is that version), a cancellation once the server
//! deleted the meeting (so a guest taking it in doesn't delete a copy the server still keeps, and
//! answer "declined" through it). A guest's answer to a meeting the server put in the calendar
//! itself (a colleague's on the same server) is the server's to take on, as it does whatever it's
//! told; one to a meeting MailCraft put there from the invitation's mail is mailed.
//!
//! Microsoft Graph sends the invitations of a meeting stored there itself: MailCraft doesn't.

use mailcraft_model::pim::{MAX_NOTICES, Notice, is_server_calendar};

use super::*;

/// How a meeting message goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// By mail, now.
    Now,
    /// By mail, once the calendar server has the change (a [`Notice`]).
    Wait,
    /// Not by MailCraft: the server sends it.
    Server,
}

impl Session {
    /// How the messages of the user's meeting `uid` in `account` go (its copy as it is now).
    pub fn meeting_route(&self, account: &str, uid: &str) -> Route {
        let Some(p) = self.pim.get(account) else { return Route::Now };
        let theirs = |id: &str| p.calendars.iter().any(|c| c.id == id && (c.foreign || c.read_only));
        let mut calendars = p.events.iter().filter(|e| e.event.uid == uid && !theirs(&e.calendar)).map(|e| e.calendar.as_str());
        let Some(calendar) = calendars.next() else { return Route::Now };
        if self.account(account).is_some_and(|a| a.config.graph.is_some()) && calendar != LOCAL {
            return Route::Server;
        }
        self.calendar_route(account, calendar)
    }

    /// How meeting messages about a copy in this calendar go.
    fn calendar_route(&self, account: &str, calendar: &str) -> Route {
        let schedules = self.pim.get(account).is_some_and(|p| p.auto_schedule);
        if schedules && is_server_calendar(calendar) { Route::Wait } else { Route::Now }
    }

    /// Tell the guests of the user's meeting of it (`REQUEST`) or that it's off (`CANCEL`), as
    /// its route says. A cancellation waits for the server to delete `href` (empty: nothing
    /// stored there). Returns how many guests are told (or will be).
    pub fn tell_guests(&mut self, account: &str, event: &CalEvent, method: &str, route: Route, href: &str) -> Result<usize, CmdError> {
        let me = self.account(account).map(|a| a.config.email.to_ascii_lowercase()).unwrap_or_default();
        let guests = event.attendees.iter().filter(|a| !a.email.eq_ignore_ascii_case(&me) && addr::is_valid_email(&a.email)).count();
        match route {
            Route::Server => Ok(0),
            // Called off before the server had it: the invitation still waiting goes too, and the
            // guests, never invited, hear nothing.
            Route::Wait if method == "CANCEL" && href.is_empty() && event.recurrence_id.is_none() => {
                let p = self.pim_mut(account);
                let before = p.notices.len();
                p.notices.retain(|n| !(n.uid == event.uid && n.method == "REQUEST"));
                let never_sent = p.notices.len() != before;
                self.save_pim(account);
                if never_sent { Ok(0) } else { self.send_invitations(event, account, method) }
            }
            Route::Wait => {
                let n = Notice { uid: event.uid.clone(), method: method.into(), event: event.clone(), href: href.into(), ..Default::default() };
                self.add_notice(account, n);
                Ok(guests)
            }
            Route::Now => self.send_invitations(event, account, method),
        }
    }

    /// Send the organizer the user's answer: now, or once the server has it, when the copy is in
    /// a calendar of a server that schedules (`calendar`). `copy` is the user's copy as it was
    /// before the answer: a declined one is deleted (its item on the server, if any).
    pub(super) fn answer_organizer(
        &mut self,
        account: &str,
        calendar: &str,
        copy: &[StoredEvent],
        event: &CalEvent,
        stat: &str,
    ) -> Result<(), CmdError> {
        let deleted = stat == "DECLINED"
            && event.recurrence_id.is_none()
            && !self.pim.get(account).is_some_and(|p| p.events.iter().any(|e| e.event.uid == event.uid));
        let href = if deleted { copy.iter().find(|e| !e.href.is_empty()).map(|e| e.href.clone()).unwrap_or_default() } else { String::new() };
        // Never stored there: nothing to wait for, and nobody but MailCraft tells the organizer.
        let nothing_stored = deleted && href.is_empty();
        if self.calendar_route(account, calendar) != Route::Wait || nothing_stored {
            return self.send_reply(account, event, stat);
        }
        let by_mail =
            copy.iter().any(|e| ical::is_guest_copy(&e.event)) || self.pim.get(account).is_some_and(|p| p.guest_copies.contains(&event.uid));
        let n = Notice { uid: event.uid.clone(), method: "REPLY".into(), answer: stat.into(), event: event.clone(), href, by_mail };
        self.add_notice(account, n);
        Ok(())
    }

    /// Keep a message to send once the server has the change: the newest of each kind only.
    fn add_notice(&mut self, account: &str, n: Notice) {
        let p = self.pim_mut(account);
        if n.method != "CANCEL" {
            p.notices.retain(|x| !(x.uid == n.uid && x.method == n.method && x.event.recurrence_id == n.event.recurrence_id));
        }
        p.notices.push(n);
        if p.notices.len() > MAX_NOTICES {
            let extra = p.notices.len() - MAX_NOTICES;
            p.notices.drain(..extra);
        }
        self.save_pim(account);
    }

    /// After a sync of the account's calendars: the guest copies MailCraft put there learnt, and
    /// the messages whose change the server now has, sent.
    pub(crate) fn after_dav_sync(&mut self, account: &str) {
        let Some(p) = self.pim.get_mut(account) else { return };
        let marked: Vec<String> = p.events.iter().filter(|e| ical::is_guest_copy(&e.event)).map(|e| e.event.uid.clone()).collect();
        for uid in marked {
            if !p.guest_copies.contains(&uid) {
                p.guest_copies.push(uid);
            }
        }
        let events = &p.events;
        let notices = &p.notices;
        p.guest_copies.retain(|u| events.iter().any(|e| &e.event.uid == u) || notices.iter().any(|n| &n.uid == u));
        if p.notices.is_empty() {
            return;
        }
        let theirs: Vec<String> = p.calendars.iter().filter(|c| c.foreign || c.read_only).map(|c| c.id.clone()).collect();
        let mut ready: Vec<(Notice, Option<CalEvent>)> = Vec::new();
        let mut waiting: Vec<Notice> = Vec::new();
        for n in std::mem::take(&mut p.notices) {
            let parts: Vec<&StoredEvent> = p.events.iter().filter(|e| e.event.uid == n.uid && !theirs.contains(&e.calendar)).collect();
            let pending = parts.iter().any(|e| e.dirty || (e.href.is_empty() && e.calendar != LOCAL))
                || (!n.href.is_empty() && p.deleted.iter().any(|t| t.href == n.href));
            if pending {
                waiting.push(n);
            } else {
                let master = parts.iter().find(|e| e.event.recurrence_id.is_none()).or_else(|| parts.first()).map(|e| e.event.clone());
                ready.push((n, master));
            }
        }
        p.notices = waiting;
        let guest_copies = p.guest_copies.clone();
        self.save_pim(account);
        for (n, stored) in ready {
            // A message that can't go (the account gone) isn't kept: it would never go.
            let _ = self.send_notice(account, n, stored, &guest_copies);
        }
    }

    /// Send a message whose change the server has: `stored` is the meeting as stored now.
    fn send_notice(&mut self, account: &str, n: Notice, stored: Option<CalEvent>, guest_copies: &[String]) -> Result<usize, CmdError> {
        match n.method.as_str() {
            // The version the server stored, unless the server mailed the guests itself after all
            // (it says it delivered them: one that doesn't do as `SCHEDULE-AGENT` asks). Gone
            // meanwhile, it isn't sent.
            "REQUEST" => match stored {
                Some(s) if !s.server_delivered => self.send_invitations(&s, account, "REQUEST"),
                _ => Ok(0),
            },
            "CANCEL" => {
                let mut e = n.event;
                // One occurrence off: as new as the series the server keeps.
                if let Some(s) = &stored {
                    e.sequence = e.sequence.max(s.sequence);
                }
                self.send_invitations(&e, account, "CANCEL")
            }
            "REPLY" => {
                let by_mail = n.by_mail || guest_copies.contains(&n.uid) || stored.as_ref().is_some_and(ical::is_guest_copy);
                if by_mail {
                    self.send_reply(account, &n.event, &n.answer)?;
                    Ok(1)
                } else {
                    Ok(0)
                }
            }
            _ => Ok(0),
        }
    }
}
