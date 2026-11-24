//! What changed here, sent to the server: deletions, events, cards and tasks.

use std::collections::BTreeMap;

use super::answers::{answers_more, unanswered};
use super::*;

impl Run<'_> {
    /// Deletions made here. One the server doesn't allow (SOGo's roles) is given up and the
    /// event read again, so it's back as it is there; so is one changed there since it was
    /// read (412: another device's change, or a task moved to another list meanwhile), said so.
    /// It was tried again at every sync, with the same version, refused each time.
    ///
    /// A deletion goes only while its own part's server is known: a contact's while the contacts'
    /// home is, an event's or task's while the calendars' is. One server may hold both, and a part
    /// not found there (or refused, Google's) waits until it's found again.
    pub(super) fn push_deletions(&self, out: &mut DavSync, refetch: &mut Vec<String>) -> Result<()> {
        let known = (out.caldav_home.is_some(), out.carddav_home.is_some());
        for t in self.pim.deleted.iter().filter(|t| self.ours(&t.href) && self.part_known(&t.href, known)) {
            match self.dav.remove(&t.href, &t.etag) {
                Ok(Ok(())) => out.deleted.push(t.href.clone()),
                Ok(Err(Refused::Forbidden)) => {
                    out.errors.push(format!("the server doesn't allow deleting {}", t.href.rsplit('/').next().unwrap_or_default()));
                    out.deleted.push(t.href.clone());
                    refetch.push(t.href.clone());
                }
                Ok(Err(Refused::Changed)) => {
                    out.errors
                        .push(format!("{} changed on the server meanwhile, so it wasn't deleted", t.href.rsplit('/').next().unwrap_or_default()));
                    out.deleted.push(t.href.clone());
                    refetch.push(t.href.clone());
                }
                Ok(Err(r)) => out.errors.push(r.to_string()),
                // Google refusing the token this server (a sign-in that granted the calendars and
                // not the contacts, or the other way): kept to delete, said once, and the rest
                // syncs; what's refused is then dropped as when it isn't found.
                Err(e) => match self.dav.permission_refused(&e) {
                    Some(why) => {
                        if !out.errors.contains(&why) {
                            out.errors.push(why);
                        }
                    }
                    None if matches!(e, NetError::Auth(_)) => return Err(e),
                    None => out.errors.push(e.to_string()),
                },
            }
        }
        Ok(())
    }

    /// Whether the server of the part an item at `href` is in is known this sync (`known`: the
    /// calendars', the contacts'; see [`Run::push_deletions`]); an item of no collection kept here
    /// goes as before.
    fn part_known(&self, href: &str, (calendars, contacts): (bool, bool)) -> bool {
        let under = |id: &str| !id.is_empty() && href.starts_with(id);
        if self.pim.books.iter().any(|b| under(&b.id)) {
            return contacts;
        }
        if self.pim.calendars.iter().any(|c| under(&c.id)) || self.pim.task_lists.iter().any(|l| under(&l.id)) {
            return calendars;
        }
        true
    }

    /// Changes made here, per item (an event with its changed instances), in the calendar it's
    /// in (a colleague's copy of the same meeting isn't part of it). Each item's parts are
    /// gathered in one pass, in the order the items are kept.
    pub(super) fn push_events(&self, out: &mut DavSync, refetch: &mut Vec<String>) -> Result<()> {
        let mut items: BTreeMap<(&str, &str), Vec<&StoredEvent>> = BTreeMap::new();
        for e in self.pim.events.iter().filter(|e| e.calendar != LOCAL) {
            items.entry((e.event.uid.as_str(), e.calendar.as_str())).or_default().push(e);
        }
        for ((uid, calendar), parts) in items {
            if parts.iter().any(|e| e.dirty) {
                self.push_event(uid, calendar, &parts, out, refetch)?;
            }
        }
        Ok(())
    }

    /// One changed item: stored, or what's done when the server holds it already, or refuses it.
    fn push_event(&self, uid: &str, calendar: &str, parts: &[&StoredEvent], out: &mut DavSync, refetch: &mut Vec<String>) -> Result<()> {
        let Some(master) = parts.iter().find(|e| e.event.recurrence_id.is_none()).or_else(|| parts.first()) else { return Ok(()) };
        let url = if master.href.is_empty() {
            format!("{}/{}", master.calendar.trim_end_matches('/'), resource_name(uid, "ics"))
        } else {
            master.href.clone()
        };
        if !self.ours(&url) {
            return Ok(());
        }
        let etag = (!master.href.is_empty()).then_some(master.etag.as_str());
        // A new meeting the calendar may have already: SOGo puts a meeting in the calendar of
        // each of its users invited itself, under the organizer's item name (MailCraft's names
        // it by its uid too), before the invitation's mail is taken in here; and it stores a new
        // item over one of the same name, though asked not to (If-None-Match). Looked for first.
        let found = match master.href.is_empty() && master.event.organizer.is_some() {
            true => match the_servers(self.dav, calendar, uid, self.local) {
                Err(e @ NetError::Auth(_)) => return Err(e),
                found => found.ok().flatten(),
            },
            false => None,
        };
        // Who sends this meeting's invitations or answers (see `ical::scheduling`).
        let role = Role::of(&self.at, self.pim, parts, self.schedules, found.is_none());
        let events: Vec<CalEvent> = parts.iter().map(|p| role.prepare(&p.event)).collect();
        let data = ical::write_for_server(&events, self.now);
        // A guest's own copy not answered yet (an invitation from elsewhere, put in the
        // calendar as it came) waits here for the answer: SOGo answers the organizer itself for
        // any answer changed in a copy it has (whatever `SCHEDULE-AGENT` says), but for none
        // stored new with it. Stored new with the answer, the copy needs no change for it.
        if role == Role::GuestCopy && master.href.is_empty() && !answered(&self.at, parts) {
            return Ok(());
        }
        // Stored over the calendar's own copy (its address and etag).
        let over = |(href, tag): (String, String)| self.dav.store(&href, CALENDAR_DATA, &data, Some(&tag)).map(|r| r.map(|t| (href, t)));
        let stored = match found.map(|f| over_theirs(f, parts, master.event.sequence)) {
            Some(Err(href)) => {
                theirs_kept(uid, href, out, refetch);
                return Ok(());
            }
            Some(Ok(found)) => over(found),
            None => match self.dav.store(&url, CALENDAR_DATA, &data, etag) {
                // A new one the calendar has already, under another name (409, "Event UID
                // already in use"): the same.
                Ok(Err(Refused::UidTaken)) if master.href.is_empty() => match the_servers(self.dav, calendar, uid, self.local) {
                    Ok(Some(f)) => match over_theirs(f, parts, master.event.sequence) {
                        Err(href) => {
                            theirs_kept(uid, href, out, refetch);
                            return Ok(());
                        }
                        Ok(found) => over(found),
                    },
                    Ok(None) => Ok(Err(Refused::UidTaken)),
                    Err(e) => Err(e),
                },
                r => r.map(|r| r.map(|t| (url.clone(), t))),
            },
        };
        match stored {
            Ok(Ok((href, new))) => {
                for p in parts {
                    out.pushed_events.push(StoredEvent { href: href.clone(), etag: new.clone(), dirty: false, ..(*p).clone() });
                }
            }
            Ok(Err(r @ (Refused::Forbidden | Refused::Changed))) if !master.href.is_empty() => {
                self.refused(role, master, parts, r, out, refetch)?;
            }
            Ok(Err(r)) => out.errors.push(format!("{}: {r}", master.event.summary)),
            Err(e @ NetError::Auth(_)) => return Err(e),
            Err(e) => out.errors.push(format!("{}: {e}", master.event.summary)),
        }
        Ok(())
    }

    /// Contacts changed here.
    pub(super) fn push_cards(&self, out: &mut DavSync) -> Result<()> {
        for c in self.pim.cards.iter().filter(|c| c.dirty && c.book != LOCAL) {
            let url =
                if c.href.is_empty() { format!("{}/{}", c.book.trim_end_matches('/'), resource_name(&c.card.uid, "vcf")) } else { c.href.clone() };
            if !self.ours(&url) {
                continue;
            }
            let etag = (!c.href.is_empty()).then_some(c.etag.as_str());
            match self.dav.put(&url, "text/vcard; charset=utf-8", &vcard::write(&c.card), etag) {
                Ok(new) => out.pushed_cards.push(StoredCard { href: url, etag: new, dirty: false, ..c.clone() }),
                Err(e @ NetError::Auth(_)) => return Err(e),
                Err(e) => out.errors.push(format!("{}: {e}", c.card.display())),
            }
        }
        Ok(())
    }
}

impl Run<'_> {
    /// Tasks changed here, each in its task list (issue #201): a `VTODO` item of its own. One
    /// the server refuses for good (a list the user may only read), or that changed there
    /// meanwhile (412), gives way to the server's copy, read again in this sync.
    pub(super) fn push_tasks(&self, out: &mut DavSync, refetch: &mut Vec<String>) -> Result<()> {
        for t in self.pim.tasks.iter().filter(|t| t.dirty && !t.list.is_empty() && !t.partial) {
            let url =
                if t.href.is_empty() { format!("{}/{}", t.list.trim_end_matches('/'), resource_name(&t.task.uid, "ics")) } else { t.href.clone() };
            if !self.ours(&url) {
                continue;
            }
            let etag = (!t.href.is_empty()).then_some(t.etag.as_str());
            let data = ical::write_calendar(&[], std::slice::from_ref(&t.task), self.now);
            let what = if t.task.title.trim().is_empty() { "a task".to_string() } else { t.task.title.clone() };
            match self.dav.store(&url, CALENDAR_DATA, &data, etag) {
                Ok(Ok(new)) => out.pushed_tasks.push(StoredTask { href: url, etag: new, dirty: false, ..t.clone() }),
                Ok(Err(r @ (Refused::Forbidden | Refused::Changed))) if !t.href.is_empty() => {
                    out.errors.push(format!("{what}: {r}"));
                    theirs_kept(&t.task.uid, t.href.clone(), out, refetch);
                }
                // New, and there's one by that name already: this task stored by a sync the
                // engine hadn't taken in yet (a real Radicale, two syncs one after the other). It's
                // read in this sync, and the engine sends the change over it, not again as new.
                Ok(Err(Refused::Changed)) => refetch.push(url),
                Ok(Err(r)) => out.errors.push(format!("{what}: {r}")),
                Err(e @ NetError::Auth(_)) => return Err(e),
                Err(e) => out.errors.push(format!("{what}: {e}")),
            }
        }
        Ok(())
    }
}

/// Who sends a meeting's invitations or answers, for the copy stored on the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    /// The server doesn't schedule: nothing is said.
    Plain,
    /// The user's own meeting on a server that schedules: MailCraft does
    /// (`SCHEDULE-AGENT=CLIENT`).
    Organizer,
    /// The user's own copy of someone else's meeting, put on the server by MailCraft (the server
    /// never had it): MailCraft answers, and the copy is marked as such.
    GuestCopy,
    /// Anything else as it is: a meeting the server put in the calendar is answered there, and
    /// the server takes the answer on.
    AsItIs,
}

impl Role {
    /// `none_found`: no copy of a new meeting was found on the server (one it put there itself).
    fn of(at: &Where, pim: &Pim, parts: &[&StoredEvent], schedules: bool, none_found: bool) -> Role {
        let Some(master) = parts.iter().find(|e| e.event.recurrence_id.is_none()).or_else(|| parts.first()) else { return Role::Plain };
        if !schedules || !at.mails {
            return Role::Plain;
        }
        let Some(organizer) = master.event.organizer.as_ref() else { return Role::AsItIs };
        if at.is_me(&organizer.email) {
            return if parts.iter().any(|p| !p.event.attendees.is_empty()) { Role::Organizer } else { Role::AsItIs };
        }
        let marked = pim.guest_copies.contains(&master.event.uid) || parts.iter().any(|p| ical::is_guest_copy(&p.event));
        if none_found && (marked || master.href.is_empty()) { Role::GuestCopy } else { Role::AsItIs }
    }

    /// The copy to store.
    pub(super) fn prepare(self, e: &CalEvent) -> CalEvent {
        let mut e = e.clone();
        match self {
            Role::Plain => e.client_scheduled = false,
            Role::Organizer => e.client_scheduled = true,
            Role::GuestCopy => {
                e.client_scheduled = true;
                ical::mark_guest_copy(&mut e);
            }
            Role::AsItIs => {}
        }
        e
    }
}

/// Whether the user has answered a meeting (any part of it).
fn answered(at: &Where, parts: &[&StoredEvent]) -> bool {
    parts.iter().flat_map(|p| p.event.attendees.iter()).any(|a| at.is_me(&a.email) && !unanswered(&a.partstat))
}

/// The calendar's own item with this uid: its address, etag, newest `SEQUENCE` and events.
fn the_servers(dav: &Dav, calendar: &str, uid: &str, local: mailcraft_model::zone::Zone) -> Result<Option<(String, String, u32, Vec<CalEvent>)>> {
    // As the server stores it: a uid kept here as its sender wrote it has what a server leaves out.
    let stored = ical::stored_uid(uid);
    let found = dav.find_uid(calendar, uid)?;
    for (href, etag, data) in dav.multiget(Kind::Calendars, calendar, &found)? {
        let events: Vec<CalEvent> = ical::parse_in(&data, local).events.into_iter().filter(|e| ical::stored_uid(&e.uid) == stored).collect();
        if let Some(sequence) = events.iter().map(|e| e.sequence).max() {
            return Ok(Some((href, etag, sequence, events)));
        }
    }
    Ok(None)
}

/// A new item the calendar holds already (`found`: [`the_servers`]): stored over it (its
/// address and etag) when it has an answer given here that the server's lacks; otherwise, as
/// when the server's is a newer version, the server's comes in its place (its address in `Err`).
/// Stored over, an answer given since on another device was undone.
fn over_theirs(found: (String, String, u32, Vec<CalEvent>), parts: &[&StoredEvent], sequence: u32) -> std::result::Result<(String, String), String> {
    let (href, tag, theirs_sequence, theirs) = found;
    if theirs_sequence > sequence || !parts.iter().any(|p| answers_more(&p.event, &theirs)) { Err(href) } else { Ok((href, tag)) }
}
