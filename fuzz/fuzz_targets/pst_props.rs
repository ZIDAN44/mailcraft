//! Text in an Outlook data file's properties: an HTML body in its internet code page and an
//! 8-bit string in its message's, as the `.pst` import reads them, and the bytes in every code
//! page MailCraft knows.
//!
//! The input is a little-endian code page, a byte saying whether the bytes are an HTML body, then
//! the bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_mime as mime;
use mailcraft_net::pst;

/// The pages with a reader of their own, and ones that read as another.
const PAGES: [u32; 18] = [0, 932, 936, 949, 1200, 1201, 1251, 1252, 20127, 50220, 50225, 51932, 54936, 65000, 65001, 10007, 28606, 99_999];

fuzz_target!(|input: &[u8]| {
    let [a, b, c, d, html, bytes @ ..] = input else { return };
    let cp = u32::from_le_bytes([*a, *b, *c, *d]);
    let html = html & 1 == 1;
    let _ = pst::text_from_bytes(bytes, cp, html);
    let _ = mime::decode_code_page(bytes, cp);
    for page in PAGES {
        let _ = pst::text_from_bytes(bytes, page, html);
        let _ = mime::decode_code_page(bytes, page);
    }
});
