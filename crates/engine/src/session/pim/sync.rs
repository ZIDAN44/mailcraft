//! Merging what a CalDAV / CardDAV sync found into what the account keeps.

use mailcraft_model::pim::{LOCAL, StoredEvent};

use super::*;

impl Session {
    /// Merge what a CalDAV / CardDAV sync found. Changes made here since stay (and go to the
    /// server next time); everything else follows the server.
    pub fn apply_dav(&mut self, account: &str, sync: &mailcraft_model::pim::DavSync) {
        let p = self.pim_mut(account);
        p.caldav_home = sync.caldav_home.clone().or_else(|| Some(String::new()));
        p.carddav_home = sync.carddav_home.clone().or_else(|| Some(String::new()));
        p.dav_checked = Some(sync.checked);
        p.deleted.retain(|t| !sync.deleted.contains(&t.href));
        // What was stored: its address and version, and no longer to send (unless it changed
        // again meanwhile).
        for x in &sync.pushed_events {
            for e in p.events.iter_mut().filter(|e| e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id) {
                e.href = x.href.clone();
                e.etag = x.etag.clone();
                // Stored in another calendar (a new event goes to the server's default one).
                e.calendar = x.calendar.clone();
                e.dirty &= e.event != x.event;
            }
        }
        // Notes stored on the server: their item there, and no longer to send.
        for x in &sync.pushed_notes {
            for n in p.notes.iter_mut().filter(|n| n.note.uid == x.note.uid) {
                n.href = x.href.clone();
                n.etag = x.etag.clone();
                n.dirty &= n.note != x.note;
            }
        }
        // The server's notes, by their item there; those changed here (and those never sent)
        // stay.
        if let Some(server) = &sync.notes {
            p.notes.retain(|n| n.dirty || n.href.is_empty());
            for x in server {
                if !p.notes.iter().any(|n| !n.href.is_empty() && n.href == x.href) {
                    p.notes.push(x.clone());
                }
            }
        }
        for x in &sync.pushed_cards {
            for c in p.cards.iter_mut().filter(|c| c.card.uid == x.card.uid) {
                c.href = x.href.clone();
                c.etag = x.etag.clone();
                c.book = x.book.clone();
                c.dirty &= c.card != x.card;
            }
        }
        if let Some(server) = &sync.calendars {
            // The calendar on this computer goes once the server has calendars, unless it holds
            // events (they stay there).
            let keep_local = server.is_empty() || p.events.iter().any(|e| e.calendar == LOCAL);
            let mut list: Vec<mailcraft_model::pim::CalendarInfo> = server.clone();
            if keep_local && let Some(l) = p.calendars.iter().find(|c| c.id == LOCAL) {
                list.push(l.clone());
            }
            // Calendars subscribed to by their links aren't the server's: they stay (issue #7).
            list.extend(p.calendars.iter().filter(|c| mailcraft_model::pim::is_web_calendar(&c.id)).cloned());
            p.events.retain(|e| e.calendar == LOCAL || list.iter().any(|c| c.id == e.calendar) || e.dirty);
            for cal in &sync.synced_calendars {
                p.events.retain(|e| &e.calendar != cal || e.dirty || e.href.is_empty() || sync.kept.contains(&e.href));
                for x in sync.events.iter().filter(|x| &x.calendar == cal) {
                    if !p.events.iter().any(|e| e.dirty && e.event.uid == x.event.uid) {
                        let web = |e: &StoredEvent| mailcraft_model::pim::is_web_calendar(&e.calendar);
                        p.events.retain(|e| web(e) || !(e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id && !e.dirty));
                        p.events.push(x.clone());
                    }
                }
            }
            p.calendars = list;
        }
        if let Some(server) = &sync.books {
            let keep_local = server.is_empty() || p.cards.iter().any(|c| c.book == LOCAL);
            let mut list: Vec<mailcraft_model::pim::BookInfo> = server.clone();
            if keep_local && let Some(l) = p.books.iter().find(|b| b.id == LOCAL) {
                list.push(l.clone());
            }
            p.cards.retain(|c| c.book == LOCAL || list.iter().any(|b| b.id == c.book) || c.dirty);
            for book in &sync.synced_books {
                p.cards.retain(|c| &c.book != book || c.dirty || c.href.is_empty() || sync.kept.contains(&c.href));
                for x in sync.cards.iter().filter(|x| &x.book == book) {
                    if !p.cards.iter().any(|c| c.dirty && c.card.uid == x.card.uid) {
                        p.cards.retain(|c| c.card.uid != x.card.uid);
                        p.cards.push(x.clone());
                    }
                }
            }
            p.books = list;
        }
        p.ensure_defaults();
        for e in sync.errors.iter().take(3) {
            self.notice(format!("calendars and contacts: {e}"), true);
        }
        self.save_pim(account);
    }
}
