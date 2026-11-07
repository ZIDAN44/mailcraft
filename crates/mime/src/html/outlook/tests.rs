use super::*;
use crate::layout::{Block, Table, Width, html_layout, inline_css};

mod cap;
mod deep;
mod marks;
mod written;

/// Every table in `blocks`, those inside cells too.
fn tables(blocks: &[Block]) -> Vec<&Table> {
    let mut out = Vec::new();
    for b in blocks {
        if let Block::Table(t) = b {
            out.push(t);
            for c in t.rows.iter().flatten() {
                out.extend(tables(&c.blocks));
            }
        }
    }
    out
}

/// What Outlook answers: Office is 16, it isn't Internet Explorer, VML is false (Word's own copy
/// of a pasted picture, the picture given beside it); what can't be made out is false.
#[test]
fn conditions_are_answered_as_outlook_does() {
    for yes in [
        "mso",
        "MSO",
        " gte mso 9",
        "gte mso 16",
        "lte mso 16",
        "gt mso 15",
        "(mso)|(IE)",
        "mso | IE",
        "!IE",
        "mso & !ie",
        "!vml",
        "mso 16",
        "!(lt mso 9)",
    ] {
        assert!(outlook_reads(&yes.to_ascii_lowercase()), "{yes}");
    }
    for no in [
        "!mso",
        "IE",
        "lt mso 9",
        "lte mso 15",
        "gt mso 16",
        "mso 12",
        "!(mso)",
        "",
        "mso)",
        "(mso",
        "gte",
        "mso |",
        "chrome",
        "gte mso x",
        "gte vml 1",
    ] {
        assert!(!outlook_reads(&no.to_ascii_lowercase()), "{no}");
    }
    // Too long or too deep: false, not a stack's worth of brackets.
    assert!(!outlook_reads(&format!("{}mso{}", "(".repeat(5_000), ")".repeat(5_000))));
    assert!(!outlook_reads(&"!".repeat(64)));
    assert!(!outlook_reads(&format!("mso {}", "| mso ".repeat(100))));
}

/// The copy a newsletter gives Outlook is markup; the one it gives everyone else goes, in both
/// ways it's written; other comments, and one never finished, stay as they were.
#[test]
fn outlooks_copy_is_read_and_the_others_dropped() {
    let html = "<p>a</p><!--[if mso]><table><tr><td>for Outlook</td></tr></table><![endif]-->\
                <!--[if !mso]><!--><div>for the others</div><!--<![endif]-->\
                <![if !mso]><div>also for the others</div><![endif]>\
                <!--[if gte mso 9]><b>Office 2000 on</b><![endif]--><!-- a note --><p>b</p>";
    let read = as_outlook_reads(html);
    assert_eq!(read, "<p>a</p><table><tr><td>for Outlook</td></tr></table><b>Office 2000 on</b><!-- a note --><p>b</p>");
    // Nothing to work out: the same HTML, not a copy.
    assert!(matches!(as_outlook_reads("<p>plain</p>"), Cow::Borrowed(_)));
    // Unfinished: left as it was (a comment, hiding what a comment hides).
    let open = "<p>a</p><!--[if !mso]><!--><p>b</p>";
    assert_eq!(as_outlook_reads(open), open);
    let open = "<p>a</p><!--[if mso]><p>b</p>";
    assert_eq!(as_outlook_reads(open), open);
}

/// VML as Outlook draws it: a rounded button a link with its colour, a shape's picture behind
/// what it holds, a picture a picture; Office's settings gone. Values that could break out of
/// the attribute they go into are left out.
#[test]
fn vml_is_read_as_html() {
    let button = r##"<!--[if mso]><v:roundrect xmlns:v="urn:schemas-microsoft-com:vml" href="https://shop.example/sale" style="height:44px;v-text-anchor:middle;width:200px" arcsize="50%" stroke="f" fillcolor="#d6336c"><w:anchorlock/><center style="color:#ffffff">Shop the sale</center></v:roundrect><![endif]--><a href="https://shop.example/sale" style="background:#d6336c;mso-hide:all">Shop the sale</a>"##;
    let read = as_outlook_reads(button);
    assert!(
        read.starts_with(r##"<a href="https://shop.example/sale" style="display:inline-block;background-color:#d6336c"><w:anchorlock/><span style="color:#ffffff">Shop the sale</span></a>"##),
        "{read}"
    );
    let banner = r##"<!--[if gte mso 9]><v:rect fill="true" stroke="false" style="width:600px"><v:fill type="frame" src="https://img.example/banner.jpg" color="#1f2a44"/><v:textbox inset="0,0,0,0"><![endif]--><h1>Autumn</h1><!--[if gte mso 9]></v:textbox></v:rect><![endif]-->"##;
    let read = as_outlook_reads(banner);
    assert_eq!(
        read,
        r##"<div style="display:block;background-color:#1f2a44;background-image:url(https://img.example/banner.jpg)"><h1>Autumn</h1></div>"##
    );
    let read = as_outlook_reads(r#"<!--[if mso]><v:image src="cid:logo@x" style="width:120px"/><![endif]-->"#);
    assert_eq!(read, r#"<img src="cid:logo@x" width="120">"#);
    let read = as_outlook_reads(
        "<head><!--[if gte mso 9]><xml><o:OfficeDocumentSettings><o:PixelsPerInch>96</o:PixelsPerInch></o:OfficeDocumentSettings></xml><![endif]--></head><p>x</p>",
    );
    assert_eq!(read, "<head></head><p>x</p>", "no stray 96");
    // Quotes and brackets in a value don't get out of it.
    let read = as_outlook_reads(r#"<v:roundrect href='https://x.example/"><script>' fillcolor="red;}<b>">Go</v:roundrect>"#);
    assert_eq!(read, r#"<div style="display:block">Go</div>"#);
    // Shapes left open are closed.
    assert_eq!(as_outlook_reads(r#"<v:rect><p>x</p>"#), r#"<div style="display:block"><p>x</p></div>"#);
}

/// Hostile input: tens of thousands of comments never closed, tags never ended, deep shapes,
/// text that isn't ASCII around every marker; quick, whole, and no panic.
#[test]
fn hostile_conditionals_stay_quick() {
    let started = std::time::Instant::now();
    let many = "<!--[if mso]>é<![if !mso]>ü<v:rect a=\"".repeat(40_000);
    let _ = as_outlook_reads(&many);
    let _ = as_outlook_reads(&"<v:rect><v:fill src=\"x\">".repeat(10_000));
    let _ = as_outlook_reads(&"<xml>".repeat(50_000));
    let _ = as_outlook_reads(&format!("<!--[if {}]>x<![endif]-->", "é".repeat(500)));
    for cut in 0..60 {
        let s = "<!--[if mso]><v:roundrect href=\"h\" fillcolor=\"#fff\">é<![endif]-->";
        let _ = as_outlook_reads(s.get(..cut).unwrap_or(s));
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
}

/// Issue #47: a newsletter of columns laid side by side with `inline-block`, which gives Outlook
/// a table of the same columns (as most newsletter makers write them): read as Outlook reads it,
/// the columns sit side by side in a 600-pixel table, each 300 wide, as they do in a browser.
#[test]
fn a_newsletters_columns_come_from_its_outlook_table() {
    let html = r#"<body><div style="max-width:600px;margin:0 auto">
<!--[if mso | IE]><table role="presentation" width="600" cellpadding="0" cellspacing="0"><tr><td width="300" valign="top"><![endif]-->
<div style="display:inline-block;width:100%;max-width:300px"><h3>Free returns</h3><p>30 days.</p></div>
<!--[if mso | IE]></td><td width="300" valign="top"><![endif]-->
<div style="display:inline-block;width:100%;max-width:300px"><h3>Two-year warranty</h3><p>On every lamp.</p></div>
<!--[if mso | IE]></td></tr></table><![endif]-->
</div></body>"#;
    let blocks = html_layout(&inline_css(&as_outlook_reads(html)));
    let table = tables(&blocks).into_iter().find(|t| t.rows.first().is_some_and(|r| r.len() == 2));
    let table = table.unwrap_or_else(|| panic!("two columns side by side: {blocks:?}"));
    assert_eq!(table.width, Some(Width::Px(600)));
    assert!(table.rows[0].iter().all(|c| c.width == Some(Width::Px(300))), "{:?}", table.rows[0]);
}

/// A message's pictures from Outlook's copy (a banner behind a heading) are listed with the
/// rest, so they're held back unless pictures are allowed, as any picture is.
#[test]
fn outlooks_pictures_are_held_back_as_any() {
    let raw = "From: a@shop.example\r\nSubject: s\r\nContent-Type: text/html\r\n\r\n<table width=\"600\"><tr><td>\
               <!--[if gte mso 9]><v:rect style=\"width:600px\"><v:fill type=\"frame\" src=\"https://img.example/banner.jpg\"/><v:textbox><![endif]-->\
               <h1>Autumn</h1><!--[if gte mso 9]></v:textbox></v:rect><![endif]--></td><td>b</td></tr></table>";
    let p = crate::parse(raw.as_bytes()).unwrap_or_default();
    assert!(p.images.iter().any(|i| i == "https://img.example/banner.jpg"), "{:?}", p.images);
}

/// A message of `html`, as received.
fn message(html: &str) -> crate::Parsed {
    let raw = format!("From: a@shop.example\r\nSubject: s\r\nContent-Type: text/html\r\n\r\n{html}");
    crate::parse(raw.as_bytes()).unwrap_or_default()
}

/// Found in review: a picture pasted from Word or Outlook comes with Word's VML copy of it
/// (`[if gte vml 1]`) and the picture for everyone else (`<![if !vml]>`). Taking the VML copy and
/// dropping the other lost the picture altogether.
#[test]
fn a_picture_pasted_from_word_is_kept() {
    let p = message(
        r#"<p>Here:</p><!--[if gte vml 1]><v:shape style="width:120px"><v:imagedata src="cid:image001.png@01D"/></v:shape><![endif]--><![if !vml]><img src="cid:image001.png@01D" width="120"><![endif]><p>Thanks</p>"#,
    );
    assert_eq!(p.images, ["cid:image001.png@01D"]);
}

/// Found in review: Word writes a list's numbers between comments of its own
/// (`<!--[if !supportLists]-->1.<!--[endif]-->`, and `<![if !supportLists]>2.<![endif]>`); the
/// condition was looked for past the comment's end, and the numbers and the text after them went.
#[test]
fn words_list_numbers_stay() {
    let html =
        "<p><!--[if !supportLists]--><span>1.</span><!--[endif]-->First item</p><p><![if !supportLists]><span>2.</span><![endif]>Second item</p>";
    let read = as_outlook_reads(html);
    assert_eq!(read, "<p><!--[if !supportLists]--><span>1.</span><!--[endif]-->First item</p><p><span>2.</span>Second item</p>");
    let text = crate::html_to_text(&read);
    assert!(text.contains("1.") && text.contains("First item") && text.contains("2.") && text.contains("Second item"), "{text}");
}

/// Found in review: a tracking pixel in Outlook's copy, drawn with VML, wasn't seen (only `<img>`
/// was looked at, in the HTML as it came) and lost its size; it's a picture of its size now, and
/// counted as a tracker.
#[test]
fn a_vml_tracking_pixel_is_seen() {
    let p = message(
        r#"<table width="600"><tr><td>Hi</td><td>there</td></tr></table><!--[if mso]><v:image src="https://t.example/open.gif" style="width:1px;height:1px"/><![endif]-->"#,
    );
    assert!(p.trackers.pictures.iter().any(|t| t.contains("t.example/open.gif")), "{:?}", p.trackers);
}

/// Found in review: a VML button's words came twice in the text (its HTML copy for the others,
/// `mso-hide: all`, wasn't left out of it); once now.
#[test]
fn a_vml_buttons_words_come_once() {
    let p = message(
        r##"<p>Sale</p><!--[if mso]><v:roundrect href="https://shop.example/sale" fillcolor="#d6336c"><center style="color:#ffffff">Shop the sale</center></v:roundrect><![endif]--><a href="https://shop.example/sale" style="background:#d6336c;mso-hide:all">Shop the sale</a>"##,
    );
    let text: String = p.rich.iter().flatten().map(|s| s.text.clone()).collect();
    assert_eq!(text.matches("Shop the sale").count(), 1, "{text}");
}

/// Found in review: attributes were found by name anywhere in the tag, a quoted value's words
/// too, and an address could carry `;` into a style.
#[test]
fn attributes_are_read_as_a_browser_reads_them() {
    let read = as_outlook_reads(r#"<v:roundrect title=" href=https://evil.example/" href="https://bank.example/">Go</v:roundrect>"#);
    assert_eq!(read, r#"<a href="https://bank.example/" style="display:inline-block">Go</a>"#);
    let read = as_outlook_reads(r#"<v:rect><v:fill src="a;display:none;x:" /><p>x</p></v:rect>"#);
    assert_eq!(read, r#"<div style="display:block"><p>x</p></div>"#);
}

/// Found in review: a comment opened inside Outlook's copy and never closed (`<!-- -->K<!--`)
/// hid the rest of the message.
#[test]
fn an_unbalanced_comment_hides_nothing_after_it() {
    assert_eq!(as_outlook_reads("<!--[if mso]><!-- -->K<!--<![endif]-->tail"), "<!-- -->Ktail");
}

/// Found in review: thousands of fills each written into their shape's style as they came, and
/// a page's style sheet of thousands of `{`, took seconds to minutes at the size of a message.
#[test]
fn many_fills_and_braces_stay_quick() {
    let started = std::time::Instant::now();
    let fills = r#"<v:rect><v:fill color="red"/></v:rect>x"#.repeat(40_000);
    assert!(as_outlook_reads(&fills).contains("background-color:red"));
    let braces = format!("<body bgcolor=x><style>{}}}</style><p>a</p>", "{".repeat(200_000));
    let _ = crate::html_to_rich(&braces);
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
}

/// Found in review: an address's `&amp;` (every address with more than one parameter) has a `;`,
/// and the address was refused for it: a button's link, a shape's picture and a VML tracking
/// pixel were lost. Their references are read as a browser reads them, and written back escaped.
#[test]
fn addresses_with_parameters_are_kept() {
    let html = r##"<p>Hi</p><!--[if mso]><v:roundrect href="https://click.example.com/ls/click?upn=abc&amp;id=7&#38;s=1" fillcolor="#0a7a5a"><v:fill type="frame" src="https://img.example.com/b.png?w=600&amp;h=300"/><center style="color:#ffffff">Read more</center></v:roundrect><v:image src="https://open.example.com/o.gif?u=1&amp;m=2" style="width:1px;height:1px"/><![endif]-->"##;
    let read = as_outlook_reads(html);
    assert!(read.contains(r#"<a href="https://click.example.com/ls/click?upn=abc&amp;id=7&amp;s=1""#), "{read}");
    assert!(read.contains("background-image:url(https://img.example.com/b.png?w=600&amp;h=300)"), "{read}");
    assert!(read.contains(r#"<img src="https://open.example.com/o.gif?u=1&amp;m=2" width="1" height="1">"#), "{read}");
    let p = message(html);
    let link = p.rich.iter().flatten().find(|s| s.text.contains("Read more")).and_then(|s| s.link.clone());
    assert_eq!(link.as_deref(), Some("https://click.example.com/ls/click?upn=abc&id=7&s=1"), "{:?}", p.rich);
    assert!(p.trackers.pictures.iter().any(|t| t == "https://open.example.com/o.gif?u=1&m=2"), "{:?}", p.trackers);
    assert!(p.images.iter().any(|i| i == "https://img.example.com/b.png?w=600&h=300"), "{:?}", p.images);
    // What a reference decodes to is checked as anything else: a quote or a `;` is still refused.
    let read = as_outlook_reads(
        r#"<v:roundrect href="https://x.example/&quot; onclick=&quot;x">Go</v:roundrect><v:rect><v:fill src="a&#59;display:none"/></v:rect>"#,
    );
    assert_eq!(read, r#"<div style="display:block">Go</div><div style="display:block"></div>"#);
}

/// Words enough to be warned of, were they hidden.
const WORDS: &str = "Our winter group show is open for submissions until the end of the month and paintings prints photography sculpture ceramics textiles and short films are all welcome from members and friends alike this year";

/// Found in review: a newsletter's copy for everyone but Outlook (`mso-hide: all`) was taken for
/// text kept from the reader, and the message said it hid text. What's hidden from Outlook alone
/// counts only in a message that gives Outlook nothing in its place; text hidden in the others'
/// copy, which Outlook's replaces here, still counts.
#[test]
fn the_copy_for_the_others_is_no_hidden_text() {
    assert_eq!(WORDS.split_whitespace().count(), 34);
    let p = message(&format!(r#"<!--[if mso]><table><tr><td>{WORDS}</td></tr></table><![endif]--><div style="mso-hide:all">{WORDS}</div>"#));
    assert_eq!((p.hidden_words, p.hidden_text.as_str()), (0, ""));
    let p = message(&format!(r#"<p>Hello</p><div style="mso-hide:all">{WORDS}</div>"#));
    assert_eq!(p.hidden_words, 34);
    let p = message(&format!(r#"<!--[if mso]><p>Hello</p><![endif]--><!--[if !mso]><!--><div style="display:none">{WORDS}</div><!--<![endif]-->"#));
    assert_eq!(p.hidden_words, 34);
    let p = message(&format!(r#"<!--[if mso]><div style="display:none">{WORDS}</div><![endif]--><p>Hello</p>"#));
    assert_eq!(p.hidden_words, 34);
}

/// Found in review: a comment, script, style sheet or title opened in Outlook's copy and not
/// closed hid everything after the copy.
#[test]
fn nothing_left_open_in_outlooks_copy_hides_the_rest() {
    assert_eq!(as_outlook_reads("<!--[if mso]>X<!-- note <![endif]-->tail<p>more</p>"), "Xtail<p>more</p>");
    let p = message("<!--[if mso]>X<!-- note <![endif]-->tail<p>more</p>");
    assert!(p.text.contains("tail") && p.text.contains("more"), "{:?}", p.text);
    for open in ["<style>p{color:red}", "<script>var a = 1;", "<title>Note", "<STYLE media=\"all\">p{}", "<textarea>"] {
        let p = message(&format!("<!--[if mso]><p>X</p>{open}<![endif]--><p>tail</p><p>more</p>"));
        assert!(p.text.contains("tail") && p.text.contains("more"), "{open}: {:?}", p.text);
    }
    // What's closed stays as it was.
    assert_eq!(as_outlook_reads("<!--[if mso]><style>p{}</style><!-- a --><b>X</b><![endif]-->"), "<style>p{}</style><!-- a --><b>X</b>");
}

/// Found in review: `mso-hide` was looked for anywhere in a tag: an address with it in got
/// `display:none;` written into it, and a class named after it kept the element from being
/// hidden by its style. Only the style's own declarations count.
#[test]
fn only_the_style_hides_from_outlook() {
    let link = r#"<a href="https://x.example/?s=mso-hide:all">Link</a>"#;
    assert_eq!(as_outlook_reads(link), link);
    assert_eq!(
        as_outlook_reads(r#"<div class="mso-hide-x" style="mso-hide:all">copy</div>"#),
        r#"<div class="mso-hide-x" style="display:none;mso-hide:all">copy</div>"#
    );
    assert_eq!(
        as_outlook_reads("<td style='color:red; MSO-HIDE: ALL !important'>x</td>"),
        "<td style='display:none;color:red; MSO-HIDE: ALL !important'>x</td>"
    );
    assert_eq!(as_outlook_reads(r#"<p style="mso-hide:none">x</p>"#), r#"<p style="mso-hide:none">x</p>"#);
    // As a message reads them: the link whole and shown, the copy hidden from Outlook not.
    let p = message(r#"<p><a href="https://x.example/?s=mso-hide:all">Link</a></p><div class="mso-hide-x" style="mso-hide:all">copy</div>"#);
    let link = p.rich.iter().flatten().find(|s| s.text.contains("Link")).and_then(|s| s.link.clone());
    assert_eq!(link.as_deref(), Some("https://x.example/?s=mso-hide:all"), "{:?}", p.rich);
    let words: String = p.rich.iter().flatten().map(|s| s.text.clone()).collect();
    assert!(!words.contains("copy") && !p.text.contains("copy"), "{words:?} {:?}", p.text);
}

/// Found in review: the message's text was read from the HTML as it came, not as it's shown:
/// the others' copy, not Outlook's.
#[test]
fn the_text_is_what_is_shown() {
    let p = message(
        r#"<!--[if mso]><p>For Outlook</p><![endif]--><!--[if !mso]><!--><p>For the others</p><!--<![endif]--><a href="https://x.example/" style="mso-hide:all">Hidden copy</a>"#,
    );
    assert!(p.text.contains("For Outlook") && !p.text.contains("For the others") && !p.text.contains("Hidden copy"), "{:?}", p.text);
}

/// The words a message shows, its lines joined by `|`.
fn shown(p: &crate::Parsed) -> String {
    p.rich.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("|")
}

/// Found in review: a comment left open in Outlook's copy was cut at its last `<!--`, wherever
/// that was: in a quoted value, in a script, or in a comment ended by `--!>`, and what followed
/// was lost. Outlook's copy is read through as a browser reads it.
#[test]
fn only_a_comment_really_left_open_is_cut() {
    for (html, rich, text) in [
        (r#"<!--[if mso]><a href="https://x.example/" title="<!--">Link</a><![endif]-->Z<p>More</p>"#, "LinkZ||More", "Z"),
        (r#"<!--[if mso]><script>var s="<!--";</script>Text<![endif]-->Z"#, "TextZ", "TextZ"),
        ("<!--[if mso]>X<!-- a --!>Y<![endif]-->Z", "XYZ", "XYZ"),
        ("<!--[if mso]>X<!-->Y<![endif]-->Z", "XYZ", "XYZ"),
    ] {
        let p = message(html);
        assert_eq!(shown(&p), rich, "{html}");
        assert!(p.text.contains(text), "{html}: {:?}", p.text);
    }
}

/// Found in review: a frame's fallback, a template, or an element whose text is taken as it is
/// (`<xmp>`, `<iframe>`, `<noembed>`, `<plaintext>`) left open in Outlook's copy, or a tag or a
/// quoted value left unended, hid what followed or showed it as markup.
#[test]
fn nothing_open_at_the_end_of_outlooks_copy_hides_what_follows() {
    // Found in review too: only 16 templates were closed, and a 17th hid the rest.
    let templates = format!("{}x", "<template>".repeat(40));
    for open in
        ["<noframes>x", "<template>x", &templates, "<xmp>x", "<iframe>x", "<noembed>x", "<plaintext>x", "<style ", "<table title=\"x", "<p class=a"]
    {
        let p = message(&format!("<!--[if mso]>{open}<![endif]-->Tail<p>More</p>"));
        assert!(shown(&p).contains("Tail") && shown(&p).ends_with("More"), "{open}: {:?}", shown(&p));
        assert!(p.text.contains("Tail") && p.text.contains("More") && !p.text.contains("<p>"), "{open}: {:?}", p.text);
    }
}

/// Found in review: what's hidden from Outlook was read twice, differently: for the message
/// shown any `mso-hide: all` declaration counted, `!important` only as written so, and before
/// the style sheet's rules were written in; for its text only the first declaration. Elements
/// were shown but left out of the text, or the other way round. One test now, at one stage.
#[test]
fn the_message_and_its_text_hide_the_same() {
    let long_style = format!("color:red;{}mso-hide:all", "margin:0;".repeat(1000));
    let many: String = (0..70).map(|i| format!("a{i}=1 ")).collect();
    for (div, hidden) in [
        (r#"<div style="mso-hide:all !IMPORTANT">HideMe</div>"#.to_string(), true),
        (r#"<div style="mso-hide:all ! important">HideMe</div>"#.into(), true),
        (r#"<div style="MSO-HIDE: ALL">HideMe</div>"#.into(), true),
        (r#"<div style="mso-hide:none;mso-hide:all">HideMe</div>"#.into(), true),
        (r#"<div style="mso-hide:all;mso-hide:none">HideMe</div>"#.into(), false),
        (r#"<div style="mso-hide:all !important;mso-hide:none">HideMe</div>"#.into(), true),
        (r#"<style>.h{mso-hide:all}</style><div class="h">HideMe</div>"#.into(), true),
        (format!(r#"<div style="{long_style}">HideMe</div>"#), true),
        (format!(r#"<div {many}style="mso-hide:all">HideMe</div>"#), true),
        (r#"<div style="mso-hide:all" data-x=a<b>HideMe</div>"#.into(), true),
        (r#"<div class="mso-hide:all">HideMe</div>"#.into(), false),
    ] {
        let p = message(&format!("{div}<p>Body</p>"));
        let what = div.get(..80).unwrap_or(&div);
        assert_eq!(shown(&p).contains("HideMe"), !hidden, "{what}: {:?}", shown(&p));
        assert_eq!(p.text.contains("HideMe"), !hidden, "{what}: {:?}", p.text);
    }
}

/// Found in review: the text read without what's hidden from Outlook lost every `</` in a text
/// box (`TA</b>` became `TAb>`), and the message shown had `display:none;` written into the tags
/// a text box holds as its text.
#[test]
fn a_text_box_keeps_its_text() {
    let html = r#"<textarea><b style="mso-hide:all">TA</b></textarea><p>Body</p>"#;
    assert_eq!(as_outlook_reads(html), html);
    let p = message(html);
    assert!(p.text.contains(r#"<b style="mso-hide:all">TA</b>"#), "{:?}", p.text);
    assert!(!shown(&p).contains("display:none"), "{:?}", shown(&p));
    // Nor in a comment or a script.
    for html in [r#"<!-- <b style="mso-hide:all"> --><p>x</p>"#, r#"<script>var s='<b style="mso-hide:all">';</script><p>x</p>"#] {
        assert_eq!(as_outlook_reads(html), html);
    }
}

/// Found in review: any comment kept for Outlook made the message Outlook's own copy, so that
/// what it hides from Outlook never counted as hidden text: Office's `<xml>` settings that
/// nearly every template has, a style sheet for Outlook, or an empty `<!--[if mso]><![endif]-->`,
/// which any sender could add to turn the warning off. Only a copy with something of its own to
/// show (text, or VML) is.
#[test]
fn only_a_copy_that_shows_something_is_outlooks_own() {
    let hidden = format!(r#"<div style="mso-hide:all">{WORDS}</div><p>Body</p>"#);
    for not_own in [
        "<!--[if mso]><![endif]-->",
        "<!--[if gte mso 9]><xml><o:OfficeDocumentSettings><o:PixelsPerInch>96</o:PixelsPerInch></o:OfficeDocumentSettings></xml><![endif]-->",
        "<!--[if mso]><style>td{padding:0}</style><![endif]-->",
        r#"<!--[if mso]><table role="presentation"><tr><td width="300">&nbsp;<!-- a --><![endif]-->"#,
        "<v:rect></v:rect>",
        // A picture with nowhere to come from.
        r#"<!--[if mso]><img alt="Banner sale"><![endif]-->"#,
        r#"<!--[if mso]><img alt="Banner sale" src=""><![endif]-->"#,
        r#"<!--[if mso]><img alt="Banner sale" src=" &#32; "> <![endif]-->"#,
        "<!--[if mso]> \r\n <![endif]-->",
    ] {
        assert_eq!(message(&format!("{not_own}{hidden}")).hidden_words, 34, "{not_own}");
    }
    // Found in review: a copy for Outlook that's only a picture (a banner) wasn't its own.
    for own in [
        "<!--[if mso]><p>For Outlook</p><![endif]-->",
        r##"<!--[if mso]><v:roundrect fillcolor="#f00"/><![endif]-->"##,
        r#"<!--[if mso]><img alt="Banner sale" src="https://x.example/b.png"><![endif]-->"#,
        "<!--[if mso]><IMG SRC=cid:banner@x><![endif]-->",
    ] {
        assert_eq!(message(&format!("{own}{hidden}")).hidden_words, 0, "{own}");
    }
}

/// Found in review: a newsletter's text, read from Outlook's copy, had its columns side by side,
/// with lines of spaces between them, and lines twice as long: its replies quoted it so, and its
/// preview read it so. The tables Outlook's copy is laid out with are read as blocks, one cell
/// after another; a table of values stays a table.
#[test]
fn a_newsletters_text_reads_one_column_after_another() {
    let column = |n: u32| {
        format!(
            r#"<div style="display:inline-block;width:300px"><table role="presentation" width="100%"><tr><td><h2>Deal number {n}</h2><p>Our customers love this item and you will too. Limited stock, while it lasts, so be quick.</p></td></tr></table></div>"#
        )
    };
    let html = format!(
        r#"<!--[if mso]><table role="presentation" width="600"><tr><td width="300" valign="top"><![endif]-->{}<!--[if mso]></td><td width="300" valign="top"><![endif]-->{}<!--[if mso]></td></tr></table><![endif]--><table><tr><td>Total</td><td>$20</td></tr></table>"#,
        column(1),
        column(2)
    );
    let p = message(&html);
    assert!(!p.text.contains("          "), "{:?}", p.text);
    assert!(p.text.lines().all(|l| l.chars().count() <= 100), "{:?}", p.text);
    let (one, two) = (p.text.find("Deal number 1"), p.text.find("Deal number 2"));
    assert!(one.is_some() && one < two, "{:?}", p.text);
    assert!(p.text.find("Our customers") < two, "{:?}", p.text);
    assert!(p.text.lines().any(|l| l.contains("Total") && l.contains("$20")), "{:?}", p.text);
}

/// The head desktop Outlook writes in every message: Office's settings for itself alone, and its
/// style sheet.
const OUTLOOK_HEAD: &str = r#"<html xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office"><head><meta http-equiv="Content-Type" content="text/html; charset=utf-8"><!--[if gte mso 9]><xml><o:OfficeDocumentSettings><o:AllowPNG/><o:PixelsPerInch>96</o:PixelsPerInch></o:OfficeDocumentSettings></xml><![endif]--><style><!-- p.MsoNormal {margin:0cm; font-size:11.0pt; font-family:"Calibri",sans-serif} --></style><!--[if gte mso 9]><xml><o:shapedefaults v:ext="edit" spidmax="1026" /></xml><![endif]--></head><body lang="EN-US"><div class="WordSection1">"#;

/// Found in review: every message with Office's settings, a conditional comment or VML anywhere
/// (every message written in desktop Outlook, and one with the words "[if needed]") had the
/// tables it holds read as layout in its text, each cell on a line of its own: an invoice's rows
/// with a long description, a receipt's with a picture. Only Outlook's own tables, and those that
/// plainly lay a message out (`role="presentation"`), are read so; a table of values reads as it
/// did before, a row to a line.
#[test]
fn a_table_of_values_reads_a_row_to_a_line() {
    let description = "Annual support plan for the studio's twelve workstations, with next-day replacement parts and on-site visits each quarter";
    assert!(description.len() > 110);
    let invoice = format!(
        r#"<p class="MsoNormal">Please find the invoice below.</p><table border="1" cellpadding="4"><tr><th>Item</th><th>Description</th><th>Qty</th><th>Amount</th></tr><tr><td>SKU-1</td><td>{description}</td><td>1</td><td>$1,200.00</td></tr><tr><td>SKU-2</td><td>Training</td><td>3</td><td>$450.00</td></tr></table>"#
    );
    let receipt = r#"<p class="MsoNormal">Thanks for your order.</p><table><tr><td><img alt="Mug" src="https://shop.example/mug.png" width="40"></td><td>Mug</td><td>2</td><td>$24.00</td></tr><tr><td><img alt="Tea" src="https://shop.example/tea.png" width="40"></td><td>Tea</td><td>1</td><td>$9.50</td></tr></table>"#;
    for (html, row) in [
        (format!("{OUTLOOK_HEAD}{invoice}</div></body></html>"), ["SKU-1", "$1,200.00"]),
        (format!("{OUTLOOK_HEAD}{receipt}</div></body></html>"), ["Mug", "$24.00"]),
        (format!("<p>Reply [if needed]</p>{invoice}"), ["SKU-2", "$450.00"]),
        (format!("<p>VML: <v:rect></v:rect></p>{receipt}"), ["Tea", "$9.50"]),
    ] {
        let p = message(&html);
        // As before Outlook's copy was read for the text: the HTML as it came, as text.
        assert_eq!(p.text, crate::html_to_text(&html), "{html}");
        assert!(p.text.lines().any(|l| l.contains(row[0]) && l.contains(row[1])), "{:?}", p.text);
        assert!(!outlook_copy(&html).tables, "{html}");
    }
    // In a newsletter that gives Outlook a table of its own (with no role), its columns read one
    // after another, and a table of values in one of them still reads a row to a line.
    let html = format!(
        r#"<!--[if mso]><table width="600"><tr><td width="300" valign="top"><![endif]--><div style="display:inline-block;width:300px"><h2>Deal number 1</h2><p>Our customers love it.</p></div><!--[if mso]></td><td width="300" valign="top"><![endif]--><div style="display:inline-block;width:300px"><h2>Deal number 2</h2>{invoice}</div><!--[if mso]></td></tr></table><![endif]-->"#
    );
    assert!(outlook_copy(&html).tables);
    let p = message(&html);
    assert!(!p.text.lines().any(|l| l.contains("Deal number 1") && l.contains("Deal number 2")), "{:?}", p.text);
    assert!(p.text.find("Our customers") < p.text.find("Deal number 2"), "{:?}", p.text);
    assert!(p.text.lines().any(|l| l.contains("SKU-1") && l.contains("$1,200.00")), "{:?}", p.text);
}

/// The HTML as it came is read for hidden text a second time only when Outlook's copy left out a
/// copy for everyone else (found in review: read for every message with a conditional comment,
/// it took twice as long).
#[test]
fn the_others_copy_is_read_again_only_when_there_is_one() {
    assert!(!outlook_copy("<!--[if mso]><p>x</p><![endif]--><p>y</p>").theirs);
    assert!(!outlook_copy("<!--[if mso]><p>x</p><![endif]--><!--[if !mso]><!--><!--<![endif]-->").theirs);
    assert!(outlook_copy("<!--[if mso]><p>x</p><![endif]--><!--[if !mso]><!--><p>y</p><!--<![endif]-->").theirs);
    // Nor for the settings desktop Outlook writes in every message, which no browser shows.
    let head = "<!--[if gte mso 9]><xml><o:OfficeDocumentSettings><o:PixelsPerInch>96</o:PixelsPerInch></o:OfficeDocumentSettings></xml><![endif]-->";
    assert!(!outlook_copy(&format!("{head}<p>y</p>")).theirs);
    assert!(!outlook_copy("<!--[if !mso]><!-- --><!--<![endif]--><!--[if IE]>--> &nbsp; <![endif]-->").theirs);
}

/// Found in review: text the others' copy hides, which Outlook's leaves out, counted only when
/// that copy was written `<!--[if !mso]><!-->`. Every form a browser shows counts: the others'
/// copy opened with an empty comment of another form, one for Internet Explorer that browsers
/// show, Office's `<xml>` outside a comment (a browser shows what's in it), and `<![if !mso]>`.
#[test]
fn hidden_text_in_the_others_copy_counts_in_any_form() {
    let hidden = format!(r#"<div style="display:none">{WORDS}</div>"#);
    for (open, close) in [
        ("<!--[if !mso]><!-->", "<!--<![endif]-->"),
        ("<!--[if !mso]><!-- -->", "<!--<![endif]-->"),
        ("<!--[if !mso]><!---->", "<!--<![endif]-->"),
        ("<!--[if !mso]><!-- the others' copy -->", "<!--<![endif]-->"),
        ("<!--[if IE]>-->", "<![endif]-->"),
        ("<!--[if !mso]><!--->", "<!--<![endif]-->"),
        ("<xml>", "</xml>"),
        ("<![if !mso]>", "<![endif]>"),
    ] {
        for outlooks in ["", "<!--[if mso]><p>For Outlook</p><![endif]-->"] {
            let p = message(&format!("{outlooks}{open}{hidden}{close}<p>Body</p>"));
            assert_eq!(p.hidden_words, 34, "{outlooks}{open}");
            assert!(p.text.contains("Body") && !p.text.contains("winter"), "{open}: {:?}", p.text);
        }
    }
}

/// Found in review: the others' copy was read as having no text when its text was in a text box
/// or an `<xmp>`, which a browser shows as written, as if it were a script's; and text hidden in
/// one went unseen. What a template holds isn't shown, in either copy.
#[test]
fn a_text_box_in_the_others_copy_has_text() {
    for tag in ["textarea", "xmp", "TEXTAREA"] {
        let hidden = format!(r#"<div style="display:none"><{tag}>{WORDS}</{tag}></div>"#);
        let alone = message(&format!("{hidden}<p>Body</p>")).hidden_words;
        assert!(alone > 0, "{tag}");
        let p = message(&format!("<!--[if !mso]><!-->{hidden}<!--<![endif]--><p>Body</p>"));
        assert_eq!(p.hidden_words, alone, "{tag}");
        assert!(outlook_copy(&format!("<!--[if !mso]><!--><{tag}>x</{tag}><!--<![endif]-->")).theirs, "{tag}");
        // Left open, it runs to the end of the copy.
        assert!(outlook_copy(&format!("<!--[if !mso]><!--><{tag}>x<!--<![endif]-->")).theirs, "{tag}");
    }
    // Spaces and a text box's spacers aren't text; `<xmp>` shows a reference as it's written.
    assert!(!outlook_copy("<!--[if !mso]><!--><textarea> &nbsp; </textarea><!--<![endif]-->").theirs);
    assert!(outlook_copy("<!--[if !mso]><!--><xmp>&nbsp;</xmp><!--<![endif]-->").theirs);
    assert!(!outlook_copy("<!--[if !mso]><!--><template><p>x</p></template><!--<![endif]-->").theirs);
    assert!(!outlook_copy("<!--[if mso]><template><template></template><p>x</p></template><![endif]-->").own);
    assert!(outlook_copy("<!--[if mso]><template></template><p>x</p><![endif]-->").own);
    assert!(outlook_copy("<!--[if mso]><textarea>x</textarea><![endif]-->").own);
}

/// Found in review: a mark written into each of the tables in Outlook's copy made the copy
/// longer than the message, and the end of a long newsletter, under the cap on what's read
/// ([`crate::HTML_MAX`]), was pushed past it and lost (its "Unsubscribe" link with it). The
/// tables Outlook's copy leaves open are marked in the room the conditional comment's own start
/// and end took.
#[test]
fn marking_outlooks_tables_pushes_nothing_past_the_cap() {
    let deal = r#"<!--[if mso]><table width="600"><tr><td><![endif]--><p>Deal of the day</p><!--[if mso]></td></tr></table><![endif]-->"#;
    let deals = deal.repeat(3_200_000 / deal.len());
    let html = format!("<html><body>{deals}<p>Unsubscribe here</p></body></html>");
    let copy = outlook_copy(&html);
    assert!(copy.tables && copy.html.len() <= html.len(), "{} > {}", copy.html.len(), html.len());
    assert!(copy.html.ends_with("<p>Unsubscribe here</p></body></html>"));
    // Tables each opened and never closed, a megabyte of them.
    let html = format!("<!--[if mso]>{}<![endif]-->Tail<p>More</p>", "<table>".repeat(150_000));
    let p = message(&html);
    assert!(shown(&p).contains("Tail") && shown(&p).contains("More"), "{:?}", shown(&p));
    assert!(p.text.contains("Tail") && p.text.contains("More"), "{:?}", p.text);
    // A ghost column's tables, each in a comment of its own, are still Outlook's.
    let column = "<!--[if mso]><table><tr><td><![endif]--><div>c</div><!--[if mso]></td></tr></table><![endif]-->";
    let columns = column.repeat(3);
    let copy = outlook_copy(&columns);
    assert!(copy.tables && copy.html.len() <= columns.len() && copy.html.contains(OUTLOOK_MARK), "{}", copy.html);
    // The tree's mark is MailCraft's alone: a table that says it's Outlook's isn't.
    let p = message(
        "<!--[if mso]><table><tr><td>Hello</td></tr></table><![endif]--><table data-mailcraft-outlook><tr><td>SKU-1</td><td>$1,200.00</td></tr></table>",
    );
    assert!(p.text.lines().any(|l| l.contains("SKU-1") && l.contains("$1,200.00")), "{:?}", p.text);
}

/// Found in review: a copy that leaves tables and a style sheet, title or other raw-text element
/// open is kept with that element's closing tag, which with the marks took more room than the
/// comment, and the end of a long message was pushed past the cap and lost.
#[test]
fn a_closing_tag_and_the_marks_push_nothing_past_the_cap() {
    for name in RAW_TEXT {
        let unit = format!("<!--[if mso]><table><table><{name}><![endif]-->x\n");
        let html = unit.repeat(1_000);
        let copy = outlook_copy(&html);
        assert!(copy.html.contains(OUTLOOK_MARK) && copy.html.len() <= html.len(), "{name}: {} > {}", copy.html.len(), html.len());
    }
    // Its text: what's shown of it has more lines than the reading pane keeps, each copy's
    // tables being blocks.
    let unit = "<!--[if mso]><table><table><noframes><![endif]-->x\n";
    let html = format!("{}<p>Unsubscribe here</p>", unit.repeat(3_788_821 / unit.len()));
    let p = message(&html);
    assert!(p.text.contains("Unsubscribe"), "{:?}", &p.text[p.text.len().saturating_sub(80)..]);
}

/// Found in review: a copy of hostile markup took half as long again to read as before Outlook's
/// copy was read for the text too. Templates left open, each closed, made the copy twice as
/// long, though nothing in them is shown: what's in them is left out. And tables each in the
/// last one's cell, read as they came, took seconds to show: nested past what's read, the
/// message is read as it came, without Outlook's copy.
#[test]
fn hostile_copies_read_no_more_than_they_show() {
    let html = format!("<!--[if mso]><p>Shown</p>{}<![endif]-->Tail<p>More</p>", "<template><p>x</p>".repeat(10_000));
    let copy = outlook_copy(&html);
    assert!(copy.html.len() < 100, "{}", copy.html.len());
    let p = message(&html);
    assert!(shown(&p).contains("Shown") && shown(&p).contains("Tail") && !shown(&p).contains('x'), "{:?}", shown(&p));
    // Closed, a template is left out as before, and what follows it shown.
    let p = message("<!--[if mso]><template><template></template>x</template><p>Shown</p><![endif]-->Tail");
    assert!(shown(&p).contains("Shown") && shown(&p).contains("Tail") && !shown(&p).contains('x'), "{:?}", shown(&p));
    let html = format!("<!--[if mso]>{}Deep<![endif]-->Tail", "<table><tr><td>".repeat(20_000));
    let copy = outlook_copy(&html);
    assert!(!crate::layout::ready(&html, &copy).copy);
    let p = message(&html);
    assert_eq!(shown(&p), "Tail");
}

/// Found in review: reading through Outlook's copy looked for a comment's `--!>`, a
/// declaration's `>` or a reference's `;` to the end of the copy from each one in turn, so a
/// megabyte of comments took half a minute, four of them eight minutes. Each is looked through
/// once now, as the whole message (the fuzz seed `outlook-read-once` has each case).
#[test]
fn outlooks_copy_is_read_through_once() {
    let mb = |unit: &str| unit.repeat(1_000_000 / unit.len());
    let started = std::time::Instant::now();
    for copy in [mb("<!-- x -->"), mb("<!-- x --!>"), mb("<!"), mb("</ "), mb("&"), mb("<? ")] {
        let _ = outlook_copy(&format!("<!--[if mso]>{copy}<![endif]-->Tail"));
        let _ = hidden_from_outlook(&format!(r#"<p style="mso-hide:all">x</p>{copy}"#));
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
}

/// Found in review: Office's `<xml>` settings were looked for in comments, scripts, style sheets
/// and text boxes too, and an `<xml>` there dropped everything up to the next `</xml>`, what a
/// browser shows between them with it (a template's notes on its `<xml>` cut out two
/// paragraphs). Only an `<xml>` element is Office's settings.
#[test]
fn xml_in_a_comment_or_raw_text_drops_nothing() {
    for (html, kept) in [
        ("<!-- <xml> --><p>Visible one</p><!-- </xml> --><p>Two</p>", &["Visible one", "Two"][..]),
        (
            "<!--[if mso]><p>O</p><![endif]--><!-- Begin <xml> notes --><p>Visible one</p><p>Visible two</p><!-- end </xml> --><p>Three</p>",
            &["O", "Visible one", "Visible two", "Three"],
        ),
        ("<script>var a = '<xml>';</script><p>Visible one</p><script>var b = '</xml>';</script><p>Two</p>", &["Visible one", "Two"]),
        ("<style>/* <xml> */</style><p>Visible one</p><style>/* </xml> */</style><p>Two</p>", &["Visible one", "Two"]),
        ("<textarea><xml></textarea><p>Visible one</p><textarea></xml></textarea><p>Two</p>", &["Visible one", "Two"]),
    ] {
        let p = message(html);
        for words in kept {
            assert!(shown(&p).contains(words), "{html}: {:?}", shown(&p));
            assert!(p.text.contains(words), "{html}: {:?}", p.text);
        }
    }
    // An `<xml>` element is still dropped, a comment in it or not.
    let p = message("<!--[if gte mso 9]><xml><!-- x --><o:PixelsPerInch>96</o:PixelsPerInch></xml><![endif]--><p>Body</p>");
    assert!(!shown(&p).contains("96") && shown(&p).contains("Body"), "{:?}", shown(&p));
}
