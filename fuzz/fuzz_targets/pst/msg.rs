//! Outlook's message files (`.msg`): the compound file, its property streams, recipients,
//! attachments, attached messages and named properties, read into a standard message, which is
//! then read back as received mail would be.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_net::msg;

fuzz_target!(|input: &[u8]| {
    if let Ok(raw) = msg::to_mime(input) {
        let _ = mailcraft_mime::parse(&raw);
    }
});
