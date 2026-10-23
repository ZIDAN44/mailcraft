//! A contact card (an attachment, a CardDAV contact) read, written back and read again.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_model::vcard;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for c in vcard::parse(&text).iter().take(16) {
        let _ = c.display();
        let _ = c.matches("a");
        let _ = vcard::parse(&vcard::write(c));
    }
    let _ = vcard::birthday_of(&text);
    let _ = vcard::birthday_for_exchange(&text);
});
