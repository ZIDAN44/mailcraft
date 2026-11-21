//! A sync of the calendars and contacts done a step at a time: what changed here sent, then each
//! calendar and address book read. Between two steps the account's mail can be looked at (issue
//! #145: a pass took two to four minutes with two shared calendars, and new mail waited for it).

use super::reading::SharedCalendar;
use super::*;

/// Where a pass is.
enum Step {
    /// What changed here, sent.
    Push,
    /// The account's own calendar.
    Calendar,
    /// The calendars of others, still to read (the next one's place in the list: its colour).
    Shared(Vec<SharedCalendar>, usize),
    Contacts,
    Public,
    Done,
}

/// A sync of the calendars and contacts, a step at a time ([`Pass::step`]).
pub struct Pass {
    out: DavSync,
    calendars: Vec<CalendarInfo>,
    books: Vec<BookInfo>,
    step: Step,
}

impl Pass {
    /// A pass at `now` (the time it says the calendars were checked).
    pub fn new(now: i64) -> Pass {
        let out = DavSync { caldav_home: Some("ews".into()), carddav_home: Some("ews".into()), checked: now, ..Default::default() };
        Pass { out, calendars: Vec::new(), books: Vec::new(), step: Step::Push }
    }

    /// One step: `Some` with what was read once the last is done. A server that can't be reached
    /// stops the pass (an error), so nothing read before is dropped for want of an answer; one
    /// folder it refuses keeps what was read of it before and doesn't hold up the others (issue
    /// #9: the address books stayed as they were whenever the calendar failed).
    pub fn step(&mut self, c: &Client, pim: &Pim) -> Result<Option<DavSync>> {
        let out = &mut self.out;
        self.step = match std::mem::replace(&mut self.step, Step::Done) {
            Step::Push => {
                push_deletions(c, pim, out);
                push_events(c, pim, out);
                // Reminders dismissed or snoozed here (issue #40).
                reminders::answer_all(c, pim, out);
                push_cards(c, pim, out);
                Step::Calendar
            }
            Step::Calendar => {
                self.calendars.push(read_calendar(c, out)?);
                Step::Shared(reading::shared_calendars(c, pim), 0)
            }
            Step::Shared(mut left, n) if !left.is_empty() => {
                let next = left.remove(0);
                read_shared_calendar(c, next, n, out, &mut self.calendars);
                Step::Shared(left, n.saturating_add(1))
            }
            Step::Shared(..) => {
                // None left: on to the contacts, now.
                self.books.push(read_contacts(c, out)?);
                Step::Contacts
            }
            Step::Contacts => {
                read_public(c, out, &mut self.calendars, &mut self.books);
                Step::Public
            }
            Step::Public => {
                // Notes (Outlook's sticky notes): what changed here goes up, then the folder is
                // read again.
                push_notes(c, pim, out);
                read_notes(c, out);
                // Tasks (issue #161), the same way.
                push_tasks(c, pim, out);
                read_tasks(c, out);
                Step::Done
            }
            Step::Done => Step::Done,
        };
        if !matches!(self.step, Step::Done) {
            return Ok(None);
        }
        let mut out = std::mem::take(&mut self.out);
        out.calendars = Some(std::mem::take(&mut self.calendars));
        out.books = Some(std::mem::take(&mut self.books));
        mailcraft_model::pim::separate_shared_uids(&mut out.events);
        Ok(Some(out))
    }
}

/// The calendars of `people` just opened, read on their own (issue #208: they came with the
/// next whole pass, minutes later on a big mailbox): each as a pass reads it, in its colour (its
/// place among those opened), marked as theirs alone ([`DavSync::people_only`]). One no longer
/// open is left out.
pub fn read_people(c: &Client, pim: &Pim, people: &[String], now: i64) -> DavSync {
    let mut out = DavSync { checked: now, people_only: true, ..Default::default() };
    let mut calendars = Vec::new();
    for email in people {
        let Some(n) = pim.shared.iter().position(|e| e == email) else { continue };
        read_shared_calendar(c, (email.clone(), email.clone(), true), n, &mut out, &mut calendars);
    }
    out.calendars = Some(calendars);
    mailcraft_model::pim::separate_shared_uids(&mut out.events);
    out
}

/// Send what changed here, then read the calendar and contacts again: a [`Pass`] whole (the
/// tests'; the worker goes a step at a time).
#[cfg(test)]
pub fn sync(c: &Client, pim: &Pim, now: i64) -> Result<DavSync> {
    let mut pass = Pass::new(now);
    // Each step moves the pass on: it ends after as many as it has (a bound all the same).
    for _ in 0..100_000 {
        if let Some(out) = pass.step(c, pim)? {
            return Ok(out);
        }
    }
    Err(NetError::Protocol("the calendars and contacts never finished".into()))
}
