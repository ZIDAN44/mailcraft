//! An account's calendar on this computer and its calendars on the server (issue #30): which
//! one new events and invitations go to (the Default calendar), and moving what was kept on
//! this computer before the server's were found.

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
        let account = self.pim_account(account)?;
        if self.account(&account).is_some_and(AccountState::is_local) {
            return Err(CmdError::Disabled("this account's calendar is on this computer only".into()));
        }
        let p = self.pim_mut(&account);
        let target = match calendar.map(str::trim).filter(|c| !c.is_empty()) {
            Some(id) => p.calendars.iter().find(|c| c.id == id).ok_or_else(|| CmdError::Params(format!("no calendar `{id}`")))?,
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

    /// Keep an account's calendar on this computer as it is: moving its events isn't offered
    /// again.
    pub fn keep_local_events(&mut self, account: Option<&str>) -> CmdResult {
        let account = self.pim_account(account)?;
        self.pim_mut(&account).keep_here = true;
        self.save_pim(&account);
        Ok(serde_json::json!({"account": account}))
    }

    /// The calendar an account's new events and invitations go to (Default calendar); empty
    /// for the one MailCraft picks (the account's own on its server).
    pub fn set_default_calendar(&mut self, account: Option<&str>, calendar: &str) -> CmdResult {
        let account = self.pim_account(account)?;
        let calendar = calendar.trim();
        let p = self.pim_mut(&account);
        if calendar.is_empty() {
            p.chosen_calendar = None;
        } else {
            let c = p.calendars.iter().find(|c| c.id == calendar).ok_or_else(|| CmdError::Params(format!("no calendar `{calendar}`")))?;
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
