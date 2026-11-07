//! What MailCraft writes from a long message, read to its end.

use super::*;

/// Found in review: the HTML written from a message under the cap on what's read
/// ([`crate::HTML_MAX`]) is longer than it (`&` written `&amp;`, values quoted, elements closed,
/// VML made HTML), and was cut at the cap again: the message's end was lost, its "Unsubscribe"
/// link with it, from its text and from what's shown. The message is cut at the cap, and what's
/// written from it read to its end. A quote in a value (`'"'`, written `"&quot;"`) is written
/// six times as long, so these need no more than a megabyte or two; the review's newsletters
/// were just under the cap, their `&` in text and links written `&amp;`.
#[test]
fn what_is_written_from_a_long_message_keeps_its_end() {
    let quotes = format!("<span title='{}'>x</span>", "\"".repeat(crate::HTML_MAX / 5));
    for html in [
        // What's hidden from Outlook: what's shown, and the text, written from the tree.
        format!(r#"<span style="mso-hide:all">h</span><p>Start</p>{quotes}<p>ENDWORD</p>"#),
        // Outlook's table: the text written from the tree.
        format!("<!--[if mso]><table><tr><td><![endif]-->{quotes}<p>ENDWORD</p>"),
        // Outlook's VML: its copy is longer.
        format!("<!--[if mso]>x<![endif]-->{}<p>ENDWORD</p>", "<v:rect></v:rect>".repeat(crate::HTML_MAX / 32)),
    ] {
        assert!(html.chars().count() < crate::HTML_MAX);
        let p = message(&html);
        assert!(p.text.contains("ENDWORD"), "{}: {:?}", &html[..40], &p.text[p.text.len().saturating_sub(40)..]);
        assert!(shown(&p).contains("ENDWORD"), "{}", &html[..40]);
    }
}

/// Found in review: a newsletter whose copy for Outlook is too deep, or too long, to read was
/// read as it came, but what it hides from Outlook was still hidden: its button, drawn for
/// Outlook in VML and for the others in a link hidden from Outlook, was in neither what's shown
/// nor its text, nor was a paragraph only Outlook shows, or the others' copy of it. It's read as
/// it was before Outlook's copy was read: the others' copy shown, and counted as hidden, as then.
#[test]
fn a_copy_for_outlook_too_big_to_read_keeps_the_others_copy() {
    let ghost = r#"<!--[if mso]><table role="presentation" width="600"><tr><td><![endif]-->"#;
    let vml = r##"<!--[if mso]><v:roundrect href="https://shop.example/buy" fillcolor="#1a73e8"><center>Buy now</center></v:roundrect><![endif]-->"##;
    let twin = r##"<a href="https://shop.example/buy" style="background:#1a73e8;color:#fff;mso-hide:all">Buy now</a>"##;
    let end = "<!--[if mso]></td></tr></table><![endif]-->";
    // Nested deeper than the tree reads (256).
    let deep = |body: &str| format!("{ghost}{vml}{twin}{}{body}{}{end}", "<div>".repeat(300), "</div>".repeat(300));
    let p = message(&deep("<p>Body</p>"));
    assert_eq!(shown(&p), "Buy now||Body");
    assert!(p.text.contains("Buy now") && p.text.contains("Body"), "{:?}", p.text);
    // Outlook's own paragraph and the others' copy of it: the copy shown, its words counted.
    let words = "word ".repeat(30);
    let own = format!(r#"<!--[if mso]><p>Outlook only words</p><![endif]--><p style="mso-hide:all">{words}</p>"#);
    let p = message(&deep(&own));
    assert!(shown(&p).contains("word word") && p.text.contains("word word"), "{:?}", shown(&p));
    // The paragraph's thirty, and the button's two.
    assert_eq!(p.hidden_words, 32);
    // More elements than the tree reads (200,000): its button shown, and in its text.
    let html = format!("{ghost}{vml}{twin}{}<p>ENDWORD</p>{end}", "<p>w</p>".repeat(200_001));
    let ready = crate::layout::ready(&html, &outlook_copy(&html));
    assert!(!ready.copy && !ready.hidden && ready.shown.contains(twin) && ready.text.contains(twin));
}

/// Found in review: past the elements the tree reads ([`crate::layout`]'s 200,000), the
/// paragraphs of a message with something hidden from Outlook, or Outlook's table around it, ran
/// together in its text, one word after another, read from the tree that left them out. A tree
/// that leaves elements out isn't written out again: the message is read as it came.
#[test]
fn paragraphs_past_the_elements_read_stay_apart() {
    let paragraphs = format!("{}<p>ENDWORD</p>", "<p>w</p>".repeat(200_001));
    for html in [format!(r#"<span style="mso-hide:all">h</span>{paragraphs}"#), format!("<!--[if mso]><table><tr><td><![endif]-->{paragraphs}")] {
        // Its text, as a message's is read (what's shown, read the same way, takes longer).
        let ready = crate::layout::ready(&html, &outlook_copy(&html));
        let text = crate::html::html_to_text_within(&ready.text, crate::WRITTEN_MAX, ready.hidden);
        assert!(text.ends_with("w\n\nw\n\nENDWORD"), "{}: {:?}", &html[..40], &text[text.len().saturating_sub(40)..]);
        assert!(!text.contains("ww") && !text.contains('h'), "{}", &html[..40]);
    }
}
