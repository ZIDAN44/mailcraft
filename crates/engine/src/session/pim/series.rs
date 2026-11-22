//! A recurring event changed as a whole: its taken-out and changed occurrences go with it. And
//! one occurrence changed on its own (issue #241), as Outlook's "this occurrence": kept as a
//! changed occurrence (`RECURRENCE-ID`) of the same event, the series' others as they are.

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

/// The part of the series `uid` (in `calendar`) for its occurrence `at` on its own: the one kept
/// already, or one made now from the series (its details, at that occurrence's time, in no item
/// of its own yet: CalDAV keeps it in the series' item, Exchange makes it from the series' own).
/// Its place among the events, and whether it was made now; `None` when the series has no
/// occurrence at `at`.
pub(super) fn occurrence_part(p: &mut Pim, calendar: &str, uid: &str, at: i64) -> Option<(usize, bool)> {
    let same = |e: &StoredEvent| e.event.uid == uid && e.calendar == calendar;
    if let Some(i) = p.events.iter().position(|e| same(e) && e.event.recurrence_id == Some(at)) {
        return Some((i, false));
    }
    let master = p.events.iter().find(|e| same(e) && e.event.recurrence_id.is_none())?;
    if !ical::is_occurrence(&master.event, at) {
        return None;
    }
    let len = master.event.end.saturating_sub(master.event.start);
    let event = CalEvent {
        recurrence_id: Some(at),
        start: at,
        end: at.saturating_add(len),
        rrule: None,
        rdates: Vec::new(),
        exdates: Vec::new(),
        ..master.event.clone()
    };
    p.events.push(StoredEvent { calendar: calendar.to_string(), href: String::new(), etag: String::new(), event, dirty: false });
    Some((p.events.len() - 1, true))
}

impl Session {
    /// Change the occurrence `at` of the series `uid` on its own (issue #241): `at` is when it
    /// starts as the series has it, which names it however it's moved. It stays that occurrence
    /// of the same event (its `RECURRENCE-ID`), one occurrence; the series and its others are
    /// left as they are. An event that doesn't repeat is changed whole. Saved unchanged, nothing
    /// is kept for it.
    pub fn update_occurrence(&mut self, uid: &str, at: i64, change: impl FnOnce(&mut CalEvent)) -> CmdResult {
        let (account, calendar, repeats) = self
            .find_event(uid)
            .map(|(a, e)| (a.to_string(), e.calendar.clone(), e.event.rrule.is_some() || !e.event.rdates.is_empty()))
            .ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        if !repeats {
            return self.update_event(uid, change);
        }
        let p = self.pim_mut(&account);
        if p.calendars.iter().any(|c| c.id == calendar && c.read_only) {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        let (i, made) = occurrence_part(p, &calendar, uid, at).ok_or_else(|| CmdError::Params(format!("the series has no occurrence at {at}")))?;
        let Some(part) = p.events.get_mut(i) else { return Err(CmdError::Params(format!("no event `{uid}`"))) };
        // Only its time came (see `update_event`): nothing of it is the user's to change.
        let hidden = ical::details_hidden(&part.event);
        let old = part.event.clone();
        let mut e = old.clone();
        change(&mut e);
        // Still that one occurrence of the series, whatever the change said.
        e.uid = old.uid.clone();
        e.recurrence_id = Some(at);
        e.rrule = None;
        e.rdates.clear();
        e.exdates.clear();
        let refused =
            if hidden { Some(CmdError::Params("only this event's time can be seen: it can't be changed".into())) } else { check_event(&e).err() };
        if refused.is_some() || e == old {
            if made {
                p.events.remove(i);
            }
            return refused.map_or_else(|| Ok(serde_json::json!({"uid": uid, "occurrence": at})), Err);
        }
        // Only what's the user's own changed (its reminder, how it shows their time): the same
        // version, so the guests aren't asked again (as `update_event`).
        if !ical::same_for_guests(&old, &e) {
            e.sequence = e.sequence.saturating_add(1);
        }
        let remote = calendar != LOCAL;
        part.event = e;
        part.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid, "occurrence": at}))
    }

    /// The occurrence `at` of the event `uid` as it is: its own part when it has one, else the
    /// series at that occurrence's time (the event itself, for one that doesn't repeat and
    /// starts then). `None`: no such occurrence.
    pub fn occurrence_event(&self, uid: &str, at: i64) -> Option<CalEvent> {
        let (account, master) = self.find_event(uid)?;
        let m = &master.event;
        if m.rrule.is_none() && m.rdates.is_empty() {
            return (m.start == at).then(|| m.clone());
        }
        let own =
            self.pim.get(account)?.events.iter().find(|e| e.event.uid == uid && e.calendar == master.calendar && e.event.recurrence_id == Some(at));
        if let Some(e) = own {
            return Some(e.event.clone());
        }
        ical::is_occurrence(m, at).then(|| {
            let len = m.end.saturating_sub(m.start);
            CalEvent {
                recurrence_id: Some(at),
                start: at,
                end: at.saturating_add(len),
                rrule: None,
                rdates: Vec::new(),
                exdates: Vec::new(),
                ..m.clone()
            }
        })
    }
}
