//! iCalendar (RFC 5545) events and tasks: reading, writing and expanding recurrences.
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
//! `BYMONTHDAY`, `BYMONTH`, `BYSETPOS`, and `BYHOUR`, `BYMINUTE` and `BYSECOND` (the times of
//! each day, or of each hour or minute); hourly, minutely and secondly ones every so long from the
//! start, on the days `BYDAY`, `BYMONTHDAY` and `BYMONTH` allow; and `RDATE`'s added dates. A
//! series repeats at its wall-clock time in its zone
//! (each occurrence turned into UTC on its own date, so summer time doesn't move it), on that
//! zone's days; `EXDATE` removes occurrences and `RECURRENCE-ID` instances replace them, both
//! matched as instants. Expansion is capped.

use serde::{Deserialize, Serialize};

use crate::dates::{civil_from_days, days_from_civil};

mod alarms;
mod find;
mod free_busy;
mod meeting;
mod notes;
mod pattern;
mod reading;
mod recurrence;
mod scheduling;
mod showing;
mod tasks;
mod zones;
pub use alarms::*;
pub use find::*;
pub use free_busy::{free_busy_request, free_busy_times};
pub use meeting::*;
pub use notes::*;
pub use pattern::*;
use reading::*;
pub use recurrence::*;
pub use scheduling::*;
pub use showing::*;
pub use tasks::TASK_STATUS;
use tasks::*;
use zones::*;
pub use zones::{ZoneChange, ZoneRule, wall_clock, zone_rule};

/// The most occurrences produced for one event in one expansion (and written, one by one, for a
/// series a format can't keep as its rule).
pub const MAX_OCCURRENCES: usize = 2000;
/// The most rule iterations tried (rules that rarely match).
const MAX_STEPS: usize = 50_000;
/// The longest input read: as much as a file opened is (a calendar exported with its events' files reads
/// back whole).
const MAX_INPUT: usize = 16 << 20;

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
    /// Whom they handed their place to (`DELEGATED-TO`, RFC 5545 §3.2.5; their answer is then
    /// `DELEGATED`, issue #254): the first one named.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub delegated_to: String,
    /// Who handed them their place (`DELEGATED-FROM`, §3.2.4): the first one named.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub delegated_from: String,
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
    /// Occurrences added to the rule's (`RDATE`; a period's start), or, with no rule, to the
    /// event's own start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rdates: Vec<i64>,
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
    /// The calendar's server leaves this meeting's invitations and answers to the program that
    /// stores it (RFC 6638 `SCHEDULE-AGENT=CLIENT` on its organizer): MailCraft mails them.
    /// Written on the organizer and every guest in a server's copy ([`write_for_server`]), never
    /// in an invitation or a file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub client_scheduled: bool,
    /// The server delivered this version to guests itself (`SCHEDULE-STATUS` 1.x or 2.x on a
    /// guest, RFC 6638 §7.3). Only read, never written.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub server_delivered: bool,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

impl CalEvent {
    /// The address that joins it online: the server's own, a `CONFERENCE` link, else the first
    /// meeting link in its location or notes (as Outlook's Join finds a Teams or Zoom meeting).
    /// [`CalEvent::join_link_in`] counts the user's own meeting hosts too.
    pub fn join_link(&self) -> Option<String> {
        self.join_link_in(&[])
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

/// A calendar file: what it asks (`METHOD`, for invitations), its events and tasks, and, for a
/// calendar published to subscribe to, its name and how often to read it again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ICalendar {
    pub method: Option<String>,
    pub events: Vec<CalEvent>,
    /// Its to-dos (`VTODO`).
    pub tasks: Vec<crate::pim::Task>,
    /// `NAME` (RFC 7986) or `X-WR-CALNAME`.
    pub name: Option<String>,
    /// `REFRESH-INTERVAL` (RFC 7986) or `X-PUBLISHED-TTL`, in seconds.
    pub refresh: Option<i64>,
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
    // At a character's start (a cut inside one would read the whole text).
    let mut end = MAX_INPUT.min(text.len());
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let text = text.get(..end).unwrap_or_default();
    let mut unfolded: Vec<String> = Vec::new();
    for l in text.split('\n') {
        // A CR inside a line is taken out, as writing takes it out: what's read is what's written.
        let l = l.replace('\r', "");
        match (l.strip_prefix(' ').or_else(|| l.strip_prefix('\t')), unfolded.last_mut()) {
            (Some(rest), Some(prev)) => prev.push_str(rest),
            _ => unfolded.push(l),
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

/// A TEXT value with its escapes undone.
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

/// Whether a character may stand in a content line: any but the control characters, of which
/// only HTAB is allowed (RFC 5545 §3.1, RFC 6350 §3.3). The others are left out of what's
/// written (but of the uid a meeting's message echoes), and of a uid as a server stores it
/// ([`stored_uid`]).
pub fn allowed(c: char) -> bool {
    c == '\t' || !c.is_ascii_control()
}

/// A `UID` value as a server stores it: what can be written of it ([`allowed`]), trimmed. A
/// task's and a card's are kept so. An event's is kept as its organizer wrote it (they look for
/// their meeting by it in an answer), and its copy on a server has this one: it's looked for
/// there, and told from the others it reads, by this.
pub fn stored_uid(v: &str) -> String {
    v.chars().filter(|c| allowed(*c)).collect::<String>().trim().to_string()
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
        delegated_to: l.param("DELEGATED-TO").map(first_address).unwrap_or_default(),
        delegated_from: l.param("DELEGATED-FROM").map(first_address).unwrap_or_default(),
    }
}

/// The first address of a list of them (`"mailto:a@b","mailto:c@d"`), without its quotes and
/// its `mailto:`.
fn first_address(v: &str) -> String {
    let first = v.split(',').next().unwrap_or_default().trim().trim_matches('"').trim();
    first.get(..7).filter(|p| p.eq_ignore_ascii_case("mailto:")).map_or(first, |_| first.get(7..).unwrap_or("")).trim().to_string()
}

/// The most lines an event or a to-do keeps as written (`extra`): its properties MailCraft doesn't
/// read, and the components it holds. One that begins past it is left out whole.
const EXTRA_MAX: usize = 200;

/// An event's alarms may go on to this many: its reminder is kept past a calendar's own
/// properties, where every reader of its alarms finds it.
const EXTRA_ALARMS_MAX: usize = 400;

/// The most events read from one file, and the most tasks.
const MAX_ITEMS: usize = 20_000;
/// The most categories an event or a task keeps.
const MAX_CATEGORIES: usize = 20;
/// The most of an event's excluded dates kept, and of its added ones (and the most lines giving
/// them, and its changed instance, waiting for its zone).
const MAX_DATES: usize = 1000;
/// The most guests an event keeps.
const MAX_ATTENDEES: usize = 500;
/// The longest calendar name kept, in characters.
const CALENDAR_NAME_MAX: usize = 200;

/// Add the categories of a `CATEGORIES` value, each once, within [`MAX_CATEGORIES`].
fn add_categories(to: &mut Vec<String>, value: &str) {
    for c in split_text_list(value) {
        if !to.contains(&c) && to.len() < MAX_CATEGORIES {
            to.push(c);
        }
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
    let mut event: Option<EventReading<'_>> = None;
    // The lines outside every event, where its to-dos are: an event's are its own, whatever they
    // begin and end, so a to-do it holds is never one of the file's too.
    let mut outside: Vec<&Line> = Vec::new();
    for l in &all {
        let Some(reading) = event.as_mut() else {
            if l.name == "BEGIN" && l.value.trim().eq_ignore_ascii_case("VEVENT") {
                event = Some(EventReading::new());
            } else {
                outside.push(l);
                calendar_property(&mut cal, l);
            }
            continue;
        };
        if reading.line(l, &zones)
            && let Some(done) = event.take()
            && cal.events.len() < MAX_ITEMS
        {
            cal.events.push(done.finish(&zones));
        }
    }
    cal.tasks = read_tasks(&outside, &zones);
    cal
}

/// A line outside every event that tells about the calendar: what it asks, its name, how often
/// to read it again.
fn calendar_property(cal: &mut ICalendar, l: &Line) {
    match l.name.as_str() {
        "METHOD" => cal.method = Some(l.value.trim().to_ascii_uppercase()),
        "NAME" | "X-WR-CALNAME" if cal.name.is_none() => {
            let name: String = unescape(&l.value).chars().filter(|c| !c.is_control()).take(CALENDAR_NAME_MAX).collect();
            cal.name = Some(name.trim().to_string()).filter(|s| !s.is_empty());
        }
        "REFRESH-INTERVAL" | "X-PUBLISHED-TTL" if cal.refresh.is_none() => cal.refresh = duration(l.value.trim()).filter(|d| *d > 0),
        _ => {}
    }
}

// ----- writing -----

/// Fold a content line at 75 octets (RFC 5545 §3.1), on character boundaries; a character it
/// may not hold ([`allowed`]) is left out.
fn fold(line: &str, out: &mut String) {
    fold_chars(line.chars().filter(|c| allowed(*c)), out);
}

/// Fold a content line of these characters at 75 octets.
fn fold_chars(line: impl Iterator<Item = char>, out: &mut String) {
    let mut width = 0;
    for c in line {
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

/// Lines kept as written (an event's or a task's, a zone's definition), made whole to write back:
/// line breaks taken out, every component they begin ended (by its own name), an `END` that ends
/// nothing left out, and a component named in `left_out` left out with all it holds (a `VEVENT`
/// in a task would begin an event when read back). Read back, they begin and end what they did.
fn whole<'a>(raw: impl IntoIterator<Item = &'a String>, left_out: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    // The components open around the line: each one's name, and whether it's written.
    let mut open: Vec<(String, bool)> = Vec::new();
    for r in raw {
        // Line breaks, and the other characters a line can't hold ([`allowed`]), taken out
        // before it's read: what it begins and ends is what it will when written and read back.
        let s: String = r.chars().filter(|c| allowed(*c)).collect();
        // Not a folded line's next part, nor nothing.
        let s = s.trim_start_matches([' ', '\t']);
        let Some(l) = parse_line(s) else { continue };
        let writing = open.iter().all(|(_, w)| *w);
        match l.name.as_str() {
            "BEGIN" => {
                let name = l.value.trim().to_string();
                let w = writing && !left_out.iter().any(|x| name.eq_ignore_ascii_case(x));
                // Written as its end is, with nothing around its name (servers refuse a
                // `BEGIN: valarm`, which reading takes).
                if w {
                    out.push(format!("BEGIN:{name}"));
                }
                open.push((name, w));
            }
            "END" => {
                if let Some((name, true)) = open.pop() {
                    out.push(format!("END:{name}"));
                }
            }
            _ if writing => out.push(s.to_string()),
            _ => {}
        }
    }
    while let Some((name, w)) = open.pop() {
        if w {
            out.push(format!("END:{name}"));
        }
    }
    out
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

fn mailbox(prop: &str, a: &Attendee, client: bool) -> String {
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
    // An address in quotes: what would end the quotes, the line or the value, out.
    let address = |v: &str| v.replace(['"', '\r', '\n', ',', ';', ':'], "");
    if !a.delegated_to.is_empty() {
        s.push_str(&format!(";DELEGATED-TO=\"mailto:{}\"", address(&a.delegated_to)));
    }
    if !a.delegated_from.is_empty() {
        s.push_str(&format!(";DELEGATED-FROM=\"mailto:{}\"", address(&a.delegated_from)));
    }
    if client {
        s.push_str(";SCHEDULE-AGENT=CLIENT");
    }
    s.push_str(&format!(":mailto:{}", a.email.replace(['\r', '\n'], "")));
    s
}

/// What a written event is for: what of the server's scheduling it carries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// A file, or an invitation: no `SCHEDULE-AGENT` (RFC 6638 §7.1: never in iTIP), and no
    /// mark of a guest's own copy.
    Plain,
    /// A CalDAV server's copy: both, as the event has them.
    Server,
}

fn write_one(e: &CalEvent, now: i64, purpose: Purpose, out: &mut String) {
    let client = purpose == Purpose::Server && e.client_scheduled;
    fold("BEGIN:VEVENT", out);
    // The uid of a file's or a message's event is the one it came with, whatever it holds but
    // line breaks: an organizer finds their meeting by it in an answer. A server's copy has it
    // as a line may hold it ([`stored_uid`]: servers store it so, or refuse it).
    match purpose {
        Purpose::Plain => fold_chars("UID:".chars().chain(e.uid.chars().filter(|c| !matches!(c, '\r' | '\n'))), out),
        Purpose::Server => fold(&format!("UID:{}", e.uid), out),
    }
    let mut lines: Vec<String> = vec![format!("DTSTAMP:{}", stamp(now))];
    // Timed events in a zone are written in it (repeating at its wall-clock time); others in UTC.
    let zone = zone_of(e);
    let when = |prop: &str, t: i64| match &zone {
        Some(z) => format!("{prop};TZID={}:{}", tzid_param(&e.tzid), local_stamp(z.local(t))),
        None => format!("{prop}:{}", stamp(t)),
    };
    if e.all_day {
        lines.push(format!("DTSTART;VALUE=DATE:{}", day(e.start)));
        lines.push(format!("DTEND;VALUE=DATE:{}", day(e.end.max(e.start.saturating_add(86_400)))));
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
    // A changed occurrence adds none.
    for x in e.rdates.iter().filter(|_| e.recurrence_id.is_none()) {
        lines.push(if e.all_day { format!("RDATE;VALUE=DATE:{}", day(*x)) } else { when("RDATE", *x) });
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
        lines.push(mailbox("ORGANIZER", o, client));
    }
    for a in &e.attendees {
        lines.push(mailbox("ATTENDEE", a, client));
    }
    lines.extend(whole(e.extra.iter().filter(|x| purpose == Purpose::Server || !is_guest_copy_line(x)), &[]));
    lines.push("END:VEVENT".into());
    for l in lines {
        fold(&l, out);
    }
}

/// A calendar file with these events (`method` for invitations: `REQUEST`, `REPLY`, `CANCEL`).
/// An invitation is stamped `now`; a stored event keeps the stamp it came with, so the copy on a
/// server still tells which invitation it was.
pub fn write(events: &[CalEvent], method: Option<&str>, now: i64) -> String {
    write_all(events, method, now, Purpose::Plain)
}

/// A calendar file with these events and tasks (`VTODO`s): a task stored in a CalDAV task list
/// (issue #201) is one with that task alone.
pub fn write_calendar(events: &[CalEvent], tasks: &[crate::pim::Task], now: i64) -> String {
    let mut out = write_events(events, None, now, Purpose::Plain);
    for t in tasks {
        write_task(t, now, &mut out);
    }
    fold("END:VCALENDAR", &mut out);
    out
}

/// A CalDAV server's copy of these events: as [`write`] stores them, with who schedules them
/// (`SCHEDULE-AGENT=CLIENT` on the organizer and every guest of a meeting MailCraft schedules)
/// and the mark of a guest's own copy ([`mark_guest_copy`]).
pub fn write_for_server(events: &[CalEvent], now: i64) -> String {
    write_all(events, None, now, Purpose::Server)
}

fn write_all(events: &[CalEvent], method: Option<&str>, now: i64, purpose: Purpose) -> String {
    let mut out = write_events(events, method, now, purpose);
    fold("END:VCALENDAR", &mut out);
    out
}

/// A calendar file's beginning, its zones and these events, folded; what's left is its end.
fn write_events(events: &[CalEvent], method: Option<&str>, now: i64, purpose: Purpose) -> String {
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
        // Whole, and holding no event, task or calendar: a definition cut short, or with one of
        // those in it, would swallow what's written after it.
        for l in whole(&definition, &["VEVENT", "VTODO", "VCALENDAR"]) {
            fold(&l, &mut out);
        }
    }
    for e in events {
        let stamp = if method.is_none() && e.dtstamp > 0 { e.dtstamp } else { now };
        write_one(e, stamp, purpose, &mut out);
    }
    out
}

#[cfg(test)]
mod tests;
