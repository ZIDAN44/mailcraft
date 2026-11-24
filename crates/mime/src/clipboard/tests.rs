use super::*;

/// The clipboard's "HTML Format" as Windows programs write it: the header's offsets counted for
/// `page`, its fragment between the markers.
fn cf_html(page: &str) -> Vec<u8> {
    let header_len = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n".len();
    let start = header_len + page.find("<!--StartFragment-->").map_or(0, |i| i + "<!--StartFragment-->".len());
    let end = header_len + page.find("<!--EndFragment-->").unwrap_or(page.len());
    let total = header_len + page.len();
    format!("Version:0.9\r\nStartHTML:{header_len:010}\r\nEndHTML:{total:010}\r\nStartFragment:{start:010}\r\nEndFragment:{end:010}\r\n{page}")
        .into_bytes()
}

/// Cells copied in Excel: its fragment is the rows alone, inside a table that opens before it,
/// under a style sheet with their lines. The whole page is what was copied (issue #268): the
/// fragment alone had no table, and the cells were pasted as text.
#[test]
fn cells_copied_in_excel_come_with_their_table() {
    let page = "<html><head><style><!--.xl65 {border:.5pt solid windowtext;}--></style></head><body>\r\n\
        <table border=0 cellpadding=0 cellspacing=0>\r\n<!--StartFragment-->\r\n <col width=64 span=2>\r\n\
        <tr><td class=xl65>Paper</td><td class=xl65 align=right>12.50</td></tr>\r\n\
        <tr><td class=xl65>Ink</td><td class=xl65 align=right>8.00</td></tr>\r\n<!--EndFragment-->\r\n</table></body></html>\r\n";
    let html = windows_clipboard_html(&cf_html(page)).expect("HTML");
    assert_eq!(html, page);
    let parts = crate::pasted_tables(&html).expect("a table");
    let table = parts.iter().find_map(|p| match p {
        crate::Pasted::Table(t) => Some(t),
        crate::Pasted::Text(..) => None,
    });
    let table = table.expect("a table");
    assert_eq!(table.text, "Paper\t12.50\nInk\t8.00");
    assert!(table.code.contains("border-top:1px solid #000000"), "its lines, from the style sheet: {}", table.code);
}

/// What other programs copy is their fragment, as before: a web page's table is in it, and
/// Word's paragraphs; the page around them isn't pasted (a table that ends before the fragment
/// included).
#[test]
fn other_programs_copy_their_fragment() {
    for (page, fragment) in [
        ("<html><body><!--StartFragment--><table><tr><td>A</td></tr></table><!--EndFragment--></body></html>", "<table><tr><td>A</td></tr></table>"),
        (
            "<html><head><style>p.MsoNormal{margin:0}</style></head><body><!--StartFragment--><p class=MsoNormal>Hello</p><!--EndFragment--></body></html>",
            "<p class=MsoNormal>Hello</p>",
        ),
        (
            "<html><body><table><tr><td>Menu</td></tr></table><TABLE><tr><td>Old</td></tr></TABLE><!--StartFragment--><p>Words</p><!--EndFragment--></body></html>",
            "<p>Words</p>",
        ),
        ("<html><body><tablet><!--StartFragment--><p>W</p><!--EndFragment--></body></html>", "<p>W</p>"),
    ] {
        assert_eq!(windows_clipboard_html(&cf_html(page)).as_deref(), Some(fragment), "{page}");
    }
}

/// The header is the other program's: offsets past the end, the wrong way round, or not numbers
/// are no fragment; the page, if it reads, is what was copied. Nothing panics, and no HTML is none.
#[test]
fn a_header_that_doesnt_add_up_is_read_as_far_as_it_does() {
    let page = "<p>Hi</p>";
    let with = |header: &str| windows_clipboard_html(format!("{header}{page}").as_bytes());
    for bad in [
        "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:9999999999\r\nStartFragment:0000000003\r\nEndFragment:0000000001\r\n",
        "Version:0.9\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:999999999999999999999\r\nEndFragment:x\r\n",
        "Version:0.9\r\nStartFragment:0000000040\r\nEndFragment:0000000020\r\n",
        "Version:0.9\r\n",
    ] {
        assert_eq!(with(bad), None, "{bad}");
    }
    // Its page, when the fragment doesn't read.
    let header = "Version:0.9\r\nStartHTML:0000000000\r\nStartFragment:9999999999\r\nEndFragment:9999999999\r\n";
    let page_at = header.len();
    let header = header.replacen("StartHTML:0000000000", &format!("StartHTML:{page_at:010}"), 1);
    assert_eq!(with(&header).as_deref(), Some(page));
    // No header: HTML as it is, and NULs at its end gone; nothing, or text, is no HTML.
    assert_eq!(windows_clipboard_html(b"<b>Hi</b>\0\0").as_deref(), Some("<b>Hi</b>"));
    assert_eq!(windows_clipboard_html(b""), None);
    assert_eq!(windows_clipboard_html(b"\0"), None);
    assert_eq!(windows_clipboard_html(b"just words"), None);
}
