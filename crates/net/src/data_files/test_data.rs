//! What the data file tests (and the import worker's) make up the same way each time: times as
//! Outlook keeps them, a plain message, a signed message's content.

use mailcraft_pst::write::NewMessage;

use super::outlook::EPOCH_1601;

/// `unix` seconds as a FILETIME (100 ns since 1601), as an item's times are kept.
pub(crate) const fn filetime(unix: i64) -> i64 {
    (unix + EPOCH_1601) * 10_000_000
}

/// A plain message to write into a data file: from Ana, a line of text, on 1 January 2024.
pub(crate) fn plain_message<'a>(subject: &'a str, message_id: &'a str) -> NewMessage<'a> {
    NewMessage {
        subject,
        sender_name: "Ana",
        sender_email: "ana@example.com",
        recipients: &[],
        body: "Kept.",
        html_body: None,
        message_id,
        delivery_time: filetime(1_704_067_200),
        message_class: "IPM.Note",
        properties: &[],
    }
}

/// A CMS `ContentInfo` of this type (2 signed data, 3 enveloped data), its content made up.
pub(crate) fn cms(kind: u8) -> Vec<u8> {
    let mut b = vec![0x30, 0x80, 0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, kind, 0xA0, 0x80];
    b.extend([0x5A; 40]);
    b
}
