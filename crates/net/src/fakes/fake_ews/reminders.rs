//! The fake server's reminders (issue #40): `GetReminders` and `PerformReminderAction`, which
//! Exchange 2013 brought; an older server refuses them.

use super::*;

/// What the fake's reminders are and what was done to them.
#[derive(Default)]
pub struct Reminders {
    /// The reminders listed: (item id, its series' id or empty, its start as `2026-…Z`).
    pub listed: Vec<(String, String, String)>,
    /// Actions taken: (`Dismiss` or `Snooze`, item id, the new reminder time).
    pub done: Vec<(String, String, String)>,
    /// An Exchange older than 2013: both operations are refused.
    pub before_2013: bool,
}

impl Data {
    /// Whether an operation must be refused for the version the request asked for.
    pub(super) fn refuses_version(&self, op: &str, version: &str) -> bool {
        matches!(op, "GetReminders" | "PerformReminderAction") && (self.reminders.before_2013 || !version.starts_with("Exchange2013"))
    }

    /// The answer to one of the reminders' operations, its response code inside.
    pub(super) fn reminders_answer(&mut self, op: &El) -> String {
        if op.name == "GetReminders" {
            let listed: String = self
                .reminders
                .listed
                .iter()
                .map(|(id, series, start)| {
                    let master = if series.is_empty() { String::new() } else { format!("<t:RecurringMasterItemId Id=\"{series}\" ChangeKey=\"m1\"/>") };
                    format!(
                        "<t:Reminder><t:Subject>Event</t:Subject><t:ReminderTime>{start}</t:ReminderTime><t:StartDate>{start}</t:StartDate>\
                         <t:EndDate>{start}</t:EndDate><t:ItemId Id=\"{id}\" ChangeKey=\"r1\"/>{master}<t:ReminderGroup>Calendar</t:ReminderGroup></t:Reminder>"
                    )
                })
                .collect();
            return format!("<m:ResponseCode>NoError</m:ResponseCode><m:Reminders>{listed}</m:Reminders>");
        }
        for a in op.find("ReminderItemActions").map(|l| l.kids.clone()).unwrap_or_default() {
            let id = a.child("ItemId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
            if !self.reminders.listed.iter().any(|(i, _, _)| *i == id) {
                return "<m:ResponseCode>ErrorItemNotFound</m:ResponseCode>".into();
            }
            let action = a.text_of("ActionType").unwrap_or_default().to_string();
            let time = a.text_of("NewReminderTime").unwrap_or_default().to_string();
            self.reminders.done.push((action, id.clone(), time));
            self.reminders.listed.retain(|(i, _, _)| *i != id);
        }
        "<m:ResponseCode>NoError</m:ResponseCode><m:UpdatedItemIds/>".into()
    }
}
