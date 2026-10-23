//! Time zones in calendar files: the `VTIMEZONE`s a file defines, which zone an event's times
//! are in (the IANA database's when it knows the name, else the file's definition), and the
//! definition written with an event in a zone.

use super::*;
use crate::zone::Zone;

/// The most lines read from one `VTIMEZONE`, and the most of them read from one file.
const MAX_ZONE_LINES: usize = 400;
const MAX_ZONES: usize = 50;

/// A time zone as a `VTIMEZONE` defines it: its standard and daylight offsets and when each
/// starts.
#[derive(Clone, Debug, Default)]
pub(super) struct VZone {
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

impl VZone {
    /// Read from a `VTIMEZONE`'s lines.
    fn read(block: &[Line]) -> VZone {
        let mut z = VZone::default();
        let mut part: Option<ZonePart> = None;
        for l in block {
            match l.name.as_str() {
                "BEGIN" if matches!(l.value.trim().to_ascii_uppercase().as_str(), "STANDARD" | "DAYLIGHT") => {
                    part = Some(ZonePart { offset: 0, yearly: None, time: 0, since: i64::MIN });
                }
                "END" if matches!(l.value.trim().to_ascii_uppercase().as_str(), "STANDARD" | "DAYLIGHT") => {
                    if z.parts.len() < MAX_ZONE_LINES {
                        z.parts.extend(part.take());
                    }
                }
                "TZOFFSETTO" => {
                    if let (Some(p), Some(o)) = (part.as_mut(), utc_offset(&l.value)) {
                        p.offset = o;
                    }
                }
                "DTSTART" => {
                    if let (Some(p), Some((t, _, _))) = (part.as_mut(), date_time(&l.value)) {
                        p.since = t;
                        p.time = t.rem_euclid(86_400);
                    }
                }
                "RRULE" => {
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
        z
    }

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

/// The zone an event's times are in.
#[derive(Clone, Debug)]
pub(super) enum EventZone {
    /// The IANA database's.
    Db(Zone),
    /// As the file defined it.
    File(VZone),
}

impl EventZone {
    /// The instant of a local wall-clock time.
    pub(super) fn to_utc(&self, local: i64) -> i64 {
        match self {
            EventZone::Db(z) => z.to_utc(local),
            EventZone::File(v) => local.saturating_sub(v.offset_at(local)),
        }
    }

    /// The local wall-clock time of an instant.
    pub(super) fn local(&self, utc: i64) -> i64 {
        match self {
            EventZone::Db(z) => z.local(utc),
            EventZone::File(v) => utc.saturating_add(v.offset_at(utc.saturating_add(v.offset_at(utc)))),
        }
    }
}

/// The zone an event's times are in: `None` for UTC and all-day events (and a zone that can't
/// be read, whose times were turned into UTC when it was read).
pub(super) fn zone_of(e: &CalEvent) -> Option<EventZone> {
    if e.all_day || e.tzid.is_empty() {
        return None;
    }
    if let Some(z) = Zone::named(&e.tzid) {
        return Some(EventZone::Db(z));
    }
    let block: Vec<Line> = e.vtimezone.iter().take(MAX_ZONE_LINES).filter_map(|r| parse_line(r)).collect();
    let v = VZone::read(&block);
    (!v.parts.is_empty()).then_some(EventZone::File(v))
}

/// The zones of one file, and the viewer's (for floating times and zones nobody defines).
pub(super) struct FileZones {
    /// Each `VTIMEZONE`: its `TZID`, its lines as written, and what they say.
    blocks: Vec<(String, Vec<String>, VZone)>,
    pub(super) local: Zone,
}

impl FileZones {
    pub(super) fn read(all: &[Line], local: Zone) -> FileZones {
        let mut blocks = Vec::new();
        let mut i = 0;
        while let Some(l) = all.get(i) {
            i += 1;
            if l.name != "BEGIN" || !l.value.trim().eq_ignore_ascii_case("VTIMEZONE") {
                continue;
            }
            let from = i - 1;
            while let Some(m) = all.get(i) {
                i += 1;
                if (m.name == "END" && m.value.trim().eq_ignore_ascii_case("VTIMEZONE")) || i - from > MAX_ZONE_LINES {
                    break;
                }
            }
            let block = all.get(from..i).unwrap_or_default();
            // Its own TZID, not one of a part inside it.
            let id = block.iter().find(|l| l.name == "TZID").map(|l| l.value.trim().to_string()).unwrap_or_default();
            if !id.is_empty() && blocks.len() < MAX_ZONES {
                blocks.push((id, block.iter().map(|l| l.raw.clone()).collect(), VZone::read(block)));
            }
        }
        FileZones { blocks, local }
    }

    /// The zone a `TZID` names: the database's if it knows the name, else the file's.
    pub(super) fn resolve(&self, id: &str) -> Option<EventZone> {
        if let Some(z) = Zone::named(id) {
            return Some(EventZone::Db(z));
        }
        self.blocks.iter().find(|(b, _, v)| b == id && !v.parts.is_empty()).map(|(_, _, v)| EventZone::File(v.clone()))
    }

    /// The `VTIMEZONE` lines kept with an event in zone `id`: only for a zone the database
    /// doesn't know (it writes its own for the ones it does).
    pub(super) fn definition(&self, id: &str) -> Vec<String> {
        if Zone::named(id).is_some() {
            return Vec::new();
        }
        self.blocks.iter().find(|(b, _, _)| b == id).map(|(_, raw, _)| raw.clone()).unwrap_or_default()
    }

    /// A DATE or DATE-TIME property as (Unix seconds, all day, the zone it was in). A local time
    /// is in its `TZID`, else in `fallback` (an exception's event's zone), else the viewer's.
    pub(super) fn time(&self, l: &Line, fallback: &str) -> Option<(i64, bool, String)> {
        let (t, all_day, utc) = date_time(&l.value)?;
        if all_day || l.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE")) {
            return Some((t - t.rem_euclid(86_400), true, String::new()));
        }
        if utc {
            return Some((t, false, String::new()));
        }
        let id = l.param("TZID").map(|z| z.trim_matches('"').trim()).filter(|z| !z.is_empty()).unwrap_or(fallback);
        if !id.is_empty()
            && let Some(z) = self.resolve(id)
        {
            return Some((z.to_utc(t), false, id.to_string()));
        }
        // Floating, or a zone nobody defines: the viewer's.
        Some((self.local.to_utc(t), false, self.local.name().unwrap_or_default().to_string()))
    }
}

/// Days since the epoch of the `nth` (1-based; negative from the end) `weekday` (0 Monday) of a
/// month.
pub(super) fn nth_weekday(year: i64, month: u32, nth: i32, weekday: u32) -> Option<i64> {
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

pub(super) fn weekday_code(s: &str) -> Option<u32> {
    ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|w| s.eq_ignore_ascii_case(w)).and_then(|p| u32::try_from(p).ok())
}

/// `+0100`, `-0530` → seconds.
pub(super) fn utc_offset(s: &str) -> Option<i64> {
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

fn offset_text(o: i64) -> String {
    let a = o.unsigned_abs();
    let sec = if a.is_multiple_of(60) { String::new() } else { format!("{:02}", a % 60) };
    format!("{}{:02}{:02}{sec}", if o < 0 { '-' } else { '+' }, a / 3600, a % 3600 / 60)
}

/// A local time as a DATE-TIME without a zone (`20260105T090000`).
pub(super) fn local_stamp(t: i64) -> String {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    let secs = t.rem_euclid(86_400);
    let (hh, mm, ss) = (secs / 3600, secs / 60 % 60, secs % 60);
    format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}")
}

/// A `TZID` parameter's value, quoted when it must be.
pub(super) fn tzid_param(id: &str) -> String {
    let id = id.replace(['\r', '\n', '"'], "");
    if id.contains([':', ';', ',']) { format!("\"{id}\"") } else { id }
}

/// A `VTIMEZONE` for a zone of the database, as its clocks change in `year`: yearly rules when
/// they change twice (summer time), else the offsets as they are.
pub(super) fn zone_definition(id: &str, zone: Zone, year: i64) -> Vec<String> {
    let year = year.clamp(1971, 9998);
    let (from, to) = (days_from_civil(year, 1, 1) * 86_400, days_from_civil(year + 1, 1, 1) * 86_400);
    // Each change: (the instant, the offset before, the offset after).
    let mut changes: Vec<(i64, i64, i64)> = Vec::new();
    let mut before = zone.offset_at(from);
    let mut t = from;
    while t < to && changes.len() < 4 {
        let next = t + 86_400;
        let after = zone.offset_at(next);
        if after != before {
            // The first second with the new offset.
            let (mut lo, mut hi) = (t, next);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if zone.offset_at(mid) == after { hi = mid } else { lo = mid }
            }
            changes.push((hi, before, after));
            before = after;
        }
        t = next;
    }
    let mut out = vec!["BEGIN:VTIMEZONE".to_string(), format!("TZID:{}", id.replace(['\r', '\n'], ""))];
    let part = |out: &mut Vec<String>, dst: bool, from: i64, to: i64, start: String, rule: Option<String>| {
        let kind = if dst { "DAYLIGHT" } else { "STANDARD" };
        out.push(format!("BEGIN:{kind}"));
        out.push(format!("TZOFFSETFROM:{}", offset_text(from)));
        out.push(format!("TZOFFSETTO:{}", offset_text(to)));
        out.push(format!("DTSTART:{start}"));
        out.extend(rule);
        out.push(format!("END:{kind}"));
    };
    if changes.len() == 2 {
        for (at, from, to) in &changes {
            // The local time it happens at, on the clock before the change.
            let local = at + from;
            let day = local.div_euclid(86_400);
            let (_, m, d) = civil_from_days(day);
            let month_len = month_day(year, m, -1).map_or(31, |last| last - days_from_civil(year, m, 1) + 1);
            let nth: i32 = if i64::from(d) + 7 > month_len { -1 } else { i32::try_from((d - 1) / 7 + 1).unwrap_or(1) };
            let wd = (day + 3).rem_euclid(7) as u32;
            let code = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].get(wd as usize).copied().unwrap_or("SU");
            // The rule's first year, as calendars write it.
            let first = nth_weekday(1970, m, nth, wd).unwrap_or(day);
            let start = local_stamp(first * 86_400 + local.rem_euclid(86_400));
            part(&mut out, zone.is_dst(*at), *from, *to, start, Some(format!("RRULE:FREQ=YEARLY;BYMONTH={m};BYDAY={nth}{code}")));
        }
    } else {
        let initial = zone.offset_at(from);
        part(&mut out, zone.is_dst(from), initial, initial, "19700101T000000".into(), None);
        for (at, from, to) in &changes {
            part(&mut out, zone.is_dst(*at), *from, *to, local_stamp(at + from), None);
        }
    }
    out.push("END:VTIMEZONE".into());
    out
}
