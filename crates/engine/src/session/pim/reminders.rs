//! Reminders (File › Reminders, as Outlook's Reminders window lists them): events' and tasks'
//! due now, and the answers given to them (dismissed, snoozed), kept with the preferences.

use super::*;

/// How long after an event has ended its reminder still waits to be answered.
pub(super) const REMINDER_KEPT_AFTER_END: i64 = 86_400;
/// Most reminders listed at once.
const REMINDERS_MAX: usize = 200;

impl Session {
    /// The reminders due at `now`, soonest event first: occurrences of the user's own calendars
    /// (shown or hidden; not others' shared with them) whose reminder time has passed, that
    /// haven't ended a day ago, weren't dismissed, and aren't snoozed past `now`; and tasks'.
    pub fn due_reminders(&self, now: i64) -> Vec<DueReminder> {
        let ahead = ical::REMINDER_MAX_MINUTES * 60;
        let mut out = Vec::new();
        for (account, p) in &self.pim {
            if self.account(account).is_none() {
                continue;
            }
            // Not others' calendars, nor ones subscribed to (a holiday list's alarms are its own).
            let own = |c: &&mailcraft_model::pim::CalendarInfo| {
                mailcraft_model::pim::shared_calendar_owner(&c.id).is_none() && !mailcraft_model::pim::is_web_calendar(&c.id)
            };
            for cal in p.calendars.iter().filter(own) {
                // Only the series with a reminder somewhere (an exception may have its own).
                let with: std::collections::BTreeSet<&str> = p
                    .events
                    .iter()
                    .filter(|e| e.calendar == cal.id && ical::reminder_minutes(&e.event).is_some())
                    .map(|e| e.event.uid.as_str())
                    .collect();
                if with.is_empty() {
                    continue;
                }
                let events: Vec<&CalEvent> =
                    p.events.iter().filter(|e| e.calendar == cal.id && with.contains(e.event.uid.as_str())).map(|e| &e.event).collect();
                for o in ical::occurrences(&events, now - REMINDER_KEPT_AFTER_END - 86_400, now + ahead + 86_400) {
                    let Some(e) = events.get(o.event) else { continue };
                    let Some(minutes) = ical::reminder_minutes(e) else { continue };
                    if e.status.eq_ignore_ascii_case("CANCELLED") {
                        continue;
                    }
                    let (start, end) = if e.all_day { (self.clock.utc(o.start), self.clock.utc(o.end)) } else { (o.start, o.end) };
                    let occurrence = e.recurrence_id.unwrap_or(o.start);
                    let key = format!("{account}|{}|{occurrence}", e.uid);
                    // Dismissed in another calendar program, which wrote so into the event
                    // (RFC 9074's `ACKNOWLEDGED`).
                    if ical::acknowledged(e).is_some_and(|at| at >= start - minutes * 60) && !self.prefs.reminders.contains_key(&key) {
                        continue;
                    }
                    let due = match self.prefs.reminders.get(&key) {
                        Some(0) => continue,
                        Some(until) => *until,
                        None => start - minutes * 60,
                    };
                    if due > now || end.max(start) < now - REMINDER_KEPT_AFTER_END {
                        continue;
                    }
                    out.push(DueReminder {
                        key,
                        account: account.clone(),
                        uid: e.uid.clone(),
                        occurrence,
                        summary: e.summary.clone(),
                        location: e.location.clone(),
                        start,
                        end,
                        all_day: e.all_day,
                        due,
                        task: false,
                    });
                }
            }
        }
        out.extend(self.task_reminders(now));
        out.sort_by(|a, b| (a.start, &a.summary, &a.key).cmp(&(b.start, &b.summary, &b.key)));
        out.dedup_by(|a, b| a.key == b.key);
        out.truncate(REMINDERS_MAX);
        out
    }

    /// Dismiss reminders: the one named by `key`, or every one due at `now` (`None`).
    pub fn dismiss_reminders(&mut self, key: Option<&str>, now: i64) -> usize {
        let keys: Vec<String> = match key {
            Some(k) => vec![k.to_string()],
            None => self.due_reminders(now).into_iter().map(|r| r.key).collect(),
        };
        for k in &keys {
            self.prefs.reminders.insert(k.clone(), 0);
            self.tell_server_reminder(k, None, now);
        }
        self.forget_old_reminders(now);
        keys.len()
    }

    /// Snooze a reminder until `until` (Unix seconds).
    pub fn snooze_reminder(&mut self, key: &str, until: i64) {
        self.prefs.reminders.insert(key.to_string(), until.max(1));
        self.tell_server_reminder(key, Some(until.max(1)), now_or(until));
        self.forget_old_reminders(now_or(until));
    }

    /// Tell the event's server a reminder was answered here, so the user's other programs
    /// (Outlook) don't show it again (issue #40): an Exchange server through its own action at
    /// the next sync ([`ReminderAnswer`]); a CalDAV server by the event's alarm acknowledged at
    /// `now` (RFC 9074), which every client reads. A snooze stays here on a CalDAV server.
    fn tell_server_reminder(&mut self, key: &str, snooze_until: Option<i64>, now: i64) {
        let mut parts = key.rsplitn(3, '|');
        let (Some(occurrence), Some(uid), Some(account)) = (parts.next().and_then(|o| o.parse::<i64>().ok()), parts.next(), parts.next()) else {
            return;
        };
        let Some(config) = self.account(account).map(|a| a.config.clone()) else { return };
        if !config.has_server() {
            return;
        }
        let exchange = config.graph.is_some() || config.ews.is_some();
        let p = self.pim_mut(account);
        let writable = |cal: &str| {
            cal != LOCAL
                && mailcraft_model::pim::shared_calendar_owner(cal).is_none()
                && !mailcraft_model::pim::is_web_calendar(cal)
                && !p.calendars.iter().any(|c| c.id == cal && c.read_only)
        };
        // The occurrence's own copy (a changed one), else its series.
        let at = p
            .events
            .iter()
            .position(|e| e.event.uid == uid && e.event.recurrence_id == Some(occurrence))
            .or_else(|| p.events.iter().position(|e| e.event.uid == uid && e.event.recurrence_id.is_none()));
        // Exchange needs the item's id; a CalDAV event not stored yet takes the answer with it.
        let Some(i) = at.filter(|&i| p.events.get(i).is_some_and(|e| writable(&e.calendar) && (!exchange || !e.href.is_empty()))) else { return };
        let Some(stored) = p.events.get(i).cloned() else { return };
        if exchange {
            p.reminder_answers.retain(|a| !(a.uid == uid && a.occurrence == occurrence));
            if p.reminder_answers.len() >= mailcraft_model::pim::MAX_REMINDER_ANSWERS {
                p.reminder_answers.remove(0);
            }
            p.reminder_answers.push(mailcraft_model::pim::ReminderAnswer {
                calendar: stored.calendar.clone(),
                href: stored.href.clone(),
                uid: uid.to_string(),
                occurrence,
                series: stored.event.rrule.is_some() && stored.event.recurrence_id.is_none(),
                snooze_until,
            });
        } else if snooze_until.is_none() {
            let Some(e) = p.events.get_mut(i) else { return };
            ical::acknowledge(&mut e.event, now);
            e.dirty = true;
        } else {
            return;
        }
        self.after_pim_change(account, true);
    }

    /// Answers for occurrences long past go (and the list is kept small).
    fn forget_old_reminders(&mut self, now: i64) {
        let cutoff = now - 30 * 86_400;
        self.prefs.reminders.retain(|k, _| k.rsplit('|').next().and_then(|t| t.parse::<i64>().ok()).is_none_or(|t| t >= cutoff));
        while self.prefs.reminders.len() > 5000 {
            let Some(first) = self.prefs.reminders.keys().next().cloned() else { break };
            self.prefs.reminders.remove(&first);
        }
    }
}
