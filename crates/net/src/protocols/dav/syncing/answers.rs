//! A change of an event the server refused: the server's copy comes back, unless this one can go
//! again over it with the guests' answers of both (SOGo writes each guest's answer into every
//! copy of a meeting itself) without losing anything changed here.

use super::pushing::Role;
use super::*;

impl Run<'_> {
    /// Not the user's to change after all (a role per classification, in SOGo), or changed on
    /// the server since it was read (SOGo writes a guest's answer into the organizer's copy
    /// itself): the server's copy comes back, rather than being sent again, refused again, at
    /// every sync.
    pub(super) fn refused(
        &self,
        role: Role,
        master: &StoredEvent,
        parts: &[&StoredEvent],
        r: Refused,
        out: &mut DavSync,
        refetch: &mut Vec<String>,
    ) -> Result<()> {
        let uid = master.event.uid.as_str();
        let server = if r == Refused::Changed { server_item(self.dav, &master.calendar, &master.href, self.local) } else { None };
        // At most one way of putting the answers together fits a meeting; the copy stored then
        // is read back.
        if let Some((tag, theirs)) = &server
            && let Some(merged) = self.together(master, parts, theirs)
        {
            let merged: Vec<CalEvent> = merged.iter().map(|e| role.prepare(e)).collect();
            match self.dav.store(&master.href, CALENDAR_DATA, &ical::write_for_server(&merged, self.now), Some(tag)) {
                Ok(Ok(_)) => {
                    theirs_kept(uid, master.href.clone(), out, refetch);
                    return Ok(());
                }
                Err(e @ NetError::Auth(_)) => return Err(e),
                _ => {}
            }
        }
        // Said, unless the server's copy has what was changed here already (SOGo put a
        // guest's answer in the organizer's copy before the organizer's MailCraft did), but
        // for the user's own answer asked again there (SOGo does when a meeting moves).
        let kept = server
            .as_ref()
            .is_some_and(|(_, theirs)| parts.iter().all(|p| theirs.iter().any(|s| has_all_of(s, &asked_again(&self.at, &p.event, s)))));
        if !kept {
            out.errors.push(format!("{}: {r}; the server's copy is kept", master.event.summary));
        }
        theirs_kept(uid, master.href.clone(), out, refetch);
        Ok(())
    }

    /// The copy to store over the server's (`theirs`), when one loses nothing.
    fn together(&self, master: &StoredEvent, parts: &[&StoredEvent], theirs: &[CalEvent]) -> Option<Vec<CalEvent>> {
        newer_version(parts, theirs).or_else(|| self.organizers_answers(master, parts, theirs)).or_else(|| self.my_answer(master, parts, theirs))
    }

    /// The same version, with guests' answers taken in here (the organizer's MailCraft took one
    /// from the mail) and others written in there meanwhile (SOGo writes a colleague's answer in
    /// the organizer's copy): the server's, with these answers, when that loses nothing changed
    /// here. It used to keep the server's and drop the answer taken in (a real Mailcow, an
    /// outside guest's answer). The organizer's only: a guest's copy the server changed is the
    /// server's (SOGo asks its users to answer a moved meeting again, and would mail the
    /// organizer the old answer as a new one).
    fn organizers_answers(&self, master: &StoredEvent, parts: &[&StoredEvent], theirs: &[CalEvent]) -> Option<Vec<CalEvent>> {
        if !master.event.organizer.as_ref().is_some_and(|o| self.at.is_me(&o.email))
            || !same_version(parts, theirs)
            || !parts.iter().any(|p| fills_in(&p.event, theirs))
        {
            return None;
        }
        let merged: Vec<CalEvent> = theirs
            .iter()
            .map(|t| parts.iter().find(|p| p.event.recurrence_id == t.recurrence_id).map_or_else(|| t.clone(), |p| with_our_answers(t, &p.event)))
            .collect();
        parts.iter().all(|p| merged.iter().any(|m| has_all_of(m, &with_newer_answers(&p.event, theirs)))).then_some(merged)
    }

    /// The user's answer, given here as a guest, to a meeting the server changed meanwhile
    /// without a new version (SOGo writes each guest's answer in every guest's copy): the
    /// server's copy with it, when that loses nothing changed here. Kept as the server's, the
    /// answer was lost: a server that schedules is left to take it to the organizer, so it isn't
    /// mailed either (a real Mailcow, a decline).
    fn my_answer(&self, master: &StoredEvent, parts: &[&StoredEvent], theirs: &[CalEvent]) -> Option<Vec<CalEvent>> {
        let at = &self.at;
        if master.event.organizer.as_ref().is_some_and(|o| at.is_me(&o.email)) {
            return None;
        }
        let given = self.pim.notices.iter().rev().find(|n| n.uid == master.event.uid && n.method == "REPLY").map(|n| n.answer.as_str())?;
        if !same_version(parts, theirs)
            || !theirs.iter().any(|t| t.attendees.iter().any(|a| at.is_me(&a.email) && !a.partstat.eq_ignore_ascii_case(given)))
        {
            return None;
        }
        // The time shown as the user's copy shows it (SOGo drops it from a guest's copy as it
        // writes the other answers in).
        let merged: Vec<CalEvent> = theirs
            .iter()
            .map(|t| {
                let shown = parts.iter().find(|p| p.event.recurrence_id == t.recurrence_id).map(|p| ical::show_as(&p.event));
                with_my_answer(at, t, given, shown)
            })
            .collect();
        let loses = |p: &&StoredEvent| {
            let mine = with_my_answer(at, &with_newer_answers(&p.event, theirs), given, Some(ical::show_as(&p.event)));
            !merged.iter().any(|m| has_all_of(m, &mine))
        };
        (!parts.iter().any(loses)).then_some(merged)
    }
}

/// A meeting the server holds an older version of than this one (its SEQUENCE): what changed
/// there is a guest's answer (SOGo writes each into the organizer's copy, and into the other
/// guests'), not a change of the meeting. This one goes again, over it, with those answers. (An
/// event without guests doesn't: another program may change one without a new SEQUENCE.)
fn newer_version(parts: &[&StoredEvent], theirs: &[CalEvent]) -> Option<Vec<CalEvent>> {
    let newest = theirs.iter().map(|e| e.sequence).max()?;
    let newer = parts.iter().any(|p| !p.event.attendees.is_empty()) && parts.iter().any(|p| p.event.sequence > newest);
    newer.then(|| parts.iter().map(|p| with_their_answers(&p.event, theirs)).collect())
}

/// Whether every part of the user's copy is the version the server has.
fn same_version(parts: &[&StoredEvent], theirs: &[CalEvent]) -> bool {
    theirs.iter().map(|e| e.sequence).max().is_some_and(|newest| parts.iter().all(|p| p.event.sequence == newest))
}

/// The server's copy of an item: its etag and events (`None` when it can't be read).
fn server_item(dav: &Dav, calendar: &str, href: &str, local: mailcraft_model::zone::Zone) -> Option<(String, Vec<CalEvent>)> {
    let got = dav.multiget(Kind::Calendars, calendar, &[href.to_string()]).ok()?;
    got.into_iter().next().map(|(_, etag, data)| (etag, ical::parse_in(&data, local).events))
}

/// A guest's answer in an event (upper case), if they're in it.
fn answer(e: &CalEvent, who: &str) -> Option<String> {
    e.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(who)).map(|a| a.partstat.to_ascii_uppercase())
}

/// An answer still to come.
pub(super) fn unanswered(partstat: &str) -> bool {
    partstat.is_empty() || partstat.eq_ignore_ascii_case("NEEDS-ACTION")
}

/// Whether the user's copy of an event (or one of its occurrences) has an answer the server's
/// copy lacks: one given here. One the server doesn't have at all counts.
pub(super) fn answers_more(mine: &CalEvent, server: &[CalEvent]) -> bool {
    let Some(s) = server.iter().find(|s| s.recurrence_id == mine.recurrence_id) else { return true };
    mine.attendees.iter().any(|a| !unanswered(&a.partstat) && answer(s, &a.email) != Some(a.partstat.to_ascii_uppercase()))
}

/// The user's copy with the answers the server's copy has where it has none yet (guests who
/// answered since it was read).
fn with_their_answers(mine: &CalEvent, server: &[CalEvent]) -> CalEvent {
    with_server_answers(mine, server, false)
}

/// The user's copy with the answers the server's copy of it has: in place of every one of its own
/// (`all`), or only where it has none.
fn with_server_answers(mine: &CalEvent, server: &[CalEvent], all: bool) -> CalEvent {
    let mut e = mine.clone();
    if let Some(s) = server.iter().find(|s| s.recurrence_id == mine.recurrence_id) {
        for a in e.attendees.iter_mut().filter(|a| all || unanswered(&a.partstat)) {
            if let Some(theirs) = s.attendees.iter().find(|x| x.email.eq_ignore_ascii_case(&a.email) && !unanswered(&x.partstat)) {
                a.partstat = theirs.partstat.clone();
            }
        }
    }
    e
}

/// The user's copy of an event with their own answer as the server's copy has it when the server
/// asks for it again (none there), and the time shown as it is there (an answer shows it).
fn asked_again(at: &Where, mine: &CalEvent, server: &CalEvent) -> CalEvent {
    let mut e = mine.clone();
    if server.recurrence_id != mine.recurrence_id {
        return e;
    }
    let mut asked = false;
    for a in e.attendees.iter_mut().filter(|a| at.is_me(&a.email)) {
        if server.attendees.iter().any(|s| s.email.eq_ignore_ascii_case(&a.email) && unanswered(&s.partstat)) {
            a.partstat = "NEEDS-ACTION".into();
            asked = true;
        }
    }
    if asked {
        ical::set_show_as(&mut e, ical::show_as(server));
    }
    e
}

/// The server's copy of an event with the answers the user's copy has (given or taken in here)
/// where the server's has none: one the server has is as new as the user's, or newer (a guest
/// who changed it there since it was read here), so it isn't put back.
fn with_our_answers(server: &CalEvent, mine: &CalEvent) -> CalEvent {
    let mut e = server.clone();
    for a in mine.attendees.iter().filter(|a| !unanswered(&a.partstat)) {
        match e.attendees.iter_mut().find(|x| x.email.eq_ignore_ascii_case(&a.email)) {
            Some(x) if unanswered(&x.partstat) => x.partstat = a.partstat.clone(),
            Some(_) => {}
            None => e.attendees.push(a.clone()),
        }
    }
    e
}

/// An event with the user's own answer `given`, and the time shown as `shown` (none: as the
/// answer shows it).
fn with_my_answer(at: &Where, e: &CalEvent, given: &str, shown: Option<ical::ShowAs>) -> CalEvent {
    let mut e = e.clone();
    for a in e.attendees.iter_mut().filter(|a| at.is_me(&a.email)) {
        a.partstat = given.to_string();
        a.rsvp = false;
    }
    match shown {
        Some(s) => ical::set_show_as(&mut e, s),
        None => ical::show_as_answered(&mut e, given),
    }
    e
}

/// Whether the user's copy of an event has an answer where the server's copy has none (a guest's
/// taken in here).
fn fills_in(mine: &CalEvent, server: &[CalEvent]) -> bool {
    let Some(s) = server.iter().find(|s| s.recurrence_id == mine.recurrence_id) else { return false };
    mine.attendees.iter().any(|a| !unanswered(&a.partstat) && answer(s, &a.email).is_none_or(|x| unanswered(&x)))
}

/// The user's copy with every answer the server's copy has in place of its own.
fn with_newer_answers(mine: &CalEvent, server: &[CalEvent]) -> CalEvent {
    with_server_answers(mine, server, true)
}

/// Whether the server's copy of an event (or of one occurrence) has everything the user's copy
/// says: what can be changed here (title, place, notes, times, repeats, status, categories,
/// privacy, Show as, reminder) and each guest's answer.
fn has_all_of(server: &CalEvent, mine: &CalEvent) -> bool {
    let what = |e: &CalEvent| {
        let mut exdates = e.exdates.clone();
        exdates.sort_unstable();
        let mut categories: Vec<String> = e.categories.iter().map(|c| c.to_lowercase()).collect();
        categories.sort();
        (
            (e.summary.trim().to_string(), e.location.trim().to_string(), e.description.trim_end().to_string()),
            (e.start, e.end, e.all_day, e.rrule.clone(), exdates),
            (e.status.to_ascii_uppercase(), categories, ical::privacy(e), ical::show_as(e), ical::reminder_minutes(e)),
        )
    };
    ical::stored_uid(&server.uid) == ical::stored_uid(&mine.uid)
        && server.recurrence_id == mine.recurrence_id
        && what(server) == what(mine)
        && mine.attendees.iter().all(|a| answer(server, &a.email) == Some(a.partstat.to_ascii_uppercase()))
}
