use super::*;

/// A table copied from another program, its text a row a line.
fn table(rows: &[&[&str]]) -> mime::Pasted {
    let html: String = rows.iter().map(|r| format!("<tr>{}</tr>", r.iter().map(|c| format!("<td>{c}</td>")).collect::<String>())).collect();
    let parts = mime::pasted_tables(&format!("<table border=1>{html}</table>")).expect("a table");
    parts.into_iter().find(|p| matches!(p, mime::Pasted::Table(_))).expect("a table")
}

fn written(body: &str) -> Compose {
    Compose { body: body.into(), styles: rich::Styles::plain(body), ..Default::default() }
}

/// A table pasted mid-line starts a line of its own; its text stands for it, and the message
/// goes out with the table in its place and the text either side, no empty lines added; its
/// plain text has the table's text.
#[test]
fn a_pasted_table_goes_out_in_its_place() {
    let mut c = written("Prices below\nThanks");
    let at = c.paste_parts(12..12, vec![table(&[&["Paper", "12.50"], &["Ink", "8.00"]])]);
    assert_eq!(c.body, "Prices below\nPaper\t12.50\nInk\t8.00\nThanks");
    assert_eq!(at, 33, "the cursor after it");
    assert_eq!(c.table_places(), vec![(0, 13..33)]);
    let (text, html, _) = c.bodies();
    assert!(text.contains("Paper\t12.50\nInk\t8.00"), "{text}");
    let html = html.expect("HTML: it has a table");
    let (before, rest) = html.split_once(&format!("<div class=\"{}\"><table", mime::TABLE_CLASS)).expect("the table in its place");
    assert!(before.contains("Prices below") && !before.contains("Paper"), "{before}");
    assert!(rest.contains("Paper") && rest.contains("12.50") && rest.ends_with("</div>\n</div>\n") || rest.contains("Thanks"), "{rest}");
    assert!(!html.contains("<div><br></div>"), "no empty lines added: {html}");
    // Plain text: its text alone.
    c.plain = true;
    assert_eq!((c.table_places(), c.bodies().1), (vec![], None));
}

/// A table whose text was changed is that text; one taken out is gone; two alike are two, each
/// where it stands; pasted over, a table goes.
#[test]
fn a_table_stands_where_its_text_is() {
    let mut c = written("");
    c.paste_parts(0..0, vec![table(&[&["A", "1"]]), mime::Pasted::Text("between".into(), rich::Styles::plain("between")), table(&[&["A", "1"]])]);
    assert_eq!(c.body, "A\t1\nbetween\nA\t1");
    assert_eq!(c.table_places(), vec![(0, 0..3), (1, 12..15)]);
    // The first one's text changed: the second stays a table.
    c.body = c.body.replacen("A\t1", "A\t2", 1);
    assert_eq!(c.table_places(), vec![(0, 12..15)]);
    let html = c.bodies().1.unwrap_or_default();
    assert_eq!(html.matches(mime::TABLE_CLASS).count(), 1, "{html}");
    // Pasted over: the table under it goes, the new one is kept in its place.
    let n = c.body.chars().count();
    c.paste_parts(12..n, vec![table(&[&["B", "2"]])]);
    assert_eq!(c.body, "A\t2\nbetween\nB\t2");
    assert_eq!(c.tables.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["B\t2"]);
    // Text alone typed like a table's is text.
    let mut typed = written("B\t2");
    typed.tables = Vec::new();
    assert!(typed.table_places().is_empty());
}

/// A draft keeps its tables: the HTML it's saved as, opened again, has the same text and the
/// same tables; what the server keeps is written again as MailCraft writes a table, nothing of
/// it sent as it came.
#[test]
fn a_draft_opened_again_has_its_tables() {
    let mut c = written("Hello\nThanks");
    c.paste_parts(6..6, vec![table(&[&["Paper", "12.50"]])]);
    let html = c.bodies().1.expect("HTML");
    let mut tables = Vec::new();
    let (text, _) = text_with_tables(&html, &mut tables);
    assert_eq!(text, c.body);
    assert_eq!(tables, c.tables);
    // A table changed on the server: what goes out is MailCraft's writing of it.
    let tampered =
        format!("<div class=\"{}\"><table><tr><td onclick=\"x()\">A<script>x()</script></td><td>B</td></tr></table></div>", mime::TABLE_CLASS);
    let mut tables = Vec::new();
    let (text, _) = text_with_tables(&tampered, &mut tables);
    assert_eq!(text, "A	B");
    assert!(!tables[0].code.contains("script") && !tables[0].code.contains("onclick"), "{}", tables[0].code);
    // One that isn't a table any more (one cell is a box of text): its text.
    let one = format!("<div class=\"{}\"><table><tr><td>Only</td></tr></table></div>", mime::TABLE_CLASS);
    let mut tables = Vec::new();
    assert_eq!(text_with_tables(&one, &mut tables).0, "Only");
    assert!(tables.is_empty());
}

/// Tables past the most kept are pasted as their text.
#[test]
fn tables_past_the_most_are_their_text() {
    let mut c = written("");
    let parts: Vec<mime::Pasted> = (0..TABLES_MAX + 3).map(|i| table(&[&[&format!("Row {i}"), "x"]])).collect();
    c.paste_parts(0..0, parts);
    assert_eq!(c.tables.len(), TABLES_MAX);
    assert_eq!(c.table_places().len(), TABLES_MAX);
    assert!(c.body.contains(&format!("Row {}\tx", TABLES_MAX + 2)), "the rest as text");
}

/// A signature's code and a table both stand in the HTML in their places.
#[test]
fn a_table_and_a_signatures_code_go_out_together() {
    let mut c = written("Hello\n\n-- \nRiley");
    c.signature = "-- \nRiley".into();
    c.signature_code = "<table><tr><td>Riley Moreno</td><td>Studio</td></tr></table>".into();
    c.paste_parts(6..6, vec![table(&[&["Paper", "12.50"]])]);
    let html = c.bodies_with(&c.signature_code.clone()).1.expect("HTML");
    let table_at = html.find(mime::TABLE_CLASS).expect("the table");
    let signature_at = html.find(mime::SIGNATURE_CLASS).expect("the signature's code");
    assert!(table_at < signature_at, "{html}");
    assert!(html.contains("Riley Moreno") && html.contains("12.50"), "{html}");
}
