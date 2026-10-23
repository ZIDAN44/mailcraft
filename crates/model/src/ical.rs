//! iCalendar (RFC 5545) events: reading, writing and expanding recurrences.
//!
//! Reading is lenient (mail and servers produce all sorts): lines are unfolded, unknown
//! properties are kept verbatim so an edited event loses nothing, and times in a named zone are
//! converted with the file's own `VTIMEZONE` rules (standard and daylight offsets with yearly
//! transition rules), else read as local ("floating") time. Writing uses UTC for timed events and
//! `VALUE=DATE` for all-day ones, folding lines at 75 octets.
//!
//! Recurrence (`RRULE`) covers what calendars use day to day: daily, weekly, monthly and yearly
//! with `INTERVAL`, `COUNT`, `UNTIL`, `BYDAY` (with ordinals for monthly and yearly rules),
//! `BYMONTHDAY` and `BYMONTH`; `EXDATE` removes occurrences and `RECURRENCE-ID` instances replace
//! them. Expansion is capped.

use serde::{Deserialize, Serialize};

use crate::dates::{civil_from_days, days_from_civil};

mod notes;
mod recurrence;
use notes::*;
pub use notes::*;
use recurrence::*;
pub use recurrence::*;

/// The most occurrences produced for one event in one expansion.
const MAX_OCCURRENCES: usize = 2000;
/// The most rule iterations tried (rules that rarely match).
const MAX_STEPS: usize = 50_000;
/// The longest input read.
const MAX_INPUT: usize = 4 << 20;

/// Someone taking part in an event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attendee {
    pub email: String,
    #[serde(default)]
    pub name: String,
    /// `NEEDS-ACTION`, `ACCEPTED`, `DECLINED`, `TENTATIVE`…
    #[serde(default)]
    pub partstat: String,
    /// Asked to reply.
    #[serde(default)]
    pub rsvp: bool,
}

/// One event (or one changed instance of a recurring event, when `recurrence_id` is set).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalEvent {
    pub uid: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: String,
    /// Unix seconds. All-day events start at UTC midnight of their first day.
    pub start: i64,
    /// Exclusive end.
    pub end: i64,
    #[serde(default)]
    pub all_day: bool,
    /// The recurrence rule, as written (`FREQ=WEEKLY;BYDAY=MO,WE`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrule: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exdates: Vec<i64>,
    /// The occurrence this instance replaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organizer: Option<Attendee>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<Attendee>,
    /// `CONFIRMED`, `TENTATIVE` or `CANCELLED` (empty: not said).
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub sequence: u32,
    /// When the organizer wrote this version (`DTSTAMP`, Unix seconds; 0: not known): of two
    /// invitations with the same `sequence`, the later one counts.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dtstamp: i64,
    /// Its categories (`CATEGORIES`), which colour it in the calendar as Outlook colours them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Properties MailCraft doesn't read, kept as written (unfolded content lines).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<String>,
    /// The address to join it online, when the server gives one apart from its notes (Microsoft
    /// Graph's `onlineMeeting.joinUrl`); otherwise found in its notes ([`CalEvent::join_link`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub join_url: String,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

impl CalEvent {
    /// The address that joins it online: the server's own, else the first meeting link in its
    /// location or notes (as Outlook's Join finds a Teams or Zoom meeting).
    pub fn join_link(&self) -> Option<String> {
        if is_meeting_link(&self.join_url) {
            return Some(self.join_url.clone());
        }
        links_in(&self.location).into_iter().chain(links_in(&self.description)).find(|l| is_meeting_link(l))
    }

    /// Every web address in its location and notes (and its join address), once each.
    pub fn links(&self) -> Vec<String> {
        let mut out = links_in(&self.location);
        for l in links_in(&self.description) {
            if !out.contains(&l) {
                out.push(l);
            }
        }
        if !self.join_url.is_empty() && !out.contains(&self.join_url) && self.join_url.starts_with("http") {
            out.insert(0, self.join_url.clone());
        }
        out.truncate(MAX_LINKS);
        out
    }
}

/// A calendar file: what it asks (`METHOD`, for invitations) and its events.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ICalendar {
    pub method: Option<String>,
    pub events: Vec<CalEvent>,
}

/// One occurrence of an event: which event (index into the list given) and when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub event: usize,
    pub start: i64,
    pub end: i64,
}

// ----- reading -----

/// A content line: name, parameters (upper-case names), value.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    name: String,
    params: Vec<(String, String)>,
    value: String,
    raw: String,
}

impl Line {
    fn param(&self, n: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str())
    }
}

/// Unfold (RFC 5545 §3.1) and split into content lines.
fn lines(text: &str) -> Vec<Line> {
    let text = text.get(..MAX_INPUT).unwrap_or(text);
    let mut unfolded: Vec<String> = Vec::new();
    for l in text.split('\n') {
        let l = l.strip_suffix('\r').unwrap_or(l);
        match (l.strip_prefix(' ').or_else(|| l.strip_prefix('\t')), unfolded.last_mut()) {
            (Some(rest), Some(prev)) => prev.push_str(rest),
            _ => unfolded.push(l.to_string()),
        }
    }
    unfolded
        .into_iter()
        .filter_map(|raw| {
            parse_line(&raw).map(|mut l| {
                l.raw = raw;
                l
            })
        })
        .collect()
}

fn parse_line(raw: &str) -> Option<Line> {
    // The value starts at the first ':' outside double quotes.
    let mut quoted = false;
    let mut colon = None;
    for (i, c) in raw.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let head = raw.get(..colon)?;
    let value = raw.get(colon + 1..)?.to_string();
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    for c in head.chars() {
        match c {
            '"' => q = !q,
            ';' if !q => parts.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    parts.push(cur);
    let mut it = parts.into_iter();
    let name = it.next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = it.filter_map(|p| p.split_once('=').map(|(k, v)| (k.trim().to_ascii_uppercase(), v.to_string()))).collect();
    Some(Line { name, params, value, raw: String::new() })
}

/// A TEXT value with its escapes undone.
/// A list of texts (`CATEGORIES:Work,Clients\, big`): split at commas that aren't escaped, each
/// unescaped and trimmed.
pub fn split_text_list(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut escaped = false;
    for ch in v.chars() {
        match ch {
            _ if escaped => {
                cur.push('\\');
                cur.push(ch);
                escaped = false;
            }
            '\\' => escaped = true,
            ',' => out.push(unescape(std::mem::take(&mut cur).trim())),
            _ => cur.push(ch),
        }
    }
    out.push(unescape(cur.trim()));
    out.into_iter().filter(|c| !c.is_empty()).collect()
}

pub fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// A TEXT value escaped for writing.
pub fn escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace("\r\n", "\\n").replace(['\n', '\r'], "\\n")
}

/// A time zone from a `VTIMEZONE`: its standard and daylight offsets and when each starts.
#[derive(Clone, Debug, Default)]
struct Zone {
    id: String,
    /// (offset seconds, yearly rule start month, BYDAY ordinal/weekday, local seconds into day, fixed start)
    parts: Vec<ZonePart>,
}

#[derive(Clone, Debug)]
struct ZonePart {
    offset: i64,
    /// `None`: a one-off transition at `since` (local); `Some`: every year in this month, on this
    /// nth weekday (`-1` last), at `time` local seconds.
    yearly: Option<(u32, i32, u32)>,
    time: i64,
    since: i64,
}

impl Zone {
    /// The offset in force at a local time.
    fn offset_at(&self, local: i64) -> i64 {
        // The latest transition at or before `local` (comparing in local time is close enough).
        let mut best: Option<(i64, i64)> = None;
        for p in &self.parts {
            let candidates: Vec<i64> = match p.yearly {
                Some((month, nth, wd)) => {
                    let (y, _, _) = civil_from_days(local.div_euclid(86_400));
                    [y - 1, y]
                        .iter()
                        .filter_map(|&yy| nth_weekday(yy, month, nth, wd).map(|d| d * 86_400 + p.time))
                        .filter(|t| *t >= p.since)
                        .collect()
                }
                None => vec![p.since],
            };
            for t in candidates.into_iter().filter(|t| *t <= local) {
                if best.is_none_or(|(bt, _)| t > bt) {
                    best = Some((t, p.offset));
                }
            }
        }
        best.map(|(_, o)| o).or_else(|| self.parts.first().map(|p| p.offset)).unwrap_or(0)
    }
}

/// Days since the epoch of the `nth` (1-based; negative from the end) `weekday` (0 Monday) of a
/// month.
fn nth_weekday(year: i64, month: u32, nth: i32, weekday: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || nth == 0 {
        return None;
    }
    let first = days_from_civil(year, month, 1);
    let next = if month == 12 { days_from_civil(year + 1, 1, 1) } else { days_from_civil(year, month + 1, 1) };
    let wd = |d: i64| (d + 3).rem_euclid(7) as u32;
    if nth > 0 {
        let offset = (i64::from(weekday) - i64::from(wd(first))).rem_euclid(7);
        let d = first + offset + 7 * i64::from(nth - 1);
        (d < next).then_some(d)
    } else {
        let last = next - 1;
        let offset = (i64::from(wd(last)) - i64::from(weekday)).rem_euclid(7);
        let d = last - offset - 7 * i64::from(-nth - 1);
        (d >= first).then_some(d)
    }
}

fn weekday_code(s: &str) -> Option<u32> {
    ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|w| s.eq_ignore_ascii_case(w)).and_then(|p| u32::try_from(p).ok())
}

/// `+0100`, `-0530` → seconds.
fn utc_offset(s: &str) -> Option<i64> {
    let s = s.trim();
    let sign = match s.get(..1)? {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let h: i64 = s.get(1..3)?.parse().ok()?;
    let m: i64 = s.get(3..5)?.parse().ok()?;
    let sec: i64 = s.get(5..7).and_then(|x| x.parse().ok()).unwrap_or(0);
    (h <= 18 && m < 60).then_some(sign * (h * 3600 + m * 60 + sec))
}

/// `YYYYMMDD` → days since the epoch.
fn date_days(s: &str) -> Option<i64> {
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: u32 = s.get(4..6)?.parse().ok()?;
    let d: u32 = s.get(6..8)?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d) && (1..=9999).contains(&y)).then(|| days_from_civil(y, m, d))
}

/// A DATE or DATE-TIME value: (local or UTC seconds, all day, UTC).
fn date_time(v: &str) -> Option<(i64, bool, bool)> {
    let v = v.trim();
    let days = date_days(v)?;
    let Some(t) = v.get(8..).filter(|r| !r.is_empty()) else { return Some((days * 86_400, true, false)) };
    let t = t.strip_prefix('T')?;
    let h: i64 = t.get(0..2)?.parse().ok()?;
    let mi: i64 = t.get(2..4)?.parse().ok()?;
    let s: i64 = t.get(4..6)?.parse().ok()?;
    if h > 24 || mi > 59 || s > 60 {
        return None;
    }
    Some((days * 86_400 + h * 3600 + mi * 60 + s, false, t.ends_with('Z') || t.ends_with('z')))
}

/// A `DURATION` (`P1W`, `PT1H30M`, `-P1D`) in seconds.
pub fn duration(v: &str) -> Option<i64> {
    let v = v.trim();
    let (sign, v) = match v.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, v.strip_prefix('+').unwrap_or(v)),
    };
    let v = v.strip_prefix('P')?;
    let mut total = 0i64;
    let mut num = String::new();
    let mut time = false;
    for c in v.chars() {
        match c {
            '0'..='9' => num.push(c),
            'T' => time = true,
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total = total.saturating_add(n.saturating_mul(match (unit, time) {
                    ('W', _) => 604_800,
                    ('D', _) => 86_400,
                    ('H', true) => 3600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                }));
            }
        }
    }
    num.is_empty().then_some(sign * total)
}

/// The longest reminder taken (four weeks before), as Outlook's own list goes.
pub const REMINDER_MAX_MINUTES: i64 = 4 * 7 * 24 * 60;

/// An event's reminder, in minutes before it starts, from its first alarm (`VALARM`) with a
/// trigger relative to the start (`TRIGGER:-PT15M`, `TRIGGER;RELATED=START:-P1D`) or at a set
/// time (`TRIGGER;VALUE=DATE-TIME:…Z`). `None`: no reminder. A trigger after the start counts as
/// "at the start".
pub fn reminder_minutes(e: &CalEvent) -> Option<i64> {
    let mut in_alarm = false;
    for raw in e.extra.iter().take(400) {
        let line = raw.trim();
        let upper = line.to_ascii_uppercase();
        if upper == "BEGIN:VALARM" {
            in_alarm = true;
            continue;
        }
        if upper == "END:VALARM" {
            in_alarm = false;
            continue;
        }
        if !in_alarm || !upper.starts_with("TRIGGER") {
            continue;
        }
        let Some((params, value)) = line.split_once(':') else { continue };
        let params = params.to_ascii_uppercase();
        if params.contains("RELATED=END") {
            continue;
        }
        // One that can't be read is passed over for the next.
        let before = if params.contains("VALUE=DATE-TIME") {
            let Some((t, _, _)) = date_time(value) else { continue };
            e.start.saturating_sub(t)
        } else {
            let Some(d) = duration(value) else { continue };
            d.saturating_neg()
        };
        return Some((before / 60).clamp(0, REMINDER_MAX_MINUTES));
    }
    None
}

/// Give an event one display alarm `minutes` before it starts, in place of any it had, or none
/// (`None`).
pub fn set_reminder(e: &mut CalEvent, minutes: Option<i64>) {
    // Every alarm goes, nested lines included.
    let mut kept = Vec::with_capacity(e.extra.len());
    let mut depth = 0usize;
    for raw in std::mem::take(&mut e.extra) {
        let upper = raw.trim().to_ascii_uppercase();
        if upper == "BEGIN:VALARM" {
            depth += 1;
            continue;
        }
        if depth > 0 {
            if upper == "END:VALARM" {
                depth -= 1;
            }
            continue;
        }
        kept.push(raw);
    }
    e.extra = kept;
    if let Some(m) = minutes {
        let m = m.clamp(0, REMINDER_MAX_MINUTES);
        e.extra.extend([
            "BEGIN:VALARM".to_string(),
            "ACTION:DISPLAY".to_string(),
            "DESCRIPTION:Reminder".to_string(),
            format!("TRIGGER:-PT{m}M"),
            "END:VALARM".to_string(),
        ]);
    }
}

/// Read a mailbox (`mailto:a@b`) and its name.
fn person(l: &Line) -> Attendee {
    let v = l.value.trim();
    let email = v.get(..7).filter(|p| p.eq_ignore_ascii_case("mailto:")).map_or(v, |_| v.get(7..).unwrap_or("")).trim().to_string();
    Attendee {
        email,
        name: l.param("CN").map(|n| n.trim_matches('"').to_string()).unwrap_or_default(),
        partstat: l.param("PARTSTAT").map(str::to_ascii_uppercase).unwrap_or_default(),
        rsvp: l.param("RSVP").is_some_and(|r| r.eq_ignore_ascii_case("TRUE")),
    }
}

/// Read a calendar file. `local_offset` (seconds east of UTC) places floating times.
pub fn parse(text: &str, local_offset: i64) -> ICalendar {
    let all = lines(text);
    let mut cal = ICalendar::default();
    // Zones first: events can come before the zones they use.
    let mut zones: Vec<Zone> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut zone: Option<Zone> = None;
    let mut part: Option<ZonePart> = None;
    for l in &all {
        match (l.name.as_str(), stack.last().map(String::as_str)) {
            ("BEGIN", _) => {
                let what = l.value.trim().to_ascii_uppercase();
                if what == "VTIMEZONE" {
                    zone = Some(Zone::default());
                } else if (what == "STANDARD" || what == "DAYLIGHT") && zone.is_some() {
                    part = Some(ZonePart { offset: 0, yearly: None, time: 0, since: i64::MIN });
                }
                stack.push(what);
                if stack.len() > 16 {
                    return cal;
                }
            }
            ("END", _) => {
                let what = stack.pop().unwrap_or_default();
                if what == "VTIMEZONE" {
                    zones.extend(zone.take());
                } else if (what == "STANDARD" || what == "DAYLIGHT")
                    && let (Some(p), Some(z)) = (part.take(), zone.as_mut())
                {
                    z.parts.push(p);
                }
            }
            ("TZID", Some("VTIMEZONE")) => {
                if let Some(z) = zone.as_mut() {
                    z.id = l.value.trim().to_string();
                }
            }
            ("TZOFFSETTO", Some("STANDARD" | "DAYLIGHT")) => {
                if let (Some(p), Some(o)) = (part.as_mut(), utc_offset(&l.value)) {
                    p.offset = o;
                }
            }
            ("DTSTART", Some("STANDARD" | "DAYLIGHT")) => {
                if let (Some(p), Some((t, _, _))) = (part.as_mut(), date_time(&l.value)) {
                    p.since = t;
                    p.time = t.rem_euclid(86_400);
                }
            }
            ("RRULE", Some("STANDARD" | "DAYLIGHT")) => {
                if let Some(p) = part.as_mut() {
                    let r = Rule::parse(&l.value);
                    if let (Some(m), Some((nth, wd))) = (r.by_month.first(), r.by_day.first()) {
                        p.yearly = Some((*m, if *nth == 0 { 1 } else { *nth }, *wd));
                    }
                }
            }
            _ => {}
        }
    }
    let to_utc = |l: &Line| -> Option<(i64, bool)> {
        let (t, all_day, utc) = date_time(&l.value)?;
        if all_day || l.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE")) {
            return Some((t - t.rem_euclid(86_400), true));
        }
        if utc {
            return Some((t, false));
        }
        let off = match l.param("TZID").map(|z| z.trim_matches('"')) {
            Some(id) => match zones.iter().find(|z| z.id == id) {
                Some(z) => z.offset_at(t),
                // A zone named but not given (common for UTC and Olson names).
                None if id.eq_ignore_ascii_case("UTC") || id.eq_ignore_ascii_case("Etc/UTC") || id.eq_ignore_ascii_case("GMT") => 0,
                None => local_offset,
            },
            None => local_offset,
        };
        Some((t - off, false))
    };

    let mut ev: Option<(CalEvent, Option<i64>)> = None;
    let mut depth_in_event = 0usize;
    for l in &all {
        let n = l.name.as_str();
        if n == "BEGIN" {
            let what = l.value.trim().to_ascii_uppercase();
            if what == "VEVENT" && ev.is_none() {
                ev = Some((CalEvent::default(), None));
                depth_in_event = 0;
                continue;
            }
            if ev.is_some() {
                // A nested component (VALARM): kept as written.
                depth_in_event += 1;
                if let Some((e, _)) = ev.as_mut() {
                    e.extra.push(l.raw.clone());
                }
            }
            continue;
        }
        if n == "END" {
            if let Some((e, _)) = ev.as_mut()
                && depth_in_event > 0
            {
                depth_in_event -= 1;
                e.extra.push(l.raw.clone());
                continue;
            }
            if l.value.trim().eq_ignore_ascii_case("VEVENT")
                && let Some((mut e, dur)) = ev.take()
            {
                if e.end <= e.start {
                    e.end = match dur {
                        Some(d) => e.start.saturating_add(d.max(0)),
                        None if e.all_day => e.start + 86_400,
                        None => e.start,
                    };
                }
                if cal.events.len() < 20_000 {
                    cal.events.push(e);
                }
            }
            continue;
        }
        let Some((e, dur)) = ev.as_mut() else {
            if n == "METHOD" {
                cal.method = Some(l.value.trim().to_ascii_uppercase());
            }
            continue;
        };
        if depth_in_event > 0 {
            e.extra.push(l.raw.clone());
            continue;
        }
        match n {
            "UID" => e.uid = l.value.trim().to_string(),
            "SUMMARY" => e.summary = unescape(&l.value),
            "DESCRIPTION" => e.description = unescape(&l.value),
            "LOCATION" => e.location = unescape(&l.value),
            "CATEGORIES" => {
                for c in split_text_list(&l.value) {
                    if !c.is_empty() && !e.categories.contains(&c) && e.categories.len() < 20 {
                        e.categories.push(c);
                    }
                }
            }
            "STATUS" => e.status = l.value.trim().to_ascii_uppercase(),
            "SEQUENCE" => e.sequence = l.value.trim().parse().unwrap_or(0),
            "DTSTART" => {
                if let Some((t, all_day)) = to_utc(l) {
                    e.start = t;
                    e.all_day = all_day;
                }
            }
            "DTEND" => {
                if let Some((t, _)) = to_utc(l) {
                    e.end = t;
                }
            }
            "DURATION" => *dur = duration(&l.value),
            "RRULE" => e.rrule = Some(l.value.trim().to_string()),
            "EXDATE" => {
                for v in l.value.split(',') {
                    let one = Line { value: v.to_string(), ..l.clone() };
                    if let Some((t, _)) = to_utc(&one)
                        && e.exdates.len() < 1000
                    {
                        e.exdates.push(t);
                    }
                }
            }
            "RECURRENCE-ID" => e.recurrence_id = to_utc(l).map(|(t, _)| t),
            "ORGANIZER" => e.organizer = Some(person(l)),
            "ATTENDEE" => {
                if e.attendees.len() < 500 {
                    e.attendees.push(person(l));
                }
            }
            "DTSTAMP" => e.dtstamp = to_utc(l).map_or(0, |(t, _)| t),
            "CREATED" | "LAST-MODIFIED" => {}
            _ => {
                if e.extra.len() < 200 {
                    e.extra.push(l.raw.clone());
                }
            }
        }
    }
    cal
}

// ----- writing -----

/// Fold a content line at 75 octets (RFC 5545 §3.1), on character boundaries.
fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += n;
    }
    out.push_str("\r\n");
}

fn stamp(t: i64) -> String {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

fn day(t: i64) -> String {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    format!("{y:04}{m:02}{d:02}")
}

fn mailbox(prop: &str, a: &Attendee) -> String {
    let mut s = prop.to_string();
    if !a.name.is_empty() {
        s.push_str(&format!(";CN=\"{}\"", a.name.replace(['"', '\r', '\n'], "")));
    }
    if !a.partstat.is_empty() {
        s.push_str(&format!(";PARTSTAT={}", a.partstat.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "")));
    }
    if a.rsvp {
        s.push_str(";RSVP=TRUE");
    }
    s.push_str(&format!(":mailto:{}", a.email.replace(['\r', '\n'], "")));
    s
}

/// One `VEVENT`, folded; `now` is its `DTSTAMP`.
pub fn write_event(e: &CalEvent, now: i64, out: &mut String) {
    let mut lines: Vec<String> = vec!["BEGIN:VEVENT".into(), format!("UID:{}", e.uid.replace(['\r', '\n'], "")), format!("DTSTAMP:{}", stamp(now))];
    if e.all_day {
        lines.push(format!("DTSTART;VALUE=DATE:{}", day(e.start)));
        lines.push(format!("DTEND;VALUE=DATE:{}", day(e.end.max(e.start + 86_400))));
    } else {
        lines.push(format!("DTSTART:{}", stamp(e.start)));
        lines.push(format!("DTEND:{}", stamp(e.end.max(e.start))));
    }
    if let Some(r) = e.recurrence_id {
        lines.push(if e.all_day { format!("RECURRENCE-ID;VALUE=DATE:{}", day(r)) } else { format!("RECURRENCE-ID:{}", stamp(r)) });
    }
    if let Some(r) = &e.rrule {
        lines.push(format!("RRULE:{}", r.replace(['\r', '\n'], "")));
    }
    for x in &e.exdates {
        lines.push(if e.all_day { format!("EXDATE;VALUE=DATE:{}", day(*x)) } else { format!("EXDATE:{}", stamp(*x)) });
    }
    if !e.summary.is_empty() {
        lines.push(format!("SUMMARY:{}", escape(&e.summary)));
    }
    if !e.location.is_empty() {
        lines.push(format!("LOCATION:{}", escape(&e.location)));
    }
    if !e.description.is_empty() {
        lines.push(format!("DESCRIPTION:{}", escape(&e.description)));
    }
    if !e.categories.is_empty() {
        lines.push(format!("CATEGORIES:{}", e.categories.iter().map(|c| escape(c)).collect::<Vec<_>>().join(",")));
    }
    if !e.status.is_empty() {
        lines.push(format!("STATUS:{}", e.status.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "")));
    }
    if e.sequence > 0 {
        lines.push(format!("SEQUENCE:{}", e.sequence));
    }
    if let Some(o) = &e.organizer {
        lines.push(mailbox("ORGANIZER", o));
    }
    for a in &e.attendees {
        lines.push(mailbox("ATTENDEE", a));
    }
    lines.extend(e.extra.iter().map(|x| x.replace(['\r', '\n'], "")));
    lines.push("END:VEVENT".into());
    for l in lines {
        fold(&l, out);
    }
}

/// A calendar file with these events (`method` for invitations: `REQUEST`, `REPLY`, `CANCEL`).
/// An invitation is stamped `now`; a stored event keeps the stamp it came with, so the copy on a
/// server still tells which invitation it was.
pub fn write(events: &[CalEvent], method: Option<&str>, now: i64) -> String {
    let mut out = String::new();
    for l in ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//MailCraft//MailCraft//EN", "CALSCALE:GREGORIAN"] {
        fold(l, &mut out);
    }
    if let Some(m) = method {
        fold(&format!("METHOD:{}", m.replace(|c: char| !c.is_ascii_alphabetic(), "")), &mut out);
    }
    for e in events {
        let stamp = if method.is_none() && e.dtstamp > 0 { e.dtstamp } else { now };
        write_event(e, stamp, &mut out);
    }
    fold("END:VCALENDAR", &mut out);
    out
}

// ----- recurrence -----

#[cfg(test)]
mod tests;
