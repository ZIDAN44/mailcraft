//! Finding events (issue #33): the calendar's search, over what's kept here (offline, as mail
//! search is), in every calendar shown or the one asked for. A recurring series is found once,
//! at its next occurrence (or, when it's over, its last).

use mailcraft_model::ical::{EventQuery, Privacy, ShowAs};

use super::*;

/// Which events a calendar search lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventWhen {
    /// Upcoming ones first (the soonest first), then past ones (the latest first).
    #[default]
    All,
    Upcoming,
    Past,
}

impl EventWhen {
    pub const ALL: [EventWhen; 3] = [EventWhen::All, EventWhen::Upcoming, EventWhen::Past];

    pub fn code(self) -> &'static str {
        match self {
            EventWhen::All => "all",
            EventWhen::Upcoming => "upcoming",
            EventWhen::Past => "past",
        }
    }

    pub fn from_code(s: &str) -> Option<EventWhen> {
        EventWhen::ALL.into_iter().find(|w| w.code().eq_ignore_ascii_case(s.trim()))
    }
}

/// An event found, at the occurrence shown for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventHit {
    pub account: String,
    pub calendar: String,
    pub uid: String,
    /// The occurrence shown, as its rule names it (opens it).
    pub occurrence: i64,
    /// When that occurrence is (all-day ones from local midnight, as [`AgendaItem`]'s).
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub summary: String,
    pub location: String,
    /// A series (shown at one of its occurrences).
    pub recurring: bool,
    /// It hasn't ended yet.
    pub upcoming: bool,
    pub categories: Vec<String>,
    /// Its calendar's colour, and its first category's ([`AgendaItem`]'s).
    pub color: u8,
    pub category: Option<u8>,
    pub privacy: Privacy,
    pub show_as: ShowAs,
}

/// The occurrence a search shows of a series: which of its events, the start its rule names it
/// by, and when it is (all-day ones on the user's days).
struct Shown {
    event: usize,
    named: i64,
    start: i64,
    end: i64,
}

/// The most events a search lists.
pub const EVENT_HITS_MAX: usize = 1000;
/// How far ahead and back a series' occurrences are looked for: a year first, then twenty.
const NEAR: i64 = 400 * 86_400;
const FAR: i64 = 20 * 366 * 86_400;

impl Session {
    /// The events matching `query` ([`EventQuery`]): in every calendar shown, or only those of
    /// `account` (and of them `calendar`, by id or name). Upcoming ones first, soonest first,
    /// then past ones, latest first; at most `limit`.
    pub fn find_events(&self, query: &str, when: EventWhen, account: Option<&str>, calendar: Option<&str>, limit: usize) -> Vec<EventHit> {
        let q = EventQuery::parse(query);
        let now = self.clock.now;
        // The dates are the user's days.
        let lo = q.after.map(|d| self.local_midnight(d));
        let hi = q.before.map(|d| self.local_midnight(d));
        let mut hits = Vec::new();
        for a in &self.accounts {
            let id = &a.config.id;
            if account.is_some_and(|x| x != id) {
                continue;
            }
            let Some(p) = self.pim.get(id) else { continue };
            for cal in &p.calendars {
                let wanted = match calendar {
                    Some(c) => cal.id == c || cal.name.eq_ignore_ascii_case(c),
                    None => self.calendar_shown(id, &cal.id),
                };
                if !wanted {
                    continue;
                }
                // Each series with its changed occurrences, in the order they're kept.
                let mut series: Vec<Vec<&CalEvent>> = Vec::new();
                let mut at: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
                for e in p.events.iter().filter(|e| e.calendar == cal.id).map(|e| &e.event) {
                    match at.get(e.uid.as_str()).and_then(|i| series.get_mut(*i)) {
                        Some(list) => list.push(e),
                        None => {
                            at.insert(&e.uid, series.len());
                            series.push(vec![e]);
                        }
                    }
                }
                for list in series {
                    // The series when it matches (its changed occurrences with it); else each
                    // changed occurrence that does, on its own.
                    let master = list.iter().find(|e| e.recurrence_id.is_none()).copied();
                    let groups: Vec<Vec<CalEvent>> = match master {
                        Some(m) if q.matches(m) => vec![list.iter().map(|e| (*e).clone()).collect()],
                        _ => list.iter().filter(|e| e.recurrence_id.is_some() && q.matches(e)).map(|e| vec![(*e).clone()]).collect(),
                    };
                    for events in groups {
                        let Some(Shown { event: o, named, start, end }) = self.shown_occurrence(&events, when, now, lo, hi) else { continue };
                        let Some(e) = events.get(o) else { continue };
                        hits.push(EventHit {
                            account: id.clone(),
                            calendar: cal.id.clone(),
                            uid: e.uid.clone(),
                            occurrence: e.recurrence_id.unwrap_or(named),
                            start,
                            end,
                            all_day: e.all_day,
                            summary: e.summary.clone(),
                            location: e.location.clone(),
                            recurring: e.rrule.is_some() || e.recurrence_id.is_some(),
                            upcoming: end > now,
                            categories: e.categories.clone(),
                            color: Session::calendar_color(a, cal),
                            category: e.categories.first().map(|c| self.category_color_of(c)),
                            privacy: ical::privacy(e),
                            show_as: ical::show_as(e),
                        });
                    }
                }
            }
        }
        hits.sort_by(|a, b| match (a.upcoming, b.upcoming) {
            (true, true) => (a.start, &a.summary).cmp(&(b.start, &b.summary)),
            (false, false) => (b.start, &a.summary).cmp(&(a.start, &b.summary)),
            (x, y) => y.cmp(&x),
        });
        hits.truncate(limit.min(EVENT_HITS_MAX));
        hits
    }

    /// The occurrence of a series a search shows: which event of it, and when. Its next one that
    /// hasn't ended (unless past ones are asked for), else its last; each starting from `lo`
    /// and before `hi`.
    fn shown_occurrence(&self, events: &[CalEvent], when: EventWhen, now: i64, lo: Option<i64>, hi: Option<i64>) -> Option<Shown> {
        let fits = |s: i64| lo.is_none_or(|l| s >= l) && hi.is_none_or(|h| s < h);
        // Every occurrence overlapping [from, to), all-day ones placed on the user's days.
        let within = |from: i64, to: i64| -> Vec<Shown> {
            if from >= to {
                return Vec::new();
            }
            ical::occurrences(events, from.saturating_sub(86_400), to.saturating_add(86_400))
                .into_iter()
                .filter_map(|o| {
                    let all_day = events.get(o.event).is_some_and(|e| e.all_day);
                    let (s, e) = if all_day { (self.clock.utc(o.start), self.clock.utc(o.end)) } else { (o.start, o.end) };
                    (s < to && e.max(s.saturating_add(1)) > from && fits(s)).then_some(Shown { event: o.event, named: o.start, start: s, end: e })
                })
                .collect()
        };
        if when != EventWhen::Past {
            let from = lo.map_or(now, |l| l.max(now));
            for span in [NEAR, FAR] {
                let to = hi.map_or(from.saturating_add(span), |h| h.min(from.saturating_add(span)));
                if let Some(next) = within(from, to).into_iter().filter(|o| o.end > now).min_by_key(|o| o.start) {
                    return Some(next);
                }
            }
            if when == EventWhen::Upcoming {
                return None;
            }
        }
        let to = hi.map_or(now, |h| h.min(now));
        for span in [NEAR, FAR, i64::MAX] {
            let from = lo.map_or(to.saturating_sub(span), |l| l.max(to.saturating_sub(span)));
            if let Some(last) = within(from, to).into_iter().filter(|o| o.end <= now).max_by_key(|o| o.start) {
                return Some(last);
            }
        }
        None
    }
}
