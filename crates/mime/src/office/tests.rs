use super::*;

/// A zip of these parts, stored (no compression), as a test archive.
pub(crate) fn zip(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut dir = Vec::new();
    for (name, body) in parts {
        let local = u32::try_from(out.len()).unwrap();
        let len = u32::try_from(body.len()).unwrap();
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(body.as_bytes());
        dir.extend_from_slice(b"PK\x01\x02");
        dir.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        dir.extend_from_slice(&[0, 0, 0, 0]);
        dir.extend_from_slice(&len.to_le_bytes());
        dir.extend_from_slice(&len.to_le_bytes());
        dir.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
        dir.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        dir.extend_from_slice(&local.to_le_bytes());
        dir.extend_from_slice(name.as_bytes());
    }
    let at = u32::try_from(out.len()).unwrap();
    let n = u16::try_from(parts.len()).unwrap();
    out.extend_from_slice(&dir);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&u32::try_from(dir.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&at.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

#[test]
fn a_word_document() {
    let doc = r#"<?xml version="1.0"?><w:document xmlns:w="x"><w:body>
        <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Plan &amp; budget</w:t></w:r></w:p>
        <w:p><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">Bold </w:t></w:r><w:r><w:rPr><w:i/><w:b w:val="0"/></w:rPr><w:t>then italic</w:t></w:r></w:p>
        <w:tbl><w:tr><w:tc><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B1</w:t></w:r></w:p><w:p><w:r><w:t>more</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
        <w:p/></w:body></w:document>"#;
    let p = preview("plan.docx", "", &zip(&[("word/document.xml", doc)])).unwrap();
    let Preview::Document(b) = p else { panic!("not a document") };
    assert_eq!(b.len(), 3, "{b:?}");
    assert!(matches!(&b[0], Block::Paragraph { runs, heading: true, .. } if runs[0].text == "Plan & budget"));
    let Block::Paragraph { runs, .. } = &b[1] else { panic!() };
    assert_eq!((runs[0].bold, runs[1].bold, runs[1].italic, runs[0].text.as_str()), (true, false, true, "Bold "));
    assert_eq!(b[2], Block::Table(vec![vec!["A1".into(), "B1\nmore".into()]]));
}

#[test]
fn a_workbook_and_slides() {
    let wb = r#"<workbook><sheets><sheet name="Costs &amp; days" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let ss = r#"<sst><si><t>Item</t></si><si><r><t>Pos</t></r><r><t>ter</t></r></si></sst>"#;
    let s1 = r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1"><v>42</v></c></row><row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2" t="b"><v>1</v></c><c r="D2" t="inlineStr"><is><t>here</t></is></c></row></sheetData></worksheet>"#;
    let p = preview("x.xlsx", "", &zip(&[("xl/workbook.xml", wb), ("xl/sharedStrings.xml", ss), ("xl/worksheets/sheet1.xml", s1)])).unwrap();
    let Preview::Workbook(sheets) = p else { panic!() };
    assert_eq!(sheets[0].0, "Costs & days");
    assert_eq!(
        sheets[0].1,
        vec![vec!["Item".to_string(), String::new(), "42".into()], vec!["Poster".into(), "TRUE".into(), String::new(), "here".into()]]
    );
    let slide = r#"<p:sld><p:txBody><a:p><a:r><a:t>Title</a:t></a:r></a:p><a:p><a:r><a:t>Point one</a:t></a:r><a:br/><a:r><a:t>two</a:t></a:r></a:p></p:txBody></p:sld>"#;
    let p = preview("deck.pptx", "", &zip(&[("ppt/slides/slide1.xml", slide), ("ppt/slides/slide2.xml", "<p:sld/>")])).unwrap();
    assert_eq!(p, Preview::Slides(vec![vec!["Title".into(), "Point one\ntwo".into()], vec![]]));
}

/// Sheets and slides as the package lists them, each by its relationship to its part: a gap
/// in the parts' numbers, or an order other than theirs. Read by number, sheet3 (after a
/// missing sheet2) was never shown, and the first part took the first name.
#[test]
fn sheets_and_slides_follow_their_relationships() {
    let wb = r#"<workbook><sheets><sheet name="Summary" sheetId="1" r:id="rId3"/><sheet name="Data" sheetId="2" r:id="rId1"/></sheets></workbook>"#;
    let rels = r#"<Relationships><Relationship Id="rId1" Type="worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="styles" Target="styles.xml"/><Relationship Id="rId3" Type="worksheet" Target="/xl/worksheets/sheet3.xml"/></Relationships>"#;
    let data = r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>data</t></is></c></row></sheetData></worksheet>"#;
    let summary = r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>summary</t></is></c></row></sheetData></worksheet>"#;
    let file = zip(&[
        ("xl/workbook.xml", wb),
        ("xl/_rels/workbook.xml.rels", rels),
        ("xl/worksheets/sheet1.xml", data),
        ("xl/worksheets/sheet3.xml", summary),
    ]);
    let Some(Preview::Workbook(sheets)) = preview("x.xlsx", "", &file) else { panic!() };
    let read: Vec<(&str, &str)> = sheets.iter().map(|(n, g)| (n.as_str(), g[0][0].as_str())).collect();
    assert_eq!(read, [("Summary", "summary"), ("Data", "data")]);
    let pres = r#"<p:presentation><p:sldIdLst><p:sldId id="256" r:id="rId7"/><p:sldId id="257" r:id="rId2"/></p:sldIdLst></p:presentation>"#;
    let rels =
        r#"<Relationships><Relationship Id="rId2" Target="slides/slide1.xml"/><Relationship Id="rId7" Target="slides/slide4.xml"/></Relationships>"#;
    let slide = |t: &str| format!("<p:sld><p:txBody><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:txBody></p:sld>");
    let (first, second) = (slide("first"), slide("second"));
    let file = zip(&[
        ("ppt/presentation.xml", pres),
        ("ppt/_rels/presentation.xml.rels", rels),
        ("ppt/slides/slide1.xml", &second),
        ("ppt/slides/slide4.xml", &first),
    ]);
    assert_eq!(preview("d.pptx", "", &file), Some(Preview::Slides(vec![vec!["first".into()], vec!["second".into()]])));
}

#[test]
fn deflated_parts_as_office_writes_them() {
    use std::io::Write;
    let body = "<w:p><w:r><w:t>Compressed hello</w:t></w:r></w:p>".repeat(50);
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(body.as_bytes()).unwrap();
    let packed = enc.finish().unwrap();
    // The stored archive, its one entry swapped for the deflated bytes (method 8).
    let name = "word/document.xml";
    let mut out = Vec::new();
    out.extend_from_slice(b"PK\x03\x04\x14\0\0\0\x08\0\0\0\0\0\0\0\0\0");
    out.extend_from_slice(&u32::try_from(packed.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&packed);
    let at = u32::try_from(out.len()).unwrap();
    let mut dir = Vec::new();
    dir.extend_from_slice(b"PK\x01\x02\x14\0\x14\0\0\0\x08\0\0\0\0\0\0\0\0\0");
    dir.extend_from_slice(&u32::try_from(packed.len()).unwrap().to_le_bytes());
    dir.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
    dir.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
    dir.extend_from_slice(&[0; 12]);
    dir.extend_from_slice(&0u32.to_le_bytes());
    dir.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&dir);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\0\x01\0");
    out.extend_from_slice(&u32::try_from(dir.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&at.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    let Some(Preview::Document(b)) = preview("a.docx", "", &out) else { panic!("unreadable") };
    assert_eq!(b.len(), 50);
}

#[test]
fn junk_is_no_preview() {
    assert_eq!(preview("a.docx", "", b"not a zip"), None);
    assert_eq!(preview("a.docx", "", &zip(&[("other.xml", "<x/>")])), None);
    assert_eq!(preview("a.txt", "", &zip(&[("word/document.xml", "<x/>")])), None);
    // Truncated anywhere: nothing panics.
    let good = zip(&[("word/document.xml", "<w:p><w:r><w:t>hi</w:t></w:r></w:p>")]);
    for n in 0..good.len() {
        let _ = preview("a.docx", "", &good[..n]);
    }
    assert_eq!(unescape("a &lt;b&gt; &#65;&#x42; &bogus; &"), "a <b> AB &bogus; &");
    assert_eq!(cell_ref("AB12"), Some((11, 27)));
    assert_eq!(cell_ref("1"), None); // Rows and cells numbered at the end of what a number holds, then ones numbered after
    // them: nothing overflows (a row after `r="18446744073709551615"` did, building it in
    // the background each time).
    let top = format!("{}", usize::MAX);
    let s1 =
        format!("<worksheet><sheetData><row r=\"{top}\"><c><v>1</v></c></row><row><c><v>2</v></c><c r=\"XFD1\"/><c/></row></sheetData></worksheet>");
    let wb = r#"<workbook><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let _ = preview("x.xlsx", "", &zip(&[("xl/workbook.xml", wb), ("xl/worksheets/sheet1.xml", &s1)]));
}
