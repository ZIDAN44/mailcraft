//! Outlook's copy nested ever deeper by what its comments leave open: read in time, as the
//! message came when it's nested past what's read.

use super::*;

/// Found in review: tables left open in Outlook's copy, past the depth to which they're opened,
/// gave each new cell in place of the deepest one open, thousands of cells two hundred
/// elements deep, and a megabyte took twenty seconds to read; ghost tables left open each in a
/// comment of its own, every one in the last one's cell, six. Then, read as blocks past a few
/// tables, they flattened the tables of messages that nest no deeper than a browser shows. A
/// copy nested past what's read, or nesting more than two dozen tables, is read as the message
/// came, without Outlook's copy.
#[test]
fn tables_left_open_are_read_in_time() {
    let sized = |unit: &str| unit.repeat(100_000 / unit.len());
    let one_comment = format!("<!--[if mso]>{}Deep<![endif]-->Tail", sized("<table><tr><td>"));
    let ghost = sized("<!--[if mso]><table><tr><td><![endif]--><div>c</div>");
    let ghosts = sized("<!--[if mso]><table><tr><td><table><tr><td><![endif]--><div>c</div>");
    let styled = sized("<!--[if mso]><table><tr><td><table><tr><td><style><![endif]--><div>c</div>");
    let first_deep = format!("<!--[if mso]>{}<![endif]-->{ghost}", "<table><tr><td>".repeat(30));
    let started = std::time::Instant::now();
    for html in [&one_comment, &ghost, &ghosts, &styled, &first_deep] {
        let copy = outlook_copy(html);
        assert!(!crate::layout::ready(html, &copy).copy, "{}", &html[..60]);
        let last = if html.ends_with("Tail") { "Tail" } else { "c" };
        let p = message(html);
        let (shown, text) = (shown(&p), p.text);
        assert!(shown.ends_with(last) && text.trim_end().ends_with(last), "{}: {:?}", &html[..60], &text[text.len().saturating_sub(40)..]);
        assert!(!shown.contains("Deep"), "{shown:?}");
    }
    // Nested more than two dozen tables deep: as the message came, its tables read as tables
    // (found in review: written two dozen tables deep, a long copy took a second and a half to
    // show).
    let html = format!(
        "<!--[if mso]><table><tr><td><![endif]-->{}<p>One</p><b>Two</td><td>Three<div>Four</td></tr><tr><td>Five",
        "<table><tr><td>".repeat(30)
    );
    let ready = crate::layout::ready(&html, &outlook_copy(&html));
    assert!(!ready.copy, "{}", ready.shown);
    let p = message(&html);
    assert!(shown(&p).split('|').any(|l| l.contains("One") && l.contains("Three")), "{:?}", shown(&p));
    assert!(p.text.lines().any(|l| l.contains("One") && l.contains("Three")), "{:?}", p.text);
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
}

/// Found in review: past four of Outlook's tables, each in the last one's cell, the next was read
/// as blocks: a newsletter's two columns, in a ghost table in five ghost wrappers, came out one
/// under the other; and past two dozen tables of any kind, a table of values in a message with a
/// style sheet or something hidden from Outlook read a cell to a line. Neither is capped now. And
/// past a dozen tables in Outlook's copy, a table of values read a cell to a line: an order's, in
/// eleven layout tables in Outlook's ghost table; then one whose items each had their picture and
/// name in a table of their own, and totals with a table in a cell; then, the outermost tables
/// made blocks, a table of columns or of values around a deeper one lost its columns; and, told
/// apart by what their cells held, a receipt with no headings whose items each had their name in
/// a table of their own, labels each in a table, and values that are only signs. Only Outlook's
/// own tables, and those that say they lay the message out, are blocks; a copy nesting more than
/// two dozen tables isn't read.
#[test]
fn tables_nested_as_newsletters_nest_them_stay_tables() {
    let wrapper = r#"<!--[if mso | IE]><table role="presentation" width="600"><tr><td><![endif]-->"#;
    let closer = "<!--[if mso | IE]></td></tr></table><![endif]-->";
    let columns = r#"<!--[if mso | IE]><table role="presentation"><tr><td width="300"><![endif]--><div style="display:inline-block;width:300px">Left text</div><!--[if mso | IE]></td><td width="300"><![endif]--><div style="display:inline-block;width:300px">Right text</div><!--[if mso | IE]></td></tr></table><![endif]-->"#;
    let p = message(&format!("{}{columns}{}", wrapper.repeat(5), closer.repeat(5)));
    assert!(shown(&p).split('|').any(|l| l.contains("Left text") && l.contains("Right text")), "{:?}", shown(&p));
    let data = "<table><tr><th>Item</th><th>Qty</th></tr><tr><td>Mug</td><td>2</td></tr><tr><td>Tea</td><td>1</td></tr></table>";
    let nest = format!("{}{data}{}", "<table><tr><td>".repeat(25), "</td></tr></table>".repeat(25));
    for head in ["<style>p{color:red}</style>", r#"<span style="mso-hide:all">h</span>"#] {
        let p = message(&format!("{head}{nest}"));
        assert!(shown(&p).split('|').any(|l| l.contains("Mug") && l.contains('2')), "{head}: {:?}", shown(&p));
        assert!(p.text.lines().any(|l| l.contains("Mug") && l.contains('2')), "{head}: {:?}", p.text);
    }
    let ghost = r#"<!--[if mso]><table role="presentation" width="600"><tr><td><![endif]-->"#;
    let ghost_end = "<!--[if mso]></td></tr></table><![endif]-->";
    for n in [11, 12, 22, 23, 30] {
        for table in [r#"<table role="presentation"><tr><td>"#, "<table><tr><td>"] {
            let p = message(&format!("{ghost}{}{data}{}{ghost_end}", table.repeat(n), "</td></tr></table>".repeat(n)));
            assert!(shown(&p).split('|').any(|l| l.contains("Mug") && l.contains('2')), "{n} {table}: {:?}", shown(&p));
            assert_eq!(p.text, "Item Qty\nMug  2\nTea  1", "{n} {table}");
        }
    }
    let order = r#"<table><tr><td>Item</td><td>Qty</td><td>Price</td></tr><tr><td><table><tr><td><img src="https://shop.example/mug.png"></td><td>Blue mug</td></tr></table></td><td>2</td><td>$18.00</td></tr><tr><td><table><tr><td><img src="https://shop.example/tea.png"></td><td>Green tea</td></tr></table></td><td>1</td><td>$6.50</td></tr></table>"#;
    let totals = "<table><tr><td>Subtotal</td><td>$24.50</td></tr><tr><td>Shipping</td><td><table><tr><td>Standard</td><td>$4.00</td></tr></table></td></tr><tr><td>Total</td><td>$28.50</td></tr></table>";
    for (values, row) in [(order, ["Blue mug", "$18.00"]), (totals, ["Shipping", "$4.00"])] {
        // Its text as without Outlook's copy, a row to a line, at any depth (found in review: in
        // any message with Outlook's tables, a table with one in a cell read a cell to a line).
        let text = message(values).text;
        assert!(text.lines().any(|l| l.contains(row[0]) && l.contains(row[1])), "{text:?}");
        for n in [0, 11, 12, 13, 20, 22, 23, 30] {
            let p = message(&format!("{ghost}{}{values}{}{ghost_end}", "<table><tr><td>".repeat(n), "</td></tr></table>".repeat(n)));
            assert!(shown(&p).split('|').any(|l| l.contains(row[0]) && l.contains(row[1])), "{n}: {:?}", shown(&p));
            assert_eq!(p.text, text, "{n}");
        }
    }
    // Any other table reads as it would without Outlook's copy, whatever its cells hold.
    let receipt = r#"<table><tr><td><table><tr><td><img src="https://shop.example/m.jpg" width="40"></td><td>Blue mug</td></tr></table></td><td>$18.00</td></tr><tr><td><table><tr><td><img src="https://shop.example/t.jpg" width="40"></td><td>Green tea</td></tr></table></td><td>$6.50</td></tr></table>"#;
    let labels = "<table><tr><td><table><tr><td>Subtotal</td></tr></table></td><td>$24.50</td></tr><tr><td><table><tr><td>Total</td></tr></table></td><td>$28.50</td></tr></table>";
    let wrapped = "<table><tr><td><table><tr><td>Blue mug</td></tr></table></td><td><table><tr><td>$18.00</td></tr></table></td></tr></table>";
    let signs = "<table><tr><td>$</td><td>&mdash;</td></tr><tr><td>x</td><td>+</td></tr></table>";
    let columns = "<table><tr><td><table><tr><td>Left column</td></tr></table></td><td>&nbsp;</td><td><table><tr><td>Right column</td></tr></table></td></tr></table>";
    // A table that says it lays the message out, inside a table of values, stays a table too.
    let presented = r#"<table><tr><th>Item</th><th>Qty</th><th>Price</th></tr><tr><td><table role="presentation"><tr><td><img alt="Mug picture" src="https://shop.example/m.jpg"></td><td>Blue mug</td></tr></table></td><td>2</td><td>$18.00</td></tr></table>"#;
    for (values, row) in [
        (receipt, ["Blue mug", "$18.00"]),
        (labels, ["Subtotal", "$24.50"]),
        (wrapped, ["Blue mug", "$18.00"]),
        (signs, ["$", "\u{2014}"]),
        (columns, ["Left", "Right"]),
        (presented, ["Blue mug", "$18.00"]),
    ] {
        let text = message(values).text;
        assert!(text.lines().any(|l| l.contains(row[0]) && l.contains(row[1])), "{text:?}");
        assert_eq!(message(&format!("{ghost}{values}{ghost_end}")).text, text, "{values}");
    }
    // A table of columns, or of values, around one nested deeper than Outlook's copy is read
    // keeps its columns.
    let deep = |inner: &str| format!("{}{inner}{}", "<table><tr><td>".repeat(25), "</td></tr></table>".repeat(25));
    let beside = format!(
        r#"<table role="presentation"><tr><td width="300">{}</td><td width="300"><table><tr><td><p>Right column words</p></td></tr></table></td></tr></table>"#,
        deep("<p>Left column words</p>")
    );
    let around = format!(
        "<table><tr><th>Item</th><th>Note</th></tr><tr><td>Mug</td><td>Blue</td></tr><tr><td>Tea</td><td>{}</td></tr></table>",
        deep("Green")
    );
    for (html, row) in [(beside, ["Left column words", "Right column words"]), (around, ["Mug", "Blue"])] {
        let p = message(&format!("{ghost}{html}{ghost_end}"));
        assert!(shown(&p).split('|').any(|l| l.contains(row[0]) && l.contains(row[1])), "{:?}", shown(&p));
        assert_eq!(p.text, message(&html).text);
    }
}

/// Found in review: boxes each left open in a comment only Outlook reads were each in the last,
/// a hundred thousand deep, and a megabyte of them took six seconds to read, four two minutes, as
/// long as a message nested that deep (which a browser shows): twice the time Outlook's copy
/// was read as it came, once for what's shown and once for its text. A copy whose comments leave
/// elements open past what's read is read as the message came, without Outlook's copy.
#[test]
fn boxes_left_open_are_read_in_time() {
    let unit = "<!--[if mso]><div><![endif]--><p>c</p>";
    let html = unit.repeat(400_000 / unit.len());
    let copy = outlook_copy(&html);
    assert!(copy.nests && !copy.tables);
    assert!(!crate::layout::ready(&html, &copy).copy);
    let started = std::time::Instant::now();
    let p = message(&html);
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    assert_eq!(p.text.lines().filter(|l| *l == "c").count(), html.len() / unit.len(), "{:?}", &p.text[..40]);
    // Comments that leave nothing open, nest nothing deep, are read as they came.
    for html in ["<!--[if mso]><div><p>x</p></div><![endif]-->", "<!--[if mso]>&amp; x<![endif]-->", "<!--[if mso]><br><img src=a><![endif]-->"] {
        assert!(!outlook_copy(html).nests, "{html}");
    }
    assert!(outlook_copy(&format!("<!--[if mso]>{}{}<![endif]-->", "<div>".repeat(40), "</div>".repeat(40))).nests);
}
