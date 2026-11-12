//! The header fields a message has once (RFC 5322, 3.6): its sender, subject and date. A message
//! with two of one is malformed, and often meant to be: the first is read, as the mail servers'
//! own listings and most mail programs read it, so MailCraft shows what they show (the parser's
//! own lookup gives the last).

use mail_parser::{AddressList, Message};

/// The first `From` field's addresses.
pub(crate) fn from<'m>(m: &'m Message<'m>) -> Option<AddressList<'m>> {
    m.root().headers().all("From").next().and_then(|h| h.value().as_address())
}

/// Whether the message has more than one `From` field: nothing vouches for such a sender.
pub(crate) fn several_from(m: &Message<'_>) -> bool {
    m.root().headers().all("From").nth(1).is_some()
}

/// The first `Subject` field, trimmed.
pub(crate) fn subject(m: &Message<'_>) -> String {
    m.root().headers().all("Subject").next().and_then(|h| h.value().as_text()).unwrap_or_default().trim().to_string()
}

/// When the first `Date` field says the message was written, in Unix seconds.
///
/// A time without seconds followed by a zone's name (`04:07 EST`, RFC 5322's obsolete form, still
/// written by old mailers and scripts) is read by the parser as if in UTC; the zone is applied here.
pub(crate) fn date(m: &Message<'_>) -> Option<i64> {
    let field = m.root().headers().all("Date").next()?;
    let d = field.value().as_datetime()?;
    let at = d.to_timestamp();
    if d.tz_hour != 0 || d.tz_minute != 0 {
        return Some(at);
    }
    let hours = std::str::from_utf8(field.raw_value()).ok().and_then(named_zone).unwrap_or(0);
    Some(at.saturating_sub(hours * 3600))
}

/// The hours east of UTC of the zone a date ends with by name, when its time has no seconds
/// (with them, the parser has applied it already).
fn named_zone(value: &str) -> Option<i64> {
    // Without a comment at its end: `EST (Eastern)`.
    let value = value.trim_end();
    let value = if value.ends_with(')') { value.rsplit_once('(').map_or(value, |(v, _)| v) } else { value };
    let mut words = value.split_ascii_whitespace().rev();
    let zone = words.next()?;
    let time = words.next()?;
    if time.matches(':').count() != 1 {
        return None;
    }
    let hours = match zone.to_ascii_uppercase().as_str() {
        "EDT" => -4,
        "EST" | "CDT" => -5,
        "CST" | "MDT" => -6,
        "MST" | "PDT" => -7,
        "PST" => -8,
        _ => return None,
    };
    Some(hours)
}

/// What the top `Authentication-Results` says of the one address in the From line.
pub(crate) fn checked_sender(m: &Message<'_>) -> crate::SenderCheck {
    let Some(header) = crate::attachments::header_top(m.root(), "Authentication-Results") else { return crate::SenderCheck::Unknown };
    let Some(from) = from(m).filter(|f| f.iter().count() == 1 && !f.has_groups() && !several_from(m)) else {
        return crate::SenderCheck::Unknown;
    };
    let address = from.first().and_then(|a| a.address()).unwrap_or_default();
    crate::trackers::sender_check(header, address.rsplit_once('@').map_or("", |(_, d)| d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zone_by_name_after_a_time_without_seconds() {
        assert_eq!(named_zone("Tue, 10 Nov 26 04:07 EST (comment)"), Some(-5));
        assert_eq!(named_zone("Tue, 10 Nov 2026 04:07 pdt"), Some(-7));
        // With seconds the parser applies it; a numeric zone or UTC needs nothing.
        assert_eq!(named_zone("Tue, 10 Nov 2026 04:07:00 EST"), None);
        assert_eq!(named_zone("Tue, 10 Nov 2026 04:07 +0100"), None);
        assert_eq!(named_zone("Tue, 10 Nov 2026 04:07 GMT"), None);
        assert_eq!(named_zone(""), None);
        // 04:07 in New York is 09:07 UTC, with seconds or without.
        let at = |date: &str| crate::parse(format!("Date: {date}\r\nSubject: s\r\n\r\nx\r\n").as_bytes()).map(|p| p.date);
        let nine = at("Tue, 10 Nov 2026 09:07:00 +0000");
        assert!(nine.is_some_and(|t| t > 0));
        assert_eq!(at("Tue, 10 Nov 26 04:07 EST (comment)"), nine);
        assert_eq!(at("Tue, 10 Nov 2026 04:07:00 EST"), nine);
        assert_eq!(at("Tue, 10 Nov 2026 04:07 -0500"), nine);
    }

    #[test]
    fn the_first_of_two_is_read() {
        let raw = b"From: First Sender <first@studio.example>\r\nFrom: Second Sender <second@studio.example>\r\nSubject: first subject\r\nSubject: second subject\r\nDate: Tue, 10 Nov 2026 09:13:00 +0000\r\nDate: Wed, 11 Nov 2026 09:13:00 +0000\r\nAuthentication-Results: mx.studio.example; dmarc=pass header.from=studio.example\r\n\r\nbody\r\n";
        let p = crate::parse(raw).unwrap_or_default();
        assert_eq!((p.from.email.as_str(), p.subject.as_str()), ("first@studio.example", "first subject"));
        let s = crate::summary_from_header(raw, Default::default(), Default::default(), 0, 0);
        assert_eq!((s.from.email.as_str(), s.subject.as_str(), s.date), ("first@studio.example", "first subject", p.date));
        // Two senders: the server's word is about one of them, and it isn't known which.
        assert_eq!(p.sender_check, crate::SenderCheck::Unknown);
        let one = b"From: First Sender <first@studio.example>\r\nSubject: s\r\nAuthentication-Results: mx.studio.example; dmarc=pass header.from=studio.example\r\n\r\nbody\r\n";
        assert_ne!(crate::parse(one).unwrap_or_default().sender_check, crate::SenderCheck::Unknown);
    }
}
