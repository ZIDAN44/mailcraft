//! Outlook's `winmail.dat`: read for its files and body (the attached messages in it too), its
//! compressed RTF taken out and read as HTML or text, and the same bytes as a message's
//! `winmail.dat` attachment.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_mime as mime;

fuzz_target!(|input: &[u8]| {
    let _ = mime::read_tnef(input);
    if let Some(rtf) = mime::decompress_rtf(input) {
        let _ = mime::rtf_body(&rtf);
    }
    let _ = mime::rtf_body(input);
    // As Outlook sends it: a text part and the file.
    let mut raw = b"From: a@b.example\r\nSubject: x\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\nx\r\n--B\r\nContent-Type: application/ms-tnef\r\nContent-Disposition: attachment; filename=\"winmail.dat\"\r\n\r\n".to_vec();
    raw.extend(input);
    raw.extend(b"\r\n--B--\r\n");
    if let Some(p) = mime::parse(&raw) {
        for i in 0..p.attachments.len().min(8) {
            let _ = mime::attachment_data(&raw, i);
        }
    }
});
