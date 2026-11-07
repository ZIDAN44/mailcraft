//! What's written out again from the tree: only what must be, and escaped only where it must.

use super::*;

/// Found in review: a message that hides something from Outlook was written out again from the
/// tree, to be shown and for its text: a big table's every implied `</td>` written out, every `&`
/// written `&amp;`, and a megabyte of it took twice as long to read. It's read as it came, what
/// it hides from Outlook marked where it is.
#[test]
fn a_message_hiding_something_from_outlook_is_read_as_it_came() {
    let html = format!(r#"<span style="mso-hide:all">h</span><table>{}</table><p>ENDWORD</p>"#, "<tr><td>a<td>b&c".repeat(100));
    let ready = crate::layout::ready(&html, &outlook_copy(&html));
    let marked = html.replacen("<span", &format!("<span {}", in_place::HIDDEN_MARK), 1);
    assert!(ready.hidden && ready.shown == marked && ready.text == marked, "{}", &ready.shown[..80]);
    let p = message(&html);
    assert!(!shown(&p).contains('h') && shown(&p).contains("b&c") && shown(&p).ends_with("ENDWORD"), "{:?}", shown(&p));
    assert!(!p.text.contains('h') && p.text.lines().any(|l| l.contains('a') && l.contains("b&c")), "{:?}", p.text);
    // What's hidden whatever else its style or the style sheet says; and nothing the sender marks.
    for (html, hidden) in [
        (r#"<style>.k{display:block !important}</style><div class="k" style="mso-hide:all">HideMe</div><p>Body</p>"#, true),
        (r#"<a href="https://x.example/" style="mso-hide:all;display:inline-block">HideMe</a><p>Body</p>"#, true),
        (r#"<div data-mailcraft-hidden>HideMe</div><p>Body</p>"#, false),
    ] {
        let p = message(html);
        assert_eq!(shown(&p).contains("HideMe"), !hidden, "{html}: {:?}", shown(&p));
        assert_eq!(p.text.contains("HideMe"), !hidden, "{html}: {:?}", p.text);
    }
}

/// Found in review: what's written from the tree had every `&` written `&amp;`, and a style
/// sheet's every `</style` followed by anything written `<\/style`: a long newsletter's HTML came
/// out longer, and slower to read. Only what could be read otherwise is escaped: a `&` before a
/// letter, a digit or `#`, or at the end of a text (what's left out after it may join a word to
/// it), a `<`; a closing tag in a text box only where it would close it. A style sheet loses its
/// `</`, as it did before Outlook's copy was read: a sheet of them took twice as long to read.
#[test]
fn what_is_written_is_escaped_only_where_it_must_be() {
    let table = |inner: &str| format!("<!--[if mso]><table><tr><td><![endif]-->{inner}<!--[if mso]></td></tr></table><![endif]-->");
    let html = table(r#"<p>Fish &amp; chips, a & b, R&amp;D, &#38;, &amp;copy, &lt;b&gt;, x&<span style="mso-hide:all">h</span>amp;</p>"#);
    let ready = crate::layout::ready(&html, &outlook_copy(&html));
    assert!(ready.copy && ready.text.contains("Fish & chips, a & b, R&amp;D"), "{}", ready.text);
    let p = message(&html);
    assert!(p.text.contains("Fish & chips, a & b, R&D, &, &copy, <b>, x&amp;"), "{:?}", p.text);
    let sheet = "p{color:red}</stylex".repeat(100);
    let html = format!("<!--[if mso]><style>{sheet}<<//style></style><![endif]-->Tail");
    let copy = outlook_copy(&html);
    let ready = crate::layout::ready(&html, &copy);
    let written = format!("<style>{}style></style>", sheet.replace("</", ""));
    assert!(ready.copy && ready.styled.contains(&written) && ready.styled.len() < copy.html.len(), "{}", &ready.styled[..80]);
    assert_eq!(shown(&message(&html)), "Tail");
    // A text box's text keeps its tags.
    let html = table("<textarea>TA</b> </textareax</textarea><p>Body</p>");
    let ready = crate::layout::ready(&html, &outlook_copy(&html));
    assert!(ready.copy && ready.text.contains("<textarea>TA</b> </textareax</textarea>"), "{}", ready.text);
    assert!(message(&html).text.contains("TA</b> </textareax"));
}
