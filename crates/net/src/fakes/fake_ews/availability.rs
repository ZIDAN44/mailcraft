//! The fake server's free/busy times (`GetUserAvailability`, issue #140): each person asked
//! about answered apart, in the order asked, outside EWS's usual response messages.

use super::*;

/// A time someone's calendar is taken, as the availability service tells it (times in UTC,
/// written as they're asked: `2026-11-03T10:00:00`).
#[derive(Clone, Default)]
pub struct FakeBusy {
    pub email: String,
    pub start: String,
    pub end: String,
    /// `Busy`, `Tentative`, `OOF`, `WorkingElsewhere`, `Free` or `NoData`.
    pub busy_type: String,
    /// Its details, given only when this is set (a calendar shared for its times alone gives none).
    pub subject: Option<String>,
    pub location: String,
    pub private: bool,
}

/// What a `GetUserAvailability` asked: the people, in order, and its window as written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AvailabilityAsk {
    pub people: Vec<String>,
    pub start: String,
    pub end: String,
    /// The time zone's offset it named, in minutes.
    pub bias: String,
}

impl Data {
    /// Someone's time taken, from `start` to `end` (`2026-11-03T10:00:00`, UTC).
    pub fn busy(&mut self, email: &str, start: &str, end: &str, busy_type: &str, subject: Option<&str>) {
        self.free_busy.push(FakeBusy {
            email: email.into(),
            start: start.into(),
            end: end.into(),
            busy_type: busy_type.into(),
            subject: subject.map(str::to_string),
            ..Default::default()
        });
    }

    /// Whether `email` has a mailbox here: the account, a shared one, someone in the directory
    /// or with times given.
    fn known(&self, email: &str) -> bool {
        let is = |a: &str| a.eq_ignore_ascii_case(email);
        is(&format!("{USER}@contoso.example"))
            || self.shared.iter().any(|s| is(&s.address))
            || self.directory.iter().any(|(_, a)| is(a))
            || self.free_busy.iter().any(|b| is(&b.email))
    }

    pub(super) fn availability(&mut self, op: &El) -> String {
        let mut asked = Vec::new();
        op.all("MailboxData", &mut asked);
        let people: Vec<String> = asked.iter().filter_map(|m| m.find("Address")).map(|a| a.text.trim().to_string()).collect();
        let window = op.find("TimeWindow");
        let at = |n: &str| window.and_then(|w| w.text_of(n)).unwrap_or("").to_string();
        let (start, end) = (at("StartTime"), at("EndTime"));
        let bias = op.child("TimeZone").and_then(|z| z.text_of("Bias")).unwrap_or("").to_string();
        self.availability_asks.push(AvailabilityAsk { people: people.clone(), start: start.clone(), end: end.clone(), bias });
        // Exchange refuses more than 100 people, and a window past 42 days, as a whole.
        let too_long = mailcraft_jmap::utc_date(&end).zip(mailcraft_jmap::utc_date(&start)).is_none_or(|(e, s)| e - s > 42 * 86_400);
        if people.len() > 100 || too_long {
            return "<FreeBusyResponseArray><FreeBusyResponse><ResponseMessage ResponseClass=\"Error\"><MessageText>refused</MessageText>\
                    <ResponseCode>ErrorInvalidRequest</ResponseCode></ResponseMessage></FreeBusyResponse></FreeBusyResponseArray>"
                .into();
        }
        let mut out = String::from("<FreeBusyResponseArray>");
        for email in &people {
            out.push_str("<FreeBusyResponse>");
            if !self.known(email) {
                out.push_str(&fail("ErrorMailRecipientNotFound", "No mailbox found."));
            } else if self.free_busy_refused.iter().any(|r| r.eq_ignore_ascii_case(email)) {
                out.push_str(&fail("ErrorNoFreeBusyAccess", "Access is denied."));
            } else {
                out.push_str("<ResponseMessage ResponseClass=\"Success\"><ResponseCode>NoError</ResponseCode></ResponseMessage>");
                out.push_str("<FreeBusyView><t:FreeBusyViewType>Detailed</t:FreeBusyViewType><t:CalendarEventArray>");
                // Those that overlap the window, as the service gives them.
                for b in self.free_busy.iter().filter(|b| b.email.eq_ignore_ascii_case(email) && b.start < end && b.end > start) {
                    out.push_str(&format!(
                        "<t:CalendarEvent><t:StartTime>{}</t:StartTime><t:EndTime>{}</t:EndTime><t:BusyType>{}</t:BusyType>",
                        b.start, b.end, b.busy_type
                    ));
                    if let Some(subject) = &b.subject {
                        out.push_str(&format!(
                            "<t:CalendarEventDetails><t:ID>00</t:ID><t:Subject>{}</t:Subject><t:Location>{}</t:Location>\
                             <t:IsMeeting>true</t:IsMeeting><t:IsPrivate>{}</t:IsPrivate></t:CalendarEventDetails>",
                            esc(subject),
                            esc(&b.location),
                            b.private
                        ));
                    }
                    out.push_str("</t:CalendarEvent>");
                }
                out.push_str("</t:CalendarEventArray>");
                if let Some((_, hours)) = self.working_hours.iter().find(|(e, _)| e.eq_ignore_ascii_case(email)) {
                    out.push_str(hours);
                }
                out.push_str("</FreeBusyView>");
            }
            out.push_str("</FreeBusyResponse>");
        }
        out.push_str("</FreeBusyResponseArray>");
        out
    }
}

/// Whether a `GetUserAvailability` request's time zone is one Exchange refuses: its two changes
/// on the same day (the same month, week and weekday).
pub(super) fn zone_refused(op: &El) -> bool {
    let Some(zone) = op.child("TimeZone") else { return false };
    let when = |name: &str| zone.child(name).map(|t| ["Month", "DayOrder", "DayOfWeek"].map(|f| t.text_of(f).unwrap_or("").trim().to_string()));
    matches!((when("StandardTime"), when("DaylightTime")), (Some(a), Some(b)) if a == b)
}

fn fail(code: &str, text: &str) -> String {
    format!("<ResponseMessage ResponseClass=\"Error\"><MessageText>{text}</MessageText><ResponseCode>{code}</ResponseCode></ResponseMessage>")
}
