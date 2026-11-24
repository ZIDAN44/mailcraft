use super::*;

/// Cells copied in Excel, as its clipboard HTML has them: the lines, bold and colour in the
/// classes of a `<style>`, the alignment there or on the cell.
const EXCEL: &str = r##"<html xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel">
<head><meta name=ProgId content=Excel.Sheet><style>
.xl65 {mso-style-parent:style0; border:.5pt solid windowtext; text-align:right;}
.xl66 {font-weight:700; background:#FFFF00; mso-pattern:black none; border:.5pt solid windowtext;}
</style></head>
<body link="#0563C1" vlink="#954F72">
<table border=0 cellpadding=0 cellspacing=0 width=200 style='border-collapse:collapse;width:150pt'>
<!--StartFragment-->
 <tr height=20 style='height:15.0pt'>
  <td class=xl66 width=100 style='height:15.0pt;width:75pt'>Item</td>
  <td class=xl66 width=100 style='width:75pt'>Price</td>
 </tr>
 <tr height=20 style='height:15.0pt'>
  <td class=xl65 style='height:15.0pt;text-align:left'>Paper</td>
  <td class=xl65>12.50</td>
 </tr>
 <tr height=20 style='height:15.0pt'>
  <td class=xl65 style='height:15.0pt;text-align:left'>Ink</td>
  <td class=xl65>8.00</td>
 </tr>
<!--EndFragment-->
</table>
</body>
</html>"##;

fn tables(parts: &[Pasted]) -> Vec<&PastedTable> {
    parts
        .iter()
        .filter_map(|p| match p {
            Pasted::Table(t) => Some(t),
            Pasted::Text(..) => None,
        })
        .collect()
}

/// Cells copied in Excel are a table of MailCraft's writing: its rows and cells, their lines
/// and colours, the headings bold and the prices to the right; its text a row a line, the cells
/// apart by tabs. Nothing of Excel's own goes out: no classes, styles or Office markup.
#[test]
fn cells_from_excel_are_a_table_of_its_own() {
    let parts = pasted_tables(EXCEL).expect("a table");
    let found = tables(&parts);
    assert_eq!(found.len(), 1, "{parts:?}");
    let t = found[0];
    assert_eq!(t.text, "Item\tPrice\nPaper\t12.50\nInk\t8.00");
    let code = &t.code;
    assert!(code.starts_with("<table") && code.ends_with("</table>"), "{code}");
    assert_eq!(code.matches("<tr>").count(), 3);
    assert_eq!(code.matches("<td ").count(), 6);
    assert!(code.contains("border-top:1px solid #000000") && code.contains("border-left:1px solid #000000"), "its lines: {code}");
    assert!(code.to_ascii_lowercase().contains("bgcolor=\"#ffff00\""), "the headings' colour: {code}");
    assert!(code.contains("<b>Item</b>") && code.contains("<b>Price</b>"), "the headings bold: {code}");
    assert!(code.contains(" align=\"right\""), "the prices to the right: {code}");
    for gone in ["class=", "xl65", "<style", "mso-", "ProgId", "StartFragment"] {
        assert!(!code.contains(gone), "{gone} left in: {code}");
    }
    // Laid out and written again (a draft opened again): the same table, the same text.
    let again = pasted_tables(code).expect("a table");
    assert_eq!(tables(&again), vec![t]);
}

/// Cells copied in a real Excel (Microsoft 365, Windows), as it puts them on the clipboard: its
/// style sheet in a comment, with `@page` and its defaults for every cell; the colour by name; a
/// cell's own style taking a line of its class away; two cells merged. (Its links to its own
/// files on the computer that copied are left out.)
const REAL_EXCEL: &str = r##"<html xmlns:o="urn:schemas-microsoft-com:office:office"
xmlns:x="urn:schemas-microsoft-com:office:excel"
xmlns="http://www.w3.org/TR/REC-html40">
<head>
<meta http-equiv=Content-Type content="text/html; charset=utf-8">
<meta name=ProgId content=Excel.Sheet>
<meta name=Generator content="Microsoft Excel 15">
<style>
<!--table
	{mso-displayed-decimal-separator:"\.";
	mso-displayed-thousand-separator:"\,";}
@page
	{margin:.75in .7in .75in .7in;
	mso-header-margin:.3in;
	mso-footer-margin:.3in;}
tr
	{mso-height-source:auto;}
col
	{mso-width-source:auto;}
br
	{mso-data-placement:same-cell;}
td
	{padding-top:1px;
	padding-right:1px;
	padding-left:1px;
	mso-ignore:padding;
	color:black;
	font-size:11.0pt;
	font-weight:400;
	font-style:normal;
	text-decoration:none;
	font-family:"Aptos Narrow", sans-serif;
	mso-font-charset:0;
	mso-number-format:General;
	text-align:general;
	vertical-align:bottom;
	border:none;
	mso-background-source:auto;
	mso-pattern:auto;
	mso-protection:locked visible;
	white-space:nowrap;
	mso-rotate:0;}
.xl65
	{border:.5pt solid windowtext;}
.xl66
	{font-weight:700;
	border:.5pt solid windowtext;
	background:yellow;
	mso-pattern:black none;}
.xl67
	{mso-number-format:Fixed;
	border:.5pt solid windowtext;}
.xl68
	{font-style:italic;
	text-align:right;
	border-top:.5pt solid windowtext;
	border-right:none;
	border-bottom:.5pt solid windowtext;
	border-left:.5pt solid windowtext;}
.xl70
	{font-style:italic;
	mso-number-format:Fixed;
	border:.5pt solid windowtext;}
-->
</style>
</head>

<body link="#467886" vlink="#96607D">

<table border=0 cellpadding=0 cellspacing=0 width=192 style='border-collapse:
 collapse;width:144pt'>
<!--StartFragment-->
 <col width=64 span=3 style='width:48pt'>
 <tr height=19 style='height:14.5pt'>
  <td height=19 class=xl66 width=64 style='height:14.5pt;width:48pt'>Item</td>
  <td class=xl66 width=64 style='border-left:none;width:48pt'>Qty</td>
  <td class=xl66 width=64 style='border-left:none;width:48pt'>Price</td>
 </tr>
 <tr height=19 style='height:14.5pt'>
  <td height=19 class=xl65 style='height:14.5pt;border-top:none'>Paper</td>
  <td class=xl65 align=right style='border-top:none;border-left:none'>4</td>
  <td class=xl67 align=right style='border-top:none;border-left:none'>12.50</td>
 </tr>
 <tr height=19 style='height:14.5pt'>
  <td height=19 class=xl65 style='height:14.5pt;border-top:none'>Ink</td>
  <td class=xl65 align=right style='border-top:none;border-left:none'>2</td>
  <td class=xl67 align=right style='border-top:none;border-left:none'>8.00</td>
 </tr>
 <tr height=19 style='height:14.5pt'>
  <td colspan=2 height=19 class=xl68 style='border-right:.5pt solid black;
  height:14.5pt'>Total</td>
  <td class=xl70 align=right style='border-top:none;border-left:none'>20.50</td>
 </tr>
<!--EndFragment-->
</table>

</body>

</html>"##;

/// What a real Excel copies is a table as it looked there: every cell's lines, the headings bold
/// on yellow, the numbers and the merged total to the right, the total in italics.
#[test]
fn cells_from_a_real_excel_are_the_table_they_were() {
    let parts = pasted_tables(REAL_EXCEL).expect("a table");
    let found = tables(&parts);
    assert_eq!(found.len(), 1, "{parts:?}");
    let t = found[0];
    assert_eq!(t.text, "Item\tQty\tPrice\nPaper\t4\t12.50\nInk\t2\t8.00\nTotal\t20.50");
    let code = &t.code;
    assert_eq!(code.matches("<td ").count(), 11, "{code}");
    assert!(code.contains("colspan=\"2\""), "the total's two cells one: {code}");
    assert!(code.to_ascii_lowercase().contains("bgcolor=\"#ffff00\""), "the headings' yellow: {code}");
    assert!(code.contains("<b>Item</b>") && code.contains("<b>Price</b>"), "the headings bold: {code}");
    assert!(code.contains("<i>Total</i>") && code.contains("<i>20.50</i>"), "the total in italics: {code}");
    assert!(code.matches(" align=\"right\"").count() >= 5, "the numbers and the total to the right: {code}");
    // Every cell has its line all round: where Excel takes one away, the next cell's is there.
    let lines = code.matches("border-top:1px solid #000000").count() + code.matches("border-bottom:1px solid #000000").count();
    assert!(lines >= 11, "the lines: {code}");
    for gone in ["class=", "xl6", "<style", "mso-", "windowtext", "@page", "<col"] {
        assert!(!code.contains(gone), "{gone} left in: {code}");
    }
    let again = pasted_tables(code).expect("a table");
    assert_eq!(tables(&again), vec![t]);
}

/// The pasted HTML is hostile: nothing in it runs, links only to the web or mail, and its
/// pictures stay behind.
#[test]
fn nothing_in_a_pasted_table_runs_or_is_fetched() {
    let html = r#"<table><tr>
        <td onclick="steal()" style="background-image:url(https://x.example/t.gif)">A<script>steal()</script></td>
        <td><a href="javascript:steal()">B</a> <a href="https://shop.example/b">site</a></td>
        <td><img src="https://x.example/pixel.gif" onerror="steal()">C</td>
        <td><iframe src="https://x.example"></iframe>D</td>
    </tr></table>"#;
    let parts = pasted_tables(html).expect("a table");
    let code = &tables(&parts)[0].code;
    for gone in ["script", "steal", "onclick", "onerror", "javascript:", "<img", "pixel.gif", "t.gif", "iframe", "background"] {
        assert!(!code.contains(gone), "{gone} left in: {code}");
    }
    assert!(code.contains("href=\"https://shop.example/b\""), "a link on the web stays: {code}");
    assert_eq!(tables(&parts)[0].text, "A\tB site\tC\tD");
}

/// Text around a table comes with it, each in its place; HTML with no table is no table.
#[test]
fn text_around_a_table_comes_with_it() {
    let html = "<p>Prices for <b>December</b>:</p><table><tr><td>Paper</td><td>12.50</td></tr></table><p>Thanks,<br>Riley</p>";
    let parts = pasted_tables(html).expect("a table");
    let kinds: Vec<String> = parts
        .iter()
        .map(|p| match p {
            Pasted::Text(t, _) => format!("text {t}"),
            Pasted::Table(t) => format!("table {}", t.text),
        })
        .collect();
    assert_eq!(kinds, ["text Prices for December:", "table Paper\t12.50", "text Thanks,\nRiley"]);
    let Pasted::Text(_, styles) = &parts[0] else { panic!("text first") };
    assert!(styles.style(11).bold, "its formatting kept");
    assert_eq!(pasted_tables("<p>Hello <b>there</b></p><div>no table</div>"), None);
    assert_eq!(pasted_tables(""), None);
}

/// A table too big to keep as one is pasted as its text, as it was before (whether the layout or
/// the cap stops it); an empty one is nothing.
#[test]
fn a_table_too_big_is_its_text() {
    let row = format!("<tr>{}</tr>", "<td>x</td>".repeat(10));
    let big = format!("<table>{}</table>", row.repeat(TABLE_CELLS_MAX / 10 + 1));
    match pasted_tables(&big) {
        None => {}
        Some(parts) => {
            assert!(tables(&parts).is_empty(), "no table kept");
            assert!(matches!(parts.first(), Some(Pasted::Text(..))), "its text: {parts:?}");
        }
    }
    assert!(pasted_tables("<table><tr><td> </td></tr></table>").is_none_or(|p| tables(&p).is_empty()), "an empty one is none");
}

/// A table in a cell stays in it: one table, written with the one inside.
#[test]
fn a_table_in_a_cell_stays_in_it() {
    let html = "<table><tr><td>Outer</td><td><table><tr><td>In 1</td><td>In 2</td></tr></table></td></tr></table>";
    let parts = pasted_tables(html).expect("a table");
    let found = tables(&parts);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].code.matches("<table").count(), 2, "{}", found[0].code);
    assert_eq!(found[0].text, "Outer\tIn 1 In 2");
}

/// A table written collapsed, as MailCraft writes one, has each line two cells share once: a
/// table of `border=1` (each cell lined all round) read again is the same, and drawn as sent.
#[test]
fn a_pasted_tables_shared_lines_are_written_once() {
    let parts = pasted_tables("<table border=1><tr><td>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></table>").expect("a table");
    let code = &tables(&parts)[0].code;
    assert_eq!(code.matches("border-left:").count(), 2, "the first column's: {code}");
    assert_eq!(code.matches("border-top:").count(), 2, "the first row's: {code}");
    assert_eq!(code.matches("border-right:").count(), 4, "{code}");
    let again = pasted_tables(code).expect("a table");
    assert_eq!(&tables(&again)[0].code, code);
}
