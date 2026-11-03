//! A repeating appointment's pattern in an Outlook data file (PidLidAppointmentRecur, MS-OXOCAL
//! 2.2.1.44), as the `.pst` import reads it: its rule, deleted occurrences and changed ones
//! (ExceptionInfo and ExtendedException, with their 8-bit and UTF-16 texts). Each changed
//! occurrence is an event of its own that replaces one occurrence and ends after it starts, and
//! the calendar they make is written and read again. The same pattern as a repeating task's
//! (PidLidTaskRecurrence) makes a task, written as a to-do and read again. And the other way: the
//! bytes as text, a rule, a zone and changed occurrences, which the export writes as a pattern
//! (with its ExceptionInfo and ExtendedException) that this reader reads back as a series.
//!
//! The input is the first occurrence's start (eight bytes, little-endian Unix seconds), then the
//! pattern's bytes; as text, its first line the rule, its second the zone, each line after a
//! changed occurrence (which one: the line's length; moved by half an hour a line; its title).
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_model::ical;
use mailcraft_net::{pst, pst_export};

fuzz_target!(|input: &[u8]| {
    let Some((start, pattern)) = input.split_first_chunk::<8>() else { return };
    let task = pst::repeating_task(pattern, i64::from_le_bytes(*start));
    let back = ical::parse(&ical::write_calendar(&[], std::slice::from_ref(&task), 0), 0);
    assert_eq!(back.tasks.len(), 1);
    let events = pst::repeating(pattern, i64::from_le_bytes(*start));
    let Some((series, changed)) = events.split_first() else { return };
    assert!(series.recurrence_id.is_none());
    assert!(changed.len() <= 5_000, "at most the changed occurrences read");
    for e in changed {
        assert!(e.recurrence_id.is_some() && e.rrule.is_none() && e.exdates.is_empty());
        assert!(e.end >= e.start);
        assert_eq!(e.uid, series.uid);
    }
    let _ = ical::parse(&ical::write(&events, None, 0), 0);
    written(i64::from_le_bytes(*start), pattern);
});

/// The export's pattern for the series the bytes say as text, read back by the import.
fn written(start: i64, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines();
    let (rule, zone) = (lines.next().unwrap_or_default(), lines.next().unwrap_or_default());
    let moves: Vec<(usize, i64, &str)> = lines.take(8).zip(-4i64..).map(|(l, k)| (l.len(), k * 1800, l)).collect();
    let Some(pattern) = pst_export::appointment_pattern(rule, start, zone, &moves) else { return };
    let events = pst::repeating(&pattern, start);
    assert!(events.first().is_some_and(|series| series.rrule.is_some()), "{rule}");
    assert!(events.len() <= moves.len() + 1);
}
