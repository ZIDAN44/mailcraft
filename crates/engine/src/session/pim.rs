//! Calendar and People: each account's events and contacts, kept in its cache (`pim.json`).
//! Accounts whose server offers CalDAV / CardDAV sync them (see `net::dav`); the others, and the
//! sample mailbox, keep a calendar and an address book on this computer.

use mailcraft_model::dates::{civil_from_days, days_from_civil};
use mailcraft_model::ical::{self, CalEvent};
use mailcraft_model::pim::{LOCAL, Pim, StoredCard, StoredEvent, Tombstone};
use mailcraft_model::vcard::Card;

use super::*;

pub use mailcraft_model::pim::PIM_DOC;

/// The part of the app on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Module {
    #[default]
    Mail,
    Calendar,
    People,
    /// Outlook's Notes.
    Notes,
}

/// How much of the calendar is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CalMode {
    Day,
    #[default]
    Week,
    Month,
}

/// One occurrence of an event, ready to draw. Times are Unix seconds; all-day events start at
/// local midnight of their first day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgendaItem {
    pub account: String,
    pub calendar: String,
    pub uid: String,
    /// When this occurrence starts as the event's rule has it (names it for edits).
    pub occurrence: i64,
    pub recurring: bool,
    pub summary: String,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub color: u8,
    /// `TENTATIVE`, `CANCELLED`…
    pub status: String,
    /// The user's reply is awaited (`NEEDS-ACTION`).
    pub needs_reply: bool,
}

/// A person in an address book.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub account: String,
    pub book: String,
    pub card: Card,
}

impl Session {
    pub(crate) fn load_pim(&mut self) {
        let Some(store) = self.store.clone() else { return };
        for a in &self.accounts {
            let mut p: Pim = store.load_account_doc(&a.config.id, PIM_DOC).unwrap_or_default();
            p.ensure_defaults();
            self.pim.insert(a.config.id.clone(), p);
        }
    }

    pub(crate) fn save_pim(&mut self, account: &str) {
        self.contacts = None;
        self.pim_revision = self.pim_revision.wrapping_add(1);
        let (Some(store), Some(p)) = (&self.store, self.pim.get(account)) else { return };
        if let Err(e) = store.save_account_doc(account, PIM_DOC, p) {
            self.notice(format!("calendar and contacts could not be saved: {e}"), true);
        }
    }

    /// An account's calendars and address books (made on first use).
    pub fn pim_mut(&mut self, account: &str) -> &mut Pim {
        self.pim.entry(account.to_string()).or_insert_with(|| {
            let mut p = Pim::default();
            p.ensure_defaults();
            p
        })
    }

    /// The account new events and people go to: the one on screen, else the first.
    fn pim_account(&self, wanted: Option<&str>) -> Result<String, CmdError> {
        match wanted {
            Some(a) if self.account(a).is_some() => Ok(a.to_string()),
            Some(a) => Err(CmdError::Params(format!("no account `{a}`"))),
            None => self.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Disabled("add an account first".into())),
        }
    }

    // ----- the calendar view -----

    /// The local day (days since the epoch) the calendar shows.
    pub fn cal_day(&self) -> i64 {
        self.cal_day.unwrap_or_else(|| self.clock.now.saturating_add(i64::from(self.clock.offset)).div_euclid(86_400))
    }

    /// The local days on screen: a day, the week (Monday first) or the weeks of the month.
    pub fn cal_days(&self) -> (i64, i64) {
        let d = self.cal_day();
        let monday = |x: i64| x - (x + 3).rem_euclid(7);
        match self.cal_mode {
            CalMode::Day => (d, d + 1),
            CalMode::Week => (monday(d), monday(d) + 7),
            CalMode::Month => {
                let (y, m, _) = civil_from_days(d);
                let first = monday(days_from_civil(y, m, 1));
                (first, first + 42)
            }
        }
    }

    /// The Unix time a local day starts.
    pub fn local_midnight(&self, day: i64) -> i64 {
        day.saturating_mul(86_400) - i64::from(self.clock.offset)
    }

    /// Move the calendar: to a date, today, or by `step` days, weeks or months (its mode).
    pub fn cal_goto(&mut self, date: Option<&str>, step: i64) -> CmdResult {
        let mut d = self.cal_day();
        match date.map(str::trim) {
            Some("today") | Some("") => d = self.clock.now.saturating_add(i64::from(self.clock.offset)).div_euclid(86_400),
            Some(s) => {
                d = parse_local(s, 0)
                    .map(|(t, _)| t.div_euclid(86_400))
                    .ok_or_else(|| CmdError::Params(format!("`{s}` is not a date (YYYY-MM-DD)")))?;
            }
            None => {}
        }
        let step = step.clamp(-1200, 1200);
        d = match self.cal_mode {
            CalMode::Day => d + step,
            CalMode::Week => d + 7 * step,
            CalMode::Month => {
                let (y, m, day) = civil_from_days(d);
                let total = i64::from(m) - 1 + step;
                let (yy, mm) = (y + total.div_euclid(12), u32::try_from(total.rem_euclid(12) + 1).unwrap_or(1));
                days_from_civil(yy, mm, day.min(28))
            }
        };
        self.cal_day = Some(d);
        let (y, m, dd) = civil_from_days(d);
        Ok(serde_json::json!({"date": format!("{y:04}-{m:02}-{dd:02}"), "mode": self.cal_mode}))
    }

    /// The occurrences overlapping `[from, to)`, by start, from every calendar.
    pub fn agenda(&self, from: i64, to: i64) -> Vec<AgendaItem> {
        let off = i64::from(self.clock.offset);
        let mut out = Vec::new();
        for (account, p) in &self.pim {
            for cal in &p.calendars {
                let stored: Vec<&StoredEvent> = p.events.iter().filter(|e| e.calendar == cal.id).collect();
                let events: Vec<CalEvent> = stored.iter().map(|e| e.event.clone()).collect();
                // All-day events are dates, not times: widen the window by a day each way.
                for o in ical::occurrences(&events, from - 86_400, to + 86_400) {
                    let Some(e) = events.get(o.event) else { continue };
                    let (start, end) = if e.all_day { (o.start - off, o.end - off) } else { (o.start, o.end) };
                    if start >= to || end.max(start + 1) <= from {
                        continue;
                    }
                    let mine: Vec<String> =
                        self.accounts.iter().flat_map(|a| [a.config.email.to_ascii_lowercase(), a.config.username.to_ascii_lowercase()]).collect();
                    let needs_reply = e.attendees.iter().any(|a| mine.contains(&a.email.to_ascii_lowercase()) && a.partstat == "NEEDS-ACTION");
                    out.push(AgendaItem {
                        account: account.clone(),
                        calendar: cal.id.clone(),
                        uid: e.uid.clone(),
                        occurrence: e.recurrence_id.unwrap_or(o.start),
                        recurring: e.rrule.is_some() || e.recurrence_id.is_some(),
                        summary: e.summary.clone(),
                        location: e.location.clone(),
                        start,
                        end,
                        all_day: e.all_day,
                        color: cal.color,
                        status: e.status.clone(),
                        needs_reply,
                    });
                }
            }
        }
        out.sort_by(|a, b| (b.all_day, a.start, &a.summary).cmp(&(a.all_day, b.start, &b.summary)));
        out
    }

    /// The event (master, not an instance) with a uid, and its account.
    pub fn find_event(&self, uid: &str) -> Option<(&str, &StoredEvent)> {
        self.pim.iter().find_map(|(a, p)| p.events.iter().find(|e| e.event.uid == uid && e.event.recurrence_id.is_none()).map(|e| (a.as_str(), e)))
    }

    fn new_uid(&mut self) -> String {
        let n = self.fresh_id();
        format!("{:016x}-{n}@mailcraft", self.seed)
    }

    /// Add an event; returns its uid. Events in a server calendar are sent there.
    pub fn create_event(&mut self, account: Option<&str>, calendar: Option<&str>, mut e: CalEvent) -> CmdResult {
        let account = self.pim_account(account)?;
        check_event(&e)?;
        if e.uid.trim().is_empty() {
            e.uid = self.new_uid();
        }
        let p = self.pim_mut(&account);
        let cal = match calendar {
            Some(c) => p
                .calendars
                .iter()
                .find(|x| x.id == c || x.name.eq_ignore_ascii_case(c))
                .ok_or_else(|| CmdError::Params(format!("no calendar `{c}`")))?,
            None => p.default_calendar().ok_or_else(|| CmdError::Disabled("no calendar to add to".into()))?,
        };
        if cal.read_only {
            return Err(CmdError::Params(format!("{} can't be changed", cal.name)));
        }
        let id = cal.id.clone();
        let uid = e.uid.clone();
        p.events.retain(|x| x.event.uid != uid);
        p.events.push(StoredEvent { calendar: id.clone(), href: String::new(), etag: String::new(), event: e, dirty: id != LOCAL });
        self.after_pim_change(&account, id != LOCAL);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    /// Change an event (all of its occurrences).
    pub fn update_event(&mut self, uid: &str, change: impl FnOnce(&mut CalEvent)) -> CmdResult {
        let (account, _) =
            self.find_event(uid).map(|(a, e)| (a.to_string(), e.calendar.clone())).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let p = self.pim_mut(&account);
        let Some(stored) = p.events.iter_mut().find(|e| e.event.uid == uid && e.event.recurrence_id.is_none()) else {
            return Err(CmdError::Params(format!("no event `{uid}`")));
        };
        if p.calendars.iter().any(|c| c.id == stored.calendar && c.read_only) {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        let mut e = stored.event.clone();
        change(&mut e);
        check_event(&e)?;
        e.sequence = e.sequence.saturating_add(1);
        stored.event = e;
        let remote = stored.calendar != LOCAL;
        stored.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid}))
    }

    /// Delete an event, or one occurrence of a recurring one (`occurrence`: its start).
    pub fn delete_event(&mut self, uid: &str, occurrence: Option<i64>) -> CmdResult {
        let account = self.find_event(uid).map(|(a, _)| a.to_string()).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let p = self.pim_mut(&account);
        let mut remote = false;
        match occurrence {
            Some(at) if p.events.iter().any(|e| e.event.uid == uid && e.event.rrule.is_some()) => {
                // One occurrence: an exception on the rule, and any changed instance goes.
                p.events.retain(|e| !(e.event.uid == uid && e.event.recurrence_id == Some(at)));
                if let Some(m) = p.events.iter_mut().find(|e| e.event.uid == uid && e.event.recurrence_id.is_none()) {
                    if !m.event.exdates.contains(&at) {
                        m.event.exdates.push(at);
                    }
                    m.event.sequence = m.event.sequence.saturating_add(1);
                    remote = m.calendar != LOCAL;
                    m.dirty |= remote;
                }
            }
            _ => {
                let gone: Vec<StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid).cloned().collect();
                for g in gone.iter().filter(|g| !g.href.is_empty()) {
                    remote = true;
                    p.deleted.push(Tombstone { href: g.href.clone(), etag: g.etag.clone(), calendar: g.calendar.clone() });
                }
                p.events.retain(|e| e.event.uid != uid);
            }
        }
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": uid}))
    }

    /// Save, and ask for a sync when the server has something to learn.
    pub(crate) fn after_pim_change(&mut self, account: &str, remote: bool) {
        self.save_pim(account);
        if remote && self.account(account).is_some_and(|a| !a.is_local()) {
            self.jobs.push(Job::SyncDav { account: account.to_string() });
        }
    }

    // ----- syncing with the server -----

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
            p.events.retain(|e| e.calendar == LOCAL || list.iter().any(|c| c.id == e.calendar) || e.dirty);
            for cal in &sync.synced_calendars {
                p.events.retain(|e| &e.calendar != cal || e.dirty || e.href.is_empty() || sync.kept.contains(&e.href));
                for x in sync.events.iter().filter(|x| &x.calendar == cal) {
                    if !p.events.iter().any(|e| e.dirty && e.event.uid == x.event.uid) {
                        p.events.retain(|e| !(e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id && !e.dirty));
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

    // ----- people -----

    /// Everyone in the address books matching a search, by name.
    pub fn people(&self, query: &str) -> Vec<Person> {
        let mut out: Vec<Person> = self
            .pim
            .iter()
            .flat_map(|(a, p)| {
                p.cards.iter().filter(|c| c.card.matches(query)).map(|c| Person { account: a.clone(), book: c.book.clone(), card: c.card.clone() })
            })
            .collect();
        out.sort_by_key(|p| p.card.display().to_lowercase());
        out
    }

    pub fn find_person(&self, uid: &str) -> Option<Person> {
        self.pim.iter().find_map(|(a, p)| {
            p.cards.iter().find(|c| c.card.uid == uid).map(|c| Person { account: a.clone(), book: c.book.clone(), card: c.card.clone() })
        })
    }

    /// Add someone; returns their uid.
    pub fn create_card(&mut self, account: Option<&str>, mut card: Card) -> CmdResult {
        let account = self.pim_account(account)?;
        check_card(&card)?;
        if card.uid.trim().is_empty() {
            card.uid = self.new_uid();
        }
        let p = self.pim_mut(&account);
        let book = p.default_book().map(|b| b.id.clone()).ok_or_else(|| CmdError::Disabled("no address book to add to".into()))?;
        let uid = card.uid.clone();
        p.cards.retain(|c| c.card.uid != uid);
        p.cards.push(StoredCard { book: book.clone(), href: String::new(), etag: String::new(), card, dirty: book != LOCAL });
        self.after_pim_change(&account, book != LOCAL);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    pub fn update_card(&mut self, uid: &str, change: impl FnOnce(&mut Card)) -> CmdResult {
        let account = self.find_person(uid).map(|p| p.account).ok_or_else(|| CmdError::Params(format!("no contact `{uid}`")))?;
        let p = self.pim_mut(&account);
        let Some(c) = p.cards.iter_mut().find(|c| c.card.uid == uid) else { return Err(CmdError::Params(format!("no contact `{uid}`"))) };
        let mut card = c.card.clone();
        change(&mut card);
        check_card(&card)?;
        c.card = card;
        let remote = c.book != LOCAL;
        c.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid}))
    }

    pub fn delete_card(&mut self, uid: &str) -> CmdResult {
        let account = self.find_person(uid).map(|p| p.account).ok_or_else(|| CmdError::Params(format!("no contact `{uid}`")))?;
        let p = self.pim_mut(&account);
        let mut remote = false;
        for c in p.cards.iter().filter(|c| c.card.uid == uid && !c.href.is_empty()) {
            remote = true;
            p.deleted.push(Tombstone { href: c.href.clone(), etag: c.etag.clone(), ..Default::default() });
        }
        p.cards.retain(|c| c.card.uid != uid);
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": uid}))
    }
}

/// A note with the account it's in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteRef {
    pub account: String,
    pub note: mailcraft_model::pim::Note,
}

impl Session {
    // ----- other people's calendars -----

    /// Open someone else's calendar from an Exchange account (Outlook's Open Calendar › From
    /// Address Book): read with the account's own, read-only, once the server shows it.
    pub fn open_shared_calendar(&mut self, account: Option<&str>, email: &str) -> CmdResult {
        let account = match account {
            Some(a) => self.pim_account(Some(a))?,
            None => self
                .accounts
                .iter()
                .find(|a| a.config.graph.is_some() || a.config.ews.is_some())
                .map(|a| a.config.id.clone())
                .ok_or_else(|| CmdError::Disabled("other people's calendars open from an Exchange account".into()))?,
        };
        if !self.account(&account).is_some_and(|a| a.config.graph.is_some() || a.config.ews.is_some()) {
            return Err(CmdError::Params("other people's calendars open from an Exchange account".into()));
        }
        let email = email.trim().to_ascii_lowercase();
        let ok =
            mailcraft_model::addr::parse_list(&email).is_ok_and(|l| l.len() == 1 && l.first().is_some_and(|a| a.email.eq_ignore_ascii_case(&email)));
        if !ok {
            return Err(CmdError::Params(format!("not an email address: {email}")));
        }
        let mine = self.account(&account).is_some_and(|a| a.config.email.eq_ignore_ascii_case(&email));
        if mine {
            return Err(CmdError::Params("that's this account's own calendar".into()));
        }
        let p = self.pim_mut(&account);
        if !p.shared.contains(&email) {
            if p.shared.len() >= 50 {
                return Err(CmdError::Params("at most 50 other calendars can be open".into()));
            }
            p.shared.push(email.clone());
        }
        self.after_pim_change(&account, true);
        Ok(serde_json::json!({"account": account, "calendar": mailcraft_model::pim::shared_calendar_id(&email)}))
    }

    /// Close someone else's calendar: it and its events go from this computer.
    pub fn close_shared_calendar(&mut self, email: &str) -> CmdResult {
        let email = email.trim().to_ascii_lowercase();
        let id = mailcraft_model::pim::shared_calendar_id(&email);
        let account = self.pim.iter().find(|(_, p)| p.shared.contains(&email) || p.calendars.iter().any(|c| c.id == id)).map(|(a, _)| a.clone());
        let Some(account) = account else { return Err(CmdError::Params(format!("no calendar of {email} is open"))) };
        let p = self.pim_mut(&account);
        p.shared.retain(|e| *e != email);
        p.calendars.retain(|c| c.id != id);
        p.events.retain(|e| e.calendar != id);
        self.after_pim_change(&account, false);
        Ok(serde_json::json!({"closed": email}))
    }

    // ----- notes -----

    /// Every note matching a search (in its text), the most recently changed first.
    pub fn notes(&self, query: &str) -> Vec<NoteRef> {
        let q = query.trim().to_lowercase();
        let mut out: Vec<NoteRef> = self
            .pim
            .iter()
            .flat_map(|(a, p)| {
                p.notes
                    .iter()
                    .filter(|n| q.is_empty() || n.note.text.to_lowercase().contains(&q))
                    .map(|n| NoteRef { account: a.clone(), note: n.note.clone() })
            })
            .collect();
        out.sort_by(|a, b| b.note.modified.cmp(&a.note.modified).then_with(|| a.note.uid.cmp(&b.note.uid)));
        out
    }

    fn note_account(&self, uid: &str) -> Result<String, CmdError> {
        self.pim
            .iter()
            .find(|(_, p)| p.notes.iter().any(|n| n.note.uid == uid))
            .map(|(a, _)| a.clone())
            .ok_or_else(|| CmdError::Params(format!("no note `{uid}`")))
    }

    /// Notes go to the server only where it keeps them (Exchange's Notes folder).
    fn notes_sync(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| !a.is_local() && a.config.ews.is_some())
    }

    /// A new note; returns its uid.
    pub fn create_note(&mut self, account: Option<&str>, text: &str, color: Option<u8>) -> CmdResult {
        let account = self.pim_account(account)?;
        let note = mailcraft_model::pim::Note {
            uid: self.new_uid(),
            text: cap_note(text),
            color: color.unwrap_or(mailcraft_model::pim::NOTE_YELLOW).min(mailcraft_model::pim::NOTE_COLORS - 1),
            modified: self.clock.now,
        };
        let uid = note.uid.clone();
        let remote = self.notes_sync(&account);
        let p = self.pim_mut(&account);
        if p.notes.len() >= MAX_NOTES {
            return Err(CmdError::Params(format!("an account keeps at most {MAX_NOTES} notes")));
        }
        p.notes.push(mailcraft_model::pim::StoredNote { note, dirty: remote, ..Default::default() });
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    /// Change a note's text or colour.
    pub fn update_note(&mut self, uid: &str, text: Option<&str>, color: Option<u8>) -> CmdResult {
        let account = self.note_account(uid)?;
        let remote = self.notes_sync(&account);
        let now = self.clock.now;
        let p = self.pim_mut(&account);
        let Some(n) = p.notes.iter_mut().find(|n| n.note.uid == uid) else { return Err(CmdError::Params(format!("no note `{uid}`"))) };
        let before = n.note.clone();
        if let Some(t) = text {
            n.note.text = cap_note(t);
        }
        if let Some(c) = color {
            n.note.color = c.min(mailcraft_model::pim::NOTE_COLORS - 1);
        }
        if n.note == before {
            return Ok(serde_json::json!({"uid": uid, "changed": false}));
        }
        n.note.modified = now;
        n.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid}))
    }

    pub fn delete_note(&mut self, uid: &str) -> CmdResult {
        let account = self.note_account(uid)?;
        let p = self.pim_mut(&account);
        let mut remote = false;
        for n in p.notes.iter().filter(|n| n.note.uid == uid && !n.href.is_empty()) {
            remote = true;
            p.deleted.push(Tombstone { href: n.href.clone(), etag: n.etag.clone(), ..Default::default() });
        }
        p.notes.retain(|n| n.note.uid != uid);
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": uid}))
    }
}

/// Most notes an account keeps, and most characters in one.
const MAX_NOTES: usize = 5_000;
const MAX_NOTE_CHARS: usize = 100_000;

fn cap_note(text: &str) -> String {
    text.chars().take(MAX_NOTE_CHARS).collect()
}

fn check_event(e: &CalEvent) -> Result<(), CmdError> {
    if e.summary.trim().is_empty() {
        return Err(CmdError::Params("give the event a title".into()));
    }
    if e.end < e.start {
        return Err(CmdError::Params("the event ends before it starts".into()));
    }
    if e.end - e.start > 366 * 86_400 {
        return Err(CmdError::Params("an event can last a year at most".into()));
    }
    Ok(())
}

fn check_card(c: &Card) -> Result<(), CmdError> {
    if c.display().trim().is_empty() {
        return Err(CmdError::Params("give the contact a name or an address".into()));
    }
    for e in &c.emails {
        if !addr::is_valid_email(e.value.trim()) {
            return Err(CmdError::Params(format!("`{}` is not an email address", e.value)));
        }
    }
    Ok(())
}

/// A local date (`YYYY-MM-DD`, all day) or date and time (`YYYY-MM-DDTHH:MM`, or with a space),
/// or Unix seconds, as (Unix seconds, all day). `offset` is the local UTC offset; all-day dates
/// come back at UTC midnight (as events keep them).
pub fn parse_local(s: &str, offset: i64) -> Option<(i64, bool)> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i64>() {
        return (n.abs() < 400_000_000_000).then_some((n, false));
    }
    let num = |a: usize, b: usize| s.get(a..b).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse::<i64>().ok());
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if s.get(4..5) != Some("-") || s.get(7..8) != Some("-") || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let day = days_from_civil(y, u32::try_from(m).ok()?, u32::try_from(d).ok()?);
    if s.len() == 10 {
        return Some((day * 86_400, true));
    }
    if !matches!(s.get(10..11), Some("T" | " ")) {
        return None;
    }
    let (h, mi) = (num(11, 13)?, num(14, 16)?);
    if h > 23 || mi > 59 || s.get(13..14) != Some(":") {
        return None;
    }
    Some((day * 86_400 + h * 3600 + mi * 60 - offset, false))
}
