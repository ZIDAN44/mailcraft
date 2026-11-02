//! A repeating appointment's pattern in an Outlook data file (PidLidAppointmentRecur, MS-OXOCAL
//! 2.2.1.44), as the `.pst` import reads it: its rule, deleted occurrences and changed ones
//! (ExceptionInfo and ExtendedException, with their 8-bit and UTF-16 texts). Each changed
//! occurrence is an event of its own that replaces one occurrence and ends after it starts, and
//! the calendar they make is written and read again. The same pattern as a repeating task's
//! (PidLidTaskRecurrence) makes a task, written as a to-do and read again.
//!
//! The input is the first occurrence's start (eight bytes, little-endian Unix seconds), then the
//! pattern's bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_model::ical;
use mailcraft_net::pst;

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
});
