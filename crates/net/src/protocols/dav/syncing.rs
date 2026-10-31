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
        let events: Vec<CalEvent> = parts.iter().map(|e| e.event.clone()).collect();
        let data = ical::write(&events, None, now);
        let etag = (!master.href.is_empty()).then_some(master.etag.as_str());
        let stored = match dav.store(&url, "text/calendar; charset=utf-8", &data, etag) {
            // A new one the calendar has already: SOGo put the meeting there itself when it was
            // organised (its users' invitations), before the invitation's mail was taken in
            // here. Stored over the server's copy, so there's one, unless that one is newer.
            Ok(Err(Refused::UidTaken)) if master.href.is_empty() => match the_servers(dav, calendar, uid, local) {
                Ok(Some((href, _, sequence))) if sequence > master.event.sequence => {
                    out.refused.push(uid.to_string());
                    refetch.push(href);
                    continue;
                }
                Ok(Some((href, tag, _))) => dav.store(&href, "text/calendar; charset=utf-8", &data, Some(&tag)).map(|r| r.map(|t| (href, t))),
                Ok(None) => Ok(Err(Refused::UidTaken)),
                Err(e) => Err(e),
            },
            r => r.map(|r| r.map(|t| (url.clone(), t))),
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
                out.errors.push(format!("{}: {r}; the server's copy is kept", master.event.summary));
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

/// The calendar's own item with this uid: its address, etag and newest `SEQUENCE`.
fn the_servers(dav: &Dav, calendar: &str, uid: &str, local: mailcraft_model::zone::Zone) -> Result<Option<(String, String, u32)>> {
    let found = dav.find_uid(calendar, uid)?;
    for (href, etag, data) in dav.multiget(Kind::Calendars, calendar, &found)? {
        let events = ical::parse_in(&data, local).events;
        if let Some(sequence) = events.iter().filter(|e| e.uid == uid).map(|e| e.sequence).max() {
            return Ok(Some((href, etag, sequence)));
        }
    }
    Ok(None)
}
