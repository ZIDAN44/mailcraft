//! The marks put on the tables Outlook's own copy lays a newsletter out with.

use super::*;

/// Found in review: every table in a comment only Outlook reads was read as laying the message
/// out, and an order or an invoice given whole in Outlook's copy read a cell to a line. Only a
/// table the comment leaves open, whose cells hold what follows it, is Outlook's; one opened and
/// closed in it is read as any other, a table of values a row to a line.
#[test]
fn only_a_table_outlooks_copy_leaves_open_is_marked() {
    let order = r#"<table><tr><td><img src="https://shop.example/a.jpg" alt="Mug" width=60></td><td>Blue mug</td><td>2</td><td>$18.00</td></tr><tr><td><img src="https://shop.example/b.jpg" alt="Tea" width=60></td><td>Green tea</td><td>1</td><td>$9.00</td></tr></table>"#;
    let invoice = r#"<table border="1"><tr><th>Item</th><th>Description</th><th>Qty</th><th>Amount</th></tr><tr><td>SKU-1</td><td>Support plan</td><td>1</td><td>$1,200.00</td></tr></table>"#;
    for (table, row) in [(order, ["Blue mug", "$18.00"]), (invoice, ["SKU-1", "$1,200.00"])] {
        let html = format!("<p>Your order</p><!--[if mso]>{table}<![endif]--><p>Thanks</p>");
        let p = message(&html);
        assert!(p.text.lines().any(|l| l.contains(row[0]) && l.contains(row[1])), "{:?}", p.text);
        let copy = outlook_copy(&html);
        assert!(copy.tables && !copy.html.contains(OUTLOOK_MARK), "{}", copy.html);
        // In a ghost table of Outlook's around it, too.
        let ghost = format!("<!--[if mso]><table width=600><tr><td><![endif]-->{table}<!--[if mso]></td></tr></table><![endif]-->");
        let p = message(&ghost);
        assert!(p.text.lines().any(|l| l.contains(row[0]) && l.contains(row[1])), "{:?}", p.text);
    }
    // A ghost table is marked, the outermost two when a comment opens more than that, and a
    // table closed before it isn't.
    let copy = outlook_copy(
        "<!--[if mso]><table><tr><td><table><tr><td><table><tr><td><![endif]-->x<!--[if mso]></td></tr></table></td></tr></table></td></tr></table><![endif]-->",
    );
    assert_eq!(copy.html.matches(OUTLOOK_MARK).count(), 2, "{}", copy.html);
    assert!(copy.html.starts_with(&format!("{OUTLOOK_MARK}<table><tr><td>{OUTLOOK_MARK}<table>")), "{}", copy.html);
    let copy = outlook_copy(
        "<!--[if mso]><table><tr><td>a</td></tr></table><table width=600><tr><td><![endif]-->x<!--[if mso]></td></tr></table><![endif]-->",
    );
    assert!(copy.html.starts_with(&format!("<table><tr><td>a</td></tr></table>{OUTLOOK_MARK}<table width=600>")), "{}", copy.html);
    // Nor one in a template, which isn't shown.
    let copy = outlook_copy("<!--[if mso]><template><table><tr><td></template><![endif]-->x");
    assert!(!copy.html.contains(OUTLOOK_MARK), "{}", copy.html);
}

/// Found in review: a conditional comment whose copy leaves a table open, in a text box,
/// `<xmp>`, `<plaintext>` or an attribute, where what's written is text, showed MailCraft's own
/// mark as words; and what followed `<plaintext>` showed the closing tag written after it.
#[test]
fn marks_and_closing_tags_never_show() {
    for (html, words) in [
        ("<textarea><!--[if mso]><table><tr><td>A<![endif]--></textarea><p>Body</p>", "<table><tr><td>A"),
        ("<xmp><!--[if mso]><table><tr><td>A<![endif]--></xmp><p>Body</p>", "<table><tr><td>A"),
        (r#"<img alt="x<!--[if mso]><table><![endif]-->y" src="https://a.example/p.png"><p>Body</p>"#, "x<table>y"),
        ("<p>Body</p><plaintext>raw <!--[if mso]><table><tr><td>A<![endif]--> end", "raw <table><tr><td>A end"),
        ("<p>Body</p><plaintext>raw <!--[if mso]><table><tr><td>A</td></tr></table><![endif]--> end", "raw <table><tr><td>A</td></tr></table> end"),
    ] {
        let p = message(html);
        for read in [shown(&p), p.text.clone()] {
            assert!(read.contains(words) && read.contains("Body"), "{html}: {read:?}");
            assert!(!read.contains("mso") && !read.contains("</plaintext>"), "{html}: {read:?}");
        }
    }
    // What follows `<plaintext>` is its text, as written, in a box of its own.
    let p = message("<div><p>Hi</p><plaintext>raw &amp; <b>x</b> end</div>");
    assert!(shown(&p).ends_with("raw &amp; <b>x</b> end</div>"), "{:?}", shown(&p));
}
