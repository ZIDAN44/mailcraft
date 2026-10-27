//! Calendars subscribed to by their links (issue #7), as Outlook's Add Calendar › From
//! Internet: an iCal or `webcal://` address, read-only beside the account's own calendars, read
//! again from the link every few hours (as often as the calendar asks, within limits), and at
//! once on Refresh.

use mailcraft_model::pim::{CalendarInfo, StoredEvent, WebCalendar, calendar_link, is_web_calendar, web_calendar_id};

use super::*;

/// How often a calendar is read again when it doesn't say.
pub const WEB_EVERY: i64 = 3 * 3600;
/// The bounds on what a calendar may ask for: not more than hourly, at least weekly.
const WEB_EVERY_MIN: i64 = 3600;
const WEB_EVERY_MAX: i64 = 7 * 86_400;
/// After a failed read, tried again this much later.
const WEB_RETRY: i64 = 30 * 60;
/// Most calendars subscribed to by one account.
const WEB_MAX: usize = 50;

impl Session {
    /// Subscribe to the calendar at `url` (iCal, `webcal://`) in an account (else the one on
    /// screen), named `name` (else the calendar's own name, else the link's host); read at once.
    pub fn subscribe_calendar(&mut self, account: Option<&str>, url: &str, name: Option<&str>) -> CmdResult {
        let account = self.pim_account(account)?;
        let link = calendar_link(url)
            .ok_or_else(|| CmdError::Params(format!("not a calendar link: {}", url.trim().chars().take(80).collect::<String>())))?;
        let id = web_calendar_id(&link);
        let name: String = name.unwrap_or("").chars().filter(|c| !c.is_control()).take(200).collect();
        let name = name.trim().to_string();
        let p = self.pim_mut(&account);
        if p.subscribed.iter().any(|w| w.id == id) {
            return Err(CmdError::Params("that calendar is subscribed to already".into()));
        }
        if p.subscribed.len() >= WEB_MAX {
            return Err(CmdError::Params(format!("at most {WEB_MAX} calendars can be subscribed to")));
        }
        let host = link.split("://").nth(1).and_then(|r| r.split(['/', '?', '#', ':']).next()).unwrap_or("").to_string();
        let color = u8::try_from(p.calendars.len() % 8).unwrap_or(0);
        p.calendars.push(CalendarInfo {
            id: id.clone(),
            name: if name.is_empty() { host } else { name.clone() },
            color,
            read_only: true,
            ctag: String::new(),
        });
        p.subscribed.push(WebCalendar { id: id.clone(), url: link, named: !name.is_empty(), ..Default::default() });
        self.fetch_web_calendar(&account, &id);
        self.after_pim_change(&account, false);
        Ok(serde_json::json!({"account": account, "calendar": id}))
    }

    /// Stop subscribing to a calendar (by its id or link): it and its events go.
    pub fn unsubscribe_calendar(&mut self, calendar: &str) -> CmdResult {
        let id =
            if is_web_calendar(calendar) { calendar.to_string() } else { calendar_link(calendar).map(|l| web_calendar_id(&l)).unwrap_or_default() };
        let account = self.pim.iter().find(|(_, p)| p.subscribed.iter().any(|w| w.id == id)).map(|(a, _)| a.clone());
        let Some(account) = account else { return Err(CmdError::Params("no such calendar is subscribed to".into())) };
        let p = self.pim_mut(&account);
        p.subscribed.retain(|w| w.id != id);
        p.calendars.retain(|c| c.id != id);
        p.events.retain(|e| e.calendar != id);
        self.after_pim_change(&account, false);
        Ok(serde_json::json!({"unsubscribed": id}))
    }

    /// Read subscribed calendars again now: one (by id), or all.
    pub fn refresh_web_calendars(&mut self, calendar: Option<&str>) -> CmdResult {
        let all: Vec<(String, String)> = self
            .pim
            .iter()
            .flat_map(|(a, p)| p.subscribed.iter().filter(|w| calendar.is_none_or(|c| w.id == c)).map(move |w| (a.clone(), w.id.clone())))
            .collect();
        if all.is_empty() {
            return Err(CmdError::Params("no calendar is subscribed to".into()));
        }
        for (a, id) in &all {
            self.fetch_web_calendar(a, id);
        }
        let accounts: std::collections::BTreeSet<String> = all.iter().map(|(a, _)| a.clone()).collect();
        for a in accounts {
            self.save_pim(&a);
        }
        Ok(serde_json::json!({"refreshing": all.len()}))
    }

    /// Calendars due to be read again (each frame: cheap when none is).
    pub(crate) fn web_calendars_due(&mut self) {
        let now = self.clock.now;
        let due: Vec<(String, String)> = self
            .pim
            .iter()
            .filter(|(a, _)| self.account(a).is_some())
            .flat_map(|(a, p)| {
                p.subscribed
                    .iter()
                    .filter(move |w| {
                        let wait = if w.error.is_some() || w.fetched < w.tried { WEB_RETRY } else { every(w) };
                        // Tried "in the future": the clock was set back; don't wait for it.
                        w.tried > now || now.saturating_sub(w.tried) >= wait
                    })
                    .map(move |w| (a.clone(), w.id.clone()))
            })
            .collect();
        if due.is_empty() {
            return;
        }
        let accounts: std::collections::BTreeSet<String> = due.iter().map(|(a, _)| a.clone()).collect();
        for (a, id) in &due {
            self.fetch_web_calendar(a, id);
        }
        for a in accounts {
            self.save_pim(&a);
        }
    }

    /// Ask for a calendar to be read, noting when.
    fn fetch_web_calendar(&mut self, account: &str, id: &str) {
        let now = self.clock.now;
        let p = self.pim_mut(account);
        let Some(w) = p.subscribed.iter_mut().find(|w| w.id == id) else { return };
        w.tried = now.max(w.tried.saturating_add(1));
        let job = Job::FetchCalendar {
            account: account.to_string(),
            calendar: id.to_string(),
            url: w.url.clone(),
            etag: w.etag.clone(),
            modified: w.modified.clone(),
        };
        self.jobs.push(job);
    }

    /// What reading a subscribed calendar came to: its events replace what was kept (unless
    /// unchanged); a failure keeps them, and says why.
    pub(crate) fn apply_web_calendar(&mut self, account: &str, id: &str, read: WebRead) {
        let now = self.clock.now;
        let zone = self.clock.local_zone();
        let p = self.pim_mut(account);
        let Some(w) = p.subscribed.iter_mut().find(|w| w.id == id) else { return };
        let first = w.fetched == 0;
        let failed = match read {
            WebRead::Failed(e) => Some(e),
            WebRead::Unchanged => {
                w.fetched = now;
                None
            }
            WebRead::Read { ics, etag, modified } => {
                let opening = ics.trim_start_matches('\u{feff}').trim_start();
                if !opening.get(..15).is_some_and(|h| h.eq_ignore_ascii_case("BEGIN:VCALENDAR")) {
                    Some("the link doesn't give a calendar".to_string())
                } else {
                    let cal = ical::parse_in(&ics, zone);
                    w.fetched = now;
                    w.etag = etag;
                    w.modified = modified;
                    w.every = cal.refresh.unwrap_or(0);
                    let named = w.named;
                    // An event without a uid gets one of this calendar's, the same each time.
                    let tag = id.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
                    let events: Vec<StoredEvent> = cal
                        .events
                        .into_iter()
                        .enumerate()
                        .map(|(i, mut e)| {
                            if e.uid.trim().is_empty() {
                                e.uid = format!("web-{tag:08x}-{i}");
                            }
                            StoredEvent { calendar: id.to_string(), href: String::new(), etag: String::new(), event: e, dirty: false }
                        })
                        .collect();
                    p.events.retain(|e| e.calendar != id);
                    p.events.extend(events);
                    if let (false, Some(name), Some(c)) = (named, cal.name, p.calendars.iter_mut().find(|c| c.id == id)) {
                        c.name = name;
                    }
                    None
                }
            }
        };
        let p = self.pim_mut(account);
        let Some(w) = p.subscribed.iter_mut().find(|w| w.id == id) else { return };
        let name = p.calendars.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_default();
        w.error = failed.clone();
        if let (Some(e), true) = (failed, first) {
            self.notice(format!("{name} couldn't be read: {e}"), true);
        }
        self.save_pim(account);
    }
}

/// How often a calendar is read again: as it asks, within bounds.
fn every(w: &WebCalendar) -> i64 {
    if w.every > 0 { w.every.clamp(WEB_EVERY_MIN, WEB_EVERY_MAX) } else { WEB_EVERY }
}

/// What reading a subscribed calendar came to.
pub(crate) enum WebRead {
    Read { ics: String, etag: String, modified: String },
    Unchanged,
    Failed(String),
}
