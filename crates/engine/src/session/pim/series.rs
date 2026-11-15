//! A recurring event changed as a whole: its taken-out and changed occurrences go with it.

use super::*;

/// Once the series `uid` has changed from `old` to `new` (its start or its rule), each occurrence
/// taken out of it (`EXDATE`) and each changed one (`RECURRENCE-ID`) is moved to the occurrence
/// of the same number ([`ical::moved_occurrences`], issue #100): left at the old time, one taken
/// out came back, and a changed one stood beside the series' new occurrence that day. A changed
/// occurrence at its own time moves with it; one moved to a time of its own stays there. One
/// the series no longer has goes (its item too, when it has one of its own). True when the
/// server has something to learn. The series' own event is `new` here; the caller stores it.
/// Only the series' own `calendar` is changed: a colleague's copy of the meeting is theirs.
pub(super) fn carry_occurrences(p: &mut Pim, calendar: &str, uid: &str, old: &CalEvent, new: &mut CalEvent) -> bool {
    if old.rrule.is_none() && old.rdates.is_empty() {
        return false;
    }
    let exdates: Vec<i64> = new.exdates.clone();
    let mut kept: Vec<i64> = ical::moved_occurrences(old, new, &exdates).into_iter().flatten().collect();
    kept.sort_unstable();
    kept.dedup();
    let mut remote = kept != new.exdates;
    new.exdates = kept;
    let changed: Vec<usize> = p
        .events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.event.uid == uid && e.calendar == calendar && e.event.recurrence_id.is_some())
        .map(|(i, _)| i)
        .collect();
    let at: Vec<i64> = changed.iter().filter_map(|&i| p.events.get(i).and_then(|e| e.event.recurrence_id)).collect();
    let moved = ical::moved_occurrences(old, new, &at);
    let mut gone = Vec::new();
    for ((&i, was), to) in changed.iter().zip(at).zip(moved) {
        let Some(e) = p.events.get_mut(i) else { continue };
        match to {
            Some(to) if to == was => {}
            Some(to) => {
                // At its occurrence's own time, it keeps to it; moved elsewhere, it stays.
                if e.event.start == was {
                    let len = e.event.end.saturating_sub(e.event.start);
                    e.event.start = to;
                    e.event.end = to.saturating_add(len);
                }
                e.event.recurrence_id = Some(to);
                e.event.sequence = e.event.sequence.saturating_add(1);
                e.dirty |= e.calendar != LOCAL;
                remote |= e.calendar != LOCAL;
            }
            None => gone.push(i),
        }
    }
    if !gone.is_empty() {
        let master_href = p
            .events
            .iter()
            .find(|e| e.event.uid == uid && e.calendar == calendar && e.event.recurrence_id.is_none())
            .map(|e| e.href.clone())
            .unwrap_or_default();
        let mut removed: Vec<StoredEvent> = Vec::new();
        for &i in gone.iter().rev() {
            if i < p.events.len() {
                removed.push(p.events.remove(i));
            }
        }
        // One kept in the series' own item goes when the series is written again; one with an
        // item of its own is deleted there.
        let own: Vec<StoredEvent> = removed.iter().filter(|e| !e.href.is_empty() && e.href != master_href).cloned().collect();
        remote |= super::invite::delete_items(p, &own) || removed.iter().any(|e| e.calendar != LOCAL);
    }
    remote
}
