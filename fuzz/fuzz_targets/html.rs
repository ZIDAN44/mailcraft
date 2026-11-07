//! An HTML body: read as Outlook reads it (its conditional comments and VML), converted to text,
//! to formatted lines and to columns (and those to text again),
//! quoted under a reply as it came, and searched for its
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
    // Read as Outlook reads it (its conditional comments and VML), as a message's body is.
    let outlook = mime::as_outlook_reads(&page);
    let _ = mime::layout::html_layout(&mime::layout::inline_css(&outlook));
    // As a message's HTML body is made ready to read: shown, its text, the text it hides.
    let _ = mime::parse(&[b"Content-Type: text/html; charset=utf-8\r\n\r\n".as_slice(), input].concat());
    let layout = mime::layout::html_layout(&page);
    // Quoted under a reply: its columns made text to edit, and its HTML as it goes out, with
    // pictures given `cid:`s by their addresses (the input's own words standing in for them).
    let _ = mime::layout::flatten(&layout);
    let words: Vec<String> = page.split_whitespace().take(8).map(str::to_string).collect();
    let given: Vec<(String, String)> = words.iter().map(|w| (w.clone(), format!("{w}@x"))).collect();
    let _ = mime::quote_html(&page, &given, &words);
    // Pasting it into a draft, and the round trip back out.
    let (text, styles) = mime::pasted_html(&page);
    let _ = rich::to_text(&text, &styles);
    let _ = rich::to_html(&text, &styles);
    // Taken as a signature, with its pictures, and kept again.
    let (text, styles, pictures) = mime::signature_html(&page);
    let _ = mime::embed_pictures(&rich::to_html(&text, &styles), &pictures);
    // Copied from older Word or Outlook: its pictures' files read (as any bytes).
    let _ = mime::local_pictures(&page, |src| Some(("image/png".to_string(), src.as_bytes().to_vec())));
    // The same input taken as MailCraft's own plain-text markup.
    let _ = rich::from_markup(&page);
    let _ = markup::to_text(&page);
    let _ = markup::to_html(&page);
});
