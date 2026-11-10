//! A sync of an account's calendars and address books with its server: what changed here is
//! sent, then what the server has is read.

use super::*;

/// The longest uid used as a file name as it is.
pub(super) const NAME_KEEP: usize = 100;

/// A file name for a new item (one already on the server keeps the address it has). A short
/// uid of plain characters is the name; any other keeps its readable start and adds `~` and a
/// hash of the whole uid, so two uids alike in what's kept still get names of their own (a plain
/// name never has a `~`).
pub(super) fn resource_name(uid: &str, ext: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if !uid.is_empty() && uid.len() <= NAME_KEEP && uid.chars().all(plain) {
        return format!("{uid}.{ext}");
    }
    let safe: String = uid.chars().map(|c| if plain(c) { c } else { '-' }).take(NAME_KEEP).collect();
    format!("{safe}~{:016x}.{ext}", mailcraft_store::fnv64(uid))
}

/// Sync an account's calendars and address books with its server. `pim` is what the engine
/// last saved; nothing is written here: the result is for the engine to merge.
pub fn sync(dav: &Dav, at: &Where, pim: &Pim, now: i64, local: mailcraft_model::zone::Zone) -> Result<DavSync> {
    let mut out = DavSync::default();
    // Where the collections live (looked up once; again a day after finding none). An address
    // the user gave is followed instead of looking, and what's wrong with it is said.
    let retry = pim.dav_checked.is_none_or(|t| now - t > 86_400);
    // A given address that holds nothing is said once and remembered as none, as one that wasn't
    // found is (looked at again a day on, or when it's changed); a refused sign-in stops the sync.
    let find = |kind: Kind, given: Option<&str>, errors: &mut Vec<String>| -> Result<Option<String>> {
        match given.filter(|g| !g.trim().is_empty()) {
            Some(g) => match dav.home_given(kind, g) {
                Ok(h) => Ok(Some(h)),
                Err(NetError::No(e)) => {
                    errors.push(e);
                    Ok(None)
                }
                Err(e) => Err(e),
            },
            None => dav.discover(kind, at.email, at.imap_host),
        }
    };
    out.caldav_home = match &pim.caldav_home {
        Some(h) if !h.is_empty() => Some(h.clone()),
        _ if retry => find(Kind::Calendars, at.caldav, &mut out.errors)?,
        _ => None,
    };
    out.carddav_home = match &pim.carddav_home {
        Some(h) if !h.is_empty() => Some(h.clone()),
        _ if retry => find(Kind::Contacts, at.carddav, &mut out.errors)?,
        _ => None,
    };
    out.checked = now;
    // Whether the calendar server schedules meetings itself (asked at every sync: it's one short
    // request, and a server's settings change). Not known, what was known stays.
    if let Some(home) = &out.caldav_home {
        out.auto_schedule = match dav.schedules(home) {
            Ok(b) => Some(b),
            Err(e @ NetError::Auth(_)) => return Err(e),
            Err(_) => None,
        };
    }
    let schedules = out.auto_schedule.unwrap_or(pim.auto_schedule);

    // Only the server the homes are on is written to: an item from another one (the calendar
    // server changed, its old items not yet moved out of the way) isn't sent there with this
    // server's password.
    let origins: Vec<String> =
        [&out.caldav_home, &out.carddav_home].into_iter().flatten().filter_map(|h| Url::parse(h).ok()).map(|u| u.origin()).collect();
    let ours = |url: &str| Url::parse(url).is_ok_and(|u| origins.contains(&u.origin()));

    // Items whose change the server refused for good: read again below, whatever their etag.
    let mut refetch: Vec<String> = Vec::new();

    // 1. Deletions made here. One the server doesn't allow (SOGo's roles) is given up and the
    //    event read again, so it's back as it is there.
    for t in pim.deleted.iter().filter(|t| ours(&t.href)) {
        match dav.remove(&t.href, &t.etag) {
            Ok(Ok(())) => out.deleted.push(t.href.clone()),
            Ok(Err(Refused::Forbidden)) => {
                out.errors.push(format!("the server doesn't allow deleting {}", t.href.rsplit('/').next().unwrap_or_default()));
                out.deleted.push(t.href.clone());
                refetch.push(t.href.clone());
            }
            Ok(Err(r)) => out.errors.push(r.to_string()),
            Err(e @ NetError::Auth(_)) => return Err(e),
            Err(e) => out.errors.push(e.to_string()),
        }
    }

    // 2. Changes made here, per item (an event with its changed instances), in the calendar
    //    it's in (a colleague's copy of the same meeting isn't part of it).
    let mut items: Vec<(&str, &str)> =
        pim.events.iter().filter(|e| e.dirty && e.calendar != LOCAL).map(|e| (e.event.uid.as_str(), e.calendar.as_str())).collect();
    items.sort_unstable();
    items.dedup();
    for (uid, calendar) in items {
        let parts: Vec<&StoredEvent> = pim.events.iter().filter(|e| e.event.uid == uid && e.calendar == calendar).collect();
        let Some(master) = parts.iter().find(|e| e.event.recurrence_id.is_none()).or_else(|| parts.first()) else { continue };
        let url = if master.href.is_empty() {
            format!("{}/{}", master.calendar.trim_end_matches('/'), resource_name(uid, "ics"))
        } else {
            master.href.clone()
        };
        if !ours(&url) {
            continue;
        }
        let etag = (!master.href.is_empty()).then_some(master.etag.as_str());
        // A new meeting the calendar may have already: SOGo puts a meeting in the calendar of
        // each of its users invited itself, under the organizer's item name (MailCraft's names
        // it by its uid too), before the invitation's mail is taken in here; and it stores a new
        // item over one of the same name, though asked not to (If-None-Match). Looked for first.
        let found = match master.href.is_empty() && master.event.organizer.is_some() {
            true => match the_servers(dav, calendar, uid, local) {
                Err(e @ NetError::Auth(_)) => return Err(e),
                found => found.ok().flatten(),
            },
            false => None,
        };
        // Who sends this meeting's invitations or answers (see `ical::scheduling`).
        let role = Role::of(at, pim, &parts, schedules, found.is_none());
        let events: Vec<CalEvent> = parts.iter().map(|p| role.prepare(&p.event)).collect();
        let data = ical::write_for_server(&events, now);
        // A guest's own copy not answered yet (an invitation from elsewhere, put in the
        // calendar as it came) waits here for the answer: SOGo answers the organizer itself for
        // any answer changed in a copy it has (whatever `SCHEDULE-AGENT` says), but for none
        // stored new with it. Stored new with the answer, the copy needs no change for it.
        if role == Role::GuestCopy && master.href.is_empty() && !answered(at, &parts) {
            continue;
        }
        let stored = match found.map(|f| over_theirs(f, &parts, master.event.sequence)) {
            Some(Err(href)) => {
                out.refused.push(uid.to_string());
                refetch.push(href);
                continue;
            }
            Some(Ok((href, tag))) => dav.store(&href, "text/calendar; charset=utf-8", &data, Some(&tag)).map(|r| r.map(|t| (href, t))),
            None => match dav.store(&url, "text/calendar; charset=utf-8", &data, etag) {
                // A new one the calendar has already, under another name (409, "Event UID
                // already in use"): the same.
                Ok(Err(Refused::UidTaken)) if master.href.is_empty() => match the_servers(dav, calendar, uid, local) {
                    Ok(Some(f)) => match over_theirs(f, &parts, master.event.sequence) {
                        Err(href) => {
                            out.refused.push(uid.to_string());
                            refetch.push(href);
                            continue;
                        }
                        Ok((href, tag)) => dav.store(&href, "text/calendar; charset=utf-8", &data, Some(&tag)).map(|r| r.map(|t| (href, t))),
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
            // Not the user's to change after all (a role per classification, in SOGo), or changed
            // on the server since it was read (SOGo writes a guest's answer into the organizer's
            // copy itself): the server's copy comes back, rather than being sent again, refused
            // again, at every sync.
            Ok(Err(r @ (Refused::Forbidden | Refused::Changed))) if !master.href.is_empty() => {
                let server = if r == Refused::Changed { server_item(dav, calendar, &master.href, local) } else { None };
                // A meeting the server holds an older version of than this one (its SEQUENCE):
                // what changed there is a guest's answer (SOGo writes each into the organizer's
                // copy, and into the other guests'), not a change of the meeting. This one goes
                // again, over it, with those answers; the copy stored then is read back. (An event
                // without guests isn't: another program may change one without a new SEQUENCE.)
                if let Some((tag, theirs)) = &server
                    && parts.iter().any(|p| !p.event.attendees.is_empty())
                    && let Some(newest) = theirs.iter().map(|e| e.sequence).max()
                    && parts.iter().any(|p| p.event.sequence > newest)
                {
                    let merged: Vec<CalEvent> = parts.iter().map(|p| role.prepare(&with_their_answers(&p.event, theirs))).collect();
                    match dav.store(&master.href, "text/calendar; charset=utf-8", &ical::write_for_server(&merged, now), Some(tag)) {
                        Ok(Ok(_)) => {
                            out.refused.push(uid.to_string());
                            refetch.push(master.href.clone());
                            continue;
                        }
                        Err(e @ NetError::Auth(_)) => return Err(e),
                        _ => {}
                    }
                }
                // The same version, with guests' answers taken in here (the organizer's MailCraft
                // took one from the mail) and others written in there meanwhile (SOGo writes a
                // colleague's answer in the organizer's copy): the server's, with these answers,
                // when that loses nothing changed here. It used to keep the server's and drop the
                // answer taken in (a real Mailcow, an outside guest's answer). The organizer's only:
                // a guest's copy the server changed is the server's (SOGo asks its users to answer
                // a moved meeting again, and would mail the organizer the old answer as a new one).
                if let Some((tag, theirs)) = &server
                    && master.event.organizer.as_ref().is_some_and(|o| at.is_me(&o.email))
                    && let Some(newest) = theirs.iter().map(|e| e.sequence).max()
                    && parts.iter().all(|p| p.event.sequence == newest)
                    && parts.iter().any(|p| fills_in(&p.event, theirs))
                {
                    let merged: Vec<CalEvent> = theirs
                        .iter()
                        .map(|t| {
                            parts
                                .iter()
                                .find(|p| p.event.recurrence_id == t.recurrence_id)
                                .map_or_else(|| t.clone(), |p| with_our_answers(t, &p.event))
                        })
                        .collect();
                    if parts.iter().all(|p| merged.iter().any(|m| has_all_of(m, &with_newer_answers(&p.event, theirs)))) {
                        let merged: Vec<CalEvent> = merged.iter().map(|e| role.prepare(e)).collect();
                        match dav.store(&master.href, "text/calendar; charset=utf-8", &ical::write_for_server(&merged, now), Some(tag)) {
                            Ok(Ok(_)) => {
                                out.refused.push(uid.to_string());
                                refetch.push(master.href.clone());
                                continue;
                            }
                            Err(e @ NetError::Auth(_)) => return Err(e),
                            _ => {}
                        }
                    }
                }
                // The user's answer, given here as a guest, to a meeting the server changed
                // meanwhile without a new version (SOGo writes each guest's answer in every guest's
                // copy): the server's copy with it, when that loses nothing changed here. Kept as
                // the server's, the answer was lost: a server that schedules is left to take it to
                // the organizer, so it isn't mailed either (a real Mailcow, a decline).
                if let Some((tag, theirs)) = &server
                    && !master.event.organizer.as_ref().is_some_and(|o| at.is_me(&o.email))
                    && let Some(given) = pim.notices.iter().rev().find(|n| n.uid == uid && n.method == "REPLY").map(|n| n.answer.as_str())
                    && let Some(newest) = theirs.iter().map(|e| e.sequence).max()
                    && parts.iter().all(|p| p.event.sequence == newest)
                    && theirs.iter().any(|t| t.attendees.iter().any(|a| at.is_me(&a.email) && !a.partstat.eq_ignore_ascii_case(given)))
                {
                    // The time shown as the user's copy shows it (SOGo drops it from a guest's copy
                    // as it writes the other answers in).
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
                    if !parts.iter().any(loses) {
                        let merged: Vec<CalEvent> = merged.iter().map(|e| role.prepare(e)).collect();
                        match dav.store(&master.href, "text/calendar; charset=utf-8", &ical::write_for_server(&merged, now), Some(tag)) {
                            Ok(Ok(_)) => {
                                out.refused.push(uid.to_string());
                                refetch.push(master.href.clone());
                                continue;
                            }
                            Err(e @ NetError::Auth(_)) => return Err(e),
                            _ => {}
                        }
                    }
                }
                // Said, unless the server's copy has what was changed here already (SOGo put a
                // guest's answer in the organizer's copy before the organizer's MailCraft did), but
                // for the user's own answer asked again there (SOGo does when a meeting moves).
                let kept = server
                    .as_ref()
                    .is_some_and(|(_, theirs)| parts.iter().all(|p| theirs.iter().any(|s| has_all_of(s, &asked_again(at, &p.event, s)))));
                if !kept {
                    out.errors.push(format!("{}: {r}; the server's copy is kept", master.event.summary));
                }
                out.refused.push(uid.to_string());
                refetch.push(master.href.clone());
            }
            Ok(Err(r)) => out.errors.push(format!("{}: {r}", master.event.summary)),
            Err(e @ NetError::Auth(_)) => return Err(e),
            Err(e) => out.errors.push(format!("{}: {e}", master.event.summary)),
        }
    }
    for c in pim.cards.iter().filter(|c| c.dirty && c.book != LOCAL) {
        let url = if c.href.is_empty() { format!("{}/{}", c.book.trim_end_matches('/'), resource_name(&c.card.uid, "vcf")) } else { c.href.clone() };
        if !ours(&url) {
            continue;
        }
        let etag = (!c.href.is_empty()).then_some(c.etag.as_str());
        match dav.put(&url, "text/vcard; charset=utf-8", &vcard::write(&c.card), etag) {
            Ok(new) => out.pushed_cards.push(StoredCard { href: url, etag: new, dirty: false, ..c.clone() }),
            Err(e @ NetError::Auth(_)) => return Err(e),
            Err(e) => out.errors.push(format!("{}: {e}", c.card.display())),
        }
    }

    // 3. What the server has.
    if let Some(home) = &out.caldav_home {
        let mut calendars = Vec::new();
        for (n, c) in dav.collections(Kind::Calendars, home)?.into_iter().enumerate() {
            let ctag = c.props.get("getctag").or_else(|| c.props.get("sync-token")).cloned().unwrap_or_default();
            let known = pim.calendars.iter().find(|k| k.id == c.href);
            let info = CalendarInfo {
                id: c.href.clone(),
                name: c.props.get("displayname").cloned().unwrap_or_else(|| "Calendar".into()),
                color: known.map_or(u8::try_from(n % 8).unwrap_or(0), |k| k.color),
                read_only: c.writable == Some(false),
                ctag: ctag.clone(),
                foreign: c.foreign,
            };
            let pushed_here = out.pushed_events.iter().any(|e| e.calendar == c.href) || refetch.iter().any(|h| h.starts_with(&c.href));
            if !ctag.is_empty() && known.is_some_and(|k| k.ctag == ctag) && !pushed_here {
                out.unchanged_calendars.push(c.href.clone());
            } else {
                let have: HashMap<&str, &str> =
                    pim.events.iter().filter(|e| e.calendar == c.href).map(|e| (e.href.as_str(), e.etag.as_str())).collect();
                let listed = dav.etags(&c.href)?;
                let fetch: Vec<String> =
                    listed.iter().filter(|(h, t)| have.get(h.as_str()) != Some(&t.as_str()) || refetch.contains(h)).map(|(h, _)| h.clone()).collect();
                // Unchanged: the engine keeps what it has.
                out.kept.extend(listed.iter().filter(|(h, _)| !fetch.contains(h)).map(|(h, _)| h.clone()));
                for (href, etag, data) in dav.multiget(Kind::Calendars, &c.href, &fetch)? {
                    for event in ical::parse_in(&data, local).events {
                        out.events.push(StoredEvent { calendar: c.href.clone(), href: href.clone(), etag: etag.clone(), event, dirty: false });
                    }
                }
                out.synced_calendars.push(c.href.clone());
            }
            calendars.push(info);
        }
        out.calendars = Some(calendars);
    }
    if let Some(home) = &out.carddav_home {
        let mut books = Vec::new();
        for c in dav.collections(Kind::Contacts, home)? {
            let ctag = c.props.get("getctag").or_else(|| c.props.get("sync-token")).cloned().unwrap_or_default();
            let known = pim.books.iter().find(|k| k.id == c.href);
            let info = BookInfo {
                id: c.href.clone(),
                name: c.props.get("displayname").cloned().unwrap_or_else(|| "Contacts".into()),
                read_only: c.writable == Some(false),
                ctag: ctag.clone(),
            };
            let pushed_here = out.pushed_cards.iter().any(|x| x.book == c.href);
            if !ctag.is_empty() && known.is_some_and(|k| k.ctag == ctag) && !pushed_here {
                out.unchanged_books.push(c.href.clone());
            } else {
                let have: HashMap<&str, &str> = pim.cards.iter().filter(|x| x.book == c.href).map(|x| (x.href.as_str(), x.etag.as_str())).collect();
                let listed = dav.etags(&c.href)?;
                let fetch: Vec<String> = listed.iter().filter(|(h, t)| have.get(h.as_str()) != Some(&t.as_str())).map(|(h, _)| h.clone()).collect();
                out.kept.extend(listed.iter().filter(|(h, _)| !fetch.contains(h)).map(|(h, _)| h.clone()));
                for (href, etag, data) in dav.multiget(Kind::Contacts, &c.href, &fetch)? {
                    for card in vcard::parse(&data) {
                        out.cards.push(StoredCard { book: c.href.clone(), href: href.clone(), etag: etag.clone(), card, dirty: false });
                    }
                }
                out.synced_books.push(c.href.clone());
            }
            books.push(info);
        }
        out.books = Some(books);
    }
    Ok(out)
}

/// Who sends a meeting's invitations or answers, for the copy stored on the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
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
    fn prepare(self, e: &CalEvent) -> CalEvent {
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
    let found = dav.find_uid(calendar, uid)?;
    for (href, etag, data) in dav.multiget(Kind::Calendars, calendar, &found)? {
        let events: Vec<CalEvent> = ical::parse_in(&data, local).events.into_iter().filter(|e| e.uid == uid).collect();
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
fn unanswered(partstat: &str) -> bool {
    partstat.is_empty() || partstat.eq_ignore_ascii_case("NEEDS-ACTION")
}

/// Whether the user's copy of an event (or one of its occurrences) has an answer the server's
/// copy lacks: one given here. One the server doesn't have at all counts.
fn answers_more(mine: &CalEvent, server: &[CalEvent]) -> bool {
    let Some(s) = server.iter().find(|s| s.recurrence_id == mine.recurrence_id) else { return true };
    mine.attendees.iter().any(|a| !unanswered(&a.partstat) && answer(s, &a.email) != Some(a.partstat.to_ascii_uppercase()))
}

/// The user's copy with the answers the server's copy has where it has none yet (guests who
/// answered since it was read).
fn with_their_answers(mine: &CalEvent, server: &[CalEvent]) -> CalEvent {
    let mut e = mine.clone();
    if let Some(s) = server.iter().find(|s| s.recurrence_id == mine.recurrence_id) {
        for a in e.attendees.iter_mut().filter(|a| unanswered(&a.partstat)) {
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
    let mut e = mine.clone();
    if let Some(s) = server.iter().find(|s| s.recurrence_id == mine.recurrence_id) {
        for a in e.attendees.iter_mut() {
            if let Some(theirs) = s.attendees.iter().find(|x| x.email.eq_ignore_ascii_case(&a.email) && !unanswered(&x.partstat)) {
                a.partstat = theirs.partstat.clone();
            }
        }
    }
    e
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
    server.uid == mine.uid
        && server.recurrence_id == mine.recurrence_id
        && what(server) == what(mine)
        && mine.attendees.iter().all(|a| answer(server, &a.email) == Some(a.partstat.to_ascii_uppercase()))
}
