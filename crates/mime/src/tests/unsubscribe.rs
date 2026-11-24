//! A mailing list's way out (`List-Unsubscribe`, RFC 2369), shown above the message as a link
//! (issue #272).

use super::*;

fn unsubscribe(header: &str) -> Option<String> {
    let raw = format!("From: news@shop.example\r\nSubject: s\r\nList-Unsubscribe: {header}\r\nContent-Type: text/plain\r\n\r\nHi\r\n");
    parse(raw.as_bytes()).and_then(|p| p.unsubscribe)
}

/// Of the ways a list gives, the web page is the link (it opens where the list says, and
/// asks there), in any order; a mail address alone is that address; none is none.
#[test]
fn the_web_page_is_the_way_out() {
    let web = "https://lists.example/unsubscribe?id=CTR_3KEd&x=1";
    assert_eq!(unsubscribe(&format!("<mailto:leave@lists.example?subject=unsubscribe>, <{web}>")).as_deref(), Some(web));
    assert_eq!(unsubscribe(&format!("<{web}>, <mailto:leave@lists.example>")).as_deref(), Some(web));
    assert_eq!(unsubscribe("<mailto:leave@lists.example?subject=unsubscribe>").as_deref(), Some("mailto:leave@lists.example?subject=unsubscribe"));
    let raw = "From: a@b.example\r\nSubject: s\r\n\r\nHi\r\n";
    assert_eq!(parse(raw.as_bytes()).and_then(|p| p.unsubscribe), None);
}
