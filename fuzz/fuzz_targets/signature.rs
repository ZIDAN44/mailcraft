//! A signature given as HTML code, or read from a file saved as a web page (issue #11): kept
//! harmless, its pictures taken out and put back, its text for the editor, and as sent.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_mime as mime;

fuzz_target!(|input: &[u8]| {
    let file = mime::signature_file(input);
    let code = mime::signature_code(&mime::pictures_beside(&file));
    let (shown, pictures) = mime::code_without_pictures(&code);
    let back = mime::code_with_pictures(&shown, &pictures);
    let _ = mime::signature_html(&back);
    let _ = mime::signature_code_parts(&back, "tag");
});
