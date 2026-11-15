//! Exchange: on-premises through EWS (signed in with NTLM, Kerberos or Basic), Exchange Online through
//! Microsoft Graph; mail, and calendars and contacts.

pub mod ews;
pub(crate) mod ews_pim;
pub mod graph;
pub mod graph_pim;
pub mod kerberos;
pub mod ntlm;
pub(crate) mod window;

/// An automatic reply's text as the HTML both transports send it as: the text's own `&`, `<`
/// and `>` escaped (a reply saying `Back <soon>` or `R&D` isn't taken for markup), its lines
/// parted by `<br>`.
pub(crate) fn reply_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\n', "<br>")
}

/// A contact's birthday as both transports give it (`1990-07-11T22:00:00Z`) as a card's: Exchange
/// keeps it as the start of that day where it was set, in UTC (the evening before, east of
/// Greenwich), MailCraft writes noon; either way the day from just after noon the day before to
/// noon is the one. Empty when it isn't a time.
pub(crate) fn birthday(stamp: &str) -> String {
    mailcraft_jmap::utc_date(stamp.trim())
        .map(|t| mailcraft_model::dates::day_string(t.saturating_add(12 * 3600 - 1).div_euclid(86_400)))
        .and_then(|b| mailcraft_model::vcard::birthday_of(&b))
        .unwrap_or_default()
}
