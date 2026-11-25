//! Colleagues' calendars shared for their free/busy time alone (issue #140), as a company's
//! usually are: those times asked for from the account's server while the calendar shows them,
//! kept a few minutes, and drawn among the events as blocks that open nothing.

use std::collections::HashMap;

use mailcraft_model::pim::free_busy::{Busy, FreeBusyAnswer};
use mailcraft_model::pim::shared_calendar_owner;

use super::*;

/// How long times asked for are shown before they're asked for again (they change as people
/// plan), and how long after a failure.
const FRESH: i64 = 300;
const RETRY: i64 = 60;
/// An answer not come in this long (the account gone, a lost job): asked again.
const WAIT_MAX: i64 = 120;
/// The longest window asked for: what Exchange gives (42 days) less an hour, a change of clocks
/// making one local day an hour longer.
const WINDOW_MAX: i64 = 42 * 86_400 - 3600;
/// Days asked for each side of those shown, so the next and last week are there at once.
const AROUND: i64 = 7;

/// One person's times, as last asked for.
#[derive(Clone, Debug, Default)]
struct Known {
    /// The window the times are of, and when they came (`None`: never yet).
    from: i64,
    to: i64,
    answered: Option<i64>,
    busy: Vec<Busy>,
    error: Option<String>,
    /// When they work, when their server said (issue #185).
    work: Option<mailcraft_model::pim::work_hours::WorkHours>,
    /// Asked for and not answered yet: the window asked, and when.
    waiting: Option<(i64, i64, i64)>,
}

/// Every person's times asked for, by account and address (in lower case).
#[derive(Debug, Default)]
pub struct FreeBusyKept {
    known: HashMap<(String, String), Known>,
    /// The guests of a meeting being written, whose times its Scheduling Assistant shows (issue
    /// #277): their account, addresses and the day shown, until when they're kept fresh.
    assisting: Option<(String, Vec<String>, i64, i64)>,
}

/// How long a meeting's form keeps its guests' times fresh after it last asked.
const ASSIST_FOR: i64 = 15 * 60;
/// The most guests whose times a meeting's form asks for (a list pasted in is cut there).
const GUESTS_MAX: usize = 100;

/// What's known of one guest's times, for a meeting's Scheduling Assistant (issue #277).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuestTimes {
    /// Asked for, not come yet.
    Asking,
    /// Their times taken from one instant to another, and when they work when their server
    /// says.
    Known { from: i64, to: i64, busy: Vec<Busy>, work: Option<mailcraft_model::pim::work_hours::WorkHours> },
    /// None could be had: why.
    Unknown(String),
}

/// What's known of the times of a calendar shared for its free/busy time alone, to find a time
/// free for everyone (issue #186).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Covered {
    /// Their times from one instant to another.
    Window(i64, i64),
    /// They can't be had (not shared, not found): no times to go by.
    Unknown,
    /// Not come yet.
    Asking,
}

/// What a calendar shared for its free/busy time alone shows now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeBusyState<'a> {
    /// Asked for, not come yet.
    Asking,
    Shown,
    /// None could be had: why.
    Failed(&'a str),
}

impl Session {
    /// The window asked for while days `d0..d1` are shown: from the Monday a week before them
    /// (whole weeks: moving a week on needs nothing new), a week past them, cut to what
    /// Exchange gives.
    fn free_busy_window(&self, (d0, d1): (i64, i64)) -> (i64, i64) {
        let first = d0 - AROUND;
        let first = first - i64::from(weekday_index(first));
        let from = self.clock.midnight(first);
        (from, self.clock.midnight(d1 + AROUND).min(from.saturating_add(WINDOW_MAX)))
    }

    /// The people whose times each account's calendars shown need: (account, addresses).
    fn free_busy_wanted(&self) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = Vec::new();
        for (account, p) in &self.pim {
            let people: Vec<String> = p
                .calendars
                .iter()
                .filter(|c| c.free_busy && self.calendar_shown(account, &c.id))
                .filter_map(|c| shared_calendar_owner(&c.id))
                .map(str::to_string)
                .collect();
            if !people.is_empty() {
                out.push((account.clone(), people));
            }
        }
        out.sort();
        out
    }

    /// Ask for the times the calendar on screen needs and doesn't have (or has had a while), and
    /// those a meeting's form shows (issue #277). Called each tick: nothing unless the calendar
    /// is shown with such a calendar ticked, or a meeting's guests were asked for lately.
    pub(crate) fn free_busy_due(&mut self) {
        if let Some((account, guests, day, until)) = self.free_busy.assisting.clone() {
            if self.clock.now < until {
                self.ask_times(&account, guests, self.free_busy_window((day, day + 1)), (self.clock.midnight(day), self.clock.midnight(day + 1)));
            } else {
                self.free_busy.assisting = None;
            }
        }
        if self.module != Module::Calendar {
            return;
        }
        let wanted = self.free_busy_wanted();
        if wanted.is_empty() {
            return;
        }
        let (d0, d1) = self.cal_days();
        let shown = (self.clock.midnight(d0), self.clock.midnight(d1));
        let window = self.free_busy_window((d0, d1));
        for (account, people) in wanted {
            self.ask_times(&account, people, window, shown);
        }
    }

    /// Ask `account`'s server for `people`'s times over `from..to`, those not known as far as
    /// `shown_from..shown_to` reaches, or known a while.
    fn ask_times(&mut self, account: &str, people: Vec<String>, (from, to): (i64, i64), (shown_from, shown_to): (i64, i64)) {
        let now = self.clock.now;
        let account = account.to_string();
        let ask: Vec<String> = people
            .into_iter()
            .filter(|email| {
                let Some(k) = self.free_busy.known.get(&(account.clone(), email.clone())) else { return true };
                if k.waiting.is_some_and(|(_, _, at)| now < at.saturating_add(WAIT_MAX)) {
                    return false;
                }
                // What's on screen, as far as a window from where these start reaches.
                let covered = k.from <= shown_from && k.to >= shown_to.min(k.from.saturating_add(WINDOW_MAX));
                let stale = k.answered.is_none_or(|at| now >= at.saturating_add(if k.error.is_some() { RETRY } else { FRESH }));
                !covered || stale
            })
            .collect();
        if ask.is_empty() {
            return;
        }
        for email in &ask {
            self.free_busy.known.entry((account.clone(), email.clone())).or_default().waiting = Some((from, to, now));
        }
        self.jobs.push(Job::FreeBusy { account, people: ask, from, to });
    }

    /// A meeting's guests (their addresses) whose times its Scheduling Assistant shows around
    /// `day` (issue #277): asked for of `account`'s server now, and kept fresh while it's open.
    pub fn want_guest_times(&mut self, account: &str, guests: &[String], day: i64) {
        let mine = self.my_addresses();
        let mut people: Vec<String> =
            guests.iter().map(|g| g.trim().to_ascii_lowercase()).filter(|g| mailcraft_model::addr::is_valid_email(g) && !mine.contains(g)).collect();
        people.sort();
        people.dedup();
        people.truncate(GUESTS_MAX);
        if people.is_empty() {
            self.free_busy.assisting = None;
            return;
        }
        let shown = (self.clock.midnight(day), self.clock.midnight(day + 1));
        self.ask_times(account, people.clone(), self.free_busy_window((day, day + 1)), shown);
        self.free_busy.assisting = Some((account.to_string(), people, day, self.clock.now.saturating_add(ASSIST_FOR)));
    }

    /// What's known of a guest's times through `account` (issue #277).
    pub fn guest_times(&self, account: &str, email: &str) -> GuestTimes {
        match self.free_busy.known.get(&(account.to_string(), email.trim().to_ascii_lowercase())) {
            Some(k) if k.answered.is_some() && k.error.is_some() => GuestTimes::Unknown(k.error.clone().unwrap_or_default()),
            Some(k) if k.answered.is_some() => GuestTimes::Known { from: k.from, to: k.to, busy: k.busy.clone(), work: k.work.clone() },
            _ => GuestTimes::Asking,
        }
    }

    /// The times asked for over `from..to` came in: kept, each person's (an answer to a window
    /// asked before the one waited for now is kept too, but the wait goes on).
    pub(crate) fn free_busy_answered(&mut self, account: &str, from: i64, to: i64, answers: Vec<FreeBusyAnswer>) {
        let now = self.clock.now;
        for a in answers {
            let k = self.free_busy.known.entry((account.to_string(), a.email.trim().to_ascii_lowercase())).or_default();
            if k.waiting.is_some_and(|(f, t, _)| (f, t) == (from, to)) {
                k.waiting = None;
            }
            // An older answer than the one shown doesn't replace it.
            if k.waiting.is_some() && k.answered.is_some() {
                continue;
            }
            *k = Known { from, to, answered: Some(now), busy: a.busy, error: a.error, work: a.work, waiting: k.waiting };
        }
    }

    /// When the owner of a calendar shared for its free/busy time alone works, when their server
    /// said (issue #185): `None` for any other calendar, or when it didn't.
    pub fn calendar_work(&self, account: &str, calendar: &str) -> Option<&mailcraft_model::pim::work_hours::WorkHours> {
        let free_busy = self.pim.get(account)?.calendars.iter().any(|c| c.id == calendar && c.free_busy);
        let email = shared_calendar_owner(calendar).filter(|_| free_busy)?;
        self.free_busy.known.get(&(account.to_string(), email.to_string()))?.work.as_ref()
    }

    /// What's known of a calendar shared for its free/busy time alone, whose owner it is: `None`
    /// for any other calendar.
    pub(crate) fn free_busy_covered(&self, account: &str, calendar: &str) -> Option<(String, Covered)> {
        let free_busy = self.pim.get(account)?.calendars.iter().any(|c| c.id == calendar && c.free_busy);
        let email = shared_calendar_owner(calendar).filter(|_| free_busy)?.to_string();
        let covered = match self.free_busy.known.get(&(account.to_string(), email.clone())) {
            // Asked for again (further on, or as they're refreshed): what comes is waited for.
            Some(k) if k.waiting.is_some() => Covered::Asking,
            Some(k) if k.answered.is_some() && k.error.is_some() => Covered::Unknown,
            Some(k) if k.answered.is_some() => Covered::Window(k.from, k.to),
            _ => Covered::Asking,
        };
        Some((email, covered))
    }

    /// What a calendar shared for its free/busy time alone shows: `None` for any other.
    pub fn free_busy_state(&self, account: &str, calendar: &str) -> Option<FreeBusyState<'_>> {
        let free_busy = self.pim.get(account)?.calendars.iter().any(|c| c.id == calendar && c.free_busy);
        let email = shared_calendar_owner(calendar).filter(|_| free_busy)?;
        let Some(k) = self.free_busy.known.get(&(account.to_string(), email.to_string())).filter(|k| k.answered.is_some()) else {
            return Some(FreeBusyState::Asking);
        };
        Some(match &k.error {
            Some(e) => FreeBusyState::Failed(e),
            None => FreeBusyState::Shown,
        })
    }

    /// The free/busy times of the calendars shown that have only those, overlapping
    /// `[from, to)`, as agenda items (their subject when shared; [`AgendaItem::busy_only`]).
    pub(crate) fn free_busy_items(&self, from: i64, to: i64, out: &mut Vec<AgendaItem>) {
        for (account, p) in &self.pim {
            let Some(acc) = self.account(account) else { continue };
            for cal in p.calendars.iter().filter(|c| c.free_busy && self.calendar_shown(account, &c.id)) {
                let Some(email) = shared_calendar_owner(&cal.id) else { continue };
                let Some(k) = self.free_busy.known.get(&(account.clone(), email.to_string())) else { continue };
                for b in k.busy.iter().filter(|b| b.start < to && b.end.max(b.start.saturating_add(1)) > from) {
                    // A whole day or more from midnight to midnight is a day's: drawn as one.
                    let midnight = |t: i64| self.clock.local(t).rem_euclid(86_400) == 0;
                    let all_day = b.end.saturating_sub(b.start) >= 86_400 && midnight(b.start) && midnight(b.end);
                    out.push(AgendaItem {
                        account: account.clone(),
                        calendar: cal.id.clone(),
                        uid: format!("busy:{email}:{}", b.start),
                        occurrence: b.start,
                        recurring: false,
                        summary: if b.private { String::new() } else { b.subject.clone() },
                        location: if b.private { String::new() } else { b.location.clone() },
                        start: b.start,
                        end: b.end,
                        all_day,
                        color: Session::calendar_color(acc, cal),
                        category: None,
                        status: if b.show_as == ical::ShowAs::Tentative { "TENTATIVE".into() } else { String::new() },
                        needs_reply: false,
                        privacy: if b.private { ical::Privacy::Private } else { ical::Privacy::Public },
                        show_as: b.show_as,
                        task: false,
                        busy_only: true,
                    });
                }
            }
        }
    }
}
