//! A message source as it comes from a server or a file: parsed for reading, for the message
//! list and for search, and its attachments taken out.
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
        let _ = mime::with_date(raw, 0);
    }
});
