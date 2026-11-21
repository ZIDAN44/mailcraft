//! Reminders (File › Reminders, as Outlook's Reminders window lists them): events', tasks' and
//! flagged mail's due now, and the answers given to them (dismissed, snoozed), kept with the
//! preferences (a flag's change the flag itself: `todo/flag_reminders.rs`).

use super::*;

/// How long after an event has ended its reminder still waits to be answered.
pub(super) const REMINDER_KEPT_AFTER_END: i64 = 86_400;
/// Most reminders listed at once.
const REMINDERS_MAX: usize = 200;
/// Most answers to reminders kept (the oldest reminders' go first).
const ANSWERS_MAX: usize = 5000;

/// What names a reminder, as [`DueReminder::key`] and the answers kept with the preferences
/// have it: `<account>|<uid>|<time>` (an occurrence's start, or a task's reminder time). The
/// account is up to the first `|` (an account's id has none), the time after the last; the uid,
/// a server's, may have `|` in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReminderKey<'a> {
    pub(crate) account: &'a str,
    pub(crate) uid: &'a str,
    pub(crate) at: i64,
}

impl<'a> ReminderKey<'a> {
    pub(crate) fn parse(key: &'a str) -> Option<ReminderKey<'a>> {
        let (account, rest) = key.split_once('|')?;
        let (uid, at) = rest.rsplit_once('|')?;
        Some(ReminderKey { account, uid, at: at.parse().ok()? })
    }
}

impl std::fmt::Display for ReminderKey<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}|{}|{}", self.account, self.uid, self.at)
    }
}

impl Session {
    /// The reminders due at `now`, soonest event first: occurrences of the user's own calendars
    /// (shown or hidden; not others' shared with them) whose reminder time has passed, that
    /// haven't ended a day ago, weren't dismissed, and aren't snoozed past `now`; tasks'; and
    /// flagged mail's.
    pub fn due_reminders(&self, now: i64) -> Vec<DueReminder> {
        let ahead = ical::REMINDER_MAX_MINUTES * 60;
        let mine = self.my_addresses();
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
                    let minutes = ical::reminders_minutes(e);
                    if minutes.is_empty() || e.status.eq_ignore_ascii_case("CANCELLED") || super::invite::declined_occurrence(e, &mine) {
                        continue;
                    }
                    let (start, end) = if e.all_day { (self.clock.utc(o.start), self.clock.utc(o.end)) } else { (o.start, o.end) };
                    let occurrence = e.recurrence_id.unwrap_or(o.start);
                    let key = ReminderKey { account, uid: &e.uid, at: occurrence }.to_string();
                    let due = match self.prefs.reminders.get(&key).copied() {
                        // Dismissed before an event could remind more than once: for good.
                        Some(0) => continue,
                        Some(until) if until > 0 => until,
                        // Each of its reminders comes in turn (issue #224: "an hour before and 2
                        // minutes before" reminded once): the latest passed since it was dismissed
                        // here (when, kept negative) or in another calendar program, which wrote
                        // so into the event (RFC 9074's `ACKNOWLEDGED`); else the next.
                        answer => {
                            let since = answer.map(|v| v.saturating_neg()).max(ical::acknowledged(e));
                            let times = minutes.iter().map(|m| start.saturating_sub(m.saturating_mul(60))).filter(|t| since.is_none_or(|s| *t > s));
                            let Some(due) = times.clone().filter(|t| *t <= now).max().or_else(|| times.min()) else { continue };
                            due
                        }
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
                        message: None,
                    });
                }
            }
        }
        out.extend(self.task_reminders(now));
        out.extend(self.mail_reminders(now));
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
            if self.answer_mail_reminder(k, None) {
                continue;
            }
            // A task's for good; an event's with when, negative, for its reminders after it.
            let answer = if self.is_task_reminder(k) { 0 } else { now.max(1).saturating_neg() };
            self.prefs.reminders.insert(k.clone(), answer);
            self.tell_server_reminder(k, None, now);
        }
        self.forget_old_reminders(now);
        self.pim_revision = self.pim_revision.wrapping_add(1);
        keys.len()
    }

    /// Snooze a reminder until `until` (Unix seconds).
    pub fn snooze_reminder(&mut self, key: &str, until: i64) {
        let now = self.clock.now;
        if self.answer_mail_reminder(key, Some(until.max(1))) {
            self.pim_revision = self.pim_revision.wrapping_add(1);
            return;
        }
        self.prefs.reminders.insert(key.to_string(), until.max(1));
        self.tell_server_reminder(key, Some(until.max(1)), now);
        self.forget_old_reminders(now);
        self.pim_revision = self.pim_revision.wrapping_add(1);
    }

    /// Whether `key` names a task's reminder: the task with its uid that reminds at its time, not
    /// an event that happens to have a task's uid.
    fn is_task_reminder(&self, key: &str) -> bool {
        let Some(ReminderKey { account, uid, at }) = ReminderKey::parse(key) else { return false };
        self.pim.get(account).is_some_and(|p| p.tasks.iter().any(|t| t.task.uid == uid && t.task.reminder == Some(at)))
    }

    /// Tell the event's server a reminder was answered here, so the user's other programs
    /// (Outlook) don't show it again (issue #40): an Exchange server through its own action at
    /// the next sync ([`ReminderAnswer`]); a CalDAV server by the event's alarm acknowledged at
    /// `now` (RFC 9074), which every client reads. A snooze stays here on a CalDAV server.
    fn tell_server_reminder(&mut self, key: &str, snooze_until: Option<i64>, now: i64) {
        let Some(ReminderKey { account, uid, at: occurrence }) = ReminderKey::parse(key) else { return };
        // A task's reminder is answered here alone (its server was told nothing).
        if self.is_task_reminder(key) {
            return;
        }
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

    /// Answers whose reminder can't be listed any more go ([`Session::reminder_may_be_listed`]),
    /// and the list is kept small, the oldest reminders' going first. They went by the
    /// occurrence's start, a month on: a dismissed reminder of an event that went on longer, or of
    /// a task due later than it reminded, came back (at once, when a snooze took its end for
    /// "now").
    fn forget_old_reminders(&mut self, now: i64) {
        let gone: Vec<String> = self.prefs.reminders.keys().filter(|k| !self.reminder_may_be_listed(k, now)).cloned().collect();
        for k in gone {
            self.prefs.reminders.remove(&k);
        }
        let over = self.prefs.reminders.len().saturating_sub(ANSWERS_MAX);
        if over > 0 {
            let mut by_time: Vec<(i64, &String)> =
                self.prefs.reminders.keys().map(|k| (ReminderKey::parse(k).map_or(i64::MIN, |r| r.at), k)).collect();
            by_time.sort_unstable();
            let oldest: Vec<String> = by_time.into_iter().take(over).map(|(_, k)| k.clone()).collect();
            for k in oldest {
                self.prefs.reminders.remove(&k);
            }
        }
    }

    /// Whether the reminder `key` names (`account|uid|time`: an occurrence's start, or a task's
    /// reminder time) may still be listed at `now` or later: its event or task is there, the
    /// occurrence hasn't ended more than [`REMINDER_KEPT_AFTER_END`] ago (and a day to spare),
    /// the task isn't done and still reminds at that time.
    fn reminder_may_be_listed(&self, key: &str, now: i64) -> bool {
        let Some(ReminderKey { account, uid, at }) = ReminderKey::parse(key) else { return false };
        let Some(p) = self.pim.get(account) else { return false };
        let since = now.saturating_sub(REMINDER_KEPT_AFTER_END).saturating_sub(86_400);
        if let Some(t) = p.tasks.iter().map(|t| &t.task).find(|t| t.uid == uid) {
            let end = t.due.map_or(at, |d| self.clock.utc(d).saturating_add(86_400));
            return !t.is_done() && t.reminder == Some(at) && end >= since;
        }
        let events: Vec<&CalEvent> = p.events.iter().map(|e| &e.event).filter(|e| e.uid == uid).collect();
        // The occurrence's own copy, else the series' length from that start.
        let (end, all_day) = match events.iter().find(|e| e.recurrence_id == Some(at)) {
            Some(e) => (e.end, e.all_day),
            None => match events.iter().find(|e| e.recurrence_id.is_none()) {
                Some(m) => (at.saturating_add(m.end.saturating_sub(m.start).max(0)), m.all_day),
                None => return false,
            },
        };
        (if all_day { self.clock.utc(end) } else { end }) >= since
    }
}
