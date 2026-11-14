//! A message source as it comes from a server or a file: parsed for reading, for the message
//! list and for search, and its attachments taken out; and as an export to a data file takes it.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_mime as mime;
use mailcraft_model::{Flags, MessageKey};

fuzz_target!(|raw: &[u8]| {
    let key = MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    let _ = mime::summary_from_header(raw, key.clone(), Flags::default(), size, 0);
    let _ = mime::summary_from_full(raw, key, Flags::default(), size, 0);
    let _ = mime::search_text(raw);
    let _ = mime::html_body(raw);
    if let Some(p) = mime::parse(raw) {
        for i in 0..p.attachments.len().min(8) {
            let _ = mime::attachment_data(raw, i);
        }
        // Several from one parse, as the reading pane takes a message's pictures.
        let _ = mime::attachments_data(raw, &[0, 1, 5, usize::MAX], 1 << 20, 4 << 20);
        let _ = mime::with_date(raw, 0);
    }
    // Its header block as the places that read one without a parse do: where it ends, a field,
    // and a report's type.
    let head = raw.get(..mime::header_end(raw).map_or(raw.len(), |blank| blank.start)).unwrap_or(raw);
    let _ = mime::field_span(head, "Message-ID");
    if let Some(content_type) = mime::header_field(head, "Content-Type") {
        let _ = mime::report_type(&content_type);
    }
    let _ = mime::is_text_first(raw);
    // As an export to a data file reads it: S/MIME, a report's status, attached messages.
    let _ = mailcraft_net::pst_export::message_class(raw);
});
