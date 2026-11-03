//! The zone an appointment's times were set in, as Outlook keeps it: PidLidTimeZoneStruct (MS-OXOCAL
//! 2.2.1.39: its offsets and when the clocks change, each a SYSTEMTIME "day of month"), its name
//! (PidLidTimeZoneDescription: the event's `TZID`, which the import reads back) and its
//! definitions (2.2.1.41's TZDEFINITION with one TZRULE: when it starts and ends,
//! PidLidAppointmentTimeZoneDefinitionStartDisplay and EndDisplay, and a series',
//! …DefinitionRecur), keyed by Windows' name for the zone when it has one. All-day events are in
//! this computer's zone, as Outlook keeps them (UTC when it isn't known), and those in no zone in
//! UTC.

use super::*;
use mailcraft_model::ical::{ZoneChange, ZoneRule};
use mailcraft_model::zone::Zone;

/// TZDEFINITION's and TZRULE's versions, TZDEFINITION's reserved word, TZRULE's size (2.2.1.41).
const VERSIONS: [u8; 2] = [0x02, 0x01];
const DEFINITION_FLAGS: u16 = 0x0002;
const RULE_SIZE: u16 = 0x003E;
/// TZRULE_FLAG_RECUR_CURRENT_TZREG and TZRULE_FLAG_EFFECTIVE_TZREG.
const RULE_RECUR: u16 = 0x0001;
const RULE_EFFECTIVE: u16 = 0x0002;
/// The longest key name written, in UTF-16 units (Windows' are under 64).
const MAX_KEY: usize = 255;

/// The zone the import reads `e`'s times back in: its `TZID` when the time zone database knows it
/// and it isn't UTC, for a timed event; else none.
pub(super) fn read_back(e: &CalEvent) -> String {
    let id = e.tzid.trim();
    let known = Zone::named(id).is_some_and(|z| z != Zone::UTC);
    if e.all_day || !known { String::new() } else { id.to_string() }
}

/// `e`'s zone in the year it starts (UTC when it has none).
fn rule(e: &CalEvent) -> (ZoneRule, u16) {
    let (y, _, _) = mailcraft_model::dates::civil_from_days(ical::wall_clock(e, e.start).div_euclid(86_400));
    (ical::zone_rule(e, y).unwrap_or_default(), u16::try_from(y.clamp(1601, 30_827)).unwrap_or(1601))
}

/// A SYSTEMTIME saying when the clocks change (none: zeros).
fn system_time(c: Option<ZoneChange>) -> [u8; 16] {
    let mut out = [0u8; 16];
    if let Some(c) = c {
        let words = [
            0,
            c.month,
            c.weekday,
            c.nth,
            u32::try_from(c.time / 3600).unwrap_or(0),
            u32::try_from(c.time % 3600 / 60).unwrap_or(0),
            u32::try_from(c.time % 60).unwrap_or(0),
            0,
        ];
        for (slot, w) in out.as_chunks_mut::<2>().0.iter_mut().zip(words) {
            *slot = u16::try_from(w).unwrap_or(0).to_le_bytes();
        }
    }
    out
}

/// lBias, lStandardBias and lDaylightBias (minutes; UTC is local time plus the bias).
fn biases(r: &ZoneRule) -> [i32; 3] {
    let minutes = |s: i64| i32::try_from(s / 60).unwrap_or(0);
    let summer = if r.to_daylight.is_some() { -minutes(r.daylight.saturating_sub(r.standard)) } else { 0 };
    [-minutes(r.standard), 0, summer]
}

/// PidLidTimeZoneStruct.
fn time_zone_struct(r: &ZoneRule) -> Vec<u8> {
    let mut out: Vec<u8> = biases(r).iter().flat_map(|b| b.to_le_bytes()).collect();
    out.extend(0u16.to_le_bytes());
    out.extend(system_time(r.to_standard));
    out.extend(0u16.to_le_bytes());
    out.extend(system_time(r.to_daylight));
    out
}

/// A TZDEFINITION named `key` with the one rule `r`, in force from `year`; `flags` the rule's.
fn definition(key: &str, r: &ZoneRule, year: u16, flags: u16) -> Vec<u8> {
    let units: Vec<u16> = key.encode_utf16().take(MAX_KEY).collect();
    let n = u16::try_from(units.len()).unwrap_or(0);
    let mut out = VERSIONS.to_vec();
    // The header's size: the reserved word, the name's length, the name and the count of rules.
    out.extend(n.saturating_mul(2).saturating_add(6).to_le_bytes());
    out.extend(DEFINITION_FLAGS.to_le_bytes());
    out.extend(n.to_le_bytes());
    units.iter().for_each(|u| out.extend(u.to_le_bytes()));
    out.extend(1u16.to_le_bytes());
    out.extend(VERSIONS);
    out.extend(RULE_SIZE.to_le_bytes());
    out.extend(flags.to_le_bytes());
    out.extend(year.to_le_bytes());
    out.extend([0u8; 14]);
    biases(r).iter().for_each(|b| out.extend(b.to_le_bytes()));
    out.extend(system_time(r.to_standard));
    out.extend(system_time(r.to_daylight));
    out
}

/// The zone properties of an appointment for `e` (a series': `recurring`); an all-day one's, of
/// this computer's zone (`owner`'s).
pub(super) fn properties(e: &CalEvent, recurring: bool, owner: &Owner) -> Vec<(Key, Value)> {
    let here;
    let e = if e.all_day && owner.zone != Zone::UTC {
        here = CalEvent { tzid: owner.zone.id(), start: owner.instant(e, e.start), ..Default::default() };
        &here
    } else {
        e
    };
    let (r, year) = rule(e);
    let id = if e.all_day || e.tzid.trim().is_empty() { "UTC".to_string() } else { e.tzid.trim().replace(['\r', '\n'], "") };
    let key = Zone::named(&id).and_then(|z| z.windows_name()).map_or_else(|| id.clone(), str::to_string);
    let display = definition(&key, &r, year, RULE_EFFECTIVE);
    let appt = |lid_, v| lid(PSETID_APPOINTMENT, lid_, v);
    let mut out = vec![
        appt(0x8233, Value::Bin(time_zone_struct(&r))),
        appt(0x8234, Value::Text(id)),
        appt(0x825E, Value::Bin(display.clone())),
        appt(0x825F, Value::Bin(display)),
    ];
    if recurring {
        out.push(appt(0x8260, Value::Bin(definition(&key, &r, year, RULE_RECUR | RULE_EFFECTIVE))));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// New York's zone as Outlook keeps it: five hours behind, an hour forward from the second
    /// Sunday of March to the first of November, both at 02:00; named the Windows way.
    #[test]
    fn a_zone_as_its_struct_and_definition() {
        let e = CalEvent { tzid: "America/New_York".into(), start: 1_790_000_000, end: 1_790_003_600, ..Default::default() };
        let (r, year) = rule(&e);
        let s = time_zone_struct(&r);
        assert_eq!(s.len(), 48);
        let int = |at: usize| i32::from_le_bytes(s[at..at + 4].try_into().unwrap());
        assert_eq!((int(0), int(4), int(8)), (300, 0, -60));
        let words = |at: usize| (0..8).map(|i| u16::from_le_bytes([s[at + 2 * i], s[at + 2 * i + 1]])).collect::<Vec<_>>();
        assert_eq!(words(14), [0, 11, 0, 1, 2, 0, 0, 0]);
        assert_eq!(words(32), [0, 3, 0, 2, 2, 0, 0, 0]);
        let d = definition("Eastern Standard Time", &r, year, RULE_EFFECTIVE);
        let key = "Eastern Standard Time".encode_utf16().count();
        assert_eq!(d.len(), 2 + 2 + 6 + key * 2 + 66);
        assert_eq!(u16::from_le_bytes([d[2], d[3]]) as usize, 6 + key * 2);
        // The import reads its key name back.
        let props = properties(&e, true, &Owner::default());
        let names: Vec<_> = props
            .iter()
            .filter_map(|(k, v)| match (k, v) {
                (Key::Lid(_, 0x825E | 0x8260), Value::Bin(b)) => Some(b.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names.len(), 2);
        assert!(props.iter().any(|(k, v)| matches!((k, v), (Key::Lid(_, 0x8234), Value::Text(t)) if t == "America/New_York")));
        // All day, or in no zone: UTC, which has no summer time.
        let (utc, _) = rule(&CalEvent { all_day: true, ..e.clone() });
        assert_eq!(biases(&utc), [0, 0, 0]);
        assert_eq!(read_back(&CalEvent { tzid: "UTC".into(), ..e.clone() }), "");
        assert_eq!(read_back(&e), "America/New_York");
    }
}
