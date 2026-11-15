use super::*;
use std::borrow::Cow;

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

/// Relationships that lead to no part there is (written wrongly, or by a path not followed)
/// leave the sheets and slides read by number, as a package without them: the preview was an
/// empty workbook. Of two relationships with one id, the first is followed.
#[test]
fn relationships_that_lead_nowhere_leave_the_parts_by_number() {
    let wb = r#"<workbook><sheets><sheet name="Costs" sheetId="1" r:id="rId1"/><sheet name="Days" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
    let rels = r#"<Relationships><Relationship Id="rId1" Target="sheets/one.xml"/><Relationship Id="rId2" Target="sheets/two.xml"/></Relationships>"#;
    let sheet =
        |t: &str| format!(r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{t}</t></is></c></row></sheetData></worksheet>"#);
    let (one, two) = (sheet("one"), sheet("two"));
    let file =
        zip(&[("xl/workbook.xml", wb), ("xl/_rels/workbook.xml.rels", rels), ("xl/worksheets/sheet1.xml", &one), ("xl/worksheets/sheet2.xml", &two)]);
    let Some(Preview::Workbook(sheets)) = preview("x.xlsx", "", &file) else { panic!() };
    let read: Vec<(&str, &str)> = sheets.iter().map(|(n, g)| (n.as_str(), g[0][0].as_str())).collect();
    assert_eq!(read, [("Costs", "one"), ("Days", "two")]);
    let pres = r#"<p:presentation><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>"#;
    let rels = r#"<Relationships><Relationship Id="rId1" Target="elsewhere/slide.xml"/></Relationships>"#;
    let slide = "<p:sld><p:txBody><a:p><a:r><a:t>first</a:t></a:r></a:p></p:txBody></p:sld>";
    let file = zip(&[("ppt/presentation.xml", pres), ("ppt/_rels/presentation.xml.rels", rels), ("ppt/slides/slide1.xml", slide)]);
    assert_eq!(preview("d.pptx", "", &file), Some(Preview::Slides(vec![vec!["first".into()]])));
    // One id twice: the first.
    let wb = r#"<workbook><sheets><sheet name="Only" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let rels = r#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/><Relationship Id="rId1" Target="worksheets/sheet2.xml"/></Relationships>"#;
    let file =
        zip(&[("xl/workbook.xml", wb), ("xl/_rels/workbook.xml.rels", rels), ("xl/worksheets/sheet1.xml", &one), ("xl/worksheets/sheet2.xml", &two)]);
    let Some(Preview::Workbook(sheets)) = preview("x.xlsx", "", &file) else { panic!() };
    assert_eq!(sheets.iter().map(|(n, g)| (n.as_str(), g[0][0].as_str())).collect::<Vec<_>>(), [("Only", "one")]);
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
fn kinds_by_name_or_type() {
    assert_eq!(kind("Plan.DOCM", ""), Some(Kind::Word));
    assert_eq!(kind("x", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"), Some(Kind::Excel));
    assert_eq!(kind("deck.pptx", "application/octet-stream"), Some(Kind::PowerPoint));
    assert_eq!(kind("notes.txt", "text/plain"), None);
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

/// The pieces of `xml` as they were read before they were read one at a time: all of them into
/// a list first, each text its own copy.
fn tokens_as_they_were(xml: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut rest = xml;
    while !rest.is_empty() && out.len() < 2_000_000 {
        match rest.find('<') {
            Some(0) => {
                let Some(close) = rest.find('>') else { break };
                let tag = rest.get(1..close).unwrap_or_default();
                rest = rest.get(close + 1..).unwrap_or_default();
                if tag.starts_with('?') || tag.starts_with('!') {
                    continue;
                }
                if let Some(name) = tag.strip_prefix('/') {
                    out.push(Tok::End(name.trim()));
                    continue;
                }
                let empty = tag.ends_with('/');
                let tag = tag.trim_end_matches('/');
                let (name, attrs) = tag.split_once(|c: char| c.is_whitespace()).unwrap_or((tag, ""));
                out.push(Tok::Start { name, attrs, empty });
            }
            Some(i) => {
                out.push(Tok::Text(Cow::Owned(unescape(rest.get(..i).unwrap_or_default()).into_owned())));
                rest = rest.get(i..).unwrap_or_default();
            }
            None => {
                out.push(Tok::Text(Cow::Owned(unescape(rest).into_owned())));
                break;
            }
        }
    }
    out
}

/// What XML (or not) is made of, for [`pieces_are_those_read_all_at_once`].
const XML_BITS: &[&str] = &[
    "<", ">", "</", "/>", "/", "?", "!", " ", "\t", "\"", "=", "w:t", "a", "&", ";", "&amp;", "&#65;", "&#x42;", "&bad;", "<!--", "-->", "<?x?>", "é",
];

proptest::proptest! {
    /// Read a piece at a time, any XML (or not) gives the pieces it gave read all at once.
    #[test]
    fn pieces_are_those_read_all_at_once(pieces in proptest::collection::vec(proptest::sample::select(XML_BITS), 0..60)) {
        let xml = pieces.concat();
        proptest::prop_assert_eq!(tokens(&xml).collect::<Vec<_>>(), tokens_as_they_were(&xml));
    }
}

/// A sheet longer than is shown: its rows up to the last shown, the rest not read (rows come in
/// order).
#[test]
fn a_long_sheet_shows_its_first_rows() {
    let mut s1 = String::from("<worksheet><sheetData>");
    for r in 1..=MAX_ROWS + 50 {
        s1.push_str(&format!("<row r=\"{r}\"><c r=\"A{r}\"><v>{r}</v></c></row>"));
    }
    s1.push_str("</sheetData></worksheet>");
    let grid = sheet(&s1, &[]);
    assert_eq!(grid.len(), MAX_ROWS);
    assert_eq!(grid.last().map(|r| r[0].as_str()), Some("500"));
}

/// What a large part costs to preview: a document of many more paragraphs than are shown, and a
/// sheet of many more rows.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_large_parts() {
    let para = "<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>A paragraph of a long report &amp; more</w:t></w:r></w:p>";
    let doc = para.repeat((30 << 20) / para.len());
    let file = zip(&[("word/document.xml", &doc)]);
    let started = std::time::Instant::now();
    let Some(Preview::Document(b)) = preview("a.docx", "", &file) else { panic!("unreadable") };
    println!("MEASURE a {} MB document, {} paragraphs shown: {} ms", doc.len() >> 20, b.len(), started.elapsed().as_millis());
    let mut s1 = String::from("<worksheet><sheetData>");
    let mut r = 0usize;
    while s1.len() < 30 << 20 {
        r += 1;
        s1.push_str(&format!("<row r=\"{r}\"><c r=\"A{r}\"><v>{r}</v></c><c r=\"B{r}\" t=\"inlineStr\"><is><t>row {r}</t></is></c></row>"));
    }
    s1.push_str("</sheetData></worksheet>");
    let wb = r#"<workbook><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let file = zip(&[("xl/workbook.xml", wb), ("xl/worksheets/sheet1.xml", &s1)]);
    let started = std::time::Instant::now();
    let Some(Preview::Workbook(sheets)) = preview("x.xlsx", "", &file) else { panic!("unreadable") };
    println!("MEASURE a {} MB sheet of {r} rows, {} shown: {} ms", s1.len() >> 20, sheets[0].1.len(), started.elapsed().as_millis());
}
