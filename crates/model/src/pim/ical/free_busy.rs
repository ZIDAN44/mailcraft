//! Free/busy over CalDAV (issue #242): the query a calendar server that schedules (RFC 6638 §5)
//! answers with each person's busy times, and those times read from its answer (a `VFREEBUSY`
//! reply, RFC 5545 §3.6.4): the same data Exchange's availability service gives (issue #140).

use super::*;

/// The most busy times read from one person's answer.
const MAX_BUSY: usize = 5_000;

/// The query: `METHOD:REQUEST` with a `VFREEBUSY` asking each of `people` for `from..to`, from
/// `organizer` (the account's own address, which the server checks is the user's).
pub fn free_busy_request(organizer: &str, people: &[String], from: i64, to: i64, uid: &str, now: i64) -> String {
    let mailto = |a: &str| format!("mailto:{}", a.trim().replace(['\r', '\n', ':', ';', ','], ""));
    let mut out = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//MailCraft//EN\r\nMETHOD:REQUEST\r\nBEGIN:VFREEBUSY\r\n");
    out.push_str(&format!("UID:{}\r\nDTSTAMP:{}\r\nDTSTART:{}\r\nDTEND:{}\r\n", escape(uid), stamp(now), stamp(from), stamp(to)));
    out.push_str(&format!("ORGANIZER:{}\r\n", mailto(organizer)));
    for p in people {
        out.push_str(&format!("ATTENDEE:{}\r\n", mailto(p)));
    }
    out.push_str("END:VFREEBUSY\r\nEND:VCALENDAR\r\n");
    out
}

/// The busy times a `VFREEBUSY` gives (each `FREEBUSY` line's periods, `start/end` or
/// `start/duration`, in UTC), with how each shows: `BUSY-TENTATIVE` tentative,
/// `BUSY-UNAVAILABLE` out of office, anything else busy; `FREE` left out. Junk is skipped.
pub fn free_busy_times(text: &str) -> Vec<(i64, i64, ShowAs)> {
    let mut out = Vec::new();
    for l in lines(text).into_iter().filter(|l| l.name == "FREEBUSY") {
        let show = match l.param("FBTYPE").map(|t| t.trim().to_ascii_uppercase()) {
            Some(t) if t == "FREE" => continue,
            Some(t) if t == "BUSY-TENTATIVE" => ShowAs::Tentative,
            Some(t) if t == "BUSY-UNAVAILABLE" => ShowAs::OutOfOffice,
            _ => ShowAs::Busy,
        };
        for period in l.value.split(',') {
            let Some((a, b)) = period.trim().split_once('/') else { continue };
            let Some((start, false, true)) = date_time(a) else { continue };
            let end = match date_time(b) {
                Some((end, false, true)) => end,
                _ => match duration(b) {
                    Some(d) if d > 0 => start.saturating_add(d),
                    _ => continue,
                },
            };
            if end > start && out.len() < MAX_BUSY {
                out.push((start, end, show));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_and_its_answer() {
        let q = free_busy_request(
            "riley@studio.example",
            &["zoe@studio.example".into(), "x@y\r\nATTENDEE:evil".into()],
            1_000_000_000,
            1_000_086_400,
            "fb-1",
            999_999_000,
        );
        assert!(q.contains("METHOD:REQUEST\r\nBEGIN:VFREEBUSY\r\n"));
        assert!(q.contains("DTSTART:20010909T014640Z\r\nDTEND:20010910T014640Z\r\n"), "{q}");
        assert!(q.contains("ORGANIZER:mailto:riley@studio.example\r\nATTENDEE:mailto:zoe@studio.example\r\n"));
        assert_eq!(q.lines().filter(|l| l.starts_with("ATTENDEE")).count(), 2, "an address can't add a line: {q}");
        // RFC 5545 §3.6.4's example, and the forms a server gives.
        let reply = "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nBEGIN:VFREEBUSY\r\nFREEBUSY:19970308T160000Z/PT8H30M\r\n\
FREEBUSY;FBTYPE=FREE:19970308T160000Z/PT3H,19970308T200000Z/PT1H\r\n\
FREEBUSY;FBTYPE=BUSY-TENTATIVE:19970309T090000Z/19970309T100000Z,19970310T090000Z/1997031\r\n 0T093000Z\r\n\
FREEBUSY;FBTYPE=BUSY-UNAVAILABLE:19970311T080000Z/PT1H\r\nFREEBUSY:junk/PT1H,19970312T080000/PT1H,19970312T090000Z/19970312T080000Z\r\n\
END:VFREEBUSY\r\nEND:VCALENDAR\r\n";
        let t = |s: &str| date_time(s).unwrap().0;
        assert_eq!(
            free_busy_times(reply),
            [
                (t("19970308T160000Z"), t("19970309T003000Z"), ShowAs::Busy),
                (t("19970309T090000Z"), t("19970309T100000Z"), ShowAs::Tentative),
                (t("19970310T090000Z"), t("19970310T093000Z"), ShowAs::Tentative),
                (t("19970311T080000Z"), t("19970311T090000Z"), ShowAs::OutOfOffice),
            ]
        );
        assert!(free_busy_times("").is_empty());
        let many = format!("FREEBUSY:{}\r\n", vec!["19970308T160000Z/PT1H"; 6000].join(","));
        assert_eq!(free_busy_times(&many).len(), MAX_BUSY);
    }
}
