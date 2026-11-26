//! An account's calendar on this computer and its calendars on the server (issue #30): which
//! one new events and invitations go to (the Default calendar), moving what was kept on this
//! computer before the server's were found, and moving an event to another of the account's
//! calendars (issue #279).

use mailcraft_model::pim::{CalendarInfo, LOCAL, is_server_calendar};

use super::*;

/// Events on this computer that could go to the account's calendar on its server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalEvents {
    /// How many (a series with its changed occurrences counts once).
    pub count: usize,
    /// Where they'd go.
    pub calendar: CalendarInfo,
}

impl Session {
    /// The events on an account's calendar on this computer that could move to its calendar on
    /// the server, offered as "Move N events to …" until they're moved or the user keeps them
    /// here.
    pub fn local_events(&self, account: &str) -> Option<LocalEvents> {
        if self.account(account).is_none_or(AccountState::is_local) {
            return None;
        }
        let p = self.pim.get(account)?;
        if p.keep_here {
            return None;
        }
        let calendar = p.server_calendar()?.clone();
        let mut uids: Vec<&str> = p.events.iter().filter(|e| e.calendar == LOCAL).map(|e| e.event.uid.as_str()).collect();
        uids.sort_unstable();
        uids.dedup();
        (!uids.is_empty()).then_some(LocalEvents { count: uids.len(), calendar })
    }

    /// Move the events on an account's calendar on this computer to one of its calendars on the
    /// server (the default one unless told), to be stored there at the next sync. One the server
    /// has already (the same uid) stays here, not to be doubled.
    pub fn move_local_events(&mut self, account: Option<&str>, calendar: Option<&str>) -> CmdResult {
        let account = self.account_or_shown(account)?;
        if self.account(&account).is_some_and(AccountState::is_local) {
            return Err(CmdError::Disabled("this account's calendar is on this computer only".into()));
        }
        let p = self.pim_mut(&account);
        let target = match calendar.map(str::trim).filter(|c| !c.is_empty()) {
            Some(c) => super::calendar_named(p, c)?.ok_or_else(|| CmdError::Params(format!("no calendar `{c}`")))?,
            None => p.server_calendar().ok_or_else(|| CmdError::Disabled("this account has no calendar on its server yet".into()))?,
        };
        if !is_server_calendar(&target.id) || target.read_only {
            return Err(CmdError::Params(format!("`{}` isn't a calendar on the server that can be added to", target.name)));
        }
        let target = target.clone();
        let mut uids: Vec<String> = p.events.iter().filter(|e| e.calendar == LOCAL).map(|e| e.event.uid.clone()).collect();
        uids.sort_unstable();
        uids.dedup();
        let (mut moved, mut kept) = (0usize, 0usize);
        // Already in one of the user's own (not in a colleague's calendar listed with them).
        let own = p.own_server_calendars();
        for uid in uids {
            if p.events.iter().any(|e| e.event.uid == uid && own.contains(&e.calendar)) {
                kept += 1;
                continue;
            }
            for e in p.events.iter_mut().filter(|e| e.event.uid == uid && e.calendar == LOCAL) {
                e.calendar = target.id.clone();
                e.href.clear();
                e.etag.clear();
                e.dirty = true;
            }
            moved += 1;
        }
        let left: Vec<String> = p.events.iter().filter(|e| e.calendar == LOCAL).map(|e| e.event.uid.clone()).collect();
        p.placed.retain(|u| left.contains(u));
        crate::session::invite::drop_empty_local(p);
        self.after_pim_change(&account, moved > 0);
        Ok(serde_json::json!({"account": account, "calendar": target.id, "moved": moved, "kept": kept}))
    }

    /// Move an event, a series with its changed occurrences, to another calendar of its account
    /// (`calendar`: its id or name; issue #279). On the server it's moved there at the next sync
    /// (a WebDAV MOVE, which tells no guest or organizer anything), or stored there if it's on
    /// this computer; one taken off the server to this computer is deleted there, and so a
    /// meeting isn't, as its guests or organizer would hear of it. Exchange's calendars aren't
    /// moved between yet.
    pub fn move_event_to(&mut self, uid: &str, calendar: &str) -> CmdResult {
        let (account, from) =
            self.find_event(uid).map(|(a, e)| (a.to_string(), e.calendar.clone())).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let p = self.pim_mut(&account);
        let to = super::calendar_named(p, calendar)?.ok_or_else(|| CmdError::Params(format!("no calendar `{calendar}`")))?.clone();
        if to.id == from {
            return Ok(serde_json::json!({"uid": uid, "account": account, "calendar": to.id, "moved": false}));
        }
        // Each a calendar of the account's own (on this computer, or its server's), that can be
        // changed: not one subscribed to by its link, nor a colleague's opened by their address.
        let usable = |id: &str| id == LOCAL || is_server_calendar(id);
        let read_only = |id: &str| p.calendars.iter().any(|c| c.id == id && c.read_only);
        if !usable(&to.id) || to.read_only {
            return Err(CmdError::Params(format!("`{}` can't be added to", to.name)));
        }
        if !usable(&from) || read_only(&from) {
            return Err(CmdError::Params("this event's calendar can't be changed".into()));
        }
        let caldav = p.caldav_home.as_deref().is_some_and(|h| !h.is_empty());
        if !caldav && (is_server_calendar(&to.id) || is_server_calendar(&from)) {
            return Err(CmdError::Disabled("moving an event to another calendar isn't done on this account's server yet".into()));
        }
        let parts: Vec<StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid && e.calendar == from).cloned().collect();
        if parts.iter().any(|e| mailcraft_model::ical::details_hidden(&e.event)) {
            return Err(CmdError::Params("only this event's time can be seen: it can't be moved".into()));
        }
        if to.id == LOCAL && parts.iter().any(|e| !e.href.is_empty() && !e.event.attendees.is_empty()) {
            return Err(CmdError::Params("a meeting stays on the server: deleted there, its guests or organizer would be told".into()));
        }
        // Off the server: deleted there.
        let remote = if to.id == LOCAL { super::invite::delete_items(p, &parts) } else { true };
        for e in p.events.iter_mut().filter(|e| e.event.uid == uid && e.calendar == from) {
            e.calendar = to.id.clone();
            if to.id == LOCAL || from == LOCAL {
                e.href.clear();
                e.etag.clear();
            }
            // Kept where it's stored on the server, to be moved from there.
            e.dirty = to.id != LOCAL;
        }
        p.placed.retain(|u| u != uid || to.id == LOCAL);
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid, "account": account, "calendar": to.id, "moved": true}))
    }

    /// Keep an account's calendar on this computer as it is: moving its events isn't offered
    /// again.
    pub fn keep_local_events(&mut self, account: Option<&str>) -> CmdResult {
        let account = self.account_or_shown(account)?;
        self.pim_mut(&account).keep_here = true;
        self.save_pim(&account);
        Ok(serde_json::json!({"account": account}))
    }

    /// The calendar an account's new events and invitations go to (Default calendar); empty
    /// for the one MailCraft picks (the account's own on its server).
    pub fn set_default_calendar(&mut self, account: Option<&str>, calendar: &str) -> CmdResult {
        let account = self.account_or_shown(account)?;
        let calendar = calendar.trim();
        let p = self.pim_mut(&account);
        if calendar.is_empty() {
            p.chosen_calendar = None;
        } else {
            let c = super::calendar_named(p, calendar)?.ok_or_else(|| CmdError::Params(format!("no calendar `{calendar}`")))?;
            if c.read_only || mailcraft_model::pim::is_web_calendar(&c.id) {
                return Err(CmdError::Params(format!("`{}` can't be added to", c.name)));
            }
            // Only one of the account's own on its server, as the calendar pane offers: what's
            // put on this computer moves to the server's once it's found, so choosing it here
            // wouldn't hold, and someone else's calendar isn't one to fill unasked.
            if !is_server_calendar(&c.id) {
                return Err(CmdError::Params(format!("`{}` isn't one of the account's calendars on its server", c.name)));
            }
            p.chosen_calendar = Some(c.id.clone());
        }
        let chosen = p.default_calendar().map(|c| c.id.clone());
        self.save_pim(&account);
        Ok(serde_json::json!({"account": account, "calendar": chosen}))
    }
}
