//! Colleagues' free/busy times from a CalDAV server that schedules (issue #242, RFC 6638 §5): a
//! `VFREEBUSY` query POSTed to the user's scheduling outbox, each person's busy times read from
//! the server's `schedule-response`, as Exchange's availability service gives them (issue #140).

use mailcraft_model::pim::free_busy::{Busy, FreeBusyAnswer};

use super::*;

/// The most answers read from one response (one per person asked).
const MAX_ANSWERS: usize = 500;

impl Dav {
    /// The user's scheduling outbox (RFC 6638 §2.1), from where their calendars are (`home`):
    /// their principal's `schedule-outbox-URL`; `None` when the server names none.
    pub fn outbox(&self, home: &str) -> Result<Option<String>> {
        let (items, at) = self.propfind(home, "0", "<d:current-user-principal/>")?;
        let Some(principal) = items.iter().find_map(|i| i.hrefs.get("current-user-principal").and_then(|h| h.first())).cloned() else {
            return Ok(None);
        };
        let principal = absolute(&at, &principal)?;
        let (items, at) = self.propfind(&principal, "0", "<c:schedule-outbox-URL/>")?;
        items.iter().find_map(|i| i.hrefs.get("schedule-outbox-url").and_then(|h| h.first())).map(|h| absolute(&at, h)).transpose()
    }

    /// When `people` are busy between `from` and `to`, asked of the `outbox` as `me` (the
    /// account's address): an answer each, with why there's none for someone the server
    /// couldn't say (not known, not shared).
    pub fn free_busy(&self, outbox: &str, me: &str, people: &[String], from: i64, to: i64, now: i64) -> Result<Vec<FreeBusyAnswer>> {
        if Url::parse(outbox).is_ok_and(|u| !u.https && !http::is_loopback(&u.host)) {
            return Err(NetError::Tls(format!("{outbox} isn't https: MailCraft won't send the calendar password there")));
        }
        let uid = format!("{:016x}@mailcraft", mailcraft_store::fnv64(&format!("{me} {from} {to} {now}")));
        let body = ical::free_busy_request(me, people, from, to, &uid, now);
        let mut req = Request {
            method: "POST".into(),
            url: outbox.into(),
            headers: vec![("Content-Type".into(), "text/calendar; charset=utf-8; method=REQUEST".into())],
            body: body.into_bytes(),
            max: MAX_BODY,
        };
        // Servers that kept the older CalDAV scheduling draft (SOGo among them) read who asks and
        // who's asked from these headers; RFC 6638's take them from the query.
        req = req.header("Originator", &format!("mailto:{me}"));
        let recipients: Vec<String> = people.iter().map(|p| format!("mailto:{}", p.trim())).collect();
        req = req.header("Recipient", &recipients.join(", "));
        let resp = self.send(&req)?;
        match resp.status {
            200..=299 => Ok(schedule_answers(&String::from_utf8_lossy(&resp.body), people)),
            401 => Err(NetError::Auth("the calendar server refused the user name or password".into())),
            s => Err(NetError::No(format!("the calendar server didn't give free/busy times (HTTP {s})"))),
        }
    }
}

/// Each person's answer in a `schedule-response` (RFC 6638 §10.1), in the order they were asked;
/// one it names no answer for is said so.
pub fn schedule_answers(xml: &str, people: &[String]) -> Vec<FreeBusyAnswer> {
    let doc = crate::xml::parse(xml);
    let mut responses = Vec::new();
    doc.all("response", &mut responses);
    let mut said: HashMap<String, FreeBusyAnswer> = HashMap::new();
    for r in responses.into_iter().take(MAX_ANSWERS) {
        // RFC 6638's `<recipient><href>mailto:…</href></recipient>`, or the address alone in it, as
        // SOGo writes it (the CalDAV scheduling draft's form; its answers were all passed over).
        let Some(recipient) = r.find("recipient") else { continue };
        let who = recipient.find("href").map_or(recipient.text.trim(), |h| h.text.trim());
        let email = who.strip_prefix("mailto:").or_else(|| who.strip_prefix("MAILTO:")).unwrap_or(who).trim().to_ascii_lowercase();
        let status = r.find("request-status").map(|s| s.text.trim()).unwrap_or_default();
        let answer = if status.starts_with("2.") {
            let data = r.find("calendar-data").map(|d| d.text.as_str()).unwrap_or_default();
            let busy =
                ical::free_busy_times(data).into_iter().map(|(start, end, show_as)| Busy { start, end, show_as, ..Default::default() }).collect();
            FreeBusyAnswer { email: email.clone(), busy, ..Default::default() }
        } else {
            // "3.7;Invalid calendar user", "3.8;No authority": the server's words after the code.
            let why = status.split_once(';').map_or(status, |(_, w)| w).trim();
            let why = if why.is_empty() { "the server gave no times for them".to_string() } else { why.chars().take(200).collect() };
            FreeBusyAnswer { email: email.clone(), error: Some(why), ..Default::default() }
        };
        said.insert(email, answer);
    }
    people
        .iter()
        .map(|p| {
            let email = p.trim().to_ascii_lowercase();
            said.remove(&email).unwrap_or_else(|| FreeBusyAnswer {
                email,
                error: Some("the server gave no times for them".into()),
                ..Default::default()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6638 §5.2's answer: busy times for one, a refusal for another, nothing for a third.
    #[test]
    fn a_schedule_response_is_read() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<C:schedule-response xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <C:response>
    <C:recipient><D:href>mailto:Zoe@studio.example</D:href></C:recipient>
    <C:request-status>2.0;Success</C:request-status>
    <C:calendar-data>BEGIN:VCALENDAR
METHOD:REPLY
BEGIN:VFREEBUSY
FREEBUSY;FBTYPE=BUSY:20261123T090000Z/20261123T100000Z
FREEBUSY;FBTYPE=BUSY-TENTATIVE:20261123T140000Z/PT30M
END:VFREEBUSY
END:VCALENDAR
</C:calendar-data>
  </C:response>
  <C:response>
    <C:recipient><D:href>mailto:nobody@studio.example</D:href></C:recipient>
    <C:request-status>3.7;Invalid calendar user</C:request-status>
  </C:response>
</C:schedule-response>"#;
        let people = ["zoe@studio.example".to_string(), "nobody@studio.example".into(), "sam@studio.example".into()];
        let a = schedule_answers(xml, &people);
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].email, "zoe@studio.example");
        let times: Vec<(i64, i64, mailcraft_model::ical::ShowAs)> = a[0].busy.iter().map(|b| (b.start, b.end, b.show_as)).collect();
        assert_eq!(
            times,
            [
                (1_795_424_400, 1_795_428_000, mailcraft_model::ical::ShowAs::Busy),
                (1_795_442_400, 1_795_444_200, mailcraft_model::ical::ShowAs::Tentative)
            ]
        );
        assert_eq!(a[0].error, None);
        assert_eq!(a[1].error.as_deref(), Some("Invalid calendar user"));
        assert_eq!(a[2].error.as_deref(), Some("the server gave no times for them"));
        // Junk: an answer each all the same.
        assert!(schedule_answers("<<not xml", &people).iter().all(|x| x.error.is_some()));
    }

    /// A real Stalwart's answer (v1.1): the times in CDATA, lines ended by LF alone.
    #[test]
    fn stalwarts_answer_is_read() {
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><A:schedule-response xmlns:D=\"DAV:\" xmlns:A=\"urn:ietf:params:xml:ns:caldav\"><A:response><A:recipient><D:href>mailto:bob@st.test</D:href></A:recipient><A:request-status>2.0;Success</A:request-status><A:calendar-data><![CDATA[BEGIN:VCALENDAR
VERSION:2.0
METHOD:REPLY
BEGIN:VFREEBUSY
DTSTART:20261123T000000Z
DTEND:20261130T000000Z
FREEBUSY;FBTYPE=BUSY-TENTATIVE:20261125T140000Z/20261125T153000Z
FREEBUSY;FBTYPE=BUSY:20261124T090000Z/20261124T100000Z
END:VFREEBUSY
END:VCALENDAR
]]></A:calendar-data></A:response><A:response><A:recipient><D:href>mailto:carol@st.test</D:href></A:recipient><A:request-status>3.8;No authority</A:request-status></A:response></A:schedule-response>";
        let a = schedule_answers(xml, &["bob@st.test".to_string(), "carol@st.test".into()]);
        let times: Vec<(i64, i64)> = a[0].busy.iter().map(|b| (b.start, b.end)).collect();
        assert_eq!(times, [(1_795_615_200, 1_795_620_600), (1_795_510_800, 1_795_514_400)]);
        assert_eq!(a[1].error.as_deref(), Some("No authority"));
    }

    /// A real SOGo's answer (5.12, Mailcow): each recipient's address with no `href` around it.
    #[test]
    fn sogos_answer_is_read() {
        let xml = "<?xml version=\"1.0\" encoding=\"utf-8\"?><n1:schedule-response xmlns:n1=\"urn:ietf:params:xml:ns:caldav\" xmlns:D=\"DAV:\"><n1:response><n1:recipient>mailto:bob@mc31.test</n1:recipient><n1:request-status>2.0;Success</n1:request-status><n1:calendar-data>BEGIN:VCALENDAR
METHOD:REPLY
BEGIN:VFREEBUSY
FREEBUSY;FBTYPE=BUSY:20261124T090000Z/20261124T100000Z
END:VFREEBUSY
END:VCALENDAR</n1:calendar-data></n1:response><n1:response><n1:recipient>mailto:nobody@mc31.test</n1:recipient><n1:request-status>3.7;Invalid Calendar User</n1:request-status></n1:response></n1:schedule-response>";
        let a = schedule_answers(xml, &["bob@mc31.test".to_string(), "nobody@mc31.test".into()]);
        assert_eq!(
            (a[0].error.as_deref(), a[0].busy.iter().map(|b| (b.start, b.end)).collect::<Vec<_>>()),
            (None, vec![(1_795_510_800, 1_795_514_400)])
        );
        assert_eq!(a[1].error.as_deref(), Some("Invalid Calendar User"));
    }
}
