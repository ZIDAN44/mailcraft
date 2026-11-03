//! A repeating appointment's pattern (PidLidAppointmentRecur, MS-OXOCAL 2.2.1.44): its rule, the
//! occurrences deleted, and those changed. Each changed occurrence has an ExceptionInfo
//! (2.2.1.44.2: its times, and the subject, location, show-as, reminder or all-day it changed) and
//! an ExtendedException (2.2.1.44.3: the subject and location again, in UTF-16), and is kept in
//! full as an attached exception item of the appointment, which `changed_events` reads it with.

use super::*;

/// Changed occurrences a pattern says it has, read at most.
const MAX_CHANGED: usize = 5_000;
/// ExceptionInfo's OverrideFlags (2.2.1.44.2): what a changed occurrence changed.
const ARO_SUBJECT: u16 = 0x0001;
const ARO_MEETINGTYPE: u16 = 0x0002;
const ARO_REMINDERDELTA: u16 = 0x0004;
const ARO_REMINDER: u16 = 0x0008;
const ARO_LOCATION: u16 = 0x0010;
const ARO_BUSYSTATUS: u16 = 0x0020;
const ARO_ATTACHMENT: u16 = 0x0040;
const ARO_SUBTYPE: u16 = 0x0080;
const ARO_APPTCOLOR: u16 = 0x0100;
const ARO_EXCEPTIONAL_BODY: u16 = 0x0200;
/// The writer version from which each ExtendedException starts with its change highlight.
const WITH_HIGHLIGHT: u32 = 0x3009;
/// How far an exception item's own time of the occurrence it replaces may be from the pattern's
/// (which reads local times at one offset, so a change of summer time between is an hour off).
const MATCH_WITHIN: i64 = 2 * 3600;

struct Bytes<'a>(&'a [u8], usize);

impl<'a> Bytes<'a> {
    fn u16(&mut self) -> Option<u16> {
        let b = self.0.get(self.1..self.1.checked_add(2)?)?;
        self.1 += 2;
        Some(u16::from_le_bytes([*b.first()?, *b.get(1)?]))
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.0.get(self.1..self.1.checked_add(4)?)?;
        self.1 += 4;
        Some(u32::from_le_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]))
    }
    /// The next `n` bytes.
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.0.get(self.1..self.1.checked_add(n)?)?;
        self.1 += n;
        Some(b)
    }
    /// A block that says its own size in a `u32` first, passed over.
    fn skip_block(&mut self) -> Option<()> {
        let n = usize::try_from(self.u32()?).ok()?;
        self.take(n).map(|_| ())
    }
}

const DAYS: [&str; 7] = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];

fn byday(mask: u32, ordinal: Option<i32>) -> String {
    let pre = ordinal.map(|n| n.to_string()).unwrap_or_default();
    DAYS.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, d)| format!("{pre}{d}")).collect::<Vec<_>>().join(",")
}

/// A changed occurrence as the pattern has it, its times in UTC (at the series' offset, as its
/// occurrences are); what it didn't change is `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Changed {
    /// The start of the occurrence it replaces (its `RECURRENCE-ID`), and its own times.
    pub original: i64,
    pub start: i64,
    pub end: i64,
    pub subject: Option<String>,
    pub location: Option<String>,
    pub busy: Option<i64>,
    /// Whether it reminds, and how many minutes ahead.
    pub reminder_set: Option<bool>,
    pub reminder: Option<i64>,
    pub all_day: Option<bool>,
    /// Its text is its own (in its exception item).
    pub own_body: bool,
}

/// What a recurrence pattern says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pattern {
    pub rule: String,
    /// The occurrences deleted, and those changed.
    pub exdates: Vec<i64>,
    pub changed: Vec<Changed>,
}

/// A recurrence pattern as (rule, deleted occurrences), given the first occurrence's start (UTC).
pub(crate) fn recurrence(blob: &[u8], start: i64) -> Option<(String, Vec<i64>)> {
    pattern(blob, start).map(|p| (p.rule, p.exdates))
}

/// A recurrence pattern, given the first occurrence's start (UTC): its rule, the occurrences
/// deleted and those changed.
pub(crate) fn pattern(blob: &[u8], start: i64) -> Option<Pattern> {
    let mut b = Bytes(blob, 0);
    let (_reader, _writer) = (b.u16()?, b.u16()?);
    let frequency = b.u16()?;
    let pattern = b.u16()?;
    let _calendar = b.u16()?;
    let _first = b.u32()?;
    let period = i64::from(b.u32()?);
    let _sliding = b.u32()?;
    let (mut a, mut c) = (0, 0);
    match pattern {
        0x0001 | 0x0002 | 0x0004 | 0x000A | 0x000C => a = b.u32()?,
        0x0003 | 0x000B => {
            a = b.u32()?;
            c = b.u32()?;
        }
        _ => {}
    }
    let end_type = b.u32()?;
    let count = b.u32()?;
    let first_dow = b.u32()?;
    let deleted_count = usize::try_from(b.u32()?).ok()?.min(MAX_EXCEPTIONS);
    let deleted: Vec<i64> = (0..deleted_count).map(|_| b.u32().map(i64::from)).collect::<Option<_>>()?;
    let modified_count = usize::try_from(b.u32()?).ok()?.min(MAX_EXCEPTIONS);
    let modified: Vec<i64> = (0..modified_count).map(|_| b.u32().map(i64::from)).collect::<Option<_>>()?;
    let first_day = i64::from(b.u32()?);
    let end_day = i64::from(b.u32()?);
    // The appointment part: when in the (local) day the occurrences start, then the changed
    // occurrences (none when it's cut short).
    let (start_offset, changed) = (|| {
        b.u32()?;
        let writer = b.u32()?;
        let offset = i64::from(b.u32()?);
        Some((offset, changes(&mut b, writer)))
    })()
    .unwrap_or((0, Vec::new()));
    let nth = |n: u32| if n >= 5 { -1 } else { i32::try_from(n).unwrap_or(1) };
    let mut rule = match (frequency, pattern) {
        (0x200A, 0x0000) => format!("FREQ=DAILY;INTERVAL={}", (period / 1440).max(1)),
        (0x200A, 0x0001) => "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_string(),
        (0x200B, 0x0001) => format!("FREQ=WEEKLY;INTERVAL={};BYDAY={}", period.max(1), byday(a, None)),
        (0x200C, 0x0002) => format!("FREQ=MONTHLY;INTERVAL={};BYMONTHDAY={a}", period.max(1)),
        (0x200C, 0x0004) => format!("FREQ=MONTHLY;INTERVAL={};BYMONTHDAY=-1", period.max(1)),
        (0x200C, 0x0003) => format!("FREQ=MONTHLY;INTERVAL={};BYDAY={}", period.max(1), byday(a, Some(nth(c)))),
        (0x200D, 0x0002 | 0x0004) => format!("FREQ=YEARLY;INTERVAL={}", (period / 12).max(1)),
        (0x200D, 0x0003) => {
            let (_, month, _) = civil_from_days(start.div_euclid(86_400));
            format!("FREQ=YEARLY;INTERVAL={};BYMONTH={month};BYDAY={}", (period / 12).max(1), byday(a, Some(nth(c))))
        }
        _ => return None,
    };
    // The day weeks start on (FirstDOW) says which weeks an every-so-many-weeks rule takes
    // (RFC 5545's WKST); Sunday, as a rule's default, needn't be said.
    if (frequency, pattern) == (0x200B, 0x0001)
        && period > 1
        && let Some(day) = usize::try_from(first_dow).ok().filter(|d| *d > 0).and_then(|d| DAYS.get(d))
    {
        rule.push_str(&format!(";WKST={day}"));
    }
    match end_type {
        0x2021 => {
            let (y, m, d) = civil_from_days((end_day - EPOCH_MINUTES).div_euclid(1440));
            rule.push_str(&format!(";UNTIL={y:04}{m:02}{d:02}T235959Z"));
        }
        0x2022 if count > 0 => rule.push_str(&format!(";COUNT={count}")),
        _ => {}
    }
    // Local day + time into it → UTC, by how far the first occurrence is from its local time.
    let local_first = (first_day - EPOCH_MINUTES + start_offset) * 60;
    let shift = start.saturating_sub(local_first);
    // Sanity: a first day far from the start means the blob isn't what it says.
    if shift.saturating_abs() > 2 * 86_400 {
        return Some(Pattern { rule, ..Default::default() });
    }
    let utc = |local_minutes: i64| (local_minutes - EPOCH_MINUTES).saturating_mul(60).saturating_add(shift);
    // A changed occurrence is deleted as well, on the day it was: not an EXDATE, as the event
    // that replaces it takes it. Without its ExceptionInfo, the day it's on now says which.
    let day = |minutes: i64| minutes.div_euclid(1440);
    let gone = |d: &i64| match changed.is_empty() {
        true => !modified.contains(d),
        false => !changed.iter().any(|c| day(c.original) == day(*d)),
    };
    let exdates = deleted.iter().filter(|d| gone(d)).map(|d| utc(d + start_offset)).collect();
    let changed = changed
        .into_iter()
        .map(|c| {
            let start = utc(c.start);
            Changed { original: utc(c.original), start, end: utc(c.end).max(start), ..c }
        })
        .collect();
    Some(Pattern { rule, exdates, changed })
}

/// The changed occurrences after a pattern's start offset (`b` at its end offset), their times in
/// local minutes; those read before a damaged one. `writer` is the pattern's WriterVersion2.
fn changes(b: &mut Bytes<'_>, writer: u32) -> Vec<Changed> {
    let mut out = Vec::new();
    let Some(count) = (|| {
        b.u32()?;
        b.u16()
    })() else {
        return out;
    };
    let mut flags = Vec::new();
    for _ in 0..usize::from(count).min(MAX_CHANGED) {
        match info(b) {
            Some((c, f)) => {
                out.push(c);
                flags.push(f);
            }
            None => return out,
        }
    }
    // The subject and location again, as written (UTF-16): these win over the 8-bit ones.
    if b.skip_block().is_none() {
        return out;
    }
    for (c, f) in out.iter_mut().zip(flags) {
        if extended(b, c, f, writer).is_none() {
            break;
        }
    }
    out
}

/// One ExceptionInfo, and its OverrideFlags.
fn info(b: &mut Bytes<'_>) -> Option<(Changed, u16)> {
    let start = i64::from(b.u32()?);
    let end = i64::from(b.u32()?);
    let original = i64::from(b.u32()?);
    let flags = b.u16()?;
    let mut c = Changed { original, start, end, ..Default::default() };
    // An 8-bit text: its length plus one, its length, its bytes (Windows-1252 as no code page is
    // given).
    let text = |b: &mut Bytes<'_>| -> Option<String> {
        b.u16()?;
        let n = usize::from(b.u16()?);
        Some(crate::pst::text_from_bytes(b.take(n)?, 1252, false).trim_end_matches('\0').to_string())
    };
    if flags & ARO_SUBJECT != 0 {
        c.subject = Some(text(b)?);
    }
    if flags & ARO_MEETINGTYPE != 0 {
        b.u32()?;
    }
    if flags & ARO_REMINDERDELTA != 0 {
        c.reminder = Some(i64::from(b.u32()?)).filter(|m| *m <= MAX_MINUTES);
    }
    if flags & ARO_REMINDER != 0 {
        c.reminder_set = Some(b.u32()? != 0);
    }
    if flags & ARO_LOCATION != 0 {
        c.location = Some(text(b)?);
    }
    if flags & ARO_BUSYSTATUS != 0 {
        c.busy = Some(i64::from(b.u32()?));
    }
    // Whether it has files of its own: its exception item has them, if so.
    if flags & ARO_ATTACHMENT != 0 {
        b.u32()?;
    }
    if flags & ARO_SUBTYPE != 0 {
        c.all_day = Some(b.u32()? != 0);
    }
    if flags & ARO_APPTCOLOR != 0 {
        b.u32()?;
    }
    c.own_body = flags & ARO_EXCEPTIONAL_BODY != 0;
    Some((c, flags))
}

/// One ExtendedException, into the changed occurrence it goes with (`flags`: its OverrideFlags).
fn extended(b: &mut Bytes<'_>, c: &mut Changed, flags: u16, writer: u32) -> Option<()> {
    if writer >= WITH_HIGHLIGHT {
        b.skip_block()?;
    }
    b.skip_block()?;
    if flags & (ARO_SUBJECT | ARO_LOCATION) != 0 {
        // Its start, end and original start again.
        b.take(12)?;
        let mut wide = || -> Option<String> {
            let n = usize::from(b.u16()?).checked_mul(2)?;
            let units: Vec<u16> = b.take(n)?.as_chunks::<2>().0.iter().map(|u| u16::from_le_bytes(*u)).collect();
            Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
        };
        if flags & ARO_SUBJECT != 0 {
            c.subject = Some(wide()?);
        }
        if flags & ARO_LOCATION != 0 {
            c.location = Some(wide()?);
        }
        b.skip_block()?;
    }
    Some(())
}

/// A changed occurrence's own item (an attached exception item of the appointment): its
/// properties, text and guests. Its subject is the pattern's to say (its own may be the series').
pub(crate) struct Own<'a> {
    pub x: &'a Extra,
    pub body: &'a str,
    pub people: &'a [EmailAddress],
}

/// The changed occurrences of the series `x` (whose `subject`, `body`, organizer and guests are
/// the rest), each an event of its own with the series' `uid` (as [`event_of`] gives it) and the
/// occurrence it replaces; with each, which of `own` (the appointment's exception items) is its
/// item, matched by the occurrence it replaces. What an occurrence didn't change is the series'.
/// An item no ExceptionInfo matches (its pattern cut short or damaged, or past [`MAX_CHANGED`])
/// is a changed occurrence all the same, replacing the series' occurrence nearest the one it says
/// it does (within [`MATCH_WITHIN`]; with none that near, its own time); one that says
/// no time, or replaces an occurrence another already does (within [`MATCH_WITHIN`]), isn't
/// among them.
pub(crate) fn changed_events(
    x: &Extra,
    subject: &str,
    body: &str,
    from: &EmailAddress,
    people: &[EmailAddress],
    uid: &str,
    own: &[Own<'_>],
) -> Vec<(CalEvent, Option<usize>)> {
    let Some(start) = x.start else { return Vec::new() };
    let Some(p) = pattern(&x.recurrence, start) else { return Vec::new() };
    let mut taken = vec![false; own.len()];
    let mut out = Vec::new();
    // What each occurrence starts from: the series without its pattern (which may be big, and
    // was read already), copied once per occurrence.
    let base = Extra { recurrence: Vec::new(), members: Vec::new(), ..x.clone() };
    let day = |t: i64| t.saturating_add(43_200).div_euclid(86_400) * 86_400;
    for c in p.changed {
        // Its item: the one whose occurrence (or, when it doesn't say, start) is nearest.
        let near = |o: &Own<'_>| {
            let (at, mine) = match o.x.replaces {
                Some(r) => (r, c.original),
                None => (o.x.start?, c.start),
            };
            let off = at.saturating_sub(mine).saturating_abs();
            (off <= MATCH_WITHIN).then_some(off)
        };
        let found = own.iter().enumerate().filter(|(i, _)| !taken.get(*i).copied().unwrap_or(true)).filter_map(|(i, o)| Some((near(o)?, i))).min();
        let found = found.map(|(_, i)| i);
        if let Some(t) = found.and_then(|i| taken.get_mut(i)) {
            *t = true;
        }
        let item = found.and_then(|i| own.get(i));
        let mut one = base.clone();
        // Its own item has its times exactly (in UTC); the pattern's are at the series' offset.
        (one.start, one.end) = match item.and_then(|o| Some((o.x.start?, o.x.end.unwrap_or(o.x.start?)))) {
            Some((s, e)) => (Some(s), Some(e.max(s))),
            None => (Some(c.start), Some(c.end)),
        };
        if let Some(location) = &c.location {
            one.location = location.clone();
        }
        if c.busy.is_some() {
            one.busy = c.busy;
        }
        if let Some(all_day) = c.all_day {
            one.all_day = all_day;
        }
        let set = c.reminder_set.unwrap_or(x.reminder.is_some());
        one.reminder = set.then(|| c.reminder.filter(|m| *m >= 0).or(x.reminder).unwrap_or(15));
        let summary = c.subject.as_deref().unwrap_or(subject);
        let text = match item {
            Some(o) if c.own_body => o.body,
            _ => body,
        };
        let guests = item.map(|o| o.people).filter(|p| !p.is_empty()).unwrap_or(people);
        let Some(mut e) = event_of(&one, summary, text, from, guests, uid.to_string()) else { continue };
        e.recurrence_id = Some(if x.all_day { day(c.original) } else { c.original });
        out.push((e, found));
    }
    // The items left: each with its own times, place, text and guests, the rest the series'.
    let mut replaced: std::collections::BTreeSet<i64> = out.iter().filter_map(|(e, _)| e.recurrence_id).collect();
    // The series' own occurrences (at its offset, as the calendar shows them), up to the last
    // a left item could replace: an item's time of its occurrence may be an hour off them (a
    // change of summer time between), and only a RECURRENCE-ID equal to one replaces it.
    let left = own.iter().enumerate().filter(|(i, _)| !taken.get(*i).copied().unwrap_or(true));
    let last = left.filter_map(|(_, o)| o.x.replaces.or(o.x.start)).max();
    let series: std::collections::BTreeSet<i64> = match (last, event_of(x, subject, "", from, &[], uid.to_string())) {
        (Some(last), Some(master)) => ical::starts(&master, last.saturating_add(MATCH_WITHIN).saturating_add(86_400)).into_iter().collect(),
        _ => Default::default(),
    };
    for (i, o) in own.iter().enumerate() {
        if taken.get(i).copied().unwrap_or(true) {
            continue;
        }
        let Some(s) = o.x.start else { continue };
        let original = o.x.replaces.unwrap_or(s);
        let original = if x.all_day { day(original) } else { original };
        // The series' occurrence nearest (within MATCH_WITHIN) is the one it replaces; with none
        // that near (its pattern cut short), its own time is.
        let original = series
            .range(original.saturating_sub(MATCH_WITHIN)..=original.saturating_add(MATCH_WITHIN))
            .min_by_key(|t| t.saturating_sub(original).saturating_abs())
            .copied()
            .unwrap_or(original);
        // One near an occurrence already replaced is another copy of it (its time may be an
        // hour off the pattern's, as a match may be).
        if replaced.range(original.saturating_sub(MATCH_WITHIN)..=original.saturating_add(MATCH_WITHIN)).next().is_some() {
            continue;
        }
        replaced.insert(original);
        let mut one = base.clone();
        (one.start, one.end) = (Some(s), Some(o.x.end.unwrap_or(s).max(s)));
        if !o.x.location.is_empty() {
            one.location = o.x.location.clone();
        }
        let text = if o.body.trim().is_empty() { body } else { o.body };
        let guests = Some(o.people).filter(|p| !p.is_empty()).unwrap_or(people);
        let Some(mut e) = event_of(&one, subject, text, from, guests, uid.to_string()) else { continue };
        e.recurrence_id = Some(original);
        out.push((e, Some(i)));
    }
    out
}
