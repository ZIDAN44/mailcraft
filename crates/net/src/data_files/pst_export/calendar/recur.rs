//! A repeating event as Outlook's AppointmentRecurrencePattern (MS-OXOCAL 2.2.1.44.5): the
//! RecurrencePattern of its rule (`super::super::pattern`) with the days of its deleted and
//! changed occurrences, when in the day its occurrences start and end, and each changed one's
//! ExceptionInfo (its times, and the subject, place, show-as, reminder or all-day it changed) and
//! ExtendedException (the subject and place again, in UTF-16; writer version 0x3009, with its
//! change highlight), as `pst_pim::recur` reads them; each changed occurrence is also an item
//! attached to the appointment (`super::exception`). A pattern is kept only when the import reads
//! it back as the same occurrences and the same ones deleted: otherwise [`series`] has none, and
//! the series goes in one occurrence at a time.

use super::*;
use crate::data_files::pst_export::pattern;

/// The class of a changed occurrence's item (MS-OXOCAL 2.2.10.1.1).
pub(super) const EXCEPTION_CLASS: &str = "IPM.OLE.CLASS.{00061055-0000-0000-C000-000000000046}";
/// ReaderVersion2 and WriterVersion2 (2.2.1.44.5): from 0x3009 each ExtendedException starts with
/// its change highlight.
const READER_VERSION: u32 = 0x3006;
const WRITER_VERSION: u32 = 0x3009;
/// ExceptionInfo's OverrideFlags (2.2.1.44.2).
const ARO_SUBJECT: u16 = 0x0001;
const ARO_REMINDERDELTA: u16 = 0x0004;
const ARO_REMINDER: u16 = 0x0008;
const ARO_LOCATION: u16 = 0x0010;
const ARO_BUSYSTATUS: u16 = 0x0020;
const ARO_ATTACHMENT: u16 = 0x0040;
const ARO_SUBTYPE: u16 = 0x0080;
const ARO_EXCEPTIONAL_BODY: u16 = 0x0200;
/// Minutes from 1601-01-01 to 1970-01-01.
const EPOCH_MINUTES: i64 = 11_644_473_600 / 60;
/// The longest subject or place an ExceptionInfo carries, in characters (its lengths are 16-bit).
const MAX_TEXT: usize = 32_000;
/// When a series that doesn't end ends, as Outlook says it (31 December 4500, 23:59, in minutes
/// from 1601).
const NO_END: i64 = 0x5AE9_80DF;

/// A series as the appointment keeps it.
pub(super) struct Series {
    pub(super) pattern: Vec<u8>,
    /// PidLidRecurrenceType: 1 daily, 2 weekly, 3 monthly, 4 yearly.
    pub(super) kind: i32,
    /// PidLidClipStart and PidLidClipEnd: the midnights of its first and last days.
    pub(super) clip: (i64, i64),
    /// Its changed occurrences as attached items, and which occurrences they (and those called
    /// off) replace; a changed occurrence that replaces none of the series' is left for the
    /// caller.
    pub(super) changed: Vec<Attached>,
    pub(super) taken: Vec<i64>,
}

/// Local minutes from 1601 of a local time (seconds, as if UTC).
fn minutes(local: i64) -> Option<u32> {
    u32::try_from(local.div_euclid(60).checked_add(EPOCH_MINUTES)?).ok()
}

/// The pattern of the series `e` with its changed occurrences `own` (each with its
/// `RECURRENCE-ID`; called off: deleted); `None` when the pattern can't say its rule, or the
/// import wouldn't read it back as the same occurrences. `now` stands in for when an occurrence
/// was last changed when it doesn't say.
pub(super) fn series(e: &CalEvent, own: &[&CalEvent], now: i64) -> Option<Series> {
    let rule = e.rrule.as_deref()?.trim();
    let all = ical::starts(e, i64::MAX);
    let local = |t: i64| ical::wall_clock(e, t);
    let first = local(e.start);
    let day = |t: i64| local(t).div_euclid(86_400).checked_mul(86_400).and_then(minutes);
    // The occurrences it replaces: changed, and called off.
    let mut changed: Vec<&CalEvent> = Vec::new();
    let mut gone: Vec<i64> = e.exdates.iter().copied().filter(|t| all.contains(t)).collect();
    let mut taken = Vec::new();
    for o in own {
        let Some(r) = o.recurrence_id.filter(|r| all.contains(r) && !e.exdates.contains(r) && !taken.contains(r)) else { continue };
        taken.push(r);
        if o.status.eq_ignore_ascii_case("CANCELLED") {
            gone.push(r);
        } else {
            changed.push(o);
        }
    }
    gone.sort_unstable();
    gone.dedup();
    changed.sort_by_key(|o| (o.start, o.recurrence_id));
    let mut deleted: Vec<u32> = gone.iter().chain(changed.iter().filter_map(|o| o.recurrence_id.as_ref())).map(|t| day(*t)).collect::<Option<_>>()?;
    deleted.sort_unstable();
    deleted.dedup();
    let modified: Vec<u32> = changed.iter().map(|o| day(o.start)).collect::<Option<_>>()?;
    let mut blob = pattern::write_with(rule, first.div_euclid(86_400).checked_mul(86_400)?, &deleted, &modified)?;
    // When in the day it starts and ends (minutes from the local midnight).
    let start_offset = u32::try_from(first.rem_euclid(86_400) / 60).ok()?;
    let end_offset = start_offset.checked_add(u32::try_from(e.end.saturating_sub(e.start).max(0) / 60).ok()?)?;
    for w in [READER_VERSION, WRITER_VERSION, start_offset, end_offset] {
        blob.extend(w.to_le_bytes());
    }
    blob.extend(u16::try_from(changed.len()).ok()?.to_le_bytes());
    let flags: Vec<u16> = changed.iter().map(|o| overrides(e, o)).collect();
    for (o, f) in changed.iter().zip(&flags) {
        info(&mut blob, o, *f, &local)?;
    }
    blob.extend(0u32.to_le_bytes());
    for (o, f) in changed.iter().zip(&flags) {
        extended(&mut blob, o, *f, &local)?;
    }
    blob.extend(0u32.to_le_bytes());
    if !reads_back(e, &blob, &all, &gone) {
        return None;
    }
    let kind = match rule.to_ascii_uppercase() {
        r if r.contains("FREQ=DAILY") => 1,
        r if r.contains("FREQ=WEEKLY") => 2,
        r if r.contains("FREQ=MONTHLY") => 3,
        _ => 4,
    };
    let midnight = |t: i64| t.saturating_sub(local(t).rem_euclid(86_400));
    let last = all.last().copied().unwrap_or(e.start);
    let ends = rule.to_ascii_uppercase().contains("COUNT=") || rule.to_ascii_uppercase().contains("UNTIL=");
    let clip_end = if ends { filetime(midnight(last)) } else { NO_END.saturating_mul(60).saturating_mul(TICKS) };
    let attached = changed.iter().filter_map(|o| Some(super::exception(e, o, o.recurrence_id?, &local, now))).collect();
    Some(Series { pattern: blob, kind, clip: (filetime(midnight(e.start)), clip_end), changed: attached, taken })
}

/// What the changed occurrence `o` of `e` changed, as OverrideFlags.
fn overrides(e: &CalEvent, o: &CalEvent) -> u16 {
    let mut f = 0;
    if o.summary != e.summary {
        f |= ARO_SUBJECT;
    }
    if o.location != e.location {
        f |= ARO_LOCATION;
    }
    if busy(o) != busy(e) {
        f |= ARO_BUSYSTATUS;
    }
    match (ical::reminder_minutes(o), ical::reminder_minutes(e)) {
        (a, b) if a == b => {}
        (Some(_), _) => f |= ARO_REMINDER | ARO_REMINDERDELTA,
        (None, _) => f |= ARO_REMINDER,
    }
    if o.all_day != e.all_day {
        f |= ARO_SUBTYPE;
    }
    if o.description != e.description {
        f |= ARO_EXCEPTIONAL_BODY;
    }
    if !files(o).is_empty() {
        f |= ARO_ATTACHMENT;
    }
    f
}

/// A text as an ExceptionInfo's 8-bit one (Windows-1252's Latin-1 part; anything else `?`),
/// its 16-bit lengths first.
fn narrow(out: &mut Vec<u8>, s: &str) -> Option<()> {
    let bytes: Vec<u8> =
        s.chars().take(MAX_TEXT).map(|c| u8::try_from(u32::from(c)).ok().filter(|b| !(0x80..0xA0).contains(b)).unwrap_or(b'?')).collect();
    let n = u16::try_from(bytes.len()).ok()?;
    out.extend(n.checked_add(1)?.to_le_bytes());
    out.extend(n.to_le_bytes());
    out.extend(bytes);
    Some(())
}

/// A text as an ExtendedException's (its length in UTF-16 units, then them).
fn wide(out: &mut Vec<u8>, s: &str) -> Option<()> {
    let units: Vec<u16> = s.chars().take(MAX_TEXT).collect::<String>().encode_utf16().collect();
    out.extend(u16::try_from(units.len()).ok()?.to_le_bytes());
    units.iter().for_each(|u| out.extend(u.to_le_bytes()));
    Some(())
}

/// Its times as an ExceptionInfo and an ExtendedException say them: start, end and the start of
/// the occurrence it replaces, in local minutes.
fn times(o: &CalEvent, local: &dyn Fn(i64) -> i64) -> Option<[u32; 3]> {
    Some([minutes(local(o.start))?, minutes(local(o.end.max(o.start)))?, minutes(local(o.recurrence_id?))?])
}

/// The ExceptionInfo of `o`, a changed occurrence that changed `f`.
fn info(out: &mut Vec<u8>, o: &CalEvent, f: u16, local: &dyn Fn(i64) -> i64) -> Option<()> {
    times(o, local)?.iter().for_each(|w| out.extend(w.to_le_bytes()));
    out.extend(f.to_le_bytes());
    let int = |out: &mut Vec<u8>, n: i64| out.extend(u32::try_from(n.max(0)).unwrap_or(0).to_le_bytes());
    if f & ARO_SUBJECT != 0 {
        narrow(out, &o.summary)?;
    }
    if f & ARO_REMINDERDELTA != 0 {
        int(out, ical::reminder_minutes(o).unwrap_or(0));
    }
    if f & ARO_REMINDER != 0 {
        int(out, i64::from(ical::reminder_minutes(o).is_some()));
    }
    if f & ARO_LOCATION != 0 {
        narrow(out, &o.location)?;
    }
    if f & ARO_BUSYSTATUS != 0 {
        int(out, i64::from(busy(o)));
    }
    if f & ARO_ATTACHMENT != 0 {
        int(out, 1);
    }
    if f & ARO_SUBTYPE != 0 {
        int(out, i64::from(o.all_day));
    }
    Some(())
}

/// The ExtendedException of `o` that changed `f`.
fn extended(out: &mut Vec<u8>, o: &CalEvent, f: u16, local: &dyn Fn(i64) -> i64) -> Option<()> {
    // Its change highlight (none), and an empty reserved block.
    out.extend(4u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    if f & (ARO_SUBJECT | ARO_LOCATION) != 0 {
        times(o, local)?.iter().for_each(|w| out.extend(w.to_le_bytes()));
        if f & ARO_SUBJECT != 0 {
            wide(out, &o.summary)?;
        }
        if f & ARO_LOCATION != 0 {
            wide(out, &o.location)?;
        }
        out.extend(0u32.to_le_bytes());
    }
    Some(())
}

/// Whether the import reads `blob` back as `e`'s occurrences (`all`), with those `gone` deleted:
/// in the zone it reads (`zone::read_back`), the rule it reads.
fn reads_back(e: &CalEvent, blob: &[u8], all: &[i64], gone: &[i64]) -> bool {
    let tzid = super::zone::read_back(e);
    let Some((rule, exdates)) = crate::pst_pim::recurrence_in(blob, e.start, &tzid) else { return false };
    let back = CalEvent { rrule: Some(rule), exdates: Vec::new(), tzid, vtimezone: Vec::new(), ..e.clone() };
    let mut exdates = exdates;
    exdates.sort_unstable();
    ical::starts(&back, i64::MAX) == all && exdates == gone
}
