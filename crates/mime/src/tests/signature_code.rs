//! A signature given as HTML code (issue #11): kept as written, harmless, its text readable.

use super::*;

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn card() -> String {
    format!(
        "<table cellpadding=\"0\"><tr><td style=\"padding-right:12px\"><img src=\"data:image/png;base64,{PNG}\" width=\"80\"></td>\
         <td style=\"border-left:2px solid #c03\"><b>Riley Moreno</b><br>Producer<br><a href=\"https://studio.example\">studio.example</a></td></tr></table>"
    )
}

/// A business card's text: the photo, then the details, no table drawn in lines.
#[test]
fn a_card_reads_as_its_cells_in_turn() {
    let (text, styles, pictures) = signature_html(&card());
    assert!(!text.contains(['│', '─', '┬']), "{text:?}");
    assert_eq!(pictures.len(), 1);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.first().map(|l| l.trim()), Some(rich::PICTURE.to_string().as_str()), "{lines:?}");
    assert!(lines.contains(&"Riley Moreno") && lines.contains(&"Producer"), "{lines:?}");
    let at = text.find("Riley").map(|b| text[..b].chars().count()).unwrap();
    assert!(styles.style(at).bold, "its look stays");
}

/// Kept as written: what's in its `<body>`, without scripts, frames, forms, handlers or
/// `javascript:`, nor Outlook's conditional markers; the table and its styles stay.
#[test]
fn the_code_is_kept_harmless() {
    let page = format!(
        "<html><head><meta charset=utf-8><title>t</title><style>p{{}}</style><script>alert(1)</script></head>\
         <body onload=\"x()\"><!--[if gte mso 9]><xml>o</xml><![endif]--><![if !vml]>{}<![endif]>\
         <iframe src=\"https://evil.example\"></iframe><form><input></form>\
         <a href=\"JavaScript:steal()\" onmouseover='y()' ONCLICK=z()>me</a><script src=x></script></body></html>",
        card()
    );
    let code = signature_code(&page);
    assert!(code.starts_with("<table") && code.contains("border-left:2px solid #c03") && code.contains("data:image/png"), "{code}");
    for gone in ["<script", "alert", "<iframe", "<form", "<input", "onmouseover", "ONCLICK", "onload", "JavaScript:", "<![", "<!--", "<title", "<xml"]
    {
        assert!(!code.contains(gone), "{gone}: {code}");
    }
    assert!(code.contains("<a href=\"#steal()\">me</a>"), "{code}");
    // Plain code with no page around it, as pasted.
    assert_eq!(signature_code("  <b>Riley</b>  "), "<b>Riley</b>");
    // Endless input stops.
    assert!(signature_code(&"<div>".repeat(100_000)).len() <= SIGNATURE_CODE_MAX);
    assert_eq!(signature_code("<script>never closed"), "");
}

/// As sent: its pictures as parts of the message, under a `cid:` of the message's own.
#[test]
fn its_pictures_go_as_parts() {
    let (html, pictures) = signature_code_parts(&card(), "abc");
    assert!(html.contains("src=\"cid:signature1.abc@mailcraft\""), "{html}");
    assert!(!html.contains("data:"));
    assert_eq!(pictures.iter().map(|p| (p.cid.as_str(), p.mime.as_str())).collect::<Vec<_>>(), [("signature1.abc@mailcraft", "image/png")]);
}

/// In the code box its pictures are short names, and come back on saving.
#[test]
fn pictures_are_short_in_the_box() {
    let two = format!("{}<img src='data:image/gif;base64,R0lGODlhAQABAAAAACw='>", card());
    let (shown, pictures) = code_without_pictures(&two);
    assert!(shown.contains("src=\"picture:1\"") && shown.contains("src='picture:2'") && !shown.contains("base64"), "{shown}");
    assert_eq!(code_with_pictures(&shown, &pictures), two);
}

/// An Outlook `.htm` signature: Windows' Western characters, pictures in the folder beside it.
#[test]
fn an_outlook_signature_file_reads() {
    let bytes = b"<html><head><meta http-equiv=Content-Type content=\"text/html; charset=windows-1252\"></head>\
        <body><p>Andr\xe9 \x96 Caf\xe9</p><img src=\"Andr%E9_files/image001.png\"><img src=\"https://x.example/a.png\"><img src=\"cid:z\"></body></html>";
    let html = signature_file(bytes);
    assert!(html.contains("André – Café"), "{html}");
    let beside = pictures_beside(&html);
    assert!(beside.contains("src=\"file:Andr%E9_files/image001.png\""), "{beside}");
    assert!(beside.contains("src=\"https://x.example/a.png\"") && beside.contains("src=\"cid:z\""), "{beside}");
    assert_eq!(signature_file("\u{feff}<b>UTF-8 é</b>".as_bytes()), "<b>UTF-8 é</b>");
}
