//! A link around boxes (issue #225): what they hold opens it, as words inside it would.

use super::*;

/// Every span of `blocks` with a picture: its address (cut short) and link.
fn pictures(blocks: &[Block]) -> Vec<(String, Option<String>)> {
    let mut all = blocks.to_vec();
    let mut out = Vec::new();
    each_span(&mut all, &mut |s| {
        if let Some(src) = &s.image {
            out.push((src.chars().take(21).collect(), s.link.clone()));
        }
    });
    out
}

/// An Apple Mail rich link card: a link around a table, its picture written into the HTML. The
/// picture opens the link; a link of its own inside wins; an address that isn't one to follow
/// isn't given to it; a link around words alone is read with them, as before.
#[test]
fn a_link_around_a_card_opens_from_what_it_holds() {
    let card = |href: &str, inner: &str| {
        format!(
            "<div>Open my shared album:<br><div><a class=\"lp-rich-link\" style=\"border-radius:10px;display:block;width:275px;overflow:hidden\" \
             href=\"{href}\" role=\"button\" width=\"275\"><table style=\"table-layout:fixed;border-collapse:collapse;width:275px;background-color:#E9E9EB\" \
             cellpadding=\"0\" cellspacing=\"0\" border=\"0\" width=\"275\"><tbody><tr><td align=\"center\">{inner}</td></tr>\
             <tr><td>Shared album <b>Summer</b></td></tr></tbody></table></a></div></div>"
        )
    };
    let picture =
        "<img src=\"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=\" />";
    let album = "https://photos.example/shared/album/EXAMPLE";
    let p = message(&card(album, picture));
    assert_eq!(pictures(&p.layout), [("data:image/png;base64".to_string(), Some(album.to_string()))]);
    for words in ["Shared album", "Summer"] {
        assert!(spans(&p.layout, words).iter().all(|s| s.link.as_deref() == Some(album)), "{words}");
    }
    // One of its own, inside: its own.
    let p = message(&card(album, "<a href=\"https://photos.example/one\"><img src=\"cid:one@x\" alt=\"one\"></a>"));
    assert_eq!(pictures(&p.layout), [("cid:one@x".to_string(), Some("https://photos.example/one".to_string()))]);
    // Not an address to follow: nothing given.
    for bad in ["javascript:alert(1)", "data:text/html,x", "file:///C:/Windows/notepad.exe"] {
        let p = message(&card(bad, picture));
        assert_eq!(pictures(&p.layout), [("data:image/png;base64".to_string(), None)], "{bad}");
    }
    // Words alone in a link: read with them, as ever.
    let p = message("<p>See <a href=\"https://shop.example/coat\">the coat</a> today.</p>");
    let coat = p.rich.iter().flatten().find(|s| s.text.trim() == "the coat");
    assert_eq!(coat.and_then(|s| s.link.as_deref()), Some("https://shop.example/coat"));
}
