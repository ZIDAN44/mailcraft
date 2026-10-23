//! iCalendar (RFC 5545) events: reading, writing and expanding recurrences.
//!
//! Reading is lenient (mail and servers produce all sorts): lines are unfolded, unknown
//! properties are kept verbatim so an edited event loses nothing. A time in a named zone is read
//! in it (the IANA database's when it knows the name, Windows' names included, else the file's
//! own `VTIMEZONE`), and the event keeps its zone (`TZID`, and the definition when only the file
//! had one); floating times are the viewer's. Writing puts a timed event in its zone, with the
//! zone's `VTIMEZONE`, else in UTC, and `VALUE=DATE` for all-day ones, folding lines at 75 octets.
//!
//! Recurrence (`RRULE`) covers what calendars use day to day: daily, weekly, monthly and yearly
//! with `INTERVAL`, `COUNT`, `UNTIL`, `BYDAY` (with ordinals for monthly and yearly rules),
//! `BYMONTHDAY`, `BYMONTH` and `BYSETPOS`. A series repeats at its wall-clock time in its zone
//! (each occurrence turned into UTC on its own date, so summer time doesn't move it), on that
//! zone's days; `EXDATE` removes occurrences and `RECURRENCE-ID` instances replace them, both
//! matched as instants. Expansion is capped.

use serde::{Deserialize, Serialize};

use crate::dates::{civil_from_days, days_from_civil};

mod alarms;
mod notes;
mod recurrence;
mod zones;
pub use alarms::*;
use notes::*;
pub use notes::*;
use recurrence::*;
pub use recurrence::*;
use zones::*;

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
    /// The time zone its times were written in (`TZID`, as written; empty: UTC, or an all-day
    /// event). A recurring event repeats at the same wall-clock time there, summer or winter.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tzid: String,
    /// That zone as the file defined it (its `VTIMEZONE` lines), written back with the event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vtimezone: Vec<String>,
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

/// Read a calendar file. `local_offset` (seconds east of UTC) places floating times; better,
/// [`parse_in`] the viewer's zone.
pub fn parse(text: &str, local_offset: i64) -> ICalendar {
    let off = i32::try_from(local_offset.clamp(-14 * 3600, 14 * 3600)).unwrap_or(0);
    parse_in(text, crate::zone::Zone::Fixed(off))
}

/// Read a calendar file. Times in a named zone are read in it (the IANA database's when it knows
/// the name, else the file's `VTIMEZONE`), and the event keeps the zone; floating times, and
/// zones nobody defines, are in `local` (the viewer's).
pub fn parse_in(text: &str, local: crate::zone::Zone) -> ICalendar {
    let all = lines(text);
    let mut cal = ICalendar::default();
    // Zones first: events can come before the zones they use.
    let zones = FileZones::read(&all, local);
    // An exception's times wait for the event's zone (DTSTART may come after them).
    let mut pending: Vec<&Line> = Vec::new();
    let mut ev: Option<(CalEvent, Option<i64>)> = None;
    let mut depth_in_event = 0usize;
    for l in &all {
        let n = l.name.as_str();
        if n == "BEGIN" {
            let what = l.value.trim().to_ascii_uppercase();
            if what == "VEVENT" && ev.is_none() {
                ev = Some((CalEvent::default(), None));
                depth_in_event = 0;
                pending.clear();
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
                for x in pending.drain(..) {
                    if x.name == "RECURRENCE-ID" {
                        e.recurrence_id = zones.time(x, &e.tzid).map(|(t, _, _)| t);
                        continue;
                    }
                    for v in x.value.split(',') {
                        let one = Line { value: v.to_string(), ..x.clone() };
                        if let Some((t, _, _)) = zones.time(&one, &e.tzid)
                            && e.exdates.len() < 1000
                        {
                            e.exdates.push(t);
                        }
                    }
                }
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
                if let Some((t, all_day, tzid)) = zones.time(l, "") {
                    e.start = t;
                    e.all_day = all_day;
                    e.vtimezone = zones.definition(&tzid);
                    e.tzid = tzid;
                }
            }
            "DTEND" => {
                if let Some((t, _, _)) = zones.time(l, "") {
                    e.end = t;
                }
            }
            "DURATION" => *dur = duration(&l.value),
            "RRULE" => e.rrule = Some(l.value.trim().to_string()),
            "EXDATE" | "RECURRENCE-ID" => {
                if pending.len() < 1000 {
                    pending.push(l);
                }
            }
            "ORGANIZER" => e.organizer = Some(person(l)),
            "ATTENDEE" => {
                if e.attendees.len() < 500 {
                    e.attendees.push(person(l));
                }
            }
            "DTSTAMP" => e.dtstamp = zones.time(l, "").map_or(0, |(t, _, _)| t),
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
    // Timed events in a zone are written in it (repeating at its wall-clock time); others in UTC.
    let zone = zone_of(e);
    let when = |prop: &str, t: i64| match &zone {
        Some(z) => format!("{prop};TZID={}:{}", tzid_param(&e.tzid), local_stamp(z.local(t))),
        None => format!("{prop}:{}", stamp(t)),
    };
    if e.all_day {
        lines.push(format!("DTSTART;VALUE=DATE:{}", day(e.start)));
        lines.push(format!("DTEND;VALUE=DATE:{}", day(e.end.max(e.start + 86_400))));
    } else {
        lines.push(when("DTSTART", e.start));
        lines.push(when("DTEND", e.end.max(e.start)));
    }
    if let Some(r) = e.recurrence_id {
        lines.push(if e.all_day { format!("RECURRENCE-ID;VALUE=DATE:{}", day(r)) } else { when("RECURRENCE-ID", r) });
    }
    if let Some(r) = &e.rrule {
        lines.push(format!("RRULE:{}", r.replace(['\r', '\n'], "")));
    }
    for x in &e.exdates {
        lines.push(if e.all_day { format!("EXDATE;VALUE=DATE:{}", day(*x)) } else { when("EXDATE", *x) });
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
    // Each zone the events are in, once: as the file that brought it defined it, else as the
    // database has it in the year of its first event.
    let mut written: Vec<&str> = Vec::new();
    for e in events {
        if written.contains(&e.tzid.as_str()) || written.len() >= 50 {
            continue;
        }
        let definition = match zone_of(e) {
            Some(EventZone::Db(z)) if e.vtimezone.is_empty() => {
                let first = events.iter().filter(|x| x.tzid == e.tzid).map(|x| x.start).min().unwrap_or(e.start);
                zone_definition(&e.tzid, z, civil_from_days(first.div_euclid(86_400)).0)
            }
            Some(_) => e.vtimezone.clone(),
            None => continue,
        };
        written.push(&e.tzid);
        for l in definition {
            fold(&l.replace(['\r', '\n'], ""), &mut out);
        }
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
