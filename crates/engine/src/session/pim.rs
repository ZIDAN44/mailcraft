//! Calendar and People: each account's events and contacts, kept in its cache (`pim.json`) with
//! its notes and tasks. Accounts whose server offers CalDAV / CardDAV sync them (see `net::dav`);
//! the others, and the sample mailbox, keep a calendar and an address book on this computer.

use mailcraft_model::dates::{civil_from_days, day_string, days_from_civil, weekday_index};
use mailcraft_model::ical::{self, CalEvent};
use mailcraft_model::pim::{LOCAL, Pim, StoredCard, StoredEvent, Tombstone};
use mailcraft_model::vcard::Card;

use super::*;

pub use mailcraft_model::pim::PIM_DOC;

pub mod directory;
pub use directory::Directory;
mod find;
pub use find::{EVENT_HITS_MAX, EventHit, EventWhen};
mod free_busy;
mod free_time;
pub use free_time::FREE_LENGTHS;
#[cfg(test)]
pub(crate) use free_time::{NONE_YET, UNKNOWN_FREE};
mod groups;
pub use free_busy::{FreeBusyKept, FreeBusyState};
pub mod invite;
mod local;
pub use local::LocalEvents;
mod moving;
mod reminders;
mod series;
mod sync;
mod tasks;
pub use tasks::{ListedTask, TASKS_CALENDAR, TaskRef, TasksOrder};
mod todo;
pub use todo::{TODO_BAR_SPLIT, TODO_BAR_WIDTH, TodoBar, TodoGroup, TodoItem, TodoMail, TodoWhat};
pub mod web;
mod work_time;

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
    /// Outlook's Tasks.
    Tasks,
}

/// How much of the calendar is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CalMode {
    Day,
    /// Monday to Friday.
    WorkWeek,
    #[default]
    Week,
    Month,
    /// A day's hours across, a row for each calendar shown (Outlook's Schedule view, issue
    /// #141): when several people are free together.
    Schedule,
}

/// A reminder whose time has come (File › Reminders, as Outlook's Reminders window lists them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DueReminder {
    /// Names it for snoozing and dismissing: `<account>|<uid>|<occurrence>`.
    pub key: String,
    pub account: String,
    pub uid: String,
    pub occurrence: i64,
    pub summary: String,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    /// When it was due (its start, less its reminder; or the end of a snooze).
    pub due: i64,
    /// A task's reminder ([`Session::task_reminders`]): `uid` is the task's.
    pub task: bool,
    /// A flagged message's reminder (gap 9): the message (one of its copies), its follow-up id
    /// the `uid`.
    pub message: Option<MessageKey>,
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
    /// The colour of its first category (an index into the category palette), which Outlook
    /// draws an appointment in rather than its calendar's.
    pub category: Option<u8>,
    /// `TENTATIVE`, `CANCELLED`…
    pub status: String,
    /// The user's reply is awaited (`NEEDS-ACTION`).
    pub needs_reply: bool,
    /// Who sees what of it, and how it shows its time (issue #32).
    pub privacy: ical::Privacy,
    pub show_as: ical::ShowAs,
    /// A task due that day ([`Session::task_items`]), not an event: `uid` is the task's.
    pub task: bool,
    /// A time a colleague's calendar shared for its free/busy time alone has taken (issue
    /// #140): its subject only when they share that, and nothing to open.
    pub busy_only: bool,
}

/// A person in an address book, borrowed from it: a card can carry its picture and files
/// (megabytes of base64), and the People module lists everyone on every frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Person<'a> {
    pub account: &'a str,
    pub book: &'a str,
    pub card: &'a Card,
    /// Its place among its account's cards (`Pim::cards`), to find it again without a search.
    pub at: usize,
}

/// A calendar colour of this or above is an account's colour ([`Session::calendar_color`]).
pub const ACCOUNT_COLOR: u8 = 16;

impl Session {
    pub(crate) fn load_pim(&mut self) {
        let Some(store) = self.store.clone() else { return };
        let mut unread = Vec::new();
        for a in &self.accounts {
            let mut p: Pim = match store.load_account_doc(&a.config.id, PIM_DOC) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("{}: calendar and contacts couldn't be read: {e}", a.config.id);
                    unread.push(a.config.id.clone());
                    Pim::default()
                }
            };
            p.ensure_defaults();
            self.pim.insert(a.config.id.clone(), p);
        }
        if !unread.is_empty() {
            self.notice("Calendar and contacts couldn't be read: changes to them won't be saved", true);
        }
        self.pim_unread.extend(unread);
    }

    pub(crate) fn save_pim(&mut self, account: &str) {
        self.drop_contacts();
        self.pim_revision = self.pim_revision.wrapping_add(1);
        // What couldn't be read isn't written over with what's shown in its place.
        if self.pim_unread.contains(account) {
            return;
        }
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

    /// The account a command acts on: the one it names (`wanted`), which must be one, else the
    /// one on screen, else the first. The rule each command taking an `account` follows (copies
    /// of it took any name, or refused with other words).
    pub(crate) fn account_or_shown(&self, wanted: Option<&str>) -> Result<String, CmdError> {
        match wanted {
            Some(a) if self.account(a).is_some() => Ok(a.to_string()),
            Some(a) => Err(CmdError::Params(format!("no account `{a}`"))),
            None => self.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Disabled("add an account first".into())),
        }
    }

    // ----- the calendar view -----

    /// The local day (days since the epoch) the calendar shows.
    pub fn cal_day(&self) -> i64 {
        self.cal_day.unwrap_or_else(|| mailcraft_model::dates::today(&self.clock))
    }

    /// The local days on screen: a day (the Schedule view's too), the work week (from its first
    /// worked day to its last, issue #185: `cal_day_list` has those alone), the week (Monday
    /// first) or the weeks of the month.
    pub fn cal_days(&self) -> (i64, i64) {
        let d = self.cal_day();
        let monday = |x: i64| x - i64::from(weekday_index(x));
        match self.cal_mode {
            CalMode::Day | CalMode::Schedule => (d, d + 1),
            CalMode::WorkWeek => match self.work_week_days()[..] {
                [first, .., last] => (first, last + 1),
                [only] => (only, only + 1),
                [] => (monday(d), monday(d) + 5),
            },
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
        self.clock.midnight(day)
    }

    /// Move the calendar: to a date, today, or by `step` days, weeks or months (its mode).
    pub fn cal_goto(&mut self, date: Option<&str>, step: i64) -> CmdResult {
        let mut d = self.cal_day();
        match date.map(str::trim) {
            Some("today") | Some("") => d = mailcraft_model::dates::today(&self.clock),
            Some(s) => {
                d = parse_local(s, &Clock::default())
                    .map(|(t, _)| t.div_euclid(86_400))
                    .ok_or_else(|| CmdError::Params(format!("`{s}` is not a date (YYYY-MM-DD)")))?;
            }
            None => {}
        }
        let step = step.clamp(-1200, 1200);
        // Moved by hand (not the view changed): a time found free is looked for from the days
        // shown again.
        if date.is_some() || step != 0 {
            self.free_slot = None;
        }
        d = match self.cal_mode {
            CalMode::Day | CalMode::Schedule => d + step,
            CalMode::WorkWeek | CalMode::Week => d + 7 * step,
            CalMode::Month => {
                let (y, m, day) = civil_from_days(d);
                let total = i64::from(m) - 1 + step;
                let (yy, mm) = (y + total.div_euclid(12), u32::try_from(total.rem_euclid(12) + 1).unwrap_or(1));
                days_from_civil(yy, mm, day.min(28))
            }
        };
        self.cal_day = Some(d);
        Ok(serde_json::json!({"date": day_string(d), "mode": self.cal_mode}))
    }

    /// The key a calendar is shown or hidden under (`Prefs::hidden_calendars`).
    pub fn calendar_key(account: &str, calendar: &str) -> String {
        format!("{account}/{calendar}")
    }

    /// Whether a calendar is shown (ticked) in the calendar.
    pub fn calendar_shown(&self, account: &str, calendar: &str) -> bool {
        !self.prefs.hidden_calendars.contains(&Session::calendar_key(account, calendar))
    }

    /// The calendars shown, in the order the side pane lists them: (account, id, name, colour).
    pub fn shown_calendars(&self) -> Vec<(String, String, String, u8)> {
        self.accounts
            .iter()
            .filter_map(|a| self.pim.get(&a.config.id).map(|p| (a, p)))
            .flat_map(|(a, p)| p.calendars.iter().map(move |c| (a.config.id.clone(), c.id.clone(), c.name.clone(), Session::calendar_color(a, c))))
            .filter(|(a, c, _, _)| self.calendar_shown(a, c))
            .collect()
    }

    /// The colour a calendar is drawn in (with its events): an account's own calendar in the
    /// account's colour, as its mail shows it, so each account's calendar can be told apart
    /// (they used to share one); its others in theirs. [`ACCOUNT_COLOR`] and above stand for
    /// an account's colour (`ACCOUNT_COLOR + AccountConfig::color`).
    pub fn calendar_color(a: &AccountState, c: &mailcraft_model::pim::CalendarInfo) -> u8 {
        if c.color == 0 { ACCOUNT_COLOR.saturating_add(a.config.color) } else { c.color }
    }

    /// The occurrences overlapping `[from, to)`, by start, from every calendar that's shown.
    /// (Each with its calendar's colour: [`Session::calendar_color`].)
    pub fn agenda(&self, from: i64, to: i64) -> Vec<AgendaItem> {
        let mut out = self.agenda_where(from, to, |account, cal| self.calendar_shown(account, &cal.id));
        self.free_busy_items(from, to, &mut out);
        out.sort_by(|a, b| (b.all_day, a.start, &a.summary).cmp(&(a.all_day, b.start, &b.summary)));
        out
    }

    /// The events between `from` and `to` of the calendars `keep` takes (an account's id, the
    /// calendar), unsorted.
    pub(super) fn agenda_where(&self, from: i64, to: i64, keep: impl Fn(&str, &mailcraft_model::pim::CalendarInfo) -> bool) -> Vec<AgendaItem> {
        // Every address of the accounts' (their others too: an invitation to one needs its
        // answer like any).
        let mine = self.my_addresses();
        let mut out = Vec::new();
        for (account, p) in &self.pim {
            let Some(acc) = self.account(account) else { continue };
            for cal in p.calendars.iter().filter(|c| keep(account, c)) {
                // Borrowed, not copied: this runs each frame the calendar is drawn, and an
                // imported event can carry files.
                let events: Vec<&CalEvent> = p.events.iter().filter(|e| e.calendar == cal.id).map(|e| &e.event).collect();
                // All-day events are dates, not times: widen the window by a day each way.
                for o in ical::occurrences(&events, from.saturating_sub(86_400), to.saturating_add(86_400)) {
                    let Some(e) = events.get(o.event) else { continue };
                    let (start, end) = if e.all_day { (self.clock.utc(o.start), self.clock.utc(o.end)) } else { (o.start, o.end) };
                    if start >= to || end.max(start.saturating_add(1)) <= from || super::invite::declined_occurrence(e, &mine) {
                        continue;
                    }
                    let needs_reply = e.attendees.iter().any(|a| a.partstat == "NEEDS-ACTION" && super::invite::is_mine(&a.email, &mine));
                    let (privacy, show_as) = ical::showing(e);
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
                        color: Session::calendar_color(acc, cal),
                        category: e.categories.first().map(|c| self.category_color_of(c)),
                        status: e.status.clone(),
                        needs_reply,
                        privacy,
                        show_as,
                        task: false,
                        busy_only: false,
                    });
                }
            }
        }
        out
    }

    /// A category's colour by its name: the one made with that name (mail and events share
    /// them), else one picked from the name (a category from the server MailCraft has no colour
    /// for still gets the same one every time).
    pub fn category_color_of(&self, name: &str) -> u8 {
        if let Some(c) = self.prefs.categories.iter().find(|c| c.name.eq_ignore_ascii_case(name.trim())) {
            return c.color;
        }
        super::organize::categories::color_from_name(name)
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
        let account = self.account_or_shown(account)?;
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
        let account = self.find_event(uid).map(|(a, _)| a.to_string()).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let p = self.pim_mut(&account);
        let Some(stored) = p.events.iter_mut().find(|e| e.event.uid == uid && e.event.recurrence_id.is_none()) else {
            return Err(CmdError::Params(format!("no event `{uid}`")));
        };
        if p.calendars.iter().any(|c| c.id == stored.calendar && c.read_only) {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        // Only its time came (SOGo's copy for a colleague who may see no more): saving it would
        // write that copy over the owner's event, which may be in a calendar the user can change.
        if mailcraft_model::ical::details_hidden(&stored.event) {
            return Err(CmdError::Params("only this event's time can be seen: it can't be changed".into()));
        }
        let old = stored.event.clone();
        let mut e = old.clone();
        change(&mut e);
        check_event(&e)?;
        // Saved unchanged: nothing to write, and no new version for the guests (issue #102).
        if e == old {
            return Ok(serde_json::json!({"uid": uid}));
        }
        // Only what's the user's own changed (its reminder, how it shows their time): the same
        // version, so the guests aren't asked again.
        if !ical::same_for_guests(&old, &e) {
            e.sequence = e.sequence.saturating_add(1);
        }
        let calendar = stored.calendar.clone();
        let remote = calendar != LOCAL;
        let occurrences = series::carry_occurrences(p, &calendar, uid, &old, &mut e);
        if let Some(stored) = p.events.iter_mut().find(|x| x.event.uid == uid && x.calendar == calendar && x.event.recurrence_id.is_none()) {
            stored.event = e;
            stored.dirty |= remote;
        }
        self.after_pim_change(&account, remote || occurrences);
        Ok(serde_json::json!({"uid": uid}))
    }

    /// Delete an event, or one occurrence of a recurring one (`occurrence`: its start).
    pub fn delete_event(&mut self, uid: &str, occurrence: Option<i64>) -> CmdResult {
        let (account, calendar) =
            self.find_event(uid).map(|(a, e)| (a.to_string(), e.calendar.clone())).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let p = self.pim_mut(&account);
        // As changing it: not in a calendar that's read-only (someone else's, one subscribed to).
        if p.calendars.iter().any(|c| c.id == calendar && c.read_only) {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        let series = p.events.iter().any(|e| e.event.uid == uid && e.event.rrule.is_some());
        // A series that keeps as many occurrences taken out as it can: this one would stay, so
        // it isn't said to be deleted (nor its guests told it's off).
        if let Some(at) = occurrence
            && series
            && super::invite::exceptions_full(p, uid, at)
        {
            return Err(CmdError::Failed("too many occurrences of this series are deleted to delete another: delete the series".into()));
        }
        // As declining or a cancellation takes them out: a changed occurrence that's an item of
        // its own on the server is deleted there, and each item once.
        let remote = match occurrence {
            Some(at) if series => {
                let remote = super::invite::drop_occurrence(p, uid, at);
                if let Some(m) = p.events.iter_mut().find(|e| e.event.uid == uid && e.event.recurrence_id.is_none()) {
                    m.event.sequence = m.event.sequence.saturating_add(1);
                }
                remote
            }
            _ => {
                let gone: Vec<StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid).cloned().collect();
                p.events.retain(|e| e.event.uid != uid);
                super::invite::delete_items(p, &gone)
            }
        };
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": uid}))
    }

    /// Save, and ask for a sync when the server has something to learn (once: a second sync
    /// asked for meanwhile would start before the first one's results are merged, and send its
    /// new items again).
    pub(crate) fn after_pim_change(&mut self, account: &str, remote: bool) {
        self.save_pim(account);
        let job = Job::SyncDav { account: account.to_string() };
        if remote && self.account(account).is_some_and(|a| a.config.has_server()) && !self.jobs.contains(&job) {
            self.jobs.push(job);
        }
    }

    // ----- people -----

    /// Everyone in the address books matching a search, by name.
    pub fn people(&self, query: &str) -> Vec<Person<'_>> {
        let q = mailcraft_model::vcard::CardSearch::new(query);
        let mut out: Vec<Person<'_>> = self
            .pim
            .iter()
            .flat_map(|(a, p)| {
                p.cards.iter().enumerate().filter(|(_, c)| c.card.matches_search(&q)).map(|(at, c)| Person {
                    account: a,
                    book: &c.book,
                    card: &c.card,
                    at,
                })
            })
            .collect();
        out.sort_by_cached_key(|p| p.card.display().to_lowercase());
        out
    }

    /// The contact someone is saved as, by their address (of several, the first by name, as
    /// [`Session::people`] lists them). The reading pane asks it of each name above a message
    /// on every frame: searching the address book for each took milliseconds.
    pub fn contact_by_address(&self, email: &str) -> Option<&Card> {
        let email = email.trim();
        if email.is_empty() {
            return None;
        }
        let has = |c: &&Card| c.emails.iter().any(|e| e.value.eq_ignore_ascii_case(email));
        let mut found = self.pim.values().flat_map(|p| p.cards.iter().map(|c| &c.card)).filter(has);
        let first = found.next()?;
        Some(found.fold(first, |best, c| if c.display().to_lowercase() < best.display().to_lowercase() { c } else { best }))
    }

    pub fn find_person(&self, uid: &str) -> Option<Person<'_>> {
        self.pim.iter().find_map(|(a, p)| {
            p.cards.iter().enumerate().find(|(_, c)| c.card.uid == uid).map(|(at, c)| Person { account: a, book: &c.book, card: &c.card, at })
        })
    }

    /// Add someone; returns their uid.
    pub fn create_card(&mut self, account: Option<&str>, mut card: Card) -> CmdResult {
        let account = self.account_or_shown(account)?;
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

    /// Whether a person is in an address book that can't be changed here (an organisation's
    /// shared contacts).
    pub fn person_read_only(&self, uid: &str) -> bool {
        self.pim.values().any(|p| p.cards.iter().any(|c| c.card.uid == uid && p.books.iter().any(|b| b.id == c.book && b.read_only)))
    }

    pub fn update_card(&mut self, uid: &str, change: impl FnOnce(&mut Card)) -> CmdResult {
        let account = self.find_person(uid).map(|p| p.account.to_string()).ok_or_else(|| CmdError::Params(format!("no contact `{uid}`")))?;
        if self.person_read_only(uid) {
            return Err(CmdError::Disabled("this contact is in a shared address book that can't be changed here".into()));
        }
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
        let account = self.find_person(uid).map(|p| p.account.to_string()).ok_or_else(|| CmdError::Params(format!("no contact `{uid}`")))?;
        if self.person_read_only(uid) {
            return Err(CmdError::Disabled("this contact is in a shared address book that can't be changed here".into()));
        }
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
pub struct NoteRef<'a> {
    pub account: &'a str,
    pub note: &'a mailcraft_model::pim::Note,
    /// Its place among its account's notes (`Pim::notes`), to find it again without a search.
    pub at: usize,
}

impl Session {
    // ----- other people's calendars -----

    /// Open someone else's calendar from an Exchange account (Outlook's Open Calendar › From
    /// Address Book): read with the account's own, read-only, once the server shows it.
    pub fn open_shared_calendar(&mut self, account: Option<&str>, email: &str) -> CmdResult {
        let account = self.exchange_account(account)?;
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
        let new = !p.shared.contains(&email);
        if new {
            if p.shared.len() >= 50 {
                return Err(CmdError::Params("at most 50 other calendars can be open".into()));
            }
            p.shared.push(email.clone());
        }
        self.read_people(&account, if new { vec![email.clone()] } else { Vec::new() });
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
        // Out of its groups too (issue #140); a group left with no one goes.
        for g in &mut p.groups {
            g.members.retain(|e| *e != email);
        }
        p.groups.retain(|g| !g.members.is_empty());
        p.calendars.retain(|c| c.id != id);
        p.events.retain(|e| e.calendar != id);
        self.after_pim_change(&account, false);
        Ok(serde_json::json!({"closed": email}))
    }

    // ----- notes -----

    /// Every note matching a search (in its text), the most recently changed first.
    pub fn notes(&self, query: &str) -> Vec<NoteRef<'_>> {
        let q = query.trim().to_lowercase();
        let mut out: Vec<NoteRef> = self
            .pim
            .iter()
            .flat_map(|(a, p)| {
                p.notes.iter().enumerate().filter(|(_, n)| q.is_empty() || n.note.text.to_lowercase().contains(&q)).map(|(at, n)| NoteRef {
                    account: a,
                    note: &n.note,
                    at,
                })
            })
            .collect();
        // A note's title is its first line, compared without case.
        let title = |n: &NoteRef<'_>| n.note.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_lowercase();
        match self.prefs.notes_order {
            super::NotesOrder::Modified => out.sort_by(|a, b| b.note.modified.cmp(&a.note.modified).then_with(|| a.note.uid.cmp(&b.note.uid))),
            super::NotesOrder::Title => out.sort_by_cached_key(|n| (title(n), n.note.uid.clone())),
            super::NotesOrder::Color => out.sort_by_cached_key(|n| (n.note.color, title(n), n.note.uid.clone())),
        }
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
        let account = self.account_or_shown(account)?;
        let note = mailcraft_model::pim::Note {
            uid: self.new_uid(),
            text: cap_note(text),
            color: color.unwrap_or(mailcraft_model::pim::NOTE_YELLOW).min(mailcraft_model::pim::NOTE_COLORS - 1),
            modified: self.clock.now,
            files: Vec::new(),
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

    /// A note's file `index` (in [`Note::kept_files`]'s order): its name and bytes.
    ///
    /// [`Note::kept_files`]: mailcraft_model::pim::Note::kept_files
    pub fn note_file(&self, uid: &str, index: usize) -> Result<(String, Vec<u8>), CmdError> {
        let note =
            self.pim.values().flat_map(|p| &p.notes).find(|n| n.note.uid == uid).ok_or_else(|| CmdError::Params(format!("no note `{uid}`")))?;
        let files = note.note.kept_files();
        let f = files.get(index).ok_or_else(|| CmdError::Params(format!("the note has no file {index}")))?;
        let data = mailcraft_mime::decode_base64(f.base64).ok_or_else(|| CmdError::Failed(format!("{} couldn't be read", f.file_name())))?;
        Ok((mailcraft_mime::sanitize_filename(f.file_name()), data))
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
    if e.end.saturating_sub(e.start) > 366 * 86_400 {
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

/// Unix seconds a time given to a command must stay within, either side of 1970 (about 12,000
/// years): a calendar's sums with times past it overflow.
pub(crate) const MAX_SECONDS: i64 = 400_000_000_000;

/// A local date (`YYYY-MM-DD`, all day) or date and time (`YYYY-MM-DDTHH:MM`, seconds after it
/// or not, a space for the `T` or not), or Unix seconds, as (Unix seconds, all day), read in
/// `clock`'s time zone; all-day dates come back at UTC midnight (as events keep them).
pub fn parse_local(s: &str, clock: &Clock) -> Option<(i64, bool)> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i64>() {
        return (-MAX_SECONDS..MAX_SECONDS).contains(&n).then_some((n, false));
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
    // Seconds may follow; nothing else (`13:30 PM` is no time, not half past one).
    match s.len() {
        16 => {}
        19 if s.get(16..17) == Some(":") && num(17, 19).is_some_and(|sec| sec < 60) => {}
        _ => return None,
    }
    Some((clock.utc(day * 86_400 + h * 3600 + mi * 60), false))
}
