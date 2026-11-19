//! Flagged mail's reminders (gap 9), in the Reminders window with events' and tasks': a flag
//! given a reminder (Follow Up › Add Reminder…) reminds when its time comes, as Outlook's do.
//! Exchange and Microsoft 365 keep the reminder on the message, as Outlook sets it; for other
//! accounts MailCraft keeps it ([`Prefs::mail_reminders`]).
//!
//! Dismissed, the flag reminds no more; snoozed, it reminds again then: both change the
//! reminder itself (on the server that keeps it, so Outlook doesn't remind of it either), not an
//! answer kept beside it as an event's is.

use mailcraft_model::{MessageKey, Op};

use super::super::reminders::ReminderKey;
use super::*;

/// What a flagged message's reminder's uid begins with ([`ReminderKey`]), its follow-up id
/// after it: no event's or task's.
const MAIL: &str = "mail:";

impl Session {
    /// Whether flagged mail may have reminders to look for: an account whose server keeps them,
    /// or one kept here. The flagged mail of every folder is read then (as the To-Do Bar's).
    pub fn mail_reminders_wanted(&self) -> bool {
        !self.prefs.mail_reminders.is_empty() || self.accounts.iter().any(|a| self.server_keeps_follow_up(&a.config.id))
    }

    /// The flagged messages' reminders due at `now`: each once, though it's in two folders.
    pub(in crate::session::pim) fn mail_reminders(&self, now: i64) -> Vec<DueReminder> {
        let mut out: Vec<DueReminder> = Vec::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for ((account, _), list) in &self.todo.read {
            if self.account(account).is_none() {
                continue;
            }
            for m in list.iter().filter(|m| !m.flags.follow_up_complete()) {
                let Some(at) = self.message_reminder(m).filter(|&at| at <= now) else { continue };
                let id = m.follow_up_id();
                if !seen.insert((account.clone(), id.clone())) {
                    continue;
                }
                let uid = format!("{MAIL}{id}");
                // Due on a day: that day, as a task's; else when it reminds.
                let (start, end, all_day) = match self.follow_up_due(m) {
                    Some(day) => {
                        let start = self.clock.midnight(day);
                        (start, start.saturating_add(86_400), true)
                    }
                    None => (at, at, false),
                };
                let bare = mailcraft_model::thread::without_prefixes(&m.subject);
                out.push(DueReminder {
                    key: ReminderKey { account, uid: &uid, at }.to_string(),
                    account: account.clone(),
                    uid: id,
                    occurrence: at,
                    summary: if bare.trim().is_empty() { m.subject.clone() } else { bare },
                    location: String::new(),
                    start,
                    end,
                    all_day,
                    due: at,
                    task: false,
                    message: Some(m.key.clone()),
                });
            }
        }
        out
    }

    /// Answer a flagged message's reminder (`key`): dismissed (`None`), or snoozed until that
    /// time. Whether `key` was one.
    pub(in crate::session::pim) fn answer_mail_reminder(&mut self, key: &str, snooze_until: Option<i64>) -> bool {
        let Some(ReminderKey { account, uid, at }) = ReminderKey::parse(key) else { return false };
        let Some(id) = uid.strip_prefix(MAIL) else { return false };
        // Its copies in every folder still reminding at that time (a snooze moved it on, or
        // another program answered it).
        let found: Vec<SharedSummary> = self
            .todo
            .read
            .iter()
            .filter(|((a, _), _)| a == account)
            .flat_map(|(_, list)| list.iter())
            .filter(|m| m.follow_up_id() == id && self.message_reminder(m) == Some(at))
            .cloned()
            .collect();
        if found.is_empty() {
            return true;
        }
        let account = account.to_string();
        if self.server_keeps_follow_up(&account) {
            let keys: Vec<MessageKey> = found.iter().map(|m| m.key.clone()).collect();
            self.apply_op(&account, Op::SetReminder { keys, at: snooze_until });
        } else {
            match snooze_until {
                Some(until) => {
                    self.prefs.mail_reminders.insert(id.to_string(), until);
                }
                None => {
                    self.prefs.mail_reminders.remove(id);
                }
            }
        }
        true
    }

    /// The To-Do Bar's copy of flagged mail follows a change to a flag's day or reminder made
    /// here (the server's answer reads the folder again later): or a dismissed reminder would
    /// come back until then.
    pub(super) fn todo_follow_up_op(&mut self, op: &Op) {
        let (keys, change): (&[MessageKey], &dyn Fn(&mut mailcraft_model::Flags)) = match op {
            Op::SetReminder { keys, at } => (keys, &|f| f.reminder = *at),
            Op::SetFollowUp { keys, due } => (keys, &|f| f.follow_up_due = due.map(|d| d.day)),
            _ => return,
        };
        let keys: HashSet<&MessageKey> = keys.iter().collect();
        for list in self.todo.read.values_mut() {
            for m in list.iter_mut().filter(|m| keys.contains(&m.key)) {
                change(&mut SharedSummary::make_mut(m).flags);
            }
        }
    }
}
