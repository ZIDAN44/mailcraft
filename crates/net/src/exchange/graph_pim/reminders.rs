//! Reminders answered in MailCraft, told to Exchange Online (issue #40): Graph's
//! `dismissReminder` and `snoozeReminder` on the event, or on the one occurrence of a series
//! (found among the series' `instances` around its start), so Outlook doesn't show them again.

use mailcraft_model::pim::ReminderAnswer;

use super::*;

/// How far from the occurrence's start its instance is looked for: a day either way, for an
/// all-day occurrence kept as its local day.
const AROUND: i64 = 86_400;

/// Graph's date-time in a query (`2026-11-03T10:00:00Z`).
fn query_time(t: i64) -> String {
    let (y, m, d) = mailcraft_model::dates::civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

/// The event to answer: the item itself, or the series' occurrence nearest its start.
fn item(c: &Client, a: &ReminderAnswer) -> Result<String> {
    if !a.series {
        return Ok(a.href.clone());
    }
    let path = format!(
        "/me/events/{}/instances?startDateTime={}&endDateTime={}&$select=id,start&$top=10",
        seg(&a.href),
        query_time(a.occurrence.saturating_sub(AROUND)),
        query_time(a.occurrence.saturating_add(AROUND))
    );
    let v = c.get(&path)?;
    let list = v.get("value").and_then(Value::as_array).cloned().unwrap_or_default();
    list.iter()
        .filter_map(|i| Some((when(i.get("start"))?, s(i, "id"))))
        .filter(|(_, id)| !id.is_empty())
        .min_by_key(|(start, _)| start.abs_diff(a.occurrence))
        .map(|(_, id)| id)
        .ok_or_else(|| NetError::No("that occurrence isn't in the calendar any more".into()))
}

/// Tell Graph one answer.
pub(super) fn answer(c: &Client, a: &ReminderAnswer) -> Result<()> {
    let id = item(c, a)?;
    match a.snooze_until {
        None => c.post(&format!("/me/events/{}/dismissReminder", seg(&id)), &json!({})),
        Some(until) => c.post(&format!("/me/events/{}/snoozeReminder", seg(&id)), &json!({ "newReminderTime": date_time(until) })),
    }
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
