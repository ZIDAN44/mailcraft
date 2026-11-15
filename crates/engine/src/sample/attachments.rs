//! The sample's attached files, made in code: a Word document, a workbook, a PDF and pictures.

/// A zip of these parts, stored uncompressed (APPNOTE 4.3), as an Office file's package.
fn stored_zip(parts: &[(&str, String)]) -> Vec<u8> {
    let (mut out, mut dir) = (Vec::new(), Vec::new());
    let n16 = |n: usize| u16::try_from(n).unwrap_or(u16::MAX).to_le_bytes();
    let n32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes();
    for (name, body) in parts {
        let (crc, at) = (crc32(body.as_bytes()).to_le_bytes(), n32(out.len()));
        // Version 2.0, no flags, stored, 1980-01-01 00:00.
        let common = [&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0][..], &crc, &n32(body.len()), &n32(body.len()), &n16(name.len())].concat();
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&common);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(body.as_bytes());
        dir.extend_from_slice(b"PK\x01\x02\x14\0");
        dir.extend_from_slice(&common);
        dir.extend_from_slice(&[0; 12]);
        dir.extend_from_slice(&at);
        dir.extend_from_slice(name.as_bytes());
    }
    let at = n32(out.len());
    out.extend_from_slice(&dir);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&n16(parts.len()));
    out.extend_from_slice(&n16(parts.len()));
    out.extend_from_slice(&n32(dir.len()));
    out.extend_from_slice(&at);
    out.extend_from_slice(&[0, 0]);
    out
}

const XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;

/// The sample's Word document: an order confirmation with a heading, bold text and a table.
pub fn sample_docx() -> Vec<u8> {
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let p = |text: &str| format!("<w:p><w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p>");
    let cell = |text: &str, bold: bool| {
        let rpr = if bold { "<w:rPr><w:b/></w:rPr>" } else { "" };
        format!("<w:tc><w:tcPr><w:tcW w:w=\"3000\" w:type=\"dxa\"/></w:tcPr><w:p><w:r>{rpr}<w:t>{text}</w:t></w:r></w:p></w:tc>")
    };
    let row = |cells: [&str; 3], bold: bool| format!("<w:tr>{}</w:tr>", cells.iter().map(|c| cell(c, bold)).collect::<String>());
    let borders = r#"<w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/></w:tblBorders>"#;
    let body = [
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="32"/></w:rPr><w:t>Order 4471: festival poster</w:t></w:r></w:p>"#.to_string(),
        p("Thank you for your order. Here is what we will print once you approve the proof."),
        r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">Delivery: </w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>Friday, to the studio front desk</w:t></w:r></w:p>"#.to_string(),
        format!("<w:tbl><w:tblPr><w:tblW w:w=\"9000\" w:type=\"dxa\"/>{borders}</w:tblPr>{}{}{}</w:tbl>", row(["Item", "Size", "Copies"], true), row(["Poster, matte", "A2", "150"], false), row(["Poster, matte", "A3", "300"], false)),
        p("Print Shop"),
    ]
    .concat();
    stored_zip(&[
        (
            "[Content_Types].xml",
            format!(
                r#"{XML}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#
            ),
        ),
        (
            "_rels/.rels",
            format!(
                r#"{XML}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        ("word/document.xml", format!(r#"{XML}<w:document xmlns:w="{w}"><w:body>{body}</w:body></w:document>"#)),
    ])
}

/// The sample's Excel workbook: the order's lines and a total.
pub fn sample_xlsx() -> Vec<u8> {
    let s = |r: &str, t: &str| format!(r#"<c r="{r}" t="inlineStr"><is><t>{t}</t></is></c>"#);
    let n = |r: &str, v: &str| format!(r#"<c r="{r}"><v>{v}</v></c>"#);
    let rows = [
        format!(r#"<row r="1">{}{}{}{}</row>"#, s("A1", "Item"), s("B1", "Size"), s("C1", "Copies"), s("D1", "Price")),
        format!(r#"<row r="2">{}{}{}{}</row>"#, s("A2", "Poster, matte"), s("B2", "A2"), n("C2", "150"), n("D2", "412.5")),
        format!(r#"<row r="3">{}{}{}{}</row>"#, s("A3", "Poster, matte"), s("B3", "A3"), n("C3", "300"), n("D3", "390")),
        format!(r#"<row r="4">{}{}</row>"#, s("A4", "Total"), r#"<c r="D4"><f>SUM(D2:D3)</f><v>802.5</v></c>"#),
    ]
    .concat();
    stored_zip(&[
        (
            "[Content_Types].xml",
            format!(
                r#"{XML}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#
            ),
        ),
        (
            "_rels/.rels",
            format!(
                r#"{XML}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/workbook.xml",
            format!(
                r#"{XML}<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Order 4471" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!(
                r#"{XML}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/worksheets/sheet1.xml",
            format!(r#"{XML}<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#),
        ),
    ])
}

/// A one-page PDF drawn in code (a poster proof: sky, sun, hills and a title), so the sample
/// carries a real PDF.
pub fn sample_pdf() -> Vec<u8> {
    // The sun: a circle from four Bézier curves.
    let (cx, cy, r) = (210.0, 380.0, 80.0);
    let k = 0.5523 * r;
    let sun = format!(
        "{} {cy} m {} {} {} {} {cx} {} c {} {} {} {} {} {cy} c {} {} {} {} {cx} {} c {} {} {} {} {} {cy} c f",
        cx + r,
        cx + r,
        cy + k,
        cx + k,
        cy + r,
        cy + r,
        cx - k,
        cy + r,
        cx - r,
        cy + k,
        cx - r,
        cx - r,
        cy - k,
        cx - k,
        cy - r,
        cy - r,
        cx + k,
        cy - r,
        cx + r,
        cy - k,
        cx + r,
    );
    let content = format!(
        "0.11 0.16 0.33 rg 0 0 420 595 re f\n0.98 0.6 0.2 rg {sun}\n0.2 0.55 0.5 rg 0 0 m 140 220 l 280 0 l f\n\
         0.13 0.42 0.45 rg 170 0 m 330 250 l 420 140 l 420 0 l f\n\
         BT /F1 40 Tf 1 1 1 rg 90 515 Td (FESTIVAL) Tj ET\nBT /F1 16 Tf 1 1 1 rg 150 485 Td (Proof 1 of 1) Tj ET"
    );
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 420 595] /Contents 4 0 R \
         /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >> >> >> >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

/// An 8×8 PNG drawn in code (a small checkerboard), so the sample carries a real image.
pub fn sample_png() -> Vec<u8> {
    png_of(8, 8, |x, y| if (x / 2 + y / 2) % 2 == 0 { [0x3B, 0x5B, 0xDB] } else { [0xF2, 0xF4, 0xFA] })
}

/// A wide banner drawn in code (deep teal into blue, with soft diagonal light), for a
/// newsletter's heading to sit on.
pub fn sample_banner_png() -> Vec<u8> {
    png_of(96, 24, |x, y| {
        let t = x as f32 / 95.0;
        let glow = if (x + 2 * y) % 24 < 6 { 14.0 } else { 0.0 };
        let mix = |a: f32, b: f32| (a + (b - a) * t + glow).clamp(0.0, 255.0) as u8;
        [mix(31.0, 27.0), mix(110.0, 63.0), mix(94.0, 140.0)]
    })
}

/// An RGB PNG of `w`×`h` from a colour per pixel (uncompressed: stored DEFLATE blocks).
fn png_of(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    // Uncompressed DEFLATE (stored blocks) keeps this dependency-free.
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0u8);
        for x in 0..w {
            raw.extend_from_slice(&pixel(x, y));
        }
    }
    let mut z = vec![0x78, 0x01];
    // A stored block holds at most 65,535 bytes; only the last is marked final (an empty
    // picture still has one, empty).
    let blocks: Vec<&[u8]> = if raw.is_empty() { vec![&[]] } else { raw.chunks(usize::from(u16::MAX)).collect() };
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = u16::try_from(block.len()).unwrap_or(u16::MAX);
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let chunk = |out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
        let mut c = Vec::with_capacity(4 + data.len());
        c.extend_from_slice(ty);
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pictures drawn in code are PNGs a decoder reads as drawn: their chunks' CRCs, the
    /// zlib stream and its Adler-32 (which decoders may skip, so checked on its own), the size
    /// and the pixels; one bigger than a stored DEFLATE block holds (65,535 bytes) too.
    #[test]
    fn pngs_are_well_formed() {
        let checker = |x: u32, y: u32| if (x / 2 + y / 2).is_multiple_of(2) { [0x3B, 0x5B, 0xDB] } else { [0xF2, 0xF4, 0xFA] };
        let ramp = |x: u32, y: u32| [x as u8, y as u8, (x ^ y) as u8];
        let pictures: [(Vec<u8>, (u32, u32), Option<&dyn Fn(u32, u32) -> [u8; 3]>); 3] = [
            (sample_png(), (8, 8), Some(&checker)),
            (sample_banner_png(), (96, 24), None),
            // 160 rows of 481 bytes: 76,960, in two blocks.
            (png_of(160, 160, ramp), (160, 160), Some(&ramp)),
        ];
        for (png, (w, h), drawn) in pictures {
            let picture = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().to_rgb8();
            assert_eq!(picture.dimensions(), (w, h));
            if let Some(drawn) = drawn {
                assert!(picture.enumerate_pixels().all(|(x, y, p)| p.0 == drawn(x, y)), "{w}×{h}");
            }
            let mut rest = png.get(8..).unwrap();
            let mut idat = Vec::new();
            while rest.len() >= 12 {
                let len = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
                if &rest[4..8] == b"IDAT" {
                    idat.extend_from_slice(&rest[8..8 + len]);
                }
                rest = &rest[12 + len..];
            }
            let mut raw = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&idat[..]), &mut raw).unwrap();
            assert_eq!(raw.len() as u32, h * (1 + 3 * w));
        }
    }
}
