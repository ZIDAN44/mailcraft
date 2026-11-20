//! How an invitation fits the user's calendar (issue #194), as Outlook shows it on a meeting
//! request: whether it clashes with something already there or is right next to it, and the
//! user's own events in the hours around it.

use super::*;

/// The hours shown either side of an invitation.
const AROUND: i64 = 2 * 3600;
/// The longest stretch shown (a long meeting is shown from its start).
const LONGEST: i64 = 14 * 3600;

/// How one of the user's events meets the invitation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// At the same time.
    Clash,
    /// Ending as it starts.
    Before,
    /// Starting as it ends.
    After,
    /// Neither, or not counted (shown as free, or cancelled).
    Apart,
}

/// The user's calendar around an invitation: the stretch shown, and their own events in it, each
/// with how it meets the invitation.
#[derive(Clone, Debug)]
pub struct InviteDay {
    pub from: i64,
    pub to: i64,
    pub items: Vec<(AgendaItem, Fit)>,
}

impl InviteDay {
    /// The events it clashes with.
    pub fn clashes(&self) -> impl Iterator<Item = &AgendaItem> {
        self.items.iter().filter(|(_, f)| *f == Fit::Clash).map(|(it, _)| it)
    }

    /// The event right before it (ending as it starts), or right after it, when nothing clashes.
    pub fn beside(&self) -> Option<(&AgendaItem, Fit)> {
        if self.clashes().next().is_some() {
            return None;
        }
        self.items.iter().find(|(_, f)| matches!(f, Fit::Before | Fit::After)).map(|(it, f)| (it, *f))
    }
}

impl Session {
    /// The user's own calendars (shown or not; not a colleague's) around the invitation `e`:
    /// `None` for one all day, or with no time. Its own copy, already in a calendar, isn't one
    /// of the events; an event shown as free or working elsewhere, or cancelled, is shown but
    /// meets nothing, as are tasks and all-day events, which aren't drawn.
    pub fn invite_day(&self, e: &CalEvent) -> Option<InviteDay> {
        if e.all_day || e.end <= e.start {
            return None;
        }
        let hour = |t: i64| t - t.rem_euclid(3600);
        let from = hour(e.start) - AROUND;
        let to = (hour(e.end.saturating_add(3599)) + AROUND).min(from + LONGEST + 2 * AROUND);
        let mine = self.agenda_where(from, to, |account, cal| !self.pim.get(account).is_some_and(|p| p.is_theirs(&cal.id)));
        let mut items: Vec<(AgendaItem, Fit)> = mine
            .into_iter()
            .filter(|it| !it.all_day && !it.task && !it.busy_only && it.uid != e.uid)
            .map(|it| {
                let counts = it.status != "CANCELLED" && !matches!(it.show_as, ical::ShowAs::Free | ical::ShowAs::WorkingElsewhere);
                let fit = match () {
                    _ if !counts => Fit::Apart,
                    _ if it.start < e.end && it.end > e.start => Fit::Clash,
                    _ if it.end == e.start => Fit::Before,
                    _ if it.start == e.end => Fit::After,
                    _ => Fit::Apart,
                };
                (it, fit)
            })
            .collect();
        items.sort_by(|(a, _), (b, _)| (a.start, a.end, &a.summary).cmp(&(b.start, b.end, &b.summary)));
        Some(InviteDay { from, to, items })
    }
}
