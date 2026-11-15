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

/// An automatic reply's text from the HTML a server keeps: what [`reply_html`] wrote comes back
/// exactly as it was typed (a `&amp;` typed stays `&amp;`, spaces stay), whether or not it had a
/// `<` in it; anything with other markup (set by another program, or wrapped by the server) is
/// read as HTML.
pub(crate) fn reply_text(html: &str) -> String {
    plain_reply(html).unwrap_or_else(|| mailcraft_mime::html_to_text(html)).trim().to_string()
}

/// The text [`reply_html`] made `html` from; `None` when it has a tag other than `<br>`.
fn plain_reply(html: &str) -> Option<String> {
    const BREAKS: [&str; 3] = ["<br>", "<br/>", "<br />"];
    const WRITTEN: [(&str, char); 3] = [("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>')];
    let starts = |rest: &str, word: &str| rest.get(..word.len()).is_some_and(|r| r.eq_ignore_ascii_case(word));
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(c) = rest.chars().next() {
        let (text, len) = match c {
            '<' => ('\n', BREAKS.iter().find(|b| starts(rest, b))?.len()),
            // Each reference is read once: `&amp;lt;` was `&lt;` typed, not a `<`.
            '&' => WRITTEN.iter().find(|(r, _)| starts(rest, r)).map_or(('&', 1), |(r, c)| (*c, r.len())),
            c => (c, c.len_utf8()),
        };
        out.push(text);
        rest = rest.get(len..)?;
    }
    Some(out)
}

/// Automatic replies that read as markup or not, for both transports' tests: each is saved,
/// read, saved as read and read again.
#[cfg(test)]
pub(crate) const REPLIES_AS_TYPED: [&str; 9] = [
    "R&D on Monday",
    "Back when 2 < 3",
    "Back -> Monday",
    "Write &amp; in HTML",
    "Not a &lt;tag&gt;, nor &amp;lt;",
    "Type <br> for a line",
    "Back <soon>\nR&D on Monday",
    "One\n\n  two   spaces\nthree",
    "Zurück & weg <bald>",
];

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
