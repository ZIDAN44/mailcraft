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
    /// Its categories (`CATEGORIES`), which colour it in the calendar as Outlook colours them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Properties MailCraft doesn't read, kept as written (unfolded content lines).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<String>,
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
            "DTSTAMP" | "CREATED" | "LAST-MODIFIED" => {}
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

/// One `VEVENT`, folded.
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
pub fn write(events: &[CalEvent], method: Option<&str>, now: i64) -> String {
    let mut out = String::new();
    for l in ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//MailCraft//MailCraft//EN", "CALSCALE:GREGORIAN"] {
        fold(l, &mut out);
    }
    if let Some(m) = method {
        fold(&format!("METHOD:{}", m.replace(|c: char| !c.is_ascii_alphabetic(), "")), &mut out);
    }
    for e in events {
        write_event(e, now, &mut out);
    }
    fold("END:VCALENDAR", &mut out);
    out
}

// ----- recurrence -----

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Rule {
    freq: String,
    interval: i64,
    count: Option<usize>,
    until: Option<i64>,
    /// (ordinal, weekday 0 = Monday); ordinal 0 = every.
    by_day: Vec<(i32, u32)>,
    by_month_day: Vec<i32>,
    by_month: Vec<u32>,
}

impl Rule {
    fn parse(s: &str) -> Rule {
        let mut r = Rule { interval: 1, ..Default::default() };
        for part in s.split(';') {
            let Some((k, v)) = part.split_once('=') else { continue };
            match k.trim().to_ascii_uppercase().as_str() {
                "FREQ" => r.freq = v.trim().to_ascii_uppercase(),
                "INTERVAL" => r.interval = v.trim().parse::<i64>().unwrap_or(1).clamp(1, 10_000),
                "COUNT" => r.count = v.trim().parse::<usize>().ok().map(|c| c.min(MAX_OCCURRENCES * 10)),
                "UNTIL" => r.until = date_time(v).map(|(t, all_day, _)| if all_day { t + 86_399 } else { t }),
                "BYDAY" => {
                    r.by_day = v
                        .split(',')
                        .filter_map(|d| {
                            let d = d.trim();
                            let split = d.len().checked_sub(2)?;
                            let (n, w) = (d.get(..split)?, d.get(split..)?);
                            let nth = if n.is_empty() || n == "+" { 0 } else { n.trim_start_matches('+').parse().ok()? };
                            Some((nth, weekday_code(w)?))
                        })
                        .take(31)
                        .collect();
                }
                "BYMONTHDAY" => {
                    r.by_month_day = v.split(',').filter_map(|d| d.trim().parse().ok()).filter(|d: &i32| *d != 0 && d.abs() <= 31).take(31).collect()
                }
                "BYMONTH" => r.by_month = v.split(',').filter_map(|m| m.trim().parse().ok()).filter(|m| (1..=12).contains(m)).take(12).collect(),
                _ => {}
            }
        }
        r
    }

    /// The candidate days (since the epoch) of the period containing `day`, in order.
    fn days_in_period(&self, day: i64, start_day: i64) -> Vec<i64> {
        let (y, m, _) = civil_from_days(day);
        let wd = |x: i64| (x + 3).rem_euclid(7) as u32;
        match self.freq.as_str() {
            "DAILY" => vec![day],
            "WEEKLY" => {
                if self.by_day.is_empty() {
                    return vec![day];
                }
                // The week (Monday first) containing `day`.
                let monday = day - i64::from(wd(day));
                let mut v: Vec<i64> = self.by_day.iter().map(|(_, w)| monday + i64::from(*w)).collect();
                v.sort_unstable();
                v.dedup();
                v
            }
            "MONTHLY" => {
                let mut v: Vec<i64> = Vec::new();
                for md in &self.by_month_day {
                    if let Some(x) = month_day(y, m, *md) {
                        v.push(x);
                    }
                }
                for (nth, w) in &self.by_day {
                    if *nth == 0 {
                        let first = days_from_civil(y, m, 1);
                        let mut x = first + (i64::from(*w) - i64::from(wd(first))).rem_euclid(7);
                        while civil_from_days(x).1 == m {
                            v.push(x);
                            x += 7;
                        }
                    } else if let Some(x) = nth_weekday(y, m, *nth, *w) {
                        v.push(x);
                    }
                }
                if v.is_empty() {
                    let (_, _, sd) = civil_from_days(start_day);
                    if let Some(x) = month_day(y, m, i32::try_from(sd).unwrap_or(1)) {
                        v.push(x);
                    }
                }
                v.sort_unstable();
                v.dedup();
                v
            }
            "YEARLY" => {
                let (_, sm, sd) = civil_from_days(start_day);
                let months: Vec<u32> = if self.by_month.is_empty() { vec![sm] } else { self.by_month.clone() };
                let mut v = Vec::new();
                for mo in months {
                    if self.by_day.is_empty()
                        && self.by_month_day.is_empty()
                        && let Some(x) = month_day(y, mo, i32::try_from(sd).unwrap_or(1))
                    {
                        v.push(x);
                    }
                    for md in &self.by_month_day {
                        v.extend(month_day(y, mo, *md));
                    }
                    for (nth, w) in &self.by_day {
                        v.extend(nth_weekday(y, mo, if *nth == 0 { 1 } else { *nth }, *w));
                    }
                }
                v.sort_unstable();
                v.dedup();
                v
            }
            _ => vec![],
        }
    }

    /// The first day of the period `k` periods after the one containing `start_day`.
    fn period(&self, start_day: i64, k: i64) -> Option<i64> {
        let (y, m, _) = civil_from_days(start_day);
        let step = k.checked_mul(self.interval)?;
        match self.freq.as_str() {
            "DAILY" => start_day.checked_add(step),
            "WEEKLY" => start_day.checked_add(step.checked_mul(7)?),
            "MONTHLY" => {
                let total = i64::from(m) - 1 + step;
                let yy = y.checked_add(total.div_euclid(12))?;
                (yy <= 9999).then(|| days_from_civil(yy, u32::try_from(total.rem_euclid(12) + 1).unwrap_or(1), 1))
            }
            "YEARLY" => {
                let yy = y.checked_add(step)?;
                (yy <= 9999).then(|| days_from_civil(yy, 1, 1))
            }
            _ => None,
        }
    }
}

/// The day of a month (`md` 1-based, negative from the end), if the month has it.
fn month_day(y: i64, m: u32, md: i32) -> Option<i64> {
    let first = days_from_civil(y, m, 1);
    let next = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
    let len = next - first;
    let d = if md > 0 { i64::from(md) } else { len + 1 + i64::from(md) };
    (1..=len).contains(&d).then_some(first + d - 1)
}

/// Every start of a (possibly recurring) event, from its first, up to `before`.
pub fn starts(e: &CalEvent, before: i64) -> Vec<i64> {
    let Some(rule) = e.rrule.as_deref().map(Rule::parse).filter(|r| !r.freq.is_empty()) else {
        return vec![e.start];
    };
    let start_day = e.start.div_euclid(86_400);
    let time = e.start.rem_euclid(86_400);
    let mut out = Vec::new();
    let mut produced = 0usize;
    let mut steps = 0usize;
    let mut k = 0i64;
    'periods: while let Some(p) = rule.period(start_day, k) {
        k += 1;
        for day in rule.days_in_period(if k == 1 { start_day } else { p }, start_day) {
            steps += 1;
            if steps > MAX_STEPS {
                break 'periods;
            }
            let t = day * 86_400 + time;
            if t < e.start {
                continue;
            }
            if !rule.by_month.is_empty() && rule.freq != "YEARLY" && !rule.by_month.contains(&civil_from_days(day).1) {
                continue;
            }
            if rule.until.is_some_and(|u| t > u) || t >= before {
                break 'periods;
            }
            produced += 1;
            if rule.count.is_some_and(|c| produced > c) {
                break 'periods;
            }
            out.push(t);
            if out.len() >= MAX_OCCURRENCES * 10 {
                break 'periods;
            }
        }
        if steps > MAX_STEPS {
            break;
        }
    }
    out
}

/// The occurrences of `events` overlapping `[from, to)`, by start. Instances with a
/// `RECURRENCE-ID` replace the occurrence of the same event they name; cancelled ones are left out.
pub fn occurrences(events: &[CalEvent], from: i64, to: i64) -> Vec<Occurrence> {
    let mut out = Vec::new();
    for (i, e) in events.iter().enumerate() {
        if e.recurrence_id.is_some() || e.status == "CANCELLED" {
            continue;
        }
        let len = (e.end - e.start).max(0);
        let overrides: Vec<i64> = events.iter().filter(|o| o.uid == e.uid && o.recurrence_id.is_some()).filter_map(|o| o.recurrence_id).collect();
        let mut n = 0;
        for s in starts(e, to) {
            if s + len.max(1) <= from || e.exdates.contains(&s) || overrides.contains(&s) {
                continue;
            }
            out.push(Occurrence { event: i, start: s, end: s + len });
            n += 1;
            if n >= MAX_OCCURRENCES {
                break;
            }
        }
    }
    for (i, e) in events.iter().enumerate() {
        if e.recurrence_id.is_some() && e.status != "CANCELLED" && e.start < to && e.end.max(e.start + 1) > from {
            out.push(Occurrence { event: i, start: e.start, end: e.end });
        }
    }
    out.sort_by_key(|o| (o.start, o.event));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i64, m: u32, d: u32, h: i64, mi: i64) -> i64 {
        days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60
    }

    const INVITE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//EN\r\nMETHOD:REQUEST\r\n\
BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\n\
BEGIN:DAYLIGHT\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nDTSTART:19700329T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\nEND:DAYLIGHT\r\n\
BEGIN:STANDARD\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nDTSTART:19701025T030000\r\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nEND:STANDARD\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:review-42@studio.example\r\nDTSTAMP:20261001T080000Z\r\n\
DTSTART;TZID=Europe/Berlin:20261015T140000\r\nDTEND;TZID=Europe/Berlin:20261015T153000\r\n\
SUMMARY:Storyboard review\\, scenes 4–7\r\nLOCATION:Room 2\\; upstairs\r\n\
DESCRIPTION:Bring the revised frames.\\nAnd the colour key.\r\n\
ORGANIZER;CN=\"Zoe Park\":mailto:zoe@studio.example\r\n\
ATTENDEE;CN=Riley Moreno;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:riley@studio.example\r\n\
X-MICROSOFT-CDO-BUSYSTATUS:BUSY\r\nBEGIN:VALARM\r\nTRIGGER:-PT15M\r\nACTION:DISPLAY\r\nEND:VALARM\r\n\
SEQUENCE:2\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    /// Categories (which colour an event) come in and go out, commas inside a name escaped.
    #[test]
    fn categories_both_ways() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:c1\r\nDTSTART:20261016T140000Z\r\nDTEND:20261016T150000Z\r\nCATEGORIES:Clients\\, big,Mix\r\nCATEGORIES:Mix,Lab\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let e = &parse(ics, 0).events[0];
        assert_eq!(e.categories, ["Clients, big", "Mix", "Lab"]);
        assert!(e.extra.iter().all(|l| !l.starts_with("CATEGORIES")));
        let out = write(std::slice::from_ref(e), None, 0);
        assert!(out.contains("CATEGORIES:Clients\\, big,Mix,Lab"), "{out}");
        assert_eq!(parse(&out, 0).events[0].categories, e.categories);
        assert_eq!(split_text_list(" a ,, b\\,c "), ["a", "b,c"]);
    }

    #[test]
    fn reads_an_invitation() {
        let cal = parse(INVITE, 0);
        assert_eq!(cal.method.as_deref(), Some("REQUEST"));
        let e = &cal.events[0];
        assert_eq!(e.uid, "review-42@studio.example");
        assert_eq!(e.summary, "Storyboard review, scenes 4–7");
        assert_eq!(e.location, "Room 2; upstairs");
        assert_eq!(e.description, "Bring the revised frames.\nAnd the colour key.");
        // 14:00 in Berlin in October (summer time, +2) is 12:00 UTC.
        assert_eq!(e.start, at(2026, 10, 15, 12, 0));
        assert_eq!(e.end - e.start, 5400);
        assert_eq!(e.organizer.as_ref().map(|o| (o.name.as_str(), o.email.as_str())), Some(("Zoe Park", "zoe@studio.example")));
        assert_eq!(e.attendees[0].partstat, "NEEDS-ACTION");
        assert!(e.attendees[0].rsvp);
        assert_eq!(e.sequence, 2);
        assert!(e.extra.iter().any(|x| x.starts_with("X-MICROSOFT")), "unknown properties are kept");
        assert!(e.extra.iter().any(|x| x == "TRIGGER:-PT15M"), "alarms too");
        // In winter the same zone is +1.
        let winter = INVITE.replace("20261015T140000", "20261215T140000").replace("20261015T153000", "20261215T153000");
        assert_eq!(parse(&winter, 0).events[0].start, at(2026, 12, 15, 13, 0));
    }

    #[test]
    fn writes_what_it_reads() {
        let cal = parse(INVITE, 0);
        let text = write(&cal.events, Some("REPLY"), at(2026, 10, 2, 9, 0));
        assert!(text.lines().all(|l| l.len() <= 76), "folded:\n{text}");
        assert!(text.contains("DTSTART:20261015T120000Z\r\n"));
        assert!(text.contains("SUMMARY:Storyboard review\\, scenes 4–7"));
        let back = parse(&text, 0);
        assert_eq!(back.method.as_deref(), Some("REPLY"));
        let (a, b) = (&cal.events[0], &back.events[0]);
        assert_eq!(
            (&a.uid, &a.summary, &a.location, &a.description, a.start, a.end),
            (&b.uid, &b.summary, &b.location, &b.description, b.start, b.end)
        );
        assert_eq!(a.attendees, b.attendees);
        assert_eq!(a.extra, b.extra, "unknown lines survive a round trip");
        // Long text folds on character boundaries.
        let long = CalEvent { uid: "x".into(), summary: "ü".repeat(100), start: 0, end: 3600, ..Default::default() };
        assert_eq!(parse(&write(std::slice::from_ref(&long), None, 0), 0).events[0].summary, long.summary);
    }

    #[test]
    fn all_day_durations_and_junk() {
        let text = "BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20261224\nSUMMARY:Holiday\nEND:VEVENT\n\
                    BEGIN:VEVENT\nUID:b\nDTSTART:20261015T090000Z\nDURATION:PT1H30M\nEND:VEVENT\n\
                    BEGIN:VEVENT\nUID:c\nDTSTART:20261015T090000\nEND:VEVENT\n";
        let cal = parse(text, 2 * 3600);
        assert!(cal.events[0].all_day);
        assert_eq!((cal.events[0].start, cal.events[0].end - cal.events[0].start), (days_from_civil(2026, 12, 24) * 86_400, 86_400));
        assert_eq!(cal.events[1].end - cal.events[1].start, 5400);
        assert_eq!(cal.events[2].start, at(2026, 10, 15, 7, 0), "floating time is local");
        assert_eq!(duration("P1W"), Some(604_800));
        assert_eq!(duration("-PT15M"), Some(-900));
        assert_eq!(duration("P1H"), None);
        for junk in ["", "BEGIN:VEVENT", "garbage\r\n:::\r\n;;;", "BEGIN:VEVENT\nDTSTART:2026\nEND:VEVENT", &"BEGIN:X\n".repeat(100)] {
            let _ = parse(junk, 0);
        }
    }

    #[test]
    fn recurrences() {
        let mk = |rrule: &str, start: i64| CalEvent { uid: "r".into(), start, end: start + 3600, rrule: Some(rrule.into()), ..Default::default() };
        let mon = at(2026, 10, 5, 9, 0); // a Monday
        // Every Monday and Wednesday, 5 times.
        let e = mk("FREQ=WEEKLY;BYDAY=MO,WE;COUNT=5", mon);
        let s = starts(&e, i64::MAX);
        assert_eq!(s, vec![mon, mon + 2 * 86_400, mon + 7 * 86_400, mon + 9 * 86_400, mon + 14 * 86_400]);
        // Every other day until a date.
        let e = mk("FREQ=DAILY;INTERVAL=2;UNTIL=20261011T000000Z", mon);
        assert_eq!(starts(&e, i64::MAX).len(), 3);
        // The last Friday of each month.
        let e = mk("FREQ=MONTHLY;BYDAY=-1FR;COUNT=3", at(2026, 10, 30, 17, 0));
        assert_eq!(starts(&e, i64::MAX), vec![at(2026, 10, 30, 17, 0), at(2026, 11, 27, 17, 0), at(2026, 12, 25, 17, 0)]);
        // The 31st: months without one are skipped.
        let e = mk("FREQ=MONTHLY;BYMONTHDAY=31;COUNT=3", at(2026, 1, 31, 8, 0));
        assert_eq!(starts(&e, i64::MAX), vec![at(2026, 1, 31, 8, 0), at(2026, 3, 31, 8, 0), at(2026, 5, 31, 8, 0)]);
        // Yearly on the start's date.
        let e = mk("FREQ=YEARLY;COUNT=2", at(2024, 2, 29, 0, 0));
        assert_eq!(starts(&e, i64::MAX), vec![at(2024, 2, 29, 0, 0), at(2028, 2, 29, 0, 0)], "a leap day recurs on leap years");
        // Endless rules stop at the window and the caps.
        let e = mk("FREQ=DAILY", mon);
        assert_eq!(starts(&e, mon + 10 * 86_400).len(), 10);
        assert!(starts(&mk("FREQ=SECONDLY", mon), i64::MAX).is_empty());
        assert!(starts(&mk("FREQ=DAILY;BYMONTH=13", mon), mon + 400 * 86_400).len() <= 400);

        // Occurrences in a window, with an exception and a moved instance.
        let mut weekly = mk("FREQ=WEEKLY", mon);
        weekly.exdates.push(mon + 7 * 86_400);
        let moved = CalEvent {
            uid: "r".into(),
            start: mon + 15 * 86_400,
            end: mon + 15 * 86_400 + 3600,
            recurrence_id: Some(mon + 14 * 86_400),
            ..Default::default()
        };
        let events = vec![weekly, moved];
        let occ = occurrences(&events, mon, mon + 22 * 86_400);
        let starts_seen: Vec<i64> = occ.iter().map(|o| o.start).collect();
        assert_eq!(starts_seen, vec![mon, mon + 15 * 86_400, mon + 21 * 86_400]);
        assert_eq!(occ[1].event, 1, "the moved instance");
    }

    #[test]
    fn nth_weekdays() {
        assert_eq!(nth_weekday(2026, 3, -1, 6), Some(days_from_civil(2026, 3, 29)), "last Sunday of March 2026");
        assert_eq!(nth_weekday(2026, 10, 2, 0), Some(days_from_civil(2026, 10, 12)), "second Monday");
        assert_eq!(nth_weekday(2026, 2, 5, 0), None, "no fifth Monday in February 2026");
        assert_eq!(month_day(2026, 2, -1), Some(days_from_civil(2026, 2, 28)));
    }
}
