use super::*;

/// Found in review: a table's footer row was left out of the message and its text (the text
/// converter reads only `<thead>` and `<tbody>`). It's read last, as a browser draws it, written
/// before the body or after it.
#[test]
fn a_tables_footer_is_read_last() {
    let before = "<table><thead><tr><th>H1</th><th>H2</th></tr></thead><tfoot><tr><td>F1</td><td>F2</td></tr></tfoot><tbody><tr><td>B1</td><td>B2</td></tr></tbody></table>";
    let after = "<table><tr><td>B1</td><td>B2</td></tr><TFOOT class=\"sum\"><tr><td>F1</td><td>F2</td></tr></TFOOT></table><p>Thanks</p>";
    for html in [before, after] {
        let text = crate::html::html_to_text(html);
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let body = lines.iter().position(|l| l.contains("B1") && l.contains("B2"));
        let foot = lines.iter().position(|l| l.contains("F1") && l.contains("F2"));
        assert!(body.is_some() && foot > body, "{html}: {text:?}");
        let rich: Vec<String> = crate::html::html_to_rich(html).iter().map(|l| l.iter().map(|s| s.text.as_str()).collect()).collect();
        let body = rich.iter().position(|l| l.contains("B1") && l.contains("B2"));
        let foot = rich.iter().position(|l| l.contains("F1") && l.contains("F2"));
        assert!(body.is_some() && foot > body, "{html}: {rich:?}");
    }
}

/// Found in review: a table laid out in columns showed its footer first in the reading pane, as
/// it was written, where its text had it last. The message, its text and its layout agree.
#[test]
fn a_footer_is_last_in_the_layout_too() {
    let html = r#"<table width="600"><thead><tr><th>ITEM</th><th>PRICE</th></tr></thead><tfoot><tr><td>TOTAL</td><td>$24.50</td></tr></tfoot><tbody><tr><td>MUG</td><td>$18.00</td></tr></tbody></table>"#;
    let raw = format!(
        "From: a@shop.example
Subject: s
Content-Type: text/html

{html}"
    );
    let p = crate::parse(raw.as_bytes()).unwrap_or_default();
    assert!(!p.layout.is_empty(), "laid out");
    let order = |s: &str| (s.find("MUG"), s.find("TOTAL"));
    for (view, said) in [("layout", format!("{:?}", p.layout)), ("rich", format!("{:?}", p.rich)), ("text", p.text.clone())] {
        let (body, foot) = order(&said);
        assert!(body.is_some() && foot > body, "{view}: {said}");
    }
}

#[test]
fn footers_go_to_the_end_of_their_own_table() {
    assert!(matches!(footers_last("<table><tr><td>a</td></tr></table>"), Cow::Borrowed(_)));
    // In the table it's in, a table in a cell keeping its own.
    let html =
        "<table><tfoot><tr><td>F</td></tr></tfoot><tr><td><table><tfoot><tr><td>f</td></tr></tfoot><tr><td>b</td></tr></table></td></tr></table>";
    assert_eq!(
        footers_last(html),
        "<table><tr><td><table><tr><td>b</td></tr><tbody><tr><td>f</td></tr></tbody></table></td></tr><tbody><tr><td>F</td></tr></tbody></table>"
    );
    // A table in a footer goes with it, as it is.
    let html =
        "<table><tfoot><tr><td><table><tfoot><tr><td>f</td></tr></tfoot><tr><td>b</td></tr></table></td></tr></tfoot><tr><td>B</td></tr></table>";
    assert_eq!(
        footers_last(html),
        "<table><tr><td>B</td></tr><tbody><tr><td><table><tbody><tr><td>f</td></tr></tbody><tr><td>b</td></tr></table></td></tr></tbody></table>"
    );
    // One left open ends where its table, or the next row group, begins.
    assert_eq!(footers_last("<table><tfoot><tr><td>F</td></table>"), "<table><tbody><tr><td>F</td></table>");
    assert_eq!(footers_last("<table><tfoot><tr><td>F<tbody><tr><td>B</table>"), "<table><tbody><tr><td>B<tbody><tr><td>F</table>");
    // Text, a comment, a script or a text box naming one is left as it is.
    let html = "<table><tr><td>1 < 2 <!-- <tfoot> --></td></tr></table><script>'<tfoot>'</script><textarea><tfoot></textarea>";
    assert_eq!(footers_last(html), html);
    // Only a table's first footer is its footer; another is a row group where it is, as a
    // browser takes it (found in review: every one went to the end).
    assert_eq!(
        footers_last(
            "<table><tfoot><tr><td>F1</td></tr></tfoot><tbody><tr><td>B1</td></tr></tbody><tfoot><tr><td>F2</td></tr></tfoot><tbody><tr><td>B2</td></tr></tbody></table>"
        ),
        "<table><tbody><tr><td>B1</td></tr></tbody><tbody><tr><td>F2</td></tr></tbody><tbody><tr><td>B2</td></tr></tbody><tbody><tr><td>F1</td></tr></tbody></table>"
    );
    // A `>` in a quoted value, a `<![CDATA[` or a `<?` naming one is no footer, and nothing after
    // it is lost (found in review: the rest of a quoted value was taken for markup).
    for html in [
        r#"<table><tr><td title="a>b<tfoot>c">XONE</td></tr><tbody><tr><td>YONE</td></tr></tbody></table>"#,
        "<table><tr><td>XONE<![CDATA[ <tfoot> ]]>YONE</td></tr></table>",
        "<table><tr><td>XONE<?x <tfoot> ?>YONE</td></tr></table>",
    ] {
        assert_eq!(footers_last(html), html);
        let text = crate::html::html_to_text(html);
        assert!(text.contains("XONE") && text.contains("YONE"), "{html}: {text:?}");
    }
    // Multibyte text around it is kept whole.
    assert_eq!(
        footers_last("<table><tfoot><tr><td>Σ €5</td></tr></tfoot><tr><td>Café</td></tr></table>"),
        "<table><tr><td>Café</td></tr><tbody><tr><td>Σ €5</td></tr></tbody></table>"
    );
}

/// Hostile markup is read once: footers and tables a megabyte deep, left open.
#[test]
fn hostile_footers_are_read_in_time() {
    let started = std::time::Instant::now();
    for unit in ["<table><tfoot>", "<tfoot>", "<table><tfoot><tr><td>x</tfoot></table>", "<table>", "<!--<tfoot>", "<script><tfoot>", "< tfoot"] {
        let html = unit.repeat(1_000_000 / unit.len());
        let out = footers_last(&html);
        assert!(out.len() <= html.len(), "{unit}");
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
}
