//! Sending what changed here: deletions, appointments made or changed, contacts and notes
//! (those two made again whole). What the server refuses is said with the sync's errors.

use super::*;

/// Delete an item (to Deleted Items). Which of a task's occurrences go is said for every item:
/// Exchange deletes no task without it ("AffectedTaskOccurrences attribute is required for Task
/// items", issue #230), and the others take it.
fn delete(c: &Client, id: &str) -> Result<()> {
    c.call_one(&format!(
        "<m:DeleteItem DeleteType=\"MoveToDeletedItems\" SendMeetingCancellations=\"SendToNone\" AffectedTaskOccurrences=\"AllOccurrences\"><m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:DeleteItem>",
        esc(id)
    ))
    .map(drop)
}

/// What was deleted here, deleted there (in someone else's calendar when it was in theirs): one
/// the server hasn't any more is deleted too. One whose id the server can't read as an id (a
/// damaged one, or one kept from another kind of account) is said once and not asked again: it
/// was refused, and said, at every sync. Any other refusal (rights, the server busy) is asked
/// again, as it may be taken later.
pub(super) fn push_deletions(c: &Client, pim: &Pim, out: &mut DavSync) {
    for t in &pim.deleted {
        match in_calendar(c, &t.calendar, || delete(c, t.href.trim_start_matches("contact:").trim_start_matches("note:").trim_start_matches("task:")))
        {
            Ok(()) => out.deleted.push(t.href.clone()),
            Err(e) if crate::ews::gone(&e) => out.deleted.push(t.href.clone()),
            Err(e) => {
                if crate::ews::no_such_id(&e) {
                    out.deleted.push(t.href.clone());
                }
                log::warn!("a deletion not sent: {e}");
                out.errors.push(e.to_string());
            }
        }
    }
}

/// The events made or changed here: made in their calendar, or changed field by field (no
/// meeting mail sent).
pub(super) fn push_events(c: &Client, pim: &Pim, out: &mut DavSync) {
    for e in pim.events.iter().filter(|e| e.dirty) {
        let fields = event_fields(&e.event);
        let owner = mailcraft_model::pim::shared_calendar_owner(&e.calendar);
        // A changed occurrence made here (issue #241): made from the series' own occurrence on
        // the server (by the series' item and its number), not a new appointment beside it. Its
        // series not on the server yet: it goes once the series is there.
        let occurrence = match e.event.recurrence_id.filter(|_| e.href.is_empty()) {
            Some(at) => match occurrence_of_series(pim, e, at) {
                Some(id) => Some(id),
                None => continue,
            },
            None => None,
        };
        let r = in_calendar(c, &e.calendar, || {
            if let Some(occurrence) = &occurrence {
                let updates = event_updates(&e.event, &fields);
                c.call_one(&format!(
                    "<m:UpdateItem ConflictResolution=\"AlwaysOverwrite\" SendMeetingInvitationsOrCancellations=\"SendToNone\"><m:ItemChanges>\
                     <t:ItemChange>{occurrence}<t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>"
                ))
                .map(|m| id_and_key(m.find("ItemId")))
            } else if e.href.is_empty() {
                let body: String = fields.iter().map(|(_, x)| x.as_str()).collect();
                c.call_one(&format!(
                    "<m:CreateItem SendMeetingInvitations=\"SendToNone\"><m:SavedItemFolderId>{}</m:SavedItemFolderId>\
                     <m:Items><t:CalendarItem>{body}</t:CalendarItem></m:Items></m:CreateItem>",
                    calendar_folder(owner)
                ))
                .map(|m| id_and_key(m.find("ItemId")))
            } else {
                let updates = event_updates(&e.event, &fields);
                c.call_one(&format!(
                    "<m:UpdateItem ConflictResolution=\"AlwaysOverwrite\" SendMeetingInvitationsOrCancellations=\"SendToNone\"><m:ItemChanges>\
                     <t:ItemChange><t:ItemId Id=\"{}\"/><t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>",
                    esc(&e.href)
                ))
                .map(|m| {
                    let (id, key) = id_and_key(m.find("ItemId"));
                    (if id.is_empty() { e.href.clone() } else { id }, key)
                })
            }
        });
        match r {
            // Someone else's event stays in their calendar.
            Ok((href, etag)) => out.pushed_events.push(StoredEvent {
                calendar: if owner.is_some() { e.calendar.clone() } else { CALENDAR.into() },
                href,
                etag,
                event: e.event.clone(),
                dirty: false,
            }),
            Err(x) => out.errors.push(format!("{}: {x}", e.event.summary)),
        }
    }
}

/// The occurrence `at` of the series `e` belongs to, as Exchange names it: the series' item and
/// the occurrence's number in it (`OccurrenceItemId`). `None` while the series has no item there.
fn occurrence_of_series(pim: &Pim, e: &StoredEvent, at: i64) -> Option<String> {
    let master = pim.events.iter().find(|m| m.event.uid == e.event.uid && m.calendar == e.calendar && m.event.recurrence_id.is_none())?;
    if master.href.is_empty() {
        return None;
    }
    let n = mailcraft_model::ical::occurrence_number(&master.event, at)?;
    Some(format!("<t:OccurrenceItemId RecurringMasterId=\"{}\" InstanceIndex=\"{n}\"/>", esc(&master.href)))
}

/// An appointment's changes, field by field. One that stopped repeating has its rule deleted; a
/// changed occurrence of a series (issue #204) has none of its own to delete, and isn't asked
/// to: the server could refuse the change for it.
pub(super) fn event_updates(e: &CalEvent, fields: &[(&str, String)]) -> String {
    let mut updates: String = fields
        .iter()
        .map(|(name, x)| {
            let uri = if matches!(*name, "Subject" | "Sensitivity" | "Body" | "Categories" | "ReminderIsSet" | "ReminderMinutesBeforeStart") {
                format!("item:{name}")
            } else {
                format!("calendar:{name}")
            };
            format!("<t:SetItemField><t:FieldURI FieldURI=\"{uri}\"/><t:CalendarItem>{x}</t:CalendarItem></t:SetItemField>")
        })
        .collect();
    if e.rrule.is_none() && e.recurrence_id.is_none() {
        updates.push_str("<t:DeleteItemField><t:FieldURI FieldURI=\"calendar:Recurrence\"/></t:DeleteItemField>");
    }
    updates
}

/// The contacts made or changed here, made again: a contact's addresses and numbers are
/// simplest replaced whole.
pub(super) fn push_cards(c: &Client, pim: &Pim, out: &mut DavSync) {
    for k in pim.cards.iter().filter(|k| k.dirty) {
        if let Some(old) = k.href.strip_prefix("contact:")
            && let Err(e) = delete(c, old)
            && !crate::ews::gone(&e)
        {
            out.errors.push(format!("{}: {e}", k.card.display()));
            continue;
        }
        let r = c.call_one(&format!(
            "<m:CreateItem><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"contacts\"/></m:SavedItemFolderId><m:Items>{}</m:Items></m:CreateItem>",
            contact_xml(&k.card)
        ));
        match r {
            Ok(m) => {
                let (id, etag) = id_and_key(m.find("ItemId"));
                out.pushed_cards.push(StoredCard { book: CONTACTS.into(), href: format!("contact:{id}"), etag, card: k.card.clone(), dirty: false });
            }
            Err(x) => out.errors.push(format!("{}: {x}", k.card.display())),
        }
    }
}

/// The notes made or changed here, made again in the Notes folder, as contacts are.
pub(super) fn push_notes(c: &Client, pim: &Pim, out: &mut DavSync) {
    for n in pim.notes.iter().filter(|n| n.dirty) {
        if let Some(old) = n.href.strip_prefix("note:")
            && let Err(e) = delete(c, old)
            && !crate::ews::gone(&e)
        {
            out.errors.push(format!("{}: {e}", n.note.title()));
            continue;
        }
        let r = c.call_one(&format!(
            "<m:CreateItem><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"notes\"/></m:SavedItemFolderId><m:Items>{}</m:Items></m:CreateItem>",
            note_xml(&n.note)
        ));
        match r {
            Ok(m) => {
                let (id, etag) = id_and_key(m.find("ItemId"));
                out.pushed_notes.push(StoredNote { href: format!("note:{id}"), etag, note: n.note.clone(), dirty: false });
            }
            Err(x) => out.errors.push(format!("{}: {x}", n.note.title())),
        }
    }
}
