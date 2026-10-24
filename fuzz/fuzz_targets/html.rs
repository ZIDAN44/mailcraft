//! An HTML body: converted to text, to formatted lines and to columns, and searched for its
//! pictures and links (and the trackers among them, links cleaned, text it hides); plus the plain-text markup
//! MailCraft writes itself.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_mime::{self as mime, markup, rich};

fuzz_target!(|input: &[u8]| {
    let page = String::from_utf8_lossy(input);
    // The ways MailCraft reads a received HTML body.
    let formatted = mime::html_to_rich(&page);
    let _ = mime::remote_images(&formatted);
    let _ = mime::html_to_text(&page);
    let _ = mime::html_links(&page);
    let _ = mime::html_images(&page);
    for link in mime::html_links(&page) {
        let _ = mime::clean_link(&link);
    }
    let _ = mime::trackers(&page);
    let _ = mime::hidden_text(&page);
    let _ = mime::clean_link(&page);
    let _ = mime::layout::inline_css(&page);
    let _ = mime::layout::html_layout(&page);
    // Pasting it into a draft, and the round trip back out.
    let (text, styles) = mime::pasted_html(&page);
    let _ = rich::to_text(&text, &styles);
    let _ = rich::to_html(&text, &styles);
    // The same input taken as MailCraft's own plain-text markup.
    let _ = rich::from_markup(&page);
    let _ = markup::to_text(&page);
    let _ = markup::to_html(&page);
});
