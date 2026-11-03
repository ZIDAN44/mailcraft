//! Reminders answered in MailCraft, told to Exchange (issue #40): EWS's `GetReminders` finds
//! the reminder of the event, or of the series' occurrence, and `PerformReminderAction`
//! dismisses or snoozes it (Exchange 2013 and later), so Outlook doesn't show it again. An older
//! server has its single events' reminders turned off instead, as dismissing one does there.

use mailcraft_model::ical::REMINDER_MAX_MINUTES;
use mailcraft_model::pim::ReminderAnswer;

use super::*;
use crate::ews::check_message;

/// How far from the occurrence's start its reminder is looked for: a day either way (an all-day
/// occurrence is kept as its local day).
const AROUND: i64 = 86_400;
/// Reminders listed at most.
const MAX_LISTED: usize = 500;

/// The reminder's item (`Id`, `ChangeKey`): the one of this event, or of the series' occurrence
/// that starts nearest the answer's, among those still to be answered.
fn reminder_item(c: &Client, a: &ReminderAnswer) -> Result<(String, String)> {
    let from = a.occurrence.saturating_sub(REMINDER_MAX_MINUTES * 60).saturating_sub(AROUND);
    let doc = c.call_2013(&format!(
        "<m:GetReminders><m:BeginTime>{}</m:BeginTime><m:EndTime>{}</m:EndTime><m:MaxItems>{MAX_LISTED}</m:MaxItems>\
         <m:ReminderType>All</m:ReminderType></m:GetReminders>",
        utc(from),
        utc(a.occurrence.saturating_add(AROUND))
    ))?;
    if let Some(m) = doc.find("GetRemindersResponse") {
        check_message(m)?;
    }
    let id_of = |r: &El, name: &str| r.child(name).and_then(|i| i.attr("Id")).unwrap_or("").to_string();
    doc.find("Reminders")
        .map(|l| l.children("Reminder").take(MAX_LISTED).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|r| if a.series { id_of(r, "RecurringMasterItemId") == a.href } else { id_of(r, "ItemId") == a.href })
        .filter_map(|r| {
            let start = r.text_of("StartDate").and_then(mailcraft_jmap::utc_date)?;
            let item = r.child("ItemId")?;
            Some((start, item.attr("Id")?.to_string(), item.attr("ChangeKey").unwrap_or("").to_string()))
        })
        .filter(|(start, _, _)| start.abs_diff(a.occurrence) <= AROUND.unsigned_abs())
        .min_by_key(|(start, _, _)| start.abs_diff(a.occurrence))
        .map(|(_, id, key)| (id, key))
        .ok_or_else(|| NetError::No("that reminder isn't on the server any more".into()))
}

/// Tell Exchange one answer.
pub(super) fn answer(c: &Client, a: &ReminderAnswer) -> Result<()> {
    let (id, key) = match reminder_item(c, a) {
        Ok(item) => item,
        // An Exchange older than 2013 doesn't know the operation: a single event's reminder is
        // turned off, as dismissing it in Outlook does; a series' waits for nothing.
        Err(NetError::Protocol(_)) if !a.series && a.snooze_until.is_none() => return reminder_off(c, &a.href),
        Err(NetError::Protocol(why)) => return Err(NetError::No(why)),
        Err(e) => return Err(e),
    };
    let (action, time) = match a.snooze_until {
        None => ("Dismiss", String::new()),
        Some(until) => ("Snooze", format!("<t:NewReminderTime>{}</t:NewReminderTime>", utc(until))),
    };
    let doc = c.call_2013(&format!(
        "<m:PerformReminderAction><m:ReminderItemActions><t:ReminderItemAction><t:ActionType>{action}</t:ActionType>\
         <t:ItemId Id=\"{}\" ChangeKey=\"{}\"/>{time}</t:ReminderItemAction></m:ReminderItemActions></m:PerformReminderAction>",
        esc(&id),
        esc(&key)
    ))?;
    match doc.find("PerformReminderActionResponse") {
        Some(m) => check_message(m),
        None => Err(NetError::Protocol("an empty answer".into())),
    }
}

/// A single event's reminder turned off.
fn reminder_off(c: &Client, href: &str) -> Result<()> {
    c.call_one(&format!(
        "<m:UpdateItem ConflictResolution=\"AlwaysOverwrite\" SendMeetingInvitationsOrCancellations=\"SendToNone\"><m:ItemChanges>\
         <t:ItemChange><t:ItemId Id=\"{}\"/><t:Updates><t:SetItemField><t:FieldURI FieldURI=\"item:ReminderIsSet\"/>\
         <t:CalendarItem><t:ReminderIsSet>false</t:ReminderIsSet></t:CalendarItem></t:SetItemField></t:Updates></t:ItemChange>\
         </m:ItemChanges></m:UpdateItem>",
        esc(href)
    ))
    .map(|_| ())
}

/// Every answer waiting: those the server took (or that no longer apply: the event is gone)
/// are said done; the rest wait for the next sync.
pub(super) fn answer_all(c: &Client, pim: &Pim, out: &mut DavSync) {
    for a in pim.reminder_answers.iter().take(mailcraft_model::pim::MAX_REMINDER_ANSWERS) {
        match answer(c, a) {
            Ok(()) | Err(NetError::No(_)) => out.answered.push((a.uid.clone(), a.occurrence)),
            Err(e) => out.errors.push(format!("a reminder: {e}")),
        }
    }
}
