//! Merging what a CalDAV / CardDAV sync found into what the account keeps.
//!
//! Every lookup goes through a set built once (an item looked for in every list for each one
//! the server sent took 0.64 s for a calendar of 20,000 events, on the window's thread).

use std::collections::HashSet;

use mailcraft_model::pim::{LOCAL, StoredCard, StoredEvent};

use super::*;

impl Session {
    /// Merge what a CalDAV / CardDAV sync found. Changes made here since stay (and go to the
    /// server next time); everything else follows the server.
    pub fn apply_dav(&mut self, account: &str, sync: &mailcraft_model::pim::DavSync) {
        if sync.people_only {
            self.apply_people(account, sync);
            return;
        }
        // The answers given here to invitations put on this computer, before the server's copy
        // of the same meeting (CalDAV scheduling) takes their place (issue #30).
        let answered = self.answered_here(account);
        let p = self.pim_mut(account);
        p.caldav_home = sync.caldav_home.clone().or_else(|| Some(String::new()));
        p.carddav_home = sync.carddav_home.clone().or_else(|| Some(String::new()));
        if !sync.homes_kept || p.dav_checked.is_none() {
            p.dav_checked = Some(sync.checked);
        }
        if let Some(schedules) = sync.auto_schedule {
            p.auto_schedule = schedules;
        }
        let deleted: HashSet<&str> = sync.deleted.iter().map(String::as_str).collect();
        p.deleted.retain(|t| !deleted.contains(t.href.as_str()));
        let told: HashSet<(&str, i64)> = sync.answered.iter().map(|(uid, at)| (uid.as_str(), *at)).collect();
        p.reminder_answers.retain(|a| !told.contains(&(a.uid.as_str(), a.occurrence)));
        let kept: HashSet<&str> = sync.kept.iter().map(String::as_str).collect();
        // Changes the server refused for good (SOGo's roles): its copy, read again in this
        // sync, comes back in their place.
        let refused: HashSet<&str> = sync.refused.iter().map(String::as_str).collect();
        for e in p.events.iter_mut().filter(|e| refused.contains(e.event.uid.as_str())) {
            e.dirty = false;
        }
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
            let mut here: HashSet<String> = p.notes.iter().filter(|n| !n.href.is_empty()).map(|n| n.href.clone()).collect();
            for x in server {
                if x.href.is_empty() || here.insert(x.href.clone()) {
                    p.notes.push(x.clone());
                }
            }
        }
        // Tasks (issue #161), as the notes: those stored on the server have their item there and
        // are no longer to send; the server's read again take the place of the rest, those
        // changed here (and those never sent) staying.
        for x in &sync.pushed_tasks {
            // In the list it was stored in (a copy of the same task in another list is another
            // item; Exchange's are in none).
            let mut found = false;
            for t in p.tasks.iter_mut().filter(|t| t.task.uid == x.task.uid && t.list == x.list) {
                t.href = x.href.clone();
                t.etag = x.etag.clone();
                t.dirty &= t.task != x.task;
                found = true;
            }
            // Moved to another list, or deleted, while it was being stored: the item stored is
            // deleted, with the version just stored (the one before is refused, 412); without
            // that, it came back as a task of its own at the next read of that list.
            if !found && !x.list.is_empty() {
                match p.deleted.iter_mut().find(|g| g.href == x.href) {
                    Some(g) => g.etag = x.etag.clone(),
                    None => p.deleted.push(mailcraft_model::pim::Tombstone { href: x.href.clone(), etag: x.etag.clone(), ..Default::default() }),
                }
            }
        }
        // CalDAV task lists (issue #201), as calendars are.
        if let Some(lists) = &sync.task_lists {
            merge_task_lists(p, lists, sync, &kept, &refused);
        }
        let mut unsent = false;
        if let Some(server) = &sync.tasks {
            p.tasks.retain(|t| t.dirty || t.href.is_empty());
            // Made here before the server kept them (or while it couldn't be reached): sent now.
            for t in p.tasks.iter_mut().filter(|t| t.href.is_empty() && !t.dirty) {
                t.dirty = true;
                unsent = true;
            }
            let mut here: HashSet<String> = p.tasks.iter().filter(|t| !t.href.is_empty()).map(|t| t.href.clone()).collect();
            for x in server {
                if x.href.is_empty() || here.insert(x.href.clone()) {
                    p.tasks.push(x.clone());
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
            // events (they stay there), or none of the server's can be written to (issue #61).
            let keep_local = server.iter().all(|c| c.read_only) || p.events.iter().any(|e| e.calendar == LOCAL);
            let mut list: Vec<mailcraft_model::pim::CalendarInfo> = server.clone();
            if keep_local && let Some(l) = p.calendars.iter().find(|c| c.id == LOCAL) {
                list.push(l.clone());
            }
            // Calendars subscribed to by their links aren't the server's: they stay (issue #7).
            list.extend(p.calendars.iter().filter(|c| mailcraft_model::pim::is_web_calendar(&c.id)).cloned());
            // So do those of people opened since this sync began, read on their own (issue
            // #208): it doesn't know of them, and they went until the next one.
            let on_list: HashSet<String> = list.iter().map(|c| c.id.clone()).collect();
            list.extend(p.calendars.iter().filter(|c| !on_list.contains(&c.id) && opened_by_hand(&p.shared, &c.id)).cloned());
            let listed: HashSet<String> = list.iter().map(|c| c.id.clone()).collect();
            p.events.retain(|e| e.calendar == LOCAL || listed.contains(&e.calendar) || e.dirty);
            // The user's own copy of a meeting comes before a colleague's: SOGo lists the
            // calendars the user subscribed to in their home, and a meeting both are in is in
            // both, under one uid (a real Mailcow, issue #30). Taken one for the other, the
            // user's copy (the one to answer in) was gone. Their own calendars are read first,
            // so a copy gone from them doesn't keep the colleague's out.
            let web = |e: &StoredEvent| mailcraft_model::pim::is_web_calendar(&e.calendar);
            // A colleague's is one listed as theirs, or one of the server's the user may only
            // read (a server that doesn't say whose it is), as where an invitation is put takes
            // it (`Pim::is_theirs`): taken out there, it would come back here.
            let theirs = |c: &mailcraft_model::pim::CalendarInfo| (c.foreign || c.read_only) && !mailcraft_model::pim::is_web_calendar(&c.id);
            let foreign_ids: HashSet<&str> = list.iter().filter(|c| theirs(c)).map(|c| c.id.as_str()).collect();
            let foreign = |id: &str| foreign_ids.contains(id);
            let mine = |e: &StoredEvent| !foreign(&e.calendar) && !web(e);
            let part = |e: &StoredEvent| (e.event.uid.clone(), e.event.recurrence_id);
            let events = as_kept(&p.events, &sync.events);
            let mut order: Vec<&String> = sync.synced_calendars.iter().collect();
            order.sort_by_key(|c| foreign(c));
            for cal in order {
                p.events.retain(|e| &e.calendar != cal || e.dirty || e.href.is_empty() || kept.contains(e.href.as_str()));
                // The user's own copies of each part, and the uids changed here (they stay as
                // they are), as they are now: the calendars before this one are merged.
                let own: HashSet<(String, Option<i64>)> = p.events.iter().chain(events.iter()).filter(|e| mine(e)).map(part).collect();
                let changed_here: HashSet<&str> = p.events.iter().filter(|e| e.dirty).map(|e| e.event.uid.as_str()).collect();
                // What the server sent for this calendar takes the place of what's here of the
                // same part (the last it sent of one, if it sent one twice).
                let mut taken: Vec<&StoredEvent> = Vec::new();
                let mut parts: HashSet<(String, Option<i64>)> = HashSet::new();
                for x in events.iter().filter(|x| &x.calendar == cal).rev() {
                    if foreign(&x.calendar) && own.contains(&part(x)) {
                        continue;
                    }
                    if !changed_here.contains(x.event.uid.as_str()) && parts.insert(part(x)) {
                        taken.push(x);
                    }
                }
                p.events.retain(|e| web(e) || e.dirty || !parts.contains(&part(e)));
                p.events.extend(taken.into_iter().rev().cloned());
            }
            // A colleague's copy read before the user's own came (or was put here) goes now.
            let own_uids: std::collections::HashSet<String> = p.events.iter().filter(|e| mine(e)).map(|e| e.event.uid.clone()).collect();
            p.events.retain(|e| !(foreign(&e.calendar) && own_uids.contains(&e.event.uid)));
            p.calendars = list;
        }
        if let Some(server) = &sync.books {
            // As the calendar: a Nextcloud user who never opened its web page has only its
            // read-only address books (the organisation's accounts, those recently contacted), and
            // new contacts had nowhere to go (issue #61).
            let keep_local = server.iter().all(|b| b.read_only) || p.cards.iter().any(|c| c.book == LOCAL);
            let mut list: Vec<mailcraft_model::pim::BookInfo> = server.clone();
            if keep_local && let Some(l) = p.books.iter().find(|b| b.id == LOCAL) {
                list.push(l.clone());
            }
            let listed: HashSet<String> = list.iter().map(|b| b.id.clone()).collect();
            p.cards.retain(|c| c.book == LOCAL || listed.contains(&c.book) || c.dirty);
            for book in &sync.synced_books {
                p.cards.retain(|c| &c.book != book || c.dirty || c.href.is_empty() || kept.contains(c.href.as_str()));
                // As the events: what the server sent takes the place of the same contact
                // here, unless it was changed here (the last it sent of one).
                let changed_here: HashSet<&str> = p.cards.iter().filter(|c| c.dirty).map(|c| c.card.uid.as_str()).collect();
                let mut taken: Vec<&StoredCard> = Vec::new();
                let mut uids: HashSet<&str> = HashSet::new();
                for x in sync.cards.iter().filter(|x| &x.book == book).rev() {
                    if !changed_here.contains(x.card.uid.as_str()) && uids.insert(x.card.uid.as_str()) {
                        taken.push(x);
                    }
                }
                p.cards.retain(|c| !uids.contains(c.card.uid.as_str()));
                p.cards.extend(taken.into_iter().rev().cloned());
            }
            p.books = list;
        }
        p.ensure_defaults();
        for e in sync.errors.iter().take(3) {
            self.notice(format!("calendars and contacts: {e}"), true);
        }
        self.save_pim(account);
        // Tasks kept here alone go to the server's default task list, as Exchange's go to its
        // Tasks folder (issue #201).
        let p = self.pim_mut(account);
        if let Some(to) = p.default_task_list().map(|l| l.id.clone()) {
            for t in p.tasks.iter_mut().filter(|t| t.list.is_empty() && t.href.is_empty()) {
                t.list = to.clone();
                t.dirty = true;
                unsent = true;
            }
            self.save_pim(account);
        }
        // Tasks made before the server kept them go there now, not at the next pass.
        if unsent {
            self.after_pim_change(account, true);
        }
        // Meeting messages waiting for the server (see `invite::notices`), and answered
        // invitations waiting to go out of the way.
        self.after_dav_sync(account);
        self.tidy_after_sync(account, &sync.refused);
        // Invitations put on this computer meanwhile move to the server's calendar, those held
        // go in (issue #30).
        self.calendars_found(account, &answered);
    }
}

impl Session {
    /// The calendars of people just opened, read on their own (issue #208): theirs taken in as
    /// a whole sync takes them, everything else left as it is. One closed again meanwhile isn't
    /// brought back; an event changed here stays.
    fn apply_people(&mut self, account: &str, sync: &mailcraft_model::pim::DavSync) {
        let p = self.pim_mut(account);
        let open = p.shared.clone();
        for c in sync.calendars.iter().flatten().filter(|c| opened_by_hand(&open, &c.id)) {
            match p.calendars.iter_mut().find(|x| x.id == c.id) {
                Some(x) => *x = c.clone(),
                None => p.calendars.push(c.clone()),
            }
        }
        for id in sync.synced_calendars.iter().filter(|id| opened_by_hand(&open, id)) {
            let changed_here: HashSet<(String, Option<i64>)> =
                p.events.iter().filter(|e| &e.calendar == id && e.dirty).map(|e| (e.event.uid.clone(), e.event.recurrence_id)).collect();
            p.events.retain(|e| &e.calendar != id || e.dirty);
            let theirs = sync.events.iter().filter(|e| &e.calendar == id && !changed_here.contains(&(e.event.uid.clone(), e.event.recurrence_id)));
            p.events.extend(theirs.cloned());
        }
        for e in sync.errors.iter().take(3) {
            self.notice(format!("calendars and contacts: {e}"), true);
        }
        self.save_pim(account);
    }
}

/// What a sync found of the CalDAV task lists (issue #201), merged as calendars' events are:
/// the lists as the server has them; for each read in full, the server's tasks in place of
/// what's here, unless changed here (and sent next time); a task the server refused for good
/// gives way to its copy. One changed here in a list that's gone goes to the default list.
fn merge_task_lists(
    p: &mut mailcraft_model::pim::Pim,
    lists: &[mailcraft_model::pim::TaskList],
    sync: &mailcraft_model::pim::DavSync,
    kept: &HashSet<&str>,
    refused: &HashSet<&str>,
) {
    for t in p.tasks.iter_mut().filter(|t| !t.list.is_empty() && refused.contains(t.task.uid.as_str())) {
        t.dirty = false;
    }
    let listed: HashSet<&str> = lists.iter().map(|l| l.id.as_str()).collect();
    p.tasks.retain(|t| t.list.is_empty() || listed.contains(t.list.as_str()) || t.dirty);
    p.task_lists = lists.to_vec();
    let fallback = p.default_task_list().map(|l| l.id.clone()).unwrap_or_default();
    for t in p.tasks.iter_mut().filter(|t| !t.list.is_empty() && !listed.contains(t.list.as_str())) {
        t.list = fallback.clone();
        t.href.clear();
        t.etag.clear();
    }
    for list in &sync.synced_task_lists {
        // Changed here and not yet stored, though the server has it in this list (stored by a
        // sync taken in after this one began): its item there, to send the change over it.
        for t in p.tasks.iter_mut().filter(|t| &t.list == list && t.dirty && t.href.is_empty()) {
            if let Some(x) = sync.list_tasks.iter().find(|x| &x.list == list && x.task.uid == t.task.uid) {
                t.href = x.href.clone();
                t.etag = x.etag.clone();
            }
        }
        p.tasks.retain(|t| &t.list != list || t.dirty || t.href.is_empty() || kept.contains(t.href.as_str()));
        // Changed here in this list: a copy of the same task in another list is another item.
        let changed_here: HashSet<String> = p.tasks.iter().filter(|t| t.dirty && &t.list == list).map(|t| t.task.uid.clone()).collect();
        let mut taken: Vec<&mailcraft_model::pim::StoredTask> = Vec::new();
        let mut uids: HashSet<&str> = HashSet::new();
        for x in sync.list_tasks.iter().filter(|x| &x.list == list).rev() {
            if !changed_here.contains(&x.task.uid) && uids.insert(x.task.uid.as_str()) {
                taken.push(x);
            }
        }
        p.tasks.retain(|t| !(&t.list == list && uids.contains(t.task.uid.as_str())));
        p.tasks.extend(taken.into_iter().rev().cloned());
    }
}

/// Whether a calendar is that of someone opened here by hand (Open shared calendar, a calendar
/// group), still open.
fn opened_by_hand(shared: &[String], calendar: &str) -> bool {
    mailcraft_model::pim::shared_calendar_owner(calendar).is_some_and(|e| shared.iter().any(|s| s == e))
}

/// The events a server sent, each under the uid kept here for it. A meeting's uid is kept as
/// its organizer wrote it, and a server stores it without the characters a line can't hold
/// (`ical::stored_uid`): its copy is that meeting's, not another one beside it, and an answer
/// still names the uid the organizer looks for. Nearly always the server's, as they are.
fn as_kept<'a>(here: &[StoredEvent], server: &'a [StoredEvent]) -> std::borrow::Cow<'a, [StoredEvent]> {
    let odd = |uid: &str| uid.contains(|c| !ical::allowed(c));
    let kept: std::collections::HashMap<String, &str> =
        here.iter().filter(|e| odd(&e.event.uid)).map(|e| (ical::stored_uid(&e.event.uid), e.event.uid.as_str())).collect();
    if kept.is_empty() {
        return std::borrow::Cow::Borrowed(server);
    }
    let mut server = server.to_vec();
    for e in &mut server {
        if let Some(uid) = kept.get(&e.event.uid) {
            e.event.uid = (*uid).to_string();
        }
    }
    std::borrow::Cow::Owned(server)
}
