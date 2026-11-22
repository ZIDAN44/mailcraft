//! What the server has: its calendars, task lists and address books, and the items in each
//! that changed.

use std::collections::HashSet;

use super::*;

/// What's in a collection by its etags: the items here as they are there (kept), and the others
/// read (address, etag and data).
type Changed = (Vec<String>, Vec<(String, String, String)>);

impl Run<'_> {
    /// The calendars and task lists (issue #201), and the events and tasks of each that changed
    /// there or were pushed to it (`refetch`: items read whatever their etag). A collection
    /// that holds both (SOGo's, Stalwart's, Radicale's calendars) is read once, for both.
    pub(super) fn read_calendars(&self, refetch: &[String], out: &mut DavSync) -> Result<()> {
        let Some(home) = out.caldav_home.clone() else { return Ok(()) };
        let refetching: HashSet<&str> = refetch.iter().map(String::as_str).collect();
        let (mut calendars, mut lists) = (Vec::new(), Vec::new());
        let mut n = 0;
        for c in self.dav.all_collections(Kind::Calendars, &home)? {
            let (events, tasks) = (holds_events(&c), holds_tasks(&c));
            if !events && !tasks {
                continue;
            }
            let ctag = ctag(&c);
            let kept_tag = keepable(&ctag, self.now);
            let name = c.props.get("displayname").cloned().unwrap_or_else(|| if events { "Calendar" } else { "Tasks" }.into());
            let known = self.pim.calendars.iter().find(|k| k.id == c.href);
            let known_list = self.pim.task_lists.iter().find(|k| k.id == c.href);
            if events {
                calendars.push(CalendarInfo {
                    id: c.href.clone(),
                    name: name.clone(),
                    color: known.map_or(u8::try_from(n % 8).unwrap_or(0), |k| k.color),
                    read_only: c.writable == Some(false),
                    ctag: kept_tag.clone(),
                    foreign: c.foreign,
                    free_busy: false,
                });
                n += 1;
            }
            if tasks {
                lists.push(TaskList {
                    id: c.href.clone(),
                    name: name.clone(),
                    read_only: c.writable == Some(false),
                    ctag: kept_tag.clone(),
                    tasks_only: !events,
                    foreign: c.foreign,
                });
            }
            // Unchanged when what's kept of it, as a calendar and as a task list, has this tag.
            let same = |k: Option<&String>| k.is_some_and(|k| *k == ctag);
            let unchanged = !ctag.is_empty()
                && (!events || same(known.map(|k| &k.ctag)))
                && (!tasks || same(known_list.map(|k| &k.ctag)))
                && !out.pushed_events.iter().any(|e| e.calendar == c.href)
                && !out.pushed_tasks.iter().any(|t| t.list == c.href)
                && !refetch.iter().any(|h| h.starts_with(&c.href));
            if unchanged {
                out.unchanged_calendars.extend(events.then(|| c.href.clone()));
                continue;
            }
            let mut have: HashMap<&str, &str> = HashMap::new();
            if events {
                have.extend(self.pim.events.iter().filter(|e| e.calendar == c.href).map(|e| (e.href.as_str(), e.etag.as_str())));
            }
            if tasks {
                have.extend(self.pim.tasks.iter().filter(|t| t.list == c.href).map(|t| (t.href.as_str(), t.etag.as_str())));
            }
            match unreadable(self.changed(Kind::Calendars, &c.href, &have, &refetching), &name, &mut out.errors)? {
                Some((kept, got)) => {
                    // Unchanged: the engine keeps what it has.
                    out.kept.extend(kept);
                    for (href, etag, data) in got {
                        let read = ical::parse_in(&data, self.local);
                        for event in read.events.into_iter().filter(|_| events) {
                            out.events.push(StoredEvent { calendar: c.href.clone(), href: href.clone(), etag: etag.clone(), event, dirty: false });
                        }
                        if tasks {
                            out.list_tasks.extend(server_tasks(read.tasks, &c.href, &href, &etag));
                        }
                    }
                    out.synced_calendars.extend(events.then(|| c.href.clone()));
                    out.synced_task_lists.extend(tasks.then(|| c.href.clone()));
                }
                None => out.unchanged_calendars.extend(events.then(|| c.href.clone())),
            }
        }
        out.calendars = Some(calendars);
        out.task_lists = Some(lists);
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
                ctag: keepable(&ctag, self.now),
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

/// The task an item of a task list holds (issue #201): its occurrences changed on their own
/// (`RECURRENCE-ID`) aren't kept, and the task is marked so ([`StoredTask::partial`]) that it
/// isn't written back without them; one with no uid (RFC 5545 asks for one) gets one made from
/// its address, the same at every sync.
fn server_tasks(read: Vec<mailcraft_model::pim::Task>, list: &str, href: &str, etag: &str) -> Vec<StoredTask> {
    let is_override = |t: &mailcraft_model::pim::Task| t.extra.iter().any(|l| l.to_ascii_uppercase().starts_with("RECURRENCE-ID"));
    let partial = read.len() > 1 || read.iter().any(is_override);
    read.into_iter()
        .filter(|t| !is_override(t))
        .map(|mut task| {
            if task.uid.trim().is_empty() {
                task.uid = format!("{:016x}@caldav", mailcraft_store::fnv64(href));
            }
            StoredTask { task, list: list.into(), href: href.into(), etag: etag.into(), dirty: false, partial }
        })
        .collect()
}

/// The tag kept for the next sync: none when it reads as this very second (or one just before
/// or after: clocks differ a little). SOGo's is the time of the last change, in seconds, so a
/// change in the second it was read in leaves it as it was, and was never read: a task changed
/// in SOGo's web page as MailCraft synced stayed as it had been (a real Mailcow, issue #201).
/// With no tag kept, the next sync compares the items' etags.
fn keepable(ctag: &str, now: i64) -> String {
    let recent = ctag.trim().parse::<i64>().is_ok_and(|t| t.abs_diff(now) <= 2);
    if recent { String::new() } else { ctag.to_string() }
}

/// A collection's tag, which changes when anything in it does (empty: none given).
fn ctag(c: &Item) -> String {
    c.props.get("getctag").or_else(|| c.props.get("sync-token")).cloned().unwrap_or_default()
}
