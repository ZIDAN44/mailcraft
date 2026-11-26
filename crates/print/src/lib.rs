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

mod blocks;
mod layout;
mod objects;
mod picture;
mod rich;
mod sheet;

pub use blocks::{Align, Block, Cell, HELD_BACK, Line, Table, VAlign, Width};
pub use picture::Picture;
pub use rich::Run;

/// What goes on the pages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Doc {
    /// Printed large at the top (the subject).
    pub title: String,
    /// `From`, `To`, `Date`… lines under it.
    pub header: Vec<(String, String)>,
    /// A line under those, when there's one to say (pictures from the web not loaded).
    pub note: String,
    /// Said at the end of a message too long for the pages (English when empty).
    pub cut: String,
    /// The text.
    pub body: String,
    /// The text with its formatting, a line at a time, for a message that has any: printed in
    /// place of `body`.
    pub rich: Vec<Vec<Run>>,
    /// The message as it's laid out on screen (its tables, their backgrounds and lines, its
    /// pictures), for one that is: printed in place of `rich`.
    pub blocks: Vec<Block>,
    /// The pictures `blocks` show ([`Run::picture`] names one by its place here).
    pub pictures: Vec<Picture>,
}

/// Font files to embed (TrueType or OpenType with TrueType outlines).
#[derive(Clone, Copy, Debug, Default)]
pub struct Fonts<'a> {
    pub regular: &'a [u8],
    pub bold: Option<&'a [u8]>,
    /// Fonts for what the regular and bold ones don't have (symbols, emoji, other scripts), the
    /// first that has a character drawing it, as on screen (issue #291: they printed as empty
    /// boxes). Each is embedded only when a character of the document needs it.
    pub fallbacks: &'a [&'a [u8]],
}

const PAGE_W: f32 = 595.28;
const PAGE_H: f32 = 841.89;
const MARGIN: f32 = 56.0;
/// Most pages written (a very long message is cut, with a note).
const MAX_PAGES: usize = 500;

/// A font as the layout and the writer see it. An embedded one measures what it doesn't have
/// with its `fallbacks`' metrics (the writer's own faces for them draw it).
enum Face<'a> {
    Embedded { data: &'a [u8], font: Box<Font<'a>>, used: BTreeMap<u16, char>, fallbacks: Vec<Font<'a>> },
    Helvetica { bold: bool },
}

/// What the layout and the writer read from a font file to embed (skrifa, in font units).
struct Font<'a> {
    charmap: skrifa::charmap::Charmap<'a>,
    advances: skrifa::metrics::GlyphMetrics<'a>,
    metrics: skrifa::metrics::Metrics,
}

impl Font<'_> {
    /// The glyph drawing `c` (0, the missing glyph, when the font has none: TrueType ids fit 16 bits).
    fn glyph(&self, c: char) -> u16 {
        let of = |c: char| self.charmap.map(c).and_then(|g| u16::try_from(g.to_u32()).ok()).unwrap_or(0);
        match of(c) {
            // The lines a table is drawn with in text, which few text fonts have: the nearest
            // character every font has (they printed as empty boxes).
            0 if ('\u{2500}'..='\u{257F}').contains(&c) => of(match c {
                '\u{2500}' | '\u{2501}' | '\u{2504}' | '\u{2505}' | '\u{2508}' | '\u{2509}' | '\u{254C}' | '\u{254D}' | '\u{2550}' => '-',
                '\u{2502}' | '\u{2503}' | '\u{2506}' | '\u{2507}' | '\u{250A}' | '\u{250B}' | '\u{254E}' | '\u{254F}' | '\u{2551}' => '|',
                _ => '+',
            }),
            glyph => glyph,
        }
    }

    /// How far a glyph moves the pen, in font units.
    fn advance(&self, gid: u16) -> f32 {
        self.advances.advance_width(skrifa::GlyphId::new(u32::from(gid))).unwrap_or(0.0)
    }

    fn units_per_em(&self) -> f32 {
        f32::from(self.metrics.units_per_em.max(1))
    }
}

impl<'a> Font<'a> {
    fn parse(data: &'a [u8]) -> Option<Font<'a>> {
        use skrifa::MetadataProvider as _;
        use skrifa::raw::TableProvider as _;
        let face = skrifa::FontRef::from_index(data, 0).ok()?;
        // CFF outlines can't go in a FontFile2 stream.
        if face.glyf().is_err() {
            return None;
        }
        let (size, at) = (skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
        Some(Font { charmap: face.charmap(), advances: face.glyph_metrics(size, at), metrics: face.metrics(size, at) })
    }

    /// Width of `c` in thousandths of the font size.
    fn width(&self, c: char) -> f32 {
        self.advance(self.glyph(c)) * 1000.0 / self.units_per_em()
    }
}

impl<'a> Face<'a> {
    fn parse(data: &'a [u8]) -> Option<Face<'a>> {
        Some(Face::Embedded { data, font: Box::new(Font::parse(data)?), used: BTreeMap::new(), fallbacks: Vec::new() })
    }

    /// Measuring what it lacks with these fonts' metrics (ones that can't be embedded left out).
    fn with_fallbacks(mut self, list: &[&'a [u8]]) -> Face<'a> {
        if let Face::Embedded { fallbacks, .. } = &mut self {
            *fallbacks = list.iter().filter_map(|d| Font::parse(d)).collect();
        }
        self
    }

    /// Which font draws `c`: 0 this one, `k` its `k`th fallback (one it hasn't and none has: this
    /// one, its missing glyph).
    fn drawn_by(&self, c: char) -> usize {
        match self {
            Face::Embedded { font, fallbacks, .. } if font.glyph(c) == 0 => fallbacks.iter().position(|f| f.glyph(c) != 0).map_or(0, |k| k + 1),
            _ => 0,
        }
    }

    /// Width of a character in thousandths of the font size.
    fn width(&self, c: char) -> f32 {
        if ignorable(c) {
            return 0.0;
        }
        match self {
            Face::Embedded { font, fallbacks, .. } => match self.drawn_by(c) {
                0 => font.width(c),
                k => fallbacks.get(k - 1).map_or(0.0, |f| f.width(c)),
            },
            Face::Helvetica { bold } => helvetica_width(c, *bold),
        }
    }

    /// `s` in pieces by the font that draws them ([`Face::drawn_by`]), in order.
    fn pieces(&self, s: &str) -> Vec<(usize, String)> {
        let mut out: Vec<(usize, String)> = Vec::new();
        for c in s.chars().filter(|c| !ignorable(*c)) {
            let k = self.drawn_by(c);
            match out.last_mut() {
                Some((last, piece)) if *last == k => piece.push(c),
                _ => out.push((k, c.to_string())),
            }
        }
        out
    }

    fn text_width(&self, s: &str, size: f32) -> f32 {
        s.chars().map(|c| self.width(c)).sum::<f32>() * size / 1000.0
    }

    /// The bytes of a PDF string drawing `s` with this font.
    fn encode(&mut self, s: &str) -> String {
        match self {
            Face::Embedded { font, used, .. } => {
                let mut hex = String::with_capacity(s.len() * 4 + 2);
                hex.push('<');
                for c in s.chars().filter(|c| !ignorable(*c)) {
                    let gid = font.glyph(c);
                    used.entry(gid).or_insert(c);
                    let _ = write!(hex, "{gid:04X}");
                }
                hex.push('>');
                hex
            }
            Face::Helvetica { .. } => {
                let mut lit = String::from("(");
                for c in s.chars().filter(|c| !ignorable(*c)) {
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

/// A character that's drawn as nothing and takes no room (Unicode's default ignorables: joiners,
/// direction marks, variation selectors, the soft hyphen…). Newsletters pad the line their list
/// shows with them; a font without them drew each as an empty box (issue #291: "a row of thin
/// coloured stripes" no one saw on screen).
fn ignorable(c: char) -> bool {
    matches!(
        u32::from(c),
        0xAD | 0x34F | 0x61C | 0x115F..=0x1160 | 0x17B4..=0x17B5 | 0x180B..=0x180F | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F
            | 0x3164 | 0xFE00..=0xFE0F | 0xFEFF | 0xFFA0 | 0xFFF0..=0xFFF8 | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0000..=0xE0FFF
    )
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

/// A piece of text to place: font (0 regular, 1 bold), size, position; and for formatted text
/// how wide it is, whether it leans, its colour, a highlight behind it and lines through it.
#[derive(Debug, Default)]
struct Placed {
    font: usize,
    size: f32,
    x: f32,
    y: f32,
    text: String,
    width: f32,
    slant: bool,
    color: Option<[u8; 3]>,
    background: Option<[u8; 3]>,
    underline: bool,
    strike: bool,
    /// Where a click on it goes.
    link: Option<String>,
    /// A picture in its place: which, and how tall (`width` is how wide).
    picture: Option<(usize, f32)>,
}

/// A colour as PDF's three numbers.
fn rgb(c: [u8; 3]) -> String {
    format!("{:.3} {:.3} {:.3}", f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0)
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
            let cut = cut_word(
                w,
                size,
                width,
                |c| face.width(c),
                |done, _| {
                    out.push(done);
                    true
                },
            );
            line = cut.map(|(rest, _)| rest).unwrap_or_default();
        }
        out.push(line);
    }
    out
}

/// Cut `word`, wider than `width`, by characters, `advance` giving each one's width in
/// thousandths of `size`: `full` is given each full piece and its width, and says whether to go
/// on. What is left of the word and its width; `None` when `full` said to stop.
///
/// The width is added up as the piece grows, not measured again at each character: characters
/// that take no room (a long run of accents) never fill a piece, which made that quadratic.
fn cut_word(word: &str, size: f32, width: f32, advance: impl Fn(char) -> f32, mut full: impl FnMut(String, f32) -> bool) -> Option<(String, f32)> {
    let mut piece = String::new();
    // In thousandths of the size, added in the order [`Face::text_width`] adds them (from -0).
    let mut units = -0.0f32;
    for c in word.chars() {
        let w = advance(c);
        let grown = units + w;
        if grown * size / 1000.0 > width && !piece.is_empty() {
            if !full(std::mem::take(&mut piece), units * size / 1000.0) {
                return None;
            }
            piece.push(c);
            units = w;
        } else {
            piece.push(c);
            units = grown;
        }
    }
    Some((piece, units * size / 1000.0))
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

/// A stream object of `data` compressed; `extra`: more of its dictionary, after its length.
fn flate_stream(extra: &str, data: &[u8]) -> Vec<u8> {
    let packed = compress(data);
    let mut s = format!("<< /Length {}{extra} /Filter /FlateDecode >>\nstream\n", packed.len()).into_bytes();
    s.extend_from_slice(&packed);
    s.extend_from_slice(b"\nendstream");
    s
}

/// Add an object; its number.
fn add(objects: &mut Vec<Vec<u8>>, body: Vec<u8>) -> usize {
    objects.push(body);
    objects.len()
}

/// The document as PDF bytes.
pub fn pdf(doc: &Doc, fonts: Option<Fonts<'_>>) -> Vec<u8> {
    let (mut faces, mut extra) = faces(fonts);
    let title = if doc.title.trim().is_empty() { "(no subject)".to_string() } else { doc.title.clone() };
    let laid = layout::layout(doc, &title, &faces);
    let n = laid.pages.len();
    // Written before the fonts: a page's text names the glyphs a font embeds.
    let contents: Vec<String> = laid
        .pages
        .iter()
        .enumerate()
        .map(|(i, page)| content_stream(&mut faces, &mut extra, page, laid.extras.get(i), (i == laid.rule.0).then_some(laid.rule.1), i + 1, n))
        .collect();

    // Objects: 1 catalog, 2 page tree, 3 info, then fonts, pictures, and page + content pairs.
    let info = format!("<< /Producer (MailCraft {}) /Title {} >>", env!("CARGO_PKG_VERSION"), info_string(&title)).into_bytes();
    let mut objects: Vec<Vec<u8>> = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), Vec::new(), info];
    let mut fonts_dict: String =
        faces.iter().enumerate().map(|(i, f)| format!("/F{} {} 0 R", i + 1, font_objects(f, i, &mut objects))).collect::<Vec<_>>().join(" ");
    // A fallback only when the text drew a character with it (each is the whole font file).
    for (k, f) in extra.iter().enumerate().filter(|(_, f)| matches!(f, Face::Embedded { used, .. } if !used.is_empty())) {
        let _ = write!(fonts_dict, " /F{} {} 0 R", k + 3, font_objects(f, k + 2, &mut objects));
    }
    // The pictures the pages draw, each once.
    let shown: std::collections::BTreeSet<usize> = laid.extras.iter().flat_map(|e| e.images.iter().map(|i| i.index)).collect();
    let mut pictures_dict = String::new();
    for index in shown {
        if let Some(p) = doc.pictures.get(index) {
            let r = objects::picture(p, &mut objects);
            let _ = write!(pictures_dict, "/Im{index} {r} 0 R ");
        }
    }
    let pictures_dict = if pictures_dict.is_empty() { String::new() } else { format!(" /XObject << {pictures_dict}>>") };
    let mut kids = Vec::new();
    for (i, c) in contents.iter().enumerate() {
        let content_ref = add(&mut objects, flate_stream("", c.as_bytes()));
        let annots = laid.extras.get(i).map(objects::Extra::annots).unwrap_or_default();
        let page = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W:.2} {PAGE_H:.2}] /Resources << /Font << {fonts_dict} >>{pictures_dict} >> /Contents {content_ref} 0 R{annots} >>"
        );
        kids.push(add(&mut objects, page.into_bytes()));
    }
    if let Some(o) = objects.get_mut(1) {
        let k: String = kids.iter().map(|r| format!("{r} 0 R")).collect::<Vec<_>>().join(" ");
        *o = format!("<< /Type /Pages /Kids [{k}] /Count {} >>", kids.len()).into_bytes();
    }
    write_file(&objects)
}

/// The regular and bold fonts: the ones given when they can be embedded, else Helvetica; and
/// the fallbacks the writer draws with (`/F3` on), with an embedded regular font only.
fn faces(fonts: Option<Fonts<'_>>) -> ([Face<'_>; 2], Vec<Face<'_>>) {
    match fonts.and_then(|f| Face::parse(f.regular).map(|r| (r, f))) {
        Some((regular, f)) => {
            // Measured with the fallbacks the writer has, in the same order.
            let usable: Vec<&[u8]> = f.fallbacks.iter().copied().filter(|d| Font::parse(d).is_some()).collect();
            let extra: Vec<Face<'_>> = usable.iter().filter_map(|d| Face::parse(d)).collect();
            let bold = f.bold.and_then(Face::parse).map_or(Face::Helvetica { bold: true }, |b| b.with_fallbacks(&usable));
            ([regular.with_fallbacks(&usable), bold], extra)
        }
        None => ([Face::Helvetica { bold: false }, Face::Helvetica { bold: true }], Vec::new()),
    }
}

/// Page `number` of `of`'s content stream: the line under the header when it's on it (`rule`,
/// how high), a laid-out message's boxes and pictures, highlights, the text, the page's number,
/// and lines under and through the text.
fn content_stream(
    faces: &mut [Face<'_>; 2],
    fallbacks: &mut [Face<'_>],
    page: &[Placed],
    extra: Option<&objects::Extra>,
    rule: Option<f32>,
    number: usize,
    of: usize,
) -> String {
    let mut c = String::new();
    if let Some(rule_y) = rule {
        let _ = writeln!(c, "0.75 G 0.6 w {MARGIN:.2} {rule_y:.2} m {:.2} {rule_y:.2} l S 0 G", PAGE_W - MARGIN);
    }
    // A laid-out message's boxes, under everything; its pictures over them.
    if let Some(e) = extra {
        e.fills_into(&mut c);
        e.images_into(&mut c);
    }
    // Highlights under the text.
    for p in page {
        // (Not behind a space alone: it stuck out past the end of a button's words.)
        if let Some(b) = p.background.filter(|_| !p.text.trim().is_empty()) {
            let _ = writeln!(c, "{} rg {:.2} {:.2} {:.2} {:.2} re f 0 g", rgb(b), p.x, p.y - p.size * 0.25, p.width, p.size * 1.2);
        }
    }
    c.push_str("BT\n");
    for p in page {
        let pieces = faces.get(p.font).map(|f| f.pieces(&p.text)).unwrap_or_default();
        if let Some(color) = p.color {
            let _ = write!(c, "{} rg ", rgb(color));
        }
        // Italic: the upright font, leaning.
        let lean = if p.slant { "0.21" } else { "0" };
        let _ = write!(c, "/F{} {:.1} Tf 1 0 {lean} 1 {:.2} {:.2} Tm", p.font + 1, p.size, p.x, p.y);
        // A piece another font draws goes on from where the one before ended, in that font.
        let mut font = p.font + 1;
        for (k, text) in pieces {
            let (f, enc) = match k {
                0 => (p.font + 1, faces.get_mut(p.font).map(|f| f.encode(&text)).unwrap_or_default()),
                k => (k + 2, fallbacks.get_mut(k - 1).map(|f| f.encode(&text)).unwrap_or_default()),
            };
            if f != font {
                let _ = write!(c, " /F{f} {:.1} Tf", p.size);
                font = f;
            }
            let _ = write!(c, " {enc} Tj");
        }
        c.push_str(if p.color.is_some() { " 0 g\n" } else { "\n" });
    }
    let footer = format!("{number} / {of}");
    let fw = faces[0].text_width(&footer, 8.0);
    let enc = faces[0].encode(&footer);
    let _ = writeln!(c, "0.45 g /F1 8.0 Tf 1 0 0 1 {:.2} {:.2} Tm {enc} Tj 0 g", (PAGE_W - fw) / 2.0, MARGIN / 2.0);
    c.push_str("ET\n");
    // Lines under and through the text, in its colour.
    for p in page.iter().filter(|p| p.underline || p.strike) {
        let ink = p.color.map_or_else(|| "0 0 0".to_string(), rgb);
        for (on, at) in [(p.underline, p.y - p.size * 0.12), (p.strike, p.y + p.size * 0.28)] {
            if on {
                let _ = writeln!(c, "{ink} RG {:.2} w {:.2} {at:.2} m {:.2} {at:.2} l S 0 G", p.size * 0.06, p.x, p.x + p.width);
            }
        }
    }
    c
}

/// Write the `i`th font's objects into `objects`; the number of the one a page names. An
/// embedded font is its file, its descriptor, a CID font of the glyphs used, and a map from
/// them back to the characters they draw (so the text can be searched and copied).
fn font_objects(face: &Face<'_>, i: usize, objects: &mut Vec<Vec<u8>>) -> usize {
    let (data, font, used) = match face {
        Face::Helvetica { bold } => {
            let name = if *bold { "Helvetica-Bold" } else { "Helvetica" };
            return add(objects, format!("<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding >>").into_bytes());
        }
        Face::Embedded { data, font, used, .. } => (data, font, used),
    };
    let upem = font.units_per_em();
    let scale = |v: f32| (v * 1000.0 / upem).round() as i32;
    let file_ref = add(objects, flate_stream(&format!(" /Length1 {}", data.len()), data));
    let m = &font.metrics;
    let bbox = m.bounds.unwrap_or(skrifa::metrics::BoundingBox { x_min: 0.0, y_min: 0.0, x_max: 0.0, y_max: 0.0 });
    let descriptor = format!(
        "<< /Type /FontDescriptor /FontName /MailCraftFont{i} /Flags 32 /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV {} /FontFile2 {file_ref} 0 R >>",
        scale(bbox.x_min),
        scale(bbox.y_min),
        scale(bbox.x_max),
        scale(bbox.y_max),
        scale(m.ascent),
        scale(m.descent),
        scale(m.cap_height.unwrap_or(m.ascent)),
        if i == 1 { 140 } else { 80 }
    );
    let desc_ref = add(objects, descriptor.into_bytes());
    let mut w = String::from("[");
    for gid in used.keys() {
        let _ = write!(w, " {gid} [{}]", scale(font.advance(*gid)));
    }
    w.push_str(" ]");
    let cid = format!(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /MailCraftFont{i} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {desc_ref} 0 R /DW 1000 /W {w} /CIDToGIDMap /Identity >>"
    );
    let cid_ref = add(objects, cid.into_bytes());
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
    let tu_ref = add(objects, flate_stream("", cmap.as_bytes()));
    add(
        objects,
        format!("<< /Type /Font /Subtype /Type0 /BaseFont /MailCraftFont{i} /Encoding /Identity-H /DescendantFonts [{cid_ref} 0 R] /ToUnicode {tu_ref} 0 R >>")
            .into_bytes(),
    )
}

/// The file: header, `objects` (numbered from 1), cross-reference table, trailer.
fn write_file(objects: &[Vec<u8>]) -> Vec<u8> {
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
    out
}

#[cfg(test)]
mod tests;
