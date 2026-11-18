//! EWS: colleagues' free/busy times (issue #140; `GetUserAvailability`, MS-OXWAVLS), for those who
//! share only that much of their calendar: when it's taken, and what by when they share that.

use mailcraft_model::ical::ShowAs;
use mailcraft_model::pim::free_busy::{Busy, FreeBusyAnswer};

use super::*;

/// People asked about in one request at most (the service's limit).
const PEOPLE_MAX: usize = 100;
/// Busy times kept of one person's answer at most.
const BUSY_MAX: usize = 5000;
/// The longest window the service gives (42 days) less an hour: the days asked for are local
/// ones, and a change of clocks makes one of them an hour longer.
pub const WINDOW_MAX: i64 = 42 * 86_400 - 3600;

/// The time zone a request is read in: UTC, with no change of clocks. Times then go and come
/// back in UTC (the service gives them without an offset, in the zone asked in, and the old
/// format of a zone holds one rule for its changes, wrong across years that changed it). Its two
/// changes, which shift nothing, fall in March and October: Exchange refuses a zone whose two
/// changes are on the same day ("The specified time zone isn't valid", HTTP 500; issue #140), as
/// both on January's first Sunday were.
const UTC: &str = "<t:TimeZone><t:Bias>0</t:Bias><t:StandardTime><t:Bias>0</t:Bias><t:Time>02:00:00</t:Time><t:DayOrder>1</t:DayOrder>\
<t:Month>3</t:Month><t:DayOfWeek>Sunday</t:DayOfWeek></t:StandardTime><t:DaylightTime><t:Bias>0</t:Bias><t:Time>02:00:00</t:Time>\
<t:DayOrder>1</t:DayOrder><t:Month>10</t:Month><t:DayOfWeek>Sunday</t:DayOfWeek></t:DaylightTime></t:TimeZone>";

impl Client {
    /// When `people`'s calendars are taken between `from` and `to` (Unix seconds; the window is
    /// cut to [`WINDOW_MAX`]): an answer each, in their order, with why when there's none.
    pub fn availability(&self, people: &[String], from: i64, to: i64) -> Result<Vec<FreeBusyAnswer>> {
        let to = to.min(from.saturating_add(WINDOW_MAX)).max(from);
        let when = mailcraft_model::dates::utc_date_time;
        let mut out = Vec::with_capacity(people.len());
        for chunk in people.chunks(PEOPLE_MAX) {
            let mailboxes: String = chunk
                .iter()
                .map(|e| {
                    format!(
                        "<t:MailboxData><t:Email><t:Address>{}</t:Address></t:Email><t:AttendeeType>Required</t:AttendeeType></t:MailboxData>",
                        esc(e)
                    )
                })
                .collect();
            let doc = self.call(&format!(
                "<m:GetUserAvailabilityRequest>{UTC}<m:MailboxDataArray>{mailboxes}</m:MailboxDataArray><t:FreeBusyViewOptions>\
                 <t:TimeWindow><t:StartTime>{}</t:StartTime><t:EndTime>{}</t:EndTime></t:TimeWindow>\
                 <t:MergedFreeBusyIntervalInMinutes>30</t:MergedFreeBusyIntervalInMinutes><t:RequestedView>Detailed</t:RequestedView>\
                 </t:FreeBusyViewOptions></m:GetUserAvailabilityRequest>",
                when(from),
                when(to)
            ))?;
            out.extend(answers_in(&doc, chunk));
        }
        Ok(out)
    }
}

/// The answers in a `GetUserAvailability` response for `people` (the service answers in the
/// order asked): one each, an error for anyone it gave none for.
pub fn answers_in(doc: &El, people: &[String]) -> Vec<FreeBusyAnswer> {
    let mut given = Vec::new();
    if let Some(list) = doc.find("FreeBusyResponseArray") {
        given.extend(list.children("FreeBusyResponse"));
    }
    people
        .iter()
        .enumerate()
        .map(|(i, email)| {
            let Some(r) = given.get(i) else {
                return FreeBusyAnswer { email: email.clone(), busy: Vec::new(), error: Some("the server gave no free/busy times".into()) };
            };
            let error = r.child("ResponseMessage").filter(|m| m.attr("ResponseClass") == Some("Error")).map(|m| {
                let code = m.text_of("ResponseCode").unwrap_or("");
                match code {
                    "ErrorMailRecipientNotFound" => "not found in the organisation".to_string(),
                    "ErrorNoFreeBusyAccess" => "doesn't share their free/busy time".to_string(),
                    _ => m.text_of("MessageText").unwrap_or(code).chars().take(300).collect(),
                }
            });
            let busy = if error.is_some() { Vec::new() } else { busy_in(r) };
            FreeBusyAnswer { email: email.clone(), busy, error }
        })
        .collect()
}

/// The times one person's calendar is taken, in a `FreeBusyResponse`.
fn busy_in(r: &El) -> Vec<Busy> {
    let Some(events) = r.find("CalendarEventArray") else { return Vec::new() };
    events
        .children("CalendarEvent")
        .filter_map(|e| {
            let show_as = match e.text_of("BusyType")? {
                "Busy" => ShowAs::Busy,
                "Tentative" => ShowAs::Tentative,
                "OOF" => ShowAs::OutOfOffice,
                "WorkingElsewhere" => ShowAs::WorkingElsewhere,
                // Free, or nothing known: not a time taken.
                _ => return None,
            };
            let start = mailcraft_jmap::utc_date(e.text_of("StartTime")?)?;
            let end = mailcraft_jmap::utc_date(e.text_of("EndTime")?)?.max(start);
            let details = e.child("CalendarEventDetails");
            let text = |name: &str| details.and_then(|d| d.text_of(name)).unwrap_or("").trim().chars().take(256).collect::<String>();
            Some(Busy {
                start,
                end,
                show_as,
                subject: text("Subject"),
                location: text("Location"),
                private: details.and_then(|d| d.text_of("IsPrivate")) == Some("true"),
            })
        })
        .take(BUSY_MAX)
        .collect()
}
