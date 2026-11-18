//! What the server has: its calendars and address books, and the items in each that changed.

use std::collections::HashSet;

use super::*;

/// What's in a collection by its etags: the items here as they are there (kept), and the others
/// read (address, etag and data).
type Changed = (Vec<String>, Vec<(String, String, String)>);

impl Run<'_> {
    /// The calendars, and the events of each that changed there or were pushed to it (`refetch`:
    /// items read whatever their etag).
    pub(super) fn read_calendars(&self, refetch: &[String], out: &mut DavSync) -> Result<()> {
        let Some(home) = out.caldav_home.clone() else { return Ok(()) };
        let refetching: HashSet<&str> = refetch.iter().map(String::as_str).collect();
        let mut calendars = Vec::new();
        for (n, c) in self.dav.collections(Kind::Calendars, &home)?.into_iter().enumerate() {
            let ctag = ctag(&c);
            let known = self.pim.calendars.iter().find(|k| k.id == c.href);
            let info = CalendarInfo {
                id: c.href.clone(),
                name: c.props.get("displayname").cloned().unwrap_or_else(|| "Calendar".into()),
                color: known.map_or(u8::try_from(n % 8).unwrap_or(0), |k| k.color),
                read_only: c.writable == Some(false),
                ctag: ctag.clone(),
                foreign: c.foreign,
                free_busy: false,
            };
            let pushed_here = out.pushed_events.iter().any(|e| e.calendar == c.href) || refetch.iter().any(|h| h.starts_with(&c.href));
            if !ctag.is_empty() && known.is_some_and(|k| k.ctag == ctag) && !pushed_here {
                out.unchanged_calendars.push(c.href.clone());
            } else {
                let have: HashMap<&str, &str> =
                    self.pim.events.iter().filter(|e| e.calendar == c.href).map(|e| (e.href.as_str(), e.etag.as_str())).collect();
                match unreadable(self.changed(Kind::Calendars, &c.href, &have, &refetching), &info.name, &mut out.errors)? {
                    Some((kept, got)) => {
                        // Unchanged: the engine keeps what it has.
                        out.kept.extend(kept);
                        for (href, etag, data) in got {
                            for event in ical::parse_in(&data, self.local).events {
                                out.events.push(StoredEvent {
                                    calendar: c.href.clone(),
                                    href: href.clone(),
                                    etag: etag.clone(),
                                    event,
                                    dirty: false,
                                });
                            }
                        }
                        out.synced_calendars.push(c.href.clone());
                    }
                    None => out.unchanged_calendars.push(c.href.clone()),
                }
            }
            calendars.push(info);
        }
        out.calendars = Some(calendars);
        Ok(())
    }

    /// The address books, and the cards of each that changed there or were pushed to it.
    pub(super) fn read_books(&self, out: &mut DavSync) -> Result<()> {
        let Some(home) = out.carddav_home.clone() else { return Ok(()) };
        let mut books = Vec::new();
        for c in self.dav.collections(Kind::Contacts, &home)? {
            let ctag = ctag(&c);
            let known = self.pim.books.iter().find(|k| k.id == c.href);
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
                let have: HashMap<&str, &str> =
                    self.pim.cards.iter().filter(|x| x.book == c.href).map(|x| (x.href.as_str(), x.etag.as_str())).collect();
                match unreadable(self.changed(Kind::Contacts, &c.href, &have, &HashSet::new()), &info.name, &mut out.errors)? {
                    Some((kept, got)) => {
                        out.kept.extend(kept);
                        for (href, etag, data) in got {
                            for card in vcard::parse(&data) {
                                out.cards.push(StoredCard { book: c.href.clone(), href: href.clone(), etag: etag.clone(), card, dirty: false });
                            }
                        }
                        out.synced_books.push(c.href.clone());
                    }
                    None => out.unchanged_books.push(c.href.clone()),
                }
            }
            books.push(info);
        }
        out.books = Some(books);
        Ok(())
    }

    /// A collection's items by their etags: those `have` has with the same etag are kept, the
    /// others (and those in `refetch`) read. Looked up in sets: on a first sync every item is
    /// read, and each was looked for in the list of all.
    fn changed(&self, kind: Kind, collection: &str, have: &HashMap<&str, &str>, refetch: &HashSet<&str>) -> Result<Changed> {
        let listed = self.dav.etags(collection)?;
        let fetch: Vec<String> = listed
            .iter()
            .filter(|(h, t)| have.get(h.as_str()) != Some(&t.as_str()) || refetch.contains(h.as_str()))
            .map(|(h, _)| h.clone())
            .collect();
        let got = self.dav.multiget(kind, collection, &fetch)?;
        let fetched: HashSet<&str> = fetch.iter().map(String::as_str).collect();
        Ok((listed.into_iter().filter(|(h, _)| !fetched.contains(h.as_str())).map(|(h, _)| h).collect(), got))
    }
}

/// A collection's tag, which changes when anything in it does (empty: none given).
fn ctag(c: &Item) -> String {
    c.props.get("getctag").or_else(|| c.props.get("sync-token")).cloned().unwrap_or_default()
}
