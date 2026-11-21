//! Calendar groups (issue #140), as Outlook's: a name, and people whose calendars are shown and
//! hidden together under it. Each member's calendar is one opened from the account (Open shared
//! calendar), read as any is, or by its free/busy times when that's all they share.

use mailcraft_model::pim::{CalendarGroup, shared_calendar_id};

use super::*;

/// The longest group name taken, and the most people in a group.
const NAME_MAX: usize = 100;
const MEMBERS_MAX: usize = 50;

impl Session {
    /// The Exchange account a calendar group or shared calendar is opened from: the one named,
    /// else the first.
    pub(super) fn exchange_account(&self, account: Option<&str>) -> Result<String, CmdError> {
        let account = match account {
            Some(a) => self.account_or_shown(Some(a))?,
            None => self
                .accounts
                .iter()
                .find(|a| a.config.graph.is_some() || a.config.ews.is_some())
                .map(|a| a.config.id.clone())
                .ok_or_else(|| CmdError::Disabled("other people's calendars open from an Exchange account".into()))?,
        };
        if !self.account(&account).is_some_and(|a| a.config.graph.is_some() || a.config.ews.is_some()) {
            return Err(CmdError::Params("other people's calendars open from an Exchange account".into()));
        }
        Ok(account)
    }

    /// Ask for the calendars of `people` just opened (issue #208): read on their own, in
    /// seconds, not with the next whole calendar pass, which took minutes on a big mailbox and
    /// waited for one under way. With no one new, the whole sync as before.
    pub(super) fn read_people(&mut self, account: &str, people: Vec<String>) {
        let server = self.account(account).is_some_and(|a| a.config.has_server());
        if people.is_empty() || !server {
            self.after_pim_change(account, true);
            return;
        }
        self.save_pim(account);
        self.jobs.push(Job::ReadShared { account: account.to_string(), people });
    }

    /// Open the calendars of everyone in `emails` (addresses as a recipient box takes them, the
    /// address book's way, issue #140): one refused says why, and the rest open all the same.
    pub fn open_shared_calendars(&mut self, account: Option<&str>, emails: &str) -> CmdResult {
        let list = mailcraft_model::addr::parse_list(emails).map_err(|e| CmdError::Params(format!("not a list of addresses: {e}")))?;
        if list.len() <= 1 {
            let one = list.first().map_or(emails.trim(), |a| a.email.as_str()).to_string();
            return self.open_shared_calendar(account, &one);
        }
        let (mut opened, mut refused) = (Vec::new(), Vec::new());
        for a in list.iter().take(MEMBERS_MAX) {
            match self.open_shared_calendar(account, &a.email) {
                Ok(v) => opened.push(v),
                Err(e) => refused.push(format!("{}: {e}", a.email)),
            }
        }
        if opened.is_empty() {
            return Err(CmdError::Params(refused.join("; ")));
        }
        Ok(serde_json::json!({"opened": opened, "refused": refused}))
    }

    /// Make a calendar group named `name` of the people in `members` (addresses, as typed in a
    /// recipient box), each one's calendar opened; to a group of that name already, they're added.
    pub fn calendar_group_create(&mut self, account: Option<&str>, name: &str, members: &str) -> CmdResult {
        let account = self.exchange_account(account)?;
        let name: String = name.trim().chars().filter(|c| !c.is_control()).take(NAME_MAX).collect();
        if name.is_empty() {
            return Err(CmdError::Params("a calendar group needs a name".into()));
        }
        let list = mailcraft_model::addr::parse_list(members).map_err(|e| CmdError::Params(format!("not a list of addresses: {e}")))?;
        let own = self.account(&account).map(|a| a.config.email.to_ascii_lowercase()).unwrap_or_default();
        let mut people: Vec<String> = Vec::new();
        for a in list {
            let email = a.email.trim().to_ascii_lowercase();
            if email.contains('@') && email != own && !people.contains(&email) {
                people.push(email);
            }
        }
        if people.is_empty() {
            return Err(CmdError::Params("a calendar group needs someone in it (not this account itself)".into()));
        }
        let p = self.pim_mut(&account);
        let at = p.groups.iter().position(|g| g.name.eq_ignore_ascii_case(&name));
        let had = at.and_then(|i| p.groups.get(i)).map_or(0, |g| g.members.len());
        if had + people.iter().filter(|e| at.and_then(|i| p.groups.get(i)).is_none_or(|g| !g.members.contains(e))).count() > MEMBERS_MAX {
            return Err(CmdError::Params(format!("a calendar group holds at most {MEMBERS_MAX} people")));
        }
        let new_ones: Vec<String> = people.iter().filter(|e| !p.shared.contains(e)).cloned().collect();
        if p.shared.len() + new_ones.len() > MEMBERS_MAX {
            return Err(CmdError::Params(format!("at most {MEMBERS_MAX} other calendars can be open")));
        }
        for e in &people {
            if !p.shared.contains(e) {
                p.shared.push(e.clone());
            }
        }
        match at.and_then(|i| p.groups.get_mut(i)) {
            Some(g) => {
                for e in people {
                    if !g.members.contains(&e) {
                        g.members.push(e);
                    }
                }
            }
            None => p.groups.push(CalendarGroup { name: name.clone(), members: people }),
        }
        self.read_people(&account, new_ones);
        Ok(serde_json::json!({"account": account, "group": name}))
    }

    /// Delete a calendar group: its people's calendars close too, but those in another group.
    pub fn calendar_group_delete(&mut self, account: Option<&str>, name: &str) -> CmdResult {
        let account = self.exchange_account(account)?;
        let p = self.pim_mut(&account);
        let Some(i) = p.groups.iter().position(|g| g.name.eq_ignore_ascii_case(name.trim())) else {
            return Err(CmdError::Params(format!("no calendar group `{}`", name.trim())));
        };
        let gone = p.groups.remove(i);
        let closing: Vec<String> = gone.members.into_iter().filter(|e| !p.groups.iter().any(|g| g.members.contains(e))).collect();
        for e in &closing {
            let id = shared_calendar_id(e);
            p.shared.retain(|x| x != e);
            p.calendars.retain(|c| c.id != id);
            p.events.retain(|ev| ev.calendar != id);
        }
        self.after_pim_change(&account, false);
        Ok(serde_json::json!({"deleted": gone.name, "closed": closing}))
    }

    /// Show or hide every calendar of a group.
    pub fn calendar_group_show(&mut self, account: Option<&str>, name: &str, shown: bool) -> CmdResult {
        let account = self.exchange_account(account)?;
        let members = self
            .pim
            .get(&account)
            .and_then(|p| p.groups.iter().find(|g| g.name.eq_ignore_ascii_case(name.trim())))
            .map(|g| g.members.clone())
            .ok_or_else(|| CmdError::Params(format!("no calendar group `{}`", name.trim())))?;
        for e in &members {
            let key = Session::calendar_key(&account, &shared_calendar_id(e));
            if shown {
                self.prefs.hidden_calendars.remove(&key);
            } else {
                self.prefs.hidden_calendars.insert(key);
            }
        }
        self.pim_revision = self.pim_revision.wrapping_add(1);
        Ok(serde_json::json!({"group": name.trim(), "shown": shown}))
    }

    /// The calendar groups of an account, and whether each is shown (every calendar of it ticked).
    pub fn calendar_groups(&self, account: &str) -> Vec<(&CalendarGroup, bool)> {
        let Some(p) = self.pim.get(account) else { return Vec::new() };
        p.groups.iter().map(|g| (g, g.members.iter().all(|e| self.calendar_shown(account, &shared_calendar_id(e))))).collect()
    }
}
