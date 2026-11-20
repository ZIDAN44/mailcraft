//! Exchange Online: colleagues' free/busy times (issue #140; `getSchedule`, which the calendar
//! permission the account signs in with already covers), for those who share only that much.

use mailcraft_model::ical::ShowAs;
use mailcraft_model::pim::free_busy::{Busy, FreeBusyAnswer};

use super::*;

/// People asked about in one request at most.
const PEOPLE_MAX: usize = 20;
/// Busy times kept of one person's answer at most.
const BUSY_MAX: usize = 5000;

impl Client {
    /// When `people`'s calendars are taken between `from` and `to` (Unix seconds; the window is
    /// cut to [`crate::ews::AVAILABILITY_WINDOW_MAX`]): an answer each, in their order.
    pub fn availability(&self, people: &[String], from: i64, to: i64) -> Result<Vec<FreeBusyAnswer>> {
        let to = to.min(from.saturating_add(crate::ews::AVAILABILITY_WINDOW_MAX)).max(from);
        let when = |t: i64| json!({ "dateTime": mailcraft_model::dates::utc_date_time(t), "timeZone": "UTC" });
        let mut out = Vec::with_capacity(people.len());
        for chunk in people.chunks(PEOPLE_MAX) {
            let body = json!({ "schedules": chunk, "startTime": when(from), "endTime": when(to), "availabilityViewInterval": 30 });
            let answer = self.post("/me/calendar/getSchedule", &body)?;
            out.extend(answers_in(&answer, chunk));
        }
        Ok(out)
    }
}

/// The answers in a `getSchedule` response for `people`: one each, matched by address (in the
/// order asked when it names none), an error for anyone it gave none for.
pub fn answers_in(answer: &Value, people: &[String]) -> Vec<FreeBusyAnswer> {
    let given: Vec<&Value> = answer.get("value").and_then(Value::as_array).map(|l| l.iter().collect()).unwrap_or_default();
    people
        .iter()
        .enumerate()
        .map(|(i, email)| {
            let named = given.iter().find(|v| v.get("scheduleId").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case(email)));
            let Some(v) = named.or_else(|| given.get(i).filter(|v| v.get("scheduleId").is_none())) else {
                return FreeBusyAnswer { email: email.clone(), error: Some("the server gave no free/busy times".into()), ..Default::default() };
            };
            if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
                let text = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("");
                let why = match text("responseCode") {
                    "ErrorMailRecipientNotFound" => "not found in the organisation".to_string(),
                    "ErrorNoFreeBusyAccess" => "doesn't share their free/busy time".to_string(),
                    code => (if text("message").is_empty() { code } else { text("message") }).chars().take(300).collect(),
                };
                return FreeBusyAnswer { email: email.clone(), error: Some(why), ..Default::default() };
            }
            FreeBusyAnswer { email: email.clone(), busy: busy_in(v), error: None, work: work_in(v) }
        })
        .collect()
}

/// When one person works, in a `scheduleInformation`'s `workingHours` (issue #185): their days,
/// hours (`08:00:00.0000000`) and zone, by its name.
fn work_in(v: &Value) -> Option<mailcraft_model::pim::work_hours::WorkHours> {
    use mailcraft_model::pim::work_hours::{WorkHours, WorkPeriod, WorkZone};
    let w = v.get("workingHours")?;
    const DAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    let days = w
        .get("daysOfWeek")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|d| DAYS.iter().position(|x| x.eq_ignore_ascii_case(d)))
        .fold(0u8, |m, i| m | (1 << i));
    let minutes = |k: &str| -> Option<u16> {
        let t = w.get(k)?.as_str()?;
        let mut parts = t.split(':');
        let (h, m) = (parts.next()?.parse::<u16>().ok()?, parts.next()?.parse::<u16>().ok()?);
        (h <= 24 && m < 60).then_some(h * 60 + m)
    };
    let name = w.get("timeZone").and_then(|z| z.get("name")).and_then(Value::as_str).map(|n| n.chars().take(100).collect());
    Some(WorkHours {
        periods: vec![WorkPeriod { days, start: minutes("startTime")?, end: minutes("endTime")? }],
        zone: WorkZone { name, ..Default::default() },
    })
}

/// The times one person's calendar is taken, in a `scheduleInformation`.
fn busy_in(v: &Value) -> Vec<Busy> {
    let Some(items) = v.get("scheduleItems").and_then(Value::as_array) else { return Vec::new() };
    items
        .iter()
        .filter_map(|it| {
            let show_as = match it.get("status").and_then(Value::as_str)? {
                "busy" => ShowAs::Busy,
                "tentative" => ShowAs::Tentative,
                "oof" => ShowAs::OutOfOffice,
                "workingElsewhere" => ShowAs::WorkingElsewhere,
                // Free, or nothing known: not a time taken.
                _ => return None,
            };
            // In UTC, as asked (no `Prefer: outlook.timezone`).
            let at = |k: &str| it.get(k).and_then(|t| t.get("dateTime")).and_then(Value::as_str).and_then(mailcraft_jmap::utc_date);
            let start = at("start")?;
            let end = at("end")?.max(start);
            let text = |k: &str| it.get(k).and_then(Value::as_str).unwrap_or("").trim().chars().take(256).collect::<String>();
            Some(Busy {
                start,
                end,
                show_as,
                subject: text("subject"),
                location: text("location"),
                private: it.get("isPrivate").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .take(BUSY_MAX)
        .collect()
}
