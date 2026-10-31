//! Who sends a meeting's invitations and answers when its calendar's server could send them
//! itself (CalDAV scheduling, RFC 6638): MailCraft does, so each guest gets one invitation and the
//! organizer one answer, wherever they are, whatever the server's settings (Mailcow's SOGo mails
//! them, a stock SOGo mails nobody outside, Stalwart mails everyone).
//!
//! On a server that schedules, MailCraft's copy of the user's own meeting says
//! `SCHEDULE-AGENT=CLIENT` on the organizer (SOGo reads that one) and on every guest (Stalwart,
//! Nextcloud and Cyrus read those): the server then mails nobody, though SOGo still puts the
//! meeting in the calendars of its own users. A meeting changed in SOGo's web pages loses the
//! organizer's `SCHEDULE-AGENT`, and SOGo mails that change itself.
//!
//! A guest's copy MailCraft puts in the calendar itself, of a meeting the server never had (an
//! invitation from elsewhere), says so too, once answered, and carries a mark of its own
//! ([`GUEST_COPY`]): the server sends no answer for it, MailCraft does, and the user's other
//! computers know it from the mark. One not answered yet stays on this computer: SOGo answers
//! the organizer itself for an answer changed in a copy it has, whatever the copy says, though
//! for none stored new with it. An answer changed later, in a copy SOGo has, reaches the organizer
//! twice when SOGo mails (Mailcow's setting): SOGo can't be told not to, and when it doesn't
//! mail, MailCraft's is the only one.
//!
//! A meeting the server put in the calendar itself (a colleague's on the same server) is answered
//! there, and the server takes the answer to the organizer: by mail, or in the organizer's
//! calendar.

use super::*;

/// The line that marks a guest's own copy of a meeting, put in the calendar by MailCraft from
/// the invitation's mail (the server never had the meeting, so answers to it go by mail).
pub const GUEST_COPY: &str = "X-MAILCRAFT-GUEST-COPY";

/// `SCHEDULE-AGENT=CLIENT` (any case, quoted or not).
pub(super) fn schedule_agent_client(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim().trim_matches('"').eq_ignore_ascii_case("CLIENT"))
}

/// A `SCHEDULE-STATUS` saying the server delivered the message, or will (1.x pending, 2.x
/// delivered; RFC 6638 §3.2.9). Several codes may be listed.
pub(super) fn server_delivered(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim_matches('"').split(',').any(|c| matches!(c.trim().as_bytes().first(), Some(b'1' | b'2'))))
}

/// Whether a raw line is the mark of a guest's own copy.
pub(super) fn is_guest_copy_line(raw: &str) -> bool {
    let name = raw.split([':', ';']).next().unwrap_or("");
    name.trim().eq_ignore_ascii_case(GUEST_COPY)
}

/// Whether this is a guest's own copy of a meeting that MailCraft put in the calendar.
pub fn is_guest_copy(e: &CalEvent) -> bool {
    e.extra.iter().any(|l| is_guest_copy_line(l))
}

/// Mark a guest's own copy of a meeting (once).
pub fn mark_guest_copy(e: &mut CalEvent) {
    if !is_guest_copy(e) {
        e.extra.push(format!("{GUEST_COPY}:TRUE"));
    }
}
