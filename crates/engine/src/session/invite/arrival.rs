//! Invitations as they arrive: into the calendar as tentative before they're answered (issue
//! #20), in the account's calendar on its server once that's known (issue #30).
//!
//! An account's calendars are found after its mail (the first sync, or again when the user gives
//! the calendar server's address), so an invitation can arrive before there's anywhere on the
//! server to put it. It's held until the account's calendars are found, then put there; one
//! that waited too long (the server is slow, refuses, or is never reached) goes to the calendar
//! on this computer, and moves to the server's calendar once there is one. Nothing is lost
//! either way: held invitations are kept with the calendar (`pim.json`), not just in memory.

use mailcraft_model::pim::{HeldInvite, LOCAL, StoredEvent, is_server_calendar};

use super::*;

/// Most arrived messages waiting for their bodies to be looked at for an invitation.
const MAX_ARRIVED_INVITES: usize = 200;
/// Most invitations held per account; more are put in the calendar on this computer at once.
const MAX_HELD: usize = 200;
/// The largest invitation held (its calendar is kept as text until it's put in the calendar).
const MAX_HELD_ICS: usize = 256 << 10;
/// Most bytes of invitations held per account (they are kept in the calendar's file).
const MAX_HELD_BYTES: usize = 2 << 20;
/// How long an invitation waits for the account's calendars to be found before it goes to the
/// calendar on this computer (it moves to the server's once that's found).
pub(crate) const HOLD_SECS: i64 = 180;
/// Most invitations remembered as put on this computer to move later.
const MAX_PLACED: usize = 1000;

impl Session {
    /// New mail that may hold an invitation (issue #20): looked at now when its body is here,
    /// else when it comes.
    pub(crate) fn invitations_arrived(&mut self, arrived: &[Summary]) {
        if !self.prefs.tentative_invites {
            return;
        }
        for m in arrived {
            if self.readable_raw(&m.key).is_some() {
                self.take_invitation(&m.key);
            } else if self.arrived_invites.len() < MAX_ARRIVED_INVITES && !self.arrived_invites.contains(&m.key) {
                self.arrived_invites.push(m.key.clone());
            }
        }
    }

    /// An arrived message's body came: its invitation, if it has one, into the calendar.
    pub(crate) fn invitation_body(&mut self, key: &MessageKey) {
        if let Some(i) = self.arrived_invites.iter().position(|k| k == key) {
            self.arrived_invites.remove(i);
            if self.prefs.tentative_invites {
                self.take_invitation(key);
            }
        }
    }

    /// Whether an account's calendars are still to be found (issue #30): it has a server that
    /// is asked for them (CalDAV, Exchange), and hasn't answered yet. An OAuth account without
    /// a calendar sign-in of its own is never asked, so never waits.
    pub fn calendars_pending(&self, account: &str) -> bool {
        let Some(a) = self.account(account) else { return false };
        let c = &a.config;
        let asked = c.graph.is_some() || c.ews.is_some() || c.oauth.is_none() || c.dav.as_ref().is_some_and(|d| d.username.is_some());
        !a.is_local() && asked && self.pim.get(account).is_none_or(|p| p.dav_checked.is_none())
    }

    /// An arrived invitation into the calendar; held while the account's calendars are still to
    /// be found (issue #30). Only an invitation or a cancellation is held (a calendar sent for
    /// reading, or a guest's answer, has nothing to put in the calendar), and only so much of
    /// them: past that, one goes to the calendar on this computer at once (it moves to the
    /// server's later), so the calendar's file stays small. Whether anything changed.
    pub(crate) fn take_invitation(&mut self, key: &MessageKey) -> bool {
        let Some((ics, sender)) = self.message_ics(key) else { return false };
        let account = key.account.clone();
        let invitation = || matches!(ical::parse_in(&ics, self.clock.local_zone()).method.as_deref(), Some("REQUEST" | "CANCEL"));
        if self.calendars_pending(&account) && ics.len() <= MAX_HELD_ICS && invitation() {
            let now = self.clock.now;
            let p = self.pim_mut(&account);
            if p.held.iter().any(|h| &h.key == key) {
                return false;
            }
            let bytes = p.held.iter().map(|h| h.ics.len()).fold(ics.len(), usize::saturating_add);
            if p.held.len() < MAX_HELD && bytes <= MAX_HELD_BYTES {
                p.held.push(HeldInvite { key: key.clone(), since: now, ics, from: sender });
                self.save_pim(&account);
                return false;
            }
        }
        self.take_calendar(&account, &ics, &sender)
    }

    /// Put an account's held invitations in its calendar, in the order they came: all of them
    /// (its calendars were found), or those that waited long enough.
    pub(crate) fn release_held(&mut self, account: &str, all: bool) {
        let now = self.clock.now;
        let gone = self.account(account).is_none();
        let Some(p) = self.pim.get_mut(account) else { return };
        if p.held.is_empty() {
            return;
        }
        // The account was removed: nowhere to put them.
        if gone {
            p.held.clear();
            return;
        }
        let (due, kept): (Vec<HeldInvite>, Vec<HeldInvite>) =
            std::mem::take(&mut p.held).into_iter().partition(|h| all || now - h.since >= HOLD_SECS);
        p.held = kept;
        if due.is_empty() {
            return;
        }
        self.save_pim(account);
        if !self.prefs.tentative_invites {
            return;
        }
        for h in due {
            self.take_calendar(account, &h.ics, &h.from);
        }
    }

    /// A meeting the user dealt with (answered or cancelled) in its message while its
    /// invitations were held: those held, no newer than the version dealt with, are let go, so
    /// a declined or cancelled meeting doesn't come back as tentative once the calendars are
    /// found. A newer update still goes in, to be seen.
    pub(super) fn forget_held(&mut self, account: &str, uid: &str, dealt: &[CalEvent]) {
        let zone = self.clock.local_zone();
        let Some(p) = self.pim.get_mut(account).filter(|p| !p.held.is_empty()) else { return };
        let before = p.held.len();
        p.held.retain(|h| {
            let cal = ical::parse_in(&h.ics, zone);
            let parts: Vec<&CalEvent> = cal.events.iter().filter(|e| e.uid == uid).collect();
            // The series dealt with covers its occurrences; an occurrence covers only itself.
            let covered = |e: &CalEvent| dealt.iter().any(|d| d.recurrence_id.is_none() || d.recurrence_id == e.recurrence_id);
            parts.is_empty() || parts.iter().any(|e| !covered(e) || newer_than(e, dealt))
        });
        if p.held.len() != before {
            self.save_pim(account);
        }
    }

    /// Invitations that waited too long for their account's calendars: into the calendar on
    /// this computer (called from [`Session::tick`]; cheap when nothing waits).
    pub(crate) fn held_invitations_due(&mut self) {
        let now = self.clock.now;
        let due: Vec<String> = self.pim.iter().filter(|(_, p)| p.held.iter().any(|h| now - h.since >= HOLD_SECS)).map(|(a, _)| a.clone()).collect();
        for a in due {
            self.release_held(&a, false);
        }
    }

    /// The user's answers to the invitations MailCraft put on this computer (uid, answer), those
    /// answered.
    pub(crate) fn answered_here(&self, account: &str) -> Vec<(String, String)> {
        let mine = self.my_addresses();
        let Some(p) = self.pim.get(account) else { return Vec::new() };
        p.placed
            .iter()
            .filter_map(|uid| {
                let answer = p
                    .events
                    .iter()
                    .filter(|e| &e.event.uid == uid && e.calendar == LOCAL)
                    .flat_map(|e| e.event.attendees.iter())
                    .find(|a| mine.contains(&a.email.to_ascii_lowercase()))
                    .map(|a| a.partstat.clone())
                    .filter(|a| !a.is_empty() && !a.eq_ignore_ascii_case("NEEDS-ACTION"))?;
                Some((uid.clone(), answer))
            })
            .collect()
    }

    /// The account's calendars were just found or synced (issue #30): invitations MailCraft put
    /// on this computer meanwhile move to its calendar on the server; one the server has
    /// already (CalDAV scheduling put it there) keeps the server's copy, with the answer given
    /// here (`answered`, read before the sync's copy took its place). Those held go in now.
    pub(crate) fn calendars_found(&mut self, account: &str, answered: &[(String, String)]) {
        let mine = self.my_addresses();
        let me = self.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
        let p = self.pim_mut(account);
        let before = (p.placed.len(), p.events.len(), p.calendars.len());
        let mut remote = false;
        if let Some(target) = p.server_calendar().filter(|_| !p.placed.is_empty()).map(|c| c.id.clone()) {
            for uid in std::mem::take(&mut p.placed) {
                let on_server = p.events.iter().any(|e| e.event.uid == uid && is_server_calendar(&e.calendar));
                if on_server {
                    p.events.retain(|e| !(e.event.uid == uid && e.calendar == LOCAL));
                    // Still unanswered there: the answer given here.
                    if let Some((_, stat)) = answered.iter().find(|(u, _)| *u == uid) {
                        for e in p.events.iter_mut().filter(|e| e.event.uid == uid && is_server_calendar(&e.calendar)) {
                            let unanswered = e
                                .event
                                .attendees
                                .iter()
                                .filter(|a| mine.contains(&a.email.to_ascii_lowercase()))
                                .all(|a| a.partstat.is_empty() || a.partstat.eq_ignore_ascii_case("NEEDS-ACTION"));
                            if unanswered {
                                set_answer(&mut e.event, &mine, &me, stat);
                                e.dirty = true;
                                remote = true;
                            }
                        }
                    }
                    continue;
                }
                for e in p.events.iter_mut().filter(|e| e.event.uid == uid && e.calendar == LOCAL) {
                    e.calendar = target.clone();
                    e.href.clear();
                    e.etag.clear();
                    e.dirty = true;
                    remote = true;
                }
            }
            drop_empty_local(p);
        }
        // Still on this computer (no calendar on the server can take them): kept to move later.
        let left: Vec<String> = p.events.iter().filter(|e| e.calendar == LOCAL).map(|e| e.event.uid.clone()).collect();
        p.placed.retain(|u| left.contains(u));
        if remote || before != (p.placed.len(), p.events.len(), p.calendars.len()) {
            self.after_pim_change(account, remote);
        }
        self.release_held(account, true);
    }

    /// An invitation put on this computer for an account with a server (its calendar there not
    /// found yet): remembered, to move there once it is.
    pub(super) fn placed_here(&mut self, account: &str, calendar: &str, uid: &str) {
        if calendar != LOCAL || self.account(account).is_none_or(AccountState::is_local) {
            return;
        }
        let p = self.pim_mut(account);
        if !p.placed.iter().any(|u| u == uid) && p.placed.len() < MAX_PLACED {
            p.placed.push(uid.to_string());
        }
    }

    /// An invitation that arrived, into the calendar before it's answered, as Outlook shows one
    /// (issue #20): a new one tentative (the user's answer `NEEDS-ACTION`), in the account's
    /// calendar, and on its server so other clients see it; an update from its organizer to one
    /// not answered yet replaces it, and one to an answered meeting changes it keeping the answer;
    /// a cancellation takes out one not answered and marks an answered one cancelled (out of the
    /// calendar's view, and so of the user's time, either way). Nothing is
    /// sent: the organizer hears only the user's own answer. One the server put in the calendar
    /// already (CalDAV scheduling) isn't added twice. Whether anything changed.
    fn take_calendar(&mut self, account: &str, ics: &str, sender: &str) -> bool {
        let cal = ical::parse_in(ics, self.clock.local_zone());
        let Some(uid) = cal.events.first().map(|e| e.uid.clone()).filter(|u| !u.trim().is_empty()) else { return false };
        let mine = self.my_addresses();
        // The user's own meeting (a copy to themselves): theirs already.
        if cal.events.iter().any(|e| e.organizer.as_ref().is_some_and(|o| mine.contains(&without_mailto(&o.email).to_ascii_lowercase()))) {
            return false;
        }
        if self.invite_problem(&cal, sender).is_some() {
            return false;
        }
        let events: Vec<CalEvent> = cal.events.iter().filter(|e| e.uid == uid).cloned().collect();
        let stored = self.stored_parts(&uid).map(|(a, parts)| (a.to_string(), self.tentative_copy(&parts)));
        match (cal.method.as_deref(), stored) {
            (Some("REQUEST"), None) => {
                let me = self.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
                let p = self.pim_mut(account);
                let calendar = p.default_calendar().map(|c| c.id.clone()).unwrap_or_else(|| LOCAL.into());
                let mut remote = false;
                for mut e in events {
                    set_answer(&mut e, &mine, &me, "NEEDS-ACTION");
                    e.attendees.iter_mut().filter(|a| mine.contains(&a.email.to_ascii_lowercase())).for_each(|a| a.rsvp = true);
                    remote |= put_occurrence(p, &calendar, e);
                }
                self.placed_here(account, &calendar, &uid);
                self.after_pim_change(account, remote);
                true
            }
            (Some("REQUEST"), Some((account, unanswered))) => {
                // An update: only when newer than what's there (invite_problem said so; the same
                // version, as the server or another copy of the mail put it, changes nothing).
                let p = self.pim_mut(&account);
                let newer = events
                    .iter()
                    .any(|m| p.events.iter().filter(|s| s.event.uid == uid && s.event.recurrence_id == m.recurrence_id).all(|s| newer(m, &s.event)));
                if !newer {
                    return false;
                }
                let me = self.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
                let p = self.pim_mut(&account);
                // A new occurrence goes where the series is.
                let calendar = p.events.iter().find(|s| s.event.uid == uid).map(|s| s.calendar.clone()).unwrap_or_else(|| LOCAL.into());
                let mut remote = false;
                for mut e in events {
                    // The user's answer stays, unless there's none yet.
                    let kept = p
                        .events
                        .iter()
                        .find(|s| s.event.uid == uid && (s.event.recurrence_id == e.recurrence_id || s.event.recurrence_id.is_none()))
                        .and_then(|s| s.event.attendees.iter().find(|a| mine.contains(&a.email.to_ascii_lowercase())).map(|a| a.partstat.clone()))
                        .filter(|_| !unanswered)
                        .unwrap_or_else(|| "NEEDS-ACTION".into());
                    set_answer(&mut e, &mine, &me, &kept);
                    remote |= put_occurrence(p, &calendar, e);
                }
                self.after_pim_change(&account, remote);
                true
            }
            (Some("CANCEL"), Some((account, unanswered))) => {
                let cancelled: Vec<Option<i64>> = events.iter().map(|e| e.recurrence_id).collect();
                let p = self.pim_mut(&account);
                let mut remote = false;
                if unanswered {
                    // Never answered: gone, as if it hadn't come.
                    if cancelled.iter().any(Option::is_none) {
                        let gone: Vec<StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid).cloned().collect();
                        remote |= delete_items(p, &gone);
                        p.events.retain(|e| e.event.uid != uid);
                    } else {
                        for at in cancelled.into_iter().flatten() {
                            remote |= drop_occurrence(p, &uid, at);
                        }
                    }
                } else {
                    // Answered: marked cancelled, on the server too, as other calendars show it; the
                    // calendar here leaves cancelled meetings out, so its time is free.
                    let whole = cancelled.iter().any(Option::is_none);
                    for e in p.events.iter_mut().filter(|e| e.event.uid == uid && (whole || cancelled.contains(&e.event.recurrence_id))) {
                        e.event.status = "CANCELLED".into();
                        e.dirty |= e.calendar != LOCAL;
                        remote |= e.calendar != LOCAL;
                    }
                }
                self.after_pim_change(&account, remote);
                true
            }
            _ => false,
        }
    }
}

/// The calendar on this computer goes once it holds nothing and the server has calendars (as a
/// sync leaves it when there's nothing in it).
pub(crate) fn drop_empty_local(p: &mut Pim) {
    let server = p.calendars.iter().any(|c| is_server_calendar(&c.id));
    if server && !p.events.iter().any(|e| e.calendar == LOCAL) {
        p.calendars.retain(|c| c.id != LOCAL);
    }
}
