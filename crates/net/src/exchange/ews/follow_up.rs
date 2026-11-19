//! A message's follow-up on Exchange, as Outlook keeps it (MS-OXOFLAG): flagged with a due day,
//! or marked complete. The day is kept twice: as a date (`PidLidTaskDueDate`, its midnight with
//! no time zone, as MailCraft keeps days) and as the instant the day starts where the user is
//! (`PidLidCommonEnd`); the start day the same way. EWS's own `item:Flag` would say the same,
//! but only Exchange 2013 and later know it: these properties every Exchange (and Gromox) keeps.
//!
//! A flag's reminder is kept on the message too, as Outlook keeps it: `PidLidReminderSet`, and
//! when it reminds, `PidLidReminderSignalTime` (`PidLidReminderTime` with it, which Outlook
//! shows). Dismissed, it's no longer set.

use mailcraft_model::dates::utc_stamp;
use mailcraft_model::{FOLLOW_UP_COMPLETE, FollowUpDay, Op};

use super::El;

/// PSETID_Task and PSETID_Common, where Outlook keeps a flag's days.
const TASK: &str = "00062003-0000-0000-C000-000000000046";
const COMMON: &str = "00062008-0000-0000-C000-000000000046";

fn named(set: &str, id: u32, kind: &str) -> String {
    format!("<t:ExtendedFieldURI PropertySetId=\"{set}\" PropertyId=\"{id}\" PropertyType=\"{kind}\"/>")
}

/// `PidLidTaskStartDate`, `PidLidTaskDueDate` (dates), `PidLidCommonStart`, `PidLidCommonEnd`
/// (their instants).
fn start_date() -> String {
    named(TASK, 0x8104, "SystemTime")
}
fn due_date() -> String {
    named(TASK, 0x8105, "SystemTime")
}
fn common_start() -> String {
    named(COMMON, 0x8516, "SystemTime")
}
fn common_end() -> String {
    named(COMMON, 0x8517, "SystemTime")
}
/// `PidLidReminderSet`, `PidLidReminderTime` and `PidLidReminderSignalTime`.
fn reminder_set() -> String {
    named(COMMON, 0x8503, "Boolean")
}
fn reminder_time() -> String {
    named(COMMON, 0x8502, "SystemTime")
}
fn reminder_signal() -> String {
    named(COMMON, 0x8560, "SystemTime")
}

/// The value of a named property `id` of the property set `set` (its GUID), asked for by it;
/// the server may name a set it knows (`distinguished`: Gromox and Exchange answer so).
fn named_value<'a>(it: &'a El, set: &str, distinguished: &str, id: u32) -> Option<&'a str> {
    let id = id.to_string();
    let in_set =
        |u: &El| u.attr("PropertySetId").is_some_and(|s| s.eq_ignore_ascii_case(set)) || u.attr("DistinguishedPropertySetId") == Some(distinguished);
    it.children("ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").is_some_and(|u| u.attr("PropertyId") == Some(id.as_str()) && in_set(u)))
        .and_then(|p| p.text_of("Value"))
}

/// When a message's flag reminds (Unix seconds), when it's set to: its signal time (a snooze
/// moves it), else its reminder time.
pub(super) fn reminder(it: &El) -> Option<i64> {
    if !named_value(it, COMMON, "Common", 0x8503).is_some_and(|v| v.trim().eq_ignore_ascii_case("true")) {
        return None;
    }
    let time = |id| named_value(it, COMMON, "Common", id).and_then(mailcraft_jmap::utc_date);
    time(0x8560).or_else(|| time(0x8502))
}

/// The day a message's follow-up is due (days since the epoch), when it has one. Kept as its
/// midnight with no time zone; one kept as a local midnight in UTC (as some clients write it) is
/// read as the nearest midnight.
pub(super) fn due_day(it: &El) -> Option<i64> {
    named_value(it, TASK, "Task", 0x8105).and_then(mailcraft_jmap::utc_date).map(|t| mailcraft_model::pim::task_day(t).div_euclid(86_400))
}

/// The flag status `PidTagFlagStatus` (`0x1090`): 2 flagged, 1 complete.
fn status(value: u8) -> String {
    format!(
        "<t:SetItemField><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Message><t:ExtendedProperty>\
         <t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>{value}</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>"
    )
}

fn set(uri: &str, value: &str) -> String {
    format!("<t:SetItemField>{uri}<t:Message><t:ExtendedProperty>{uri}<t:Value>{value}</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>")
}

fn delete(uri: &str) -> String {
    format!("<t:DeleteItemField>{uri}</t:DeleteItemField>")
}

/// The days of a follow-up set (a time flag, `PidTagToDoItemFlags` 1, not begun), or taken off.
fn days(due: Option<FollowUpDay>) -> String {
    match due {
        Some(d) => {
            let date = utc_stamp(d.day.saturating_mul(86_400));
            let at = utc_stamp(d.at);
            [
                set(&start_date(), &date),
                set(&due_date(), &date),
                set(&common_start(), &at),
                set(&common_end(), &at),
                set("<t:ExtendedFieldURI PropertyTag=\"0x0E2B\" PropertyType=\"Integer\"/>", "1"),
            ]
            .concat()
        }
        None => [start_date(), due_date(), common_start(), common_end()].iter().map(|u| delete(u)).collect(),
    }
}

/// A flag's reminder set to remind at `at` (both times: Outlook shows the one, reminds at the
/// other), or no longer set (dismissed).
fn reminds(at: Option<i64>) -> String {
    match at {
        Some(at) => {
            let at = utc_stamp(at);
            [set(&reminder_set(), "true"), set(&reminder_time(), &at), set(&reminder_signal(), &at)].concat()
        }
        None => [set(&reminder_set(), "false"), delete(&reminder_signal())].concat(),
    }
}

/// The changes an op makes to a message's follow-up on Exchange: its days, its reminder, or
/// marked complete/// (`FOLLOW_UP_COMPLETE` on: status 1; off: the status taken off, the flag set again after it
/// when it's flagged again). `None`: not a follow-up's.
pub(super) fn updates(op: &Op) -> Option<String> {
    match op {
        Op::SetFollowUp { due, .. } => Some(days(*due)),
        Op::SetReminder { at, .. } => Some(reminds(*at)),
        Op::SetKeyword { keyword, on: true, .. } if keyword.eq_ignore_ascii_case(FOLLOW_UP_COMPLETE) => Some(status(1)),
        Op::SetKeyword { keyword, on: false, .. } if keyword.eq_ignore_ascii_case(FOLLOW_UP_COMPLETE) => {
            Some(delete("<t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/>"))
        }
        _ => None,
    }
}

/// Against a real Exchange Web Services server on this computer (grommunio in Docker; never a
/// real account), run by hand: a message given a day, marked complete, and its day taken off,
/// each read back as a sync reads a message's flag.
///
/// ```text
/// MAILCRAFT_REAL_EWS="http://127.0.0.1:18080/EWS/Exchange.asmx user@domain password" \
///   cargo test -p mailcraft-net real_ews_follow_up -- --ignored --nocapture
/// ```
#[cfg(test)]
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_follow_up() {
    let Some((url, user, password)) = std::env::var("MAILCRAFT_REAL_EWS").ok().and_then(|v| {
        let mut p = v.split_whitespace().map(str::to_string);
        Some((p.next()?, p.next()?, p.next()?))
    }) else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let c = super::Client::connect(&url, &user, &password, &user).unwrap();
    let made = c
        .call_one(
            "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"inbox\"/></m:SavedItemFolderId>\
             <m:Items><t:Message><t:Subject>MailCraft follow-up test</t:Subject><t:Body BodyType=\"Text\">x</t:Body></t:Message></m:Items></m:CreateItem>",
        )
        .unwrap();
    let id = made.find("ItemId").and_then(|i| i.attr("Id")).unwrap().to_string();
    let change = |updates: &str| {
        c.call_one(&format!(
            "<m:UpdateItem MessageDisposition=\"SaveOnly\" ConflictResolution=\"AlwaysOverwrite\"><m:ItemChanges><t:ItemChange>\
             <t:ItemId Id=\"{id}\"/><t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>"
        ))
        .unwrap();
    };
    let read = || {
        let got = c
            .call_one(&format!(
                "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{}</t:AdditionalProperties></m:ItemShape>\
                 <m:ItemIds><t:ItemId Id=\"{id}\"/></m:ItemIds></m:GetItem>",
                super::FLAG_FIELDS
            ))
            .unwrap();
        let it = got.find("Message").cloned().unwrap_or_default();
        eprintln!("read: {it:?}");
        super::flags(&it)
    };
    let day = mailcraft_model::dates::days_from_civil(2026, 11, 25);
    // Flagged, due the 25th (it starts an hour east of UTC).
    change(&format!("{}{}", status(2), days(Some(FollowUpDay { day, at: day * 86_400 - 3600 }))));
    let f = read();
    assert!(f.flagged && f.follow_up_due == Some(day), "{f:?}");
    // Marked complete: the flag off, complete; its day kept.
    change(&status(1));
    let f = read();
    assert!(!f.flagged && f.has(FOLLOW_UP_COMPLETE) && f.follow_up_due == Some(day), "{f:?}");
    // Flagged again with no day.
    change(&format!("{}{}", status(2), days(None)));
    let f = read();
    assert!(f.flagged && !f.has(FOLLOW_UP_COMPLETE) && f.follow_up_due.is_none(), "{f:?}");
    // A reminder at nine on the 25th; snoozed to half past (its signal time alone, as Outlook
    // snoozes); dismissed.
    let at = day * 86_400 + 9 * 3600;
    change(&reminds(Some(at)));
    assert_eq!(read().reminder, Some(at));
    change(&set(&reminder_signal(), &utc_stamp(at + 1800)));
    assert_eq!(read().reminder, Some(at + 1800));
    change(&reminds(None));
    let f = read();
    assert!(f.flagged && f.reminder.is_none(), "{f:?}");
    c.call_one(&format!("<m:DeleteItem DeleteType=\"HardDelete\"><m:ItemIds><t:ItemId Id=\"{id}\"/></m:ItemIds></m:DeleteItem>")).unwrap();
}
