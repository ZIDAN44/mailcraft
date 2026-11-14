//! What the mime tests share, one copy of each: a message of some HTML, the words it shows, the
//! tables laid out, a page as templates lay one out, a message's key, a picture.

use crate::layout::{Block, Table};
use mailcraft_model::MessageKey;

/// A picture of one pixel (PNG, base64).
pub(crate) const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// A message of `html`, as received.
pub(crate) fn message(html: &str) -> crate::Parsed {
    let raw = format!("From: a@shop.example\r\nSubject: s\r\nContent-Type: text/html\r\n\r\n{html}");
    crate::parse(raw.as_bytes()).expect("the message parses")
}

/// The words a message shows, its lines joined by `|`.
pub(crate) fn shown(p: &crate::Parsed) -> String {
    p.rich.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("|")
}

/// Every table in `blocks`, those in their cells too, outermost first.
pub(crate) fn tables(blocks: &[Block]) -> Vec<&Table> {
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

/// `inner` on a page as templates lay one out: in a table 600 pixels wide, in the middle.
pub(crate) fn page(inner: &str) -> String {
    format!(r#"<table width="600" align="center" cellpadding="0" cellspacing="0"><tr><td>{inner}</td></tr></table>"#)
}

/// A message's key, in an account's inbox.
pub(crate) fn key() -> MessageKey {
    MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid: 1 }
}
