//! A signature given as HTML code (issue #11): kept as written, harmless, its text readable.

use super::*;

fn card() -> String {
    format!(
        "<table cellpadding=\"0\"><tr><td style=\"padding-right:12px\"><img src=\"data:image/png;base64,{PNG_1X1}\" width=\"80\"></td>\
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

/// A tag full of `javascript:` (in a received message, quoted in a reply or forward) is emptied
/// in one pass: each one was found by lowering the whole tag again and moving its tail, so a
/// megabyte of them hung the reply. The bytes looked through are counted, held in proportion to
/// the message whatever the machine's load.
#[test]
fn a_tag_full_of_javascript_is_emptied_at_once() {
    let html = tag_full_of_javascript();
    let quoted = mailcraft_model::work::linear(html.len(), || quote_html(&html, &[], &[]));
    assert!(quoted.contains(&format!("<a href=\"{}\">x</a>", "#".repeat(JAVASCRIPTS))), "{}", quoted.chars().take(200).collect::<String>());
}

/// How many `javascript:` [`tag_full_of_javascript`] holds.
const JAVASCRIPTS: usize = 90_000;

/// A link whose address is a megabyte of `javascript:`.
fn tag_full_of_javascript() -> String {
    format!("<p><a href=\"{}\">x</a></p>", "JavaScript:".repeat(JAVASCRIPTS))
}

/// Handlers and scripts as a browser reads a tag, not only as written with a space before them:
/// after a quoted value with none (`"x"onerror=`), after a `/`, past a `>` in a quoted value, and
/// `javascript:` behind character references or spaces a browser drops. Each went into a reply
/// (or a signature) as it came.
#[test]
fn handlers_and_scripts_are_found_as_a_browser_reads_them() {
    for (code, gone) in [
        ("<img src=\"x\"onerror=\"alert(1)\">", "onerror"),
        ("<img/onerror=alert(1) src=x>", "onerror"),
        ("<img alt=\"a>b\" onerror=alert(1)>", "onerror"),
        ("<a href=\"java&#x09;script:alert(1)\">x</a>", "alert"),
        ("<a href=\"java&#9script:alert(1)\">x</a>", "alert"),
        ("<a href=\"  java\tscript:alert(1)\">x</a>", "alert"),
        ("<a href=\"&#106;avascript:alert(1)\">x</a>", "alert"),
        ("<a href=\"javascript&colon;alert(1)\">x</a>", "alert"),
        ("<a href=\"VBScript:msgbox(1)\">x</a>", "msgbox"),
    ] {
        let kept = signature_code(code);
        assert!(!kept.contains(gone), "{code} -> {kept}");
        let quoted = quote_html(code, &[], &[]);
        assert!(!quoted.contains(gone), "{code} quoted -> {quoted}");
    }
    // What isn't one stays as it was.
    for code in [
        "<a href=\"https://studio.example/?q=scripts\" title=\"on time\">x</a>",
        "<img src=x/onerror=y alt=\"a>b\">",
        "<p data-one=\"1\" class=onward>x</p>",
    ] {
        assert_eq!(signature_code(code), code);
    }
}

/// A tag ends where a browser ends it, so nothing a browser reads as the next tag goes out
/// inside one: an `=` no name comes before is no value's start (`<p=">`), a `<` no name follows
/// starts no tag, and what a `<style>` or `<textarea>` holds is text up to its end tag. Each hid a
/// script or a handler in what was read as one harmless tag's quoted value.
#[test]
fn nothing_hides_in_a_tag_a_browser_ends_sooner() {
    for code in [
        "<p=\"> <script>alert(1)</script><img src=x onerror=alert(1) alt=\">",
        "<p =\"x> <script>alert(1)</script><img src=x onerror=alert(1) y=\">",
        "<p a=\"b\"=\"c> <script>alert(1)</script><img src=x onerror=alert(1) d=\">",
        "<p a='b'='c> <script>alert(1)</script><img src=x onerror=alert(1) d='>",
        "</p=\"> <script>alert(1)</script><img src=x onerror=alert(1) alt=\">",
        "<3 a=\"> <script>alert(1)</script>\">",
        "</ a=\"> <script>alert(1)</script>\">",
        "<!x a=\"> <script>alert(1)</script>\">",
        "<?x a=\"> <script>alert(1)</script>\">",
        "<div><style><p a=\"</style><script>alert(1)</script>\"></style></div>",
        "<div><style></stylex><p a=\"</style><script>alert(1)</script>\"></div>",
        "<textarea><p a=\"</textarea><script>alert(1)</script>\">",
        "<xmp><p a=\"</xmp><script>alert(1)</script>\">",
        "<noscript><p a=\"</noscript><script>alert(1)</script>\">",
    ] {
        for kept in [signature_code(code), quote_html(code, &[], &[])] {
            let lower = kept.to_ascii_lowercase();
            assert!(!lower.contains("<script") && !lower.contains("alert") && !lower.contains("onerror"), "{code} -> {kept}");
        }
    }
    // What such an element holds starts no tag where a browser reads tags there (in an `<svg>`).
    for code in ["<svg><style><img src=x onerror=alert(1)></style></svg>", "<textarea></textareax><img src=x onerror=alert(1)>"] {
        let kept = signature_code(code);
        assert!(!kept.contains("<img"), "{code} -> {kept}");
    }
    // What isn't hidden stays as it was: a `<` in text, a style sheet, a value with a `>`.
    for code in ["<p>1 < 2 and a<b</p>", "<style>p > a { color: #c03 }</style><p title=\"a>b\">x</p>", "<p=\">x</p>", "<textarea>a < b</textarea>"] {
        assert_eq!(signature_code(code), code);
    }
}

/// A numbered character reference is read with every digit it has, as a browser reads it: cut
/// at eight, a `j` written with leading zeros hid a `javascript:`.
#[test]
fn a_reference_is_read_with_all_its_digits() {
    for code in [
        "<a href=\"&#00000000106;avascript:alert(1)\">x</a>",
        "<a href=\"&#x0000000006A;avascript:alert(1)\">x</a>",
        "<a href=\"&#X00000000000000000000006a;avascript:alert(1)\">x</a>",
        "<a href=\"java&#0000000000009;script&#000000000058;alert(1)\">x</a>",
        "<a href=\"vbscript&#x000000000003a;alert(1)\">x</a>",
    ] {
        for kept in [signature_code(code), quote_html(code, &[], &[])] {
            assert!(!kept.contains("alert"), "{code} -> {kept}");
        }
    }
    // A number no character has is no letter of a scheme, and stays as written.
    for code in ["<a href=\"&#99999999999999999999;avascript:x\">x</a>", "<a href=\"&#xFFFFFFFFFFFFFFFFFFFF;avascript:x\">x</a>"] {
        assert_eq!(signature_code(code), code);
    }
}

/// A 2 MB newsletter quoted with 100 pictures behind its boxes, a signature with ten pictures
/// of 1 MB made ready to send, and the tag full of `javascript:` quoted, timed. Run under the
/// heavy lock:
/// `cargo test --release -p mailcraft-mime --lib measure_pictures_written_in -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_pictures_written_in() {
    let boxes: String =
        (0..100).map(|i| format!("<table><tr><td background=\"https://pictures.example/p{i}.png\">Item {i}</td></tr></table>")).collect();
    let html = format!("{boxes}{}", "<p>Some words of a long newsletter, kept to see a reply's cost.</p>".repeat(30_000));
    let given: Vec<(String, String)> = (0..100).map(|i| (format!("https://pictures.example/p{i}.png"), format!("p{i}@x.example"))).collect();
    let t = std::time::Instant::now();
    let quoted = quote_html(&html, &given, &[]);
    println!("MEASURE quote 2 MB with 100 pictures: {} ms", t.elapsed().as_millis());
    assert!(quoted.contains("background=\"cid:p99@x.example\""));
    let b64 = "QUJD".repeat(1 << 18);
    let code: String = (0..10).map(|i| format!("<img src=\"data:image/png;base64,{b64}{i}\">")).collect();
    let t = std::time::Instant::now();
    let (sent, pictures) = signature_code_parts(&code, "tag");
    println!("MEASURE a signature of ten 1 MB pictures: {} ms", t.elapsed().as_millis());
    assert_eq!(pictures.len(), 10);
    assert!(sent.len() < 2000, "{}", sent.len());
    let html = tag_full_of_javascript();
    let t = std::time::Instant::now();
    let _ = quote_html(&html, &[], &[]);
    println!("MEASURE quote a tag of {JAVASCRIPTS} javascript: {} ms", t.elapsed().as_millis());
}

/// Pictures behind a quoted message's boxes take the `cid:` they're sent under, however their
/// address is written; one with none given, or written another way, stays as it was.
#[test]
fn pictures_behind_boxes_take_their_cids() {
    let html = "<td background=\"a.png\">1</td><td background='a.png'>2</td><div style=\"background:url(a.png)\">3</div>\
                <div style=\"background:url('b.png')\">4</div><div style='background:url(\"b.png\")'>5</div>\
                <td background=a.png>6</td><div style=\"background:url(c.png)\">7</div>";
    let given = [("a.png".to_string(), "a@x".to_string()), ("b.png".to_string(), "b@x".to_string()), ("a.png".to_string(), "other@x".to_string())];
    let quoted = quote_html(html, &given, &[]);
    for kept in
        ["background=\"cid:a@x\"", "background='cid:a@x'", "url(cid:a@x)", "url('cid:b@x')", "url(\"cid:b@x\")", "background=a.png", "url(c.png)"]
    {
        assert!(quoted.contains(kept), "{kept} in {quoted}");
    }
    assert!(!quoted.contains("other@x"), "the first given for an address: {quoted}");
}

/// A picture behind a box is found after one left open too: an opener that ran on to another's
/// closer took the ones between with it, and they kept their addresses.
#[test]
fn a_picture_behind_a_box_is_found_after_one_left_open() {
    let given = [("a.png".to_string(), "a@x".to_string())];
    let html = "<div style=\"background:url(x.png\">1</div><td background=\"a.png\">2</td><div style=\"background:url(a.png)\">3</div>";
    let quoted = quote_html(html, &given, &[]);
    assert!(quoted.contains("background=\"cid:a@x\"") && quoted.contains("url(cid:a@x)"), "{quoted}");
    let html = "<td background=\"x.png>1</td><div style='background:url(a.png)'>2</div><td background=\"a.png\">3</td>";
    let quoted = quote_html(html, &given, &[]);
    assert!(quoted.contains("url(cid:a@x)") && quoted.contains("background=\"cid:a@x\""), "{quoted}");
}

/// Boxes whose pictures are left open by the thousand are read through once: each looked for
/// its closer to the message's end.
#[test]
fn pictures_left_open_behind_boxes_are_read_once() {
    let given = [("a.png".to_string(), "a@x".to_string())];
    for open in ["<p style=\"background:url('x\">", "<p style='background:url(\"x'>", "<p style=\"background:url(x\">", "<td background='x>"] {
        let html = format!("{}<td background=\"a.png\">1</td>", open.repeat(20_000));
        let quoted = mailcraft_model::work::linear(html.len(), || quote_html(&html, &given, &[]));
        assert!(quoted.len() >= html.len() / 2, "{open}");
    }
}

/// A signature's pictures are named one way and read back the same way; a `cid:` that only
/// starts like one (another sender's `signature.png@…`) isn't taken for one.
#[test]
fn signature_cids_are_named_and_read_back() {
    let own = message_signature_cid(&signature_cid(2), "0a1b2c3d4e5f");
    assert_eq!(own, "signature2.0a1b2c3d4e5f@mailcraft");
    assert_eq!(signature_cid_tag(&own), Some("0a1b2c3d4e5f"));
    for other in [
        "signature.png@01DA2B3C.4D5E6F70",
        "signature1@mailcraft",
        "signature_logo.x@mailcraft",
        "signature1.@mailcraft",
        "signature.x@mailcraft",
        "quoted1-ab@mailcraft",
        "signature1.a@b@mailcraft",
    ] {
        assert_eq!(signature_cid_tag(other), None, "{other}");
    }
}
