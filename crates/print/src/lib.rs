//! A message on paper: laid out on A4 pages and written as a PDF (ISO 32000).
//!
//! The installed system font is embedded when the app has one (a TrueType font, as a CID font
//! with identity encoding and a Unicode map, so the text can be searched and copied); without
//! one the standard Helvetica is used and characters outside Windows-1252 print as `?`. Pages
//! carry the subject, the header, the text and page numbers. Pure Rust, no I/O: the caller
//! writes the bytes.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// What goes on the pages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Doc {
    /// Printed large at the top (the subject).
    pub title: String,
    /// `From`, `To`, `Date`… lines under it.
    pub header: Vec<(String, String)>,
    /// The text.
    pub body: String,
}

/// Font files to embed (TrueType or OpenType with TrueType outlines).
#[derive(Clone, Copy, Debug)]
pub struct Fonts<'a> {
    pub regular: &'a [u8],
    pub bold: Option<&'a [u8]>,
}

const PAGE_W: f32 = 595.28;
const PAGE_H: f32 = 841.89;
const MARGIN: f32 = 56.0;
/// Most pages written (a very long message is cut, with a note).
const MAX_PAGES: usize = 500;

/// A font as the layout and the writer see it.
enum Face<'a> {
    Embedded { data: &'a [u8], face: Box<ttf_parser::Face<'a>>, used: BTreeMap<u16, char> },
    Helvetica { bold: bool },
}

impl<'a> Face<'a> {
    fn parse(data: &'a [u8]) -> Option<Face<'a>> {
        let face = ttf_parser::Face::parse(data, 0).ok()?;
        // CFF outlines can't go in a FontFile2 stream.
        if !face.tables().glyf.is_some() {
            return None;
        }
        Some(Face::Embedded { data, face: Box::new(face), used: BTreeMap::new() })
    }

    /// Width of a character in thousandths of the font size.
    fn width(&self, c: char) -> f32 {
        match self {
            Face::Embedded { face, .. } => {
                let upem = f32::from(face.units_per_em().max(1));
                let gid = face.glyph_index(c).unwrap_or(ttf_parser::GlyphId(0));
                f32::from(face.glyph_hor_advance(gid).unwrap_or(0)) * 1000.0 / upem
            }
            Face::Helvetica { bold } => helvetica_width(c, *bold),
        }
    }

    fn text_width(&self, s: &str, size: f32) -> f32 {
        s.chars().map(|c| self.width(c)).sum::<f32>() * size / 1000.0
    }

    /// The bytes of a PDF string drawing `s` with this font.
    fn encode(&mut self, s: &str) -> String {
        match self {
            Face::Embedded { face, used, .. } => {
                let mut hex = String::with_capacity(s.len() * 4 + 2);
                hex.push('<');
                for c in s.chars() {
                    let gid = face.glyph_index(c).map_or(0, |g| g.0);
                    used.entry(gid).or_insert(c);
                    let _ = write!(hex, "{gid:04X}");
                }
                hex.push('>');
                hex
            }
            Face::Helvetica { .. } => {
                let mut lit = String::from("(");
                for c in s.chars() {
                    match win_ansi(c) {
                        b'(' | b')' | b'\\' => {
                            lit.push('\\');
                            lit.push(char::from(win_ansi(c)));
                        }
                        b if b >= 0x80 => {
                            let _ = write!(lit, "\\{b:03o}");
                        }
                        b => lit.push(char::from(b)),
                    }
                }
                lit.push(')');
                lit
            }
        }
    }
}

/// Windows-1252 code of a character (`?` when it has none).
fn win_ansi(c: char) -> u8 {
    match c {
        ' '..='~' => c as u8,
        '\u{A0}'..='\u{FF}' => u8::try_from(u32::from(c)).unwrap_or(b'?'),
        '€' => 0x80,
        '‚' => 0x82,
        '„' => 0x84,
        '…' => 0x85,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        '™' => 0x99,
        _ => b'?',
    }
}

/// Helvetica's widths (the standard metrics of the PDF base fonts) for printable ASCII; other
/// characters get the width of `n`.
fn helvetica_width(c: char, bold: bool) -> f32 {
    const REGULAR: [u16; 95] = [
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278,
        584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944,
        667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500,
        278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
    ];
    const BOLD: [u16; 95] = [
        278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333,
        584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944,
        667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556,
        333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
    ];
    let table = if bold { &BOLD } else { &REGULAR };
    let i = (c as usize).wrapping_sub(32);
    f32::from(table.get(i).copied().unwrap_or(556))
}

/// A line of text to place: font (0 regular, 1 bold), size, position.
struct Placed {
    font: usize,
    size: f32,
    x: f32,
    y: f32,
    text: String,
}

/// Split `text` into lines no wider than `width` (words, then characters for long words).
fn wrap(face: &Face<'_>, text: &str, size: f32, width: f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.replace("\r\n", "\n").split('\n') {
        let para = para.replace('\t', "    ");
        let mut line = String::new();
        // Keep leading spaces (quoting, indentation).
        let lead: String = para.chars().take_while(|c| *c == ' ').collect();
        let words: Vec<&str> = para.split(' ').filter(|w| !w.is_empty()).collect();
        if words.is_empty() {
            out.push(String::new());
            continue;
        }
        line.push_str(&lead);
        for w in words {
            let candidate = if line.trim().is_empty() { format!("{line}{w}") } else { format!("{line} {w}") };
            if face.text_width(&candidate, size) <= width {
                line = candidate;
                continue;
            }
            if !line.trim().is_empty() {
                out.push(std::mem::take(&mut line));
            }
            // A word wider than the line is cut by characters.
            let mut piece = String::new();
            for c in w.chars() {
                piece.push(c);
                if face.text_width(&piece, size) > width && piece.chars().count() > 1 {
                    let last = piece.pop().unwrap_or(' ');
                    out.push(std::mem::take(&mut piece));
                    piece.push(last);
                }
            }
            line = piece;
        }
        out.push(line);
    }
    out
}

fn compress(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, 6)
}

/// PDF text-string literal of `s` (for the document info), escaped; non-ASCII as `?`.
fn info_string(s: &str) -> String {
    let mut out = String::from("(");
    for c in s.chars().take(200) {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            ' '..='~' => out.push(c),
            _ => out.push('?'),
        }
    }
    out.push(')');
    out
}

/// The document as PDF bytes.
pub fn pdf(doc: &Doc, fonts: Option<Fonts<'_>>) -> Result<Vec<u8>, String> {
    let mut faces: Vec<Face<'_>> = match fonts.and_then(|f| Face::parse(f.regular).map(|r| (r, f.bold))) {
        Some((regular, bold)) => {
            let bold = bold.and_then(Face::parse);
            match bold {
                Some(b) => vec![regular, b],
                None => vec![regular, Face::Helvetica { bold: true }],
            }
        }
        None => vec![Face::Helvetica { bold: false }, Face::Helvetica { bold: true }],
    };
    let width = PAGE_W - 2.0 * MARGIN;

    // Lay out.
    let mut pages: Vec<Vec<Placed>> = vec![Vec::new()];
    let mut y = PAGE_H - MARGIN;
    let mut truncated = false;
    let place = |pages: &mut Vec<Vec<Placed>>, y: &mut f32, font: usize, size: f32, lead: f32, x: f32, text: String| -> bool {
        if *y - lead < MARGIN + 20.0 {
            if pages.len() >= MAX_PAGES {
                return false;
            }
            pages.push(Vec::new());
            *y = PAGE_H - MARGIN;
        }
        *y -= lead;
        if let Some(p) = pages.last_mut() {
            p.push(Placed { font, size, x, y: *y, text });
        }
        true
    };
    let title = if doc.title.trim().is_empty() { "(no subject)".to_string() } else { doc.title.clone() };
    for line in wrap(&faces[1], &title, 16.0, width) {
        place(&mut pages, &mut y, 1, 16.0, 21.0, MARGIN, line);
    }
    y -= 6.0;
    let label_w = doc.header.iter().map(|(k, _)| faces[1].text_width(&format!("{k}:"), 9.5)).fold(0.0f32, f32::max) + 6.0;
    for (k, v) in &doc.header {
        let lines = wrap(&faces[0], v, 9.5, width - label_w);
        for (i, line) in lines.into_iter().enumerate() {
            if i == 0 {
                let ok = place(&mut pages, &mut y, 1, 9.5, 13.0, MARGIN, format!("{k}:"));
                if ok && let Some(p) = pages.last_mut() {
                    p.push(Placed { font: 0, size: 9.5, x: MARGIN + label_w, y, text: line });
                }
            } else {
                place(&mut pages, &mut y, 0, 9.5, 13.0, MARGIN + label_w, line);
            }
        }
    }
    y -= 8.0;
    let rule_page = pages.len() - 1;
    let rule_y = y;
    y -= 6.0;
    for line in wrap(&faces[0], &doc.body, 11.0, width) {
        if !place(&mut pages, &mut y, 0, 11.0, 14.5, MARGIN, line) {
            truncated = true;
            break;
        }
    }
    if truncated {
        place(&mut pages, &mut y, 1, 11.0, 18.0, MARGIN, "(The rest of the message didn't fit in this document.)".into());
    }

    // Content streams.
    let n = pages.len();
    let mut contents: Vec<String> = Vec::with_capacity(n);
    for (i, page) in pages.iter().enumerate() {
        let mut c = String::new();
        if i == rule_page {
            let _ = writeln!(c, "0.75 G 0.6 w {MARGIN:.2} {rule_y:.2} m {:.2} {rule_y:.2} l S 0 G", PAGE_W - MARGIN);
        }
        c.push_str("BT\n");
        for p in page {
            let enc = faces.get_mut(p.font).map(|f| f.encode(&p.text)).unwrap_or_default();
            let _ = writeln!(c, "/F{} {:.1} Tf 1 0 0 1 {:.2} {:.2} Tm {enc} Tj", p.font + 1, p.size, p.x, p.y);
        }
        let footer = format!("{} / {n}", i + 1);
        let fw = faces[0].text_width(&footer, 8.0);
        let enc = faces[0].encode(&footer);
        let _ = writeln!(c, "0.45 g /F1 8.0 Tf 1 0 0 1 {:.2} {:.2} Tm {enc} Tj 0 g", (PAGE_W - fw) / 2.0, MARGIN / 2.0);
        c.push_str("ET\n");
        contents.push(c);
    }

    // Objects: 1 catalog, 2 pages, 3 info, then fonts, then page + content pairs.
    let mut objects: Vec<Vec<u8>> = Vec::new();
    let add = |objects: &mut Vec<Vec<u8>>, body: Vec<u8>| -> usize {
        objects.push(body);
        objects.len()
    };
    add(&mut objects, Vec::new()); // catalog, filled in last
    add(&mut objects, Vec::new()); // pages, filled in last
    add(&mut objects, format!("<< /Producer (MailCraft {}) /Title {} >>", env!("CARGO_PKG_VERSION"), info_string(&title)).into_bytes());
    let mut font_refs = Vec::new();
    for (i, f) in faces.iter().enumerate() {
        let r = match f {
            Face::Helvetica { bold } => add(
                &mut objects,
                format!(
                    "<< /Type /Font /Subtype /Type1 /BaseFont /{} /Encoding /WinAnsiEncoding >>",
                    if *bold { "Helvetica-Bold" } else { "Helvetica" }
                )
                .into_bytes(),
            ),
            Face::Embedded { data, face, used } => {
                let upem = f32::from(face.units_per_em().max(1));
                let scale = |v: i16| (f32::from(v) * 1000.0 / upem).round() as i32;
                let file = compress(data);
                let mut stream = format!("<< /Length {} /Length1 {} /Filter /FlateDecode >>\nstream\n", file.len(), data.len()).into_bytes();
                stream.extend_from_slice(&file);
                stream.extend_from_slice(b"\nendstream");
                let file_ref = add(&mut objects, stream);
                let bbox = face.global_bounding_box();
                let descriptor = format!(
                    "<< /Type /FontDescriptor /FontName /MailCraftFont{i} /Flags 32 /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV {} /FontFile2 {file_ref} 0 R >>",
                    scale(bbox.x_min),
                    scale(bbox.y_min),
                    scale(bbox.x_max),
                    scale(bbox.y_max),
                    scale(face.ascender()),
                    scale(face.descender()),
                    scale(face.capital_height().unwrap_or(face.ascender())),
                    if i == 1 { 140 } else { 80 }
                );
                let desc_ref = add(&mut objects, descriptor.into_bytes());
                let mut w = String::from("[");
                for gid in used.keys() {
                    let adv = face.glyph_hor_advance(ttf_parser::GlyphId(*gid)).unwrap_or(0);
                    let _ = write!(w, " {gid} [{}]", (f32::from(adv) * 1000.0 / upem).round() as i32);
                }
                w.push_str(" ]");
                let cid = format!(
                    "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /MailCraftFont{i} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {desc_ref} 0 R /DW 1000 /W {w} /CIDToGIDMap /Identity >>"
                );
                let cid_ref = add(&mut objects, cid.into_bytes());
                let mut cmap = String::from(
                    "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /Adobe-Identity-UCS def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
                );
                let pairs: Vec<(&u16, &char)> = used.iter().collect();
                for chunk in pairs.chunks(100) {
                    let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
                    for (gid, c) in chunk {
                        let mut units = [0u16; 2];
                        let hex: String = c.encode_utf16(&mut units).iter().map(|u| format!("{u:04X}")).collect();
                        let _ = writeln!(cmap, "<{gid:04X}> <{hex}>");
                    }
                    cmap.push_str("endbfchar\n");
                }
                cmap.push_str("endcmap CMapName currentdict /CMap defineresource pop end end");
                let packed = compress(cmap.as_bytes());
                let mut s = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", packed.len()).into_bytes();
                s.extend_from_slice(&packed);
                s.extend_from_slice(b"\nendstream");
                let tu_ref = add(&mut objects, s);
                add(
                    &mut objects,
                    format!("<< /Type /Font /Subtype /Type0 /BaseFont /MailCraftFont{i} /Encoding /Identity-H /DescendantFonts [{cid_ref} 0 R] /ToUnicode {tu_ref} 0 R >>")
                        .into_bytes(),
                )
            }
        };
        font_refs.push(r);
    }
    let fonts_dict: String = font_refs.iter().enumerate().map(|(i, r)| format!("/F{} {r} 0 R", i + 1)).collect::<Vec<_>>().join(" ");
    let mut kids = Vec::new();
    for c in &contents {
        let packed = compress(c.as_bytes());
        let mut s = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", packed.len()).into_bytes();
        s.extend_from_slice(&packed);
        s.extend_from_slice(b"\nendstream");
        let content_ref = add(&mut objects, s);
        let page = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W:.2} {PAGE_H:.2}] /Resources << /Font << {fonts_dict} >> >> /Contents {content_ref} 0 R >>"
        );
        kids.push(add(&mut objects, page.into_bytes()));
    }
    if let Some(o) = objects.get_mut(0) {
        *o = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    }
    if let Some(o) = objects.get_mut(1) {
        let k: String = kids.iter().map(|r| format!("{r} 0 R")).collect::<Vec<_>>().join(" ");
        *o = format!("<< /Type /Pages /Kids [{k}] /Count {} >>", kids.len()).into_bytes();
    }

    // The file: header, objects, cross-reference table, trailer.
    let mut out: Vec<u8> = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(body: &str) -> Doc {
        Doc {
            title: "Storyboard (v2) review".into(),
            header: vec![("From".into(), "Zoe Park <zoe@studio.example>".into()), ("Date".into(), "Wed 14 Oct 2026 09:30".into())],
            body: body.into(),
        }
    }

    /// Every object starts exactly where the cross-reference table says (on the bytes: the
    /// streams are binary).
    fn objects_are_where_the_xref_says(pdf: &[u8]) {
        let find_last = |needle: &[u8]| pdf.windows(needle.len()).rposition(|w| w == needle);
        let sx = find_last(b"startxref\n").unwrap() + 10;
        let tail = String::from_utf8_lossy(&pdf[sx..]);
        let start: usize = tail.lines().next().unwrap().trim().parse().unwrap();
        assert!(pdf[start..].starts_with(b"xref"));
        let table = String::from_utf8_lossy(&pdf[start..sx]).to_string();
        let mut n = 0;
        for (i, line) in table.lines().skip(3).take_while(|l| l.ends_with(" n ")).enumerate() {
            let off: usize = line[..10].parse().unwrap();
            assert!(pdf[off..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "object {} at {off}", i + 1);
            n += 1;
        }
        assert!(n >= 4);
    }

    #[test]
    fn helvetica_pages() {
        let long = (0..400).map(|i| format!("Line {i}: the quick brown fox jumps over the lazy dog (café, “quotes”)")).collect::<Vec<_>>().join("\n");
        let p = pdf(&doc(&long), None).unwrap();
        assert!(p.starts_with(b"%PDF-1.7"));
        assert!(p.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&p);
        assert!(text.contains("/BaseFont /Helvetica"));
        let count: usize = text.split("/Count ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap();
        assert!(count >= 8, "{count} pages");
        assert!(text.contains("/Title (Storyboard \\(v2\\) review)"));
        objects_are_where_the_xref_says(&p);
    }

    #[test]
    fn wrapping() {
        let f = Face::Helvetica { bold: false };
        let lines = wrap(&f, "a b c", 11.0, 1000.0);
        assert_eq!(lines, vec!["a b c".to_string()]);
        let lines = wrap(&f, &"word ".repeat(100), 11.0, 200.0);
        assert!(lines.len() > 5 && lines.iter().all(|l| f.text_width(l, 11.0) <= 200.0));
        let lines = wrap(&f, &"x".repeat(500), 11.0, 100.0);
        assert!(lines.len() > 5 && lines.iter().all(|l| f.text_width(l, 11.0) <= 100.0), "long words are cut");
        assert_eq!(wrap(&f, "> quoted\n\n  indented", 11.0, 400.0), vec!["> quoted".to_string(), String::new(), "  indented".to_string()]);
        assert_eq!(win_ansi('é'), 0xE9);
        assert_eq!(win_ansi('€'), 0x80);
        assert_eq!(win_ansi('漢'), b'?');
    }

    #[test]
    fn empty_and_hostile_input() {
        let p = pdf(&Doc::default(), None).unwrap();
        objects_are_where_the_xref_says(&p);
        // Not a font: falls back to Helvetica rather than failing.
        let p = pdf(&doc("x"), Some(Fonts { regular: b"not a font", bold: None })).unwrap();
        assert!(String::from_utf8_lossy(&p).contains("/Helvetica"));
        let huge = "y\n".repeat(200_000);
        let p = pdf(&doc(&huge), None).unwrap();
        assert!(String::from_utf8_lossy(&p).contains(&format!("/Count {MAX_PAGES}")));
    }

    #[test]
    fn embeds_a_system_font_when_there_is_one() {
        let candidates = [
            "C:\\Windows\\Fonts\\segoeui.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ];
        let Some(data) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else { return };
        let p = pdf(&doc("Hello café — ünïcödé"), Some(Fonts { regular: &data, bold: None })).unwrap();
        let text = String::from_utf8_lossy(&p);
        assert!(text.contains("/Subtype /CIDFontType2") && text.contains("/FontFile2") && text.contains("/ToUnicode"));
        assert!(p.len() < data.len(), "the font file is compressed");
        objects_are_where_the_xref_says(&p);
    }
}
