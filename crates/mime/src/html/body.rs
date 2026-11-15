//! The text a message shows ([`crate::Parsed::text`]): from its text parts, or from its HTML
//! body as it's shown (its own, or the one in its `winmail.dat`), read for the reading pane and,
//! alone, for a summary's preview line.

use std::borrow::Cow;

use mail_parser::Message;

use super::*;
use crate::layout::Ready;

/// The HTML a message's text is read from.
#[derive(Clone, Copy)]
enum TextFrom<'h> {
    /// Its HTML body (its own, or its `winmail.dat`'s), read as it's shown.
    Body(&'h str),
    /// What stands in for one in a message without (text parts mail-parser lists among the HTML
    /// bodies), read as it came.
    StandingIn(&'h str),
}

/// The HTML a `winmail.dat` carries, read when the message has no HTML of its own (issue #18).
fn tnef_html(tnef: &TnefBody, real_html: bool) -> Option<&str> {
    match tnef {
        TnefBody::Html(h) if !real_html => Some(h.as_str()),
        _ => None,
    }
}

/// A message's HTML body, built once: its own when it has real HTML, else its `winmail.dat`'s;
/// and, without real HTML, what mail-parser lists among the HTML bodies, which stands for one
/// (its text parts).
pub(crate) fn html_bodies<'m>(m: &'m Message<'m>, real_html: bool, tnef: &'m TnefBody) -> (Option<Cow<'m, str>>, Option<Cow<'m, str>>) {
    match html_whole(m) {
        Some(h) if real_html => (Some(h), None),
        whole => (tnef_html(tnef, real_html).map(Cow::Borrowed), whole),
    }
}

/// The text a message shows, and whether it had HTML: its text parts, or its HTML body's text as
/// it's shown (`ready`: Outlook's copy, without what's hidden from Outlook), or what stands in for
/// one as it came; a `winmail.dat`'s plain text when there's none.
pub(crate) fn shown_text(
    m: &Message<'_>,
    real_html: bool,
    tnef: &TnefBody,
    html: Option<&str>,
    standing_in: Option<&str>,
    ready: Option<&Ready>,
) -> (String, bool) {
    let as_text = |from: TextFrom<'_>| match (from, ready) {
        (TextFrom::Body(_), Some(r)) => html_to_text_within(&r.text, crate::WRITTEN_MAX, r.hidden),
        (TextFrom::Body(h) | TextFrom::StandingIn(h), _) => html_to_text(h),
    };
    let from = html.map(TextFrom::Body).or(standing_in.map(TextFrom::StandingIn));
    let (text, had_html) = match (m.text_body().len() > 0, m.body_text(0), from) {
        _ if let Some(h) = tnef_html(tnef, real_html) => (as_text(TextFrom::Body(h)), true),
        // Prefer the HTML version when the text part is an auto-generated stub.
        (_, Some(t), Some(from)) if t.trim().is_empty() => (as_text(from), true),
        // Every text part (one after an attachment too), soft-wrapped lines whole again.
        (true, Some(t), _) if !m.text_body().next().is_some_and(|p| crate::is_html(&p)) => {
            (tidy_text(&plain_body(m).unwrap_or(t.into_owned())), real_html)
        }
        (_, _, Some(from)) => (as_text(from), true),
        (_, Some(t), None) => (tidy_text(&plain_body(m).unwrap_or(t.into_owned())), false),
        _ => (String::new(), false),
    };
    // Plain text Outlook wrapped, when the message has none of its own.
    let text = match tnef {
        TnefBody::Text(t) if text.trim().is_empty() => tidy_text(t),
        _ => text,
    };
    (text, had_html)
}
