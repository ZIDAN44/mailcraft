//! The message the tests write when what it says doesn't matter (the crate's own tests and those
//! in `tests/`). Whoever takes this in has `write` in scope: the crate's module, or its import.

use super::write::NewMessage;

/// A short plain message from Ann, `subject` the one part a test picks; a test wanting more
/// changes the rest with `..message(subject)`.
pub(crate) fn message(subject: &str) -> NewMessage<'_> {
    NewMessage {
        subject,
        sender_name: "Ann",
        sender_email: "ann@example.org",
        recipients: &[],
        body: "The body.",
        html_body: None,
        message_id: "<one@example.org>",
        delivery_time: 133_000_000_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    }
}
