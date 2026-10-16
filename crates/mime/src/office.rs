//! What an Office attachment says, for the reading pane's preview, as Outlook previews Word,
//! Excel and PowerPoint files: a document's paragraphs (bold, italic, headings) and tables, a
//! workbook's sheets as grids, a presentation's slides as their text.
//!
//! The files are zip archives of XML (ECMA-376 / ISO 29500). A small zip reader (stored and
//! deflated entries) and a tag scanner read just the parts a preview needs. Everything here comes
//! from mail, so sizes, counts and nesting are capped and nothing can panic.

use std::io::Read;

/// Largest archive read, and largest part unpacked from it.
const MAX_FILE: usize = 64 << 20;
const MAX_PART: u64 = 32 << 20;
/// Most paragraphs, table rows, sheet rows and columns, slides shown.
const MAX_BLOCKS: usize = 4_000;
const MAX_ROWS: usize = 500;
const MAX_COLS: usize = 40;
const MAX_SHEETS: usize = 20;
const MAX_SLIDES: usize = 200;

/// A preview of an Office file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    /// A Word document: its paragraphs and tables, in order.
    Document(Vec<Block>),
    /// An Excel workbook: each sheet's name and its cells (rows of columns, from A1).
    Workbook(Vec<(String, Vec<Vec<String>>)>),
    /// A PowerPoint presentation: each slide's paragraphs.
    Slides(Vec<Vec<String>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph { runs: Vec<Run>, heading: bool, list: bool },
    Table(Vec<Vec<String>>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

/// The kind of Office file a name (or type) is, when it's one this can preview.
pub fn kind(name: &str, mime: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let m = mime.to_ascii_lowercase();
    if ext == "docx" || ext == "docm" || m.contains("wordprocessingml") {
        Some("docx")
    } else if ext == "xlsx" || ext == "xlsm" || m.contains("spreadsheetml") {
        Some("xlsx")
    } else if ext == "pptx" || ext == "pptm" || m.contains("presentationml") {
        Some("pptx")
    } else {
        None
    }
}

/// Read an Office file for its preview; `None` when it isn't one, or can't be read.
pub fn preview(name: &str, mime: &str, data: &[u8]) -> Option<Preview> {
    if data.len() > MAX_FILE {
        return None;
    }
    let zip = Zip::new(data)?;
    match kind(name, mime)? {
        "docx" => document(&String::from_utf8_lossy(&zip.read("word/document.xml")?)),
        "xlsx" => workbook(&zip),
        _ => slides(&zip),
    }
}

// ----- zip -----

struct Entry {
    name: String,
    method: u16,
    compressed: usize,
    size: u64,
    local: usize,
}

/// A zip archive's directory (APPNOTE 4.3.12, 4.3.16), read from its end.
struct Zip<'a> {
    data: &'a [u8],
    entries: Vec<Entry>,
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*d.get(at)?, *d.get(at + 1)?]))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*d.get(at)?, *d.get(at + 1)?, *d.get(at + 2)?, *d.get(at + 3)?]))
}

impl<'a> Zip<'a> {
    fn new(data: &'a [u8]) -> Option<Zip<'a>> {
        // The end of central directory record: within the last 64 KiB (its comment), last first.
        let from = data.len().saturating_sub(22 + 65_535);
        let eocd = (from..=data.len().saturating_sub(22)).rev().find(|&i| data.get(i..i + 4) == Some(&b"PK\x05\x06"[..]))?;
        let count = usize::from(u16_at(data, eocd + 10)?);
        let mut at = usize::try_from(u32_at(data, eocd + 16)?).ok()?;
        let mut entries = Vec::with_capacity(count.min(4096));
        for _ in 0..count.min(4096) {
            if data.get(at..at + 4) != Some(&b"PK\x01\x02"[..]) {
                break;
            }
            let method = u16_at(data, at + 10)?;
            let compressed = usize::try_from(u32_at(data, at + 20)?).ok()?;
            let size = u64::from(u32_at(data, at + 24)?);
            let name_len = usize::from(u16_at(data, at + 28)?);
            let extra = usize::from(u16_at(data, at + 30)?);
            let comment = usize::from(u16_at(data, at + 32)?);
            let local = usize::try_from(u32_at(data, at + 42)?).ok()?;
            let name = String::from_utf8_lossy(data.get(at + 46..at + 46 + name_len)?).replace('\\', "/");
            entries.push(Entry { name, method, compressed, size, local });
            at = at.checked_add(46 + name_len + extra + comment)?;
        }
        Some(Zip { data, entries })
    }

    /// A part by its name (case-insensitive, as Office itself reads them).
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        let e = self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))?;
        if e.size > MAX_PART {
            return None;
        }
        let d = self.data;
        if d.get(e.local..e.local + 4) != Some(&b"PK\x03\x04"[..]) {
            return None;
        }
        let start = e.local + 30 + usize::from(u16_at(d, e.local + 26)?) + usize::from(u16_at(d, e.local + 28)?);
        let raw = d.get(start..start.checked_add(e.compressed)?)?;
        match e.method {
            0 => Some(raw.to_vec()),
            8 => {
                let mut out = Vec::new();
                flate2::read::DeflateDecoder::new(raw).take(MAX_PART).read_to_end(&mut out).ok()?;
                Some(out)
            }
            _ => None,
        }
    }

    fn has(&self, name: &str) -> bool {
        self.entries.iter().any(|e| e.name.eq_ignore_ascii_case(name))
    }
}

// ----- XML -----

/// An XML piece: a start tag (its name, attributes and whether it closes itself), an end tag, or
/// text (entities decoded).
#[derive(Debug, PartialEq, Eq)]
enum Tok<'a> {
    Start { name: &'a str, attrs: &'a str, empty: bool },
    End(&'a str),
    Text(String),
}

/// The XML split into tags and text; comments, declarations and processing instructions left
/// out.
fn tokens(xml: &str) -> Vec<Tok<'_>> {
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
                out.push(Tok::Text(unescape(rest.get(..i).unwrap_or_default())));
                rest = rest.get(i..).unwrap_or_default();
            }
            None => {
                out.push(Tok::Text(unescape(rest)));
                break;
            }
        }
    }
    out
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or_default());
        rest = rest.get(i..).unwrap_or_default();
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = rest.get(1..).unwrap_or_default();
            continue;
        };
        let ent = rest.get(1..end).unwrap_or_default();
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = rest.get(end + 1..).unwrap_or_default();
            }
            None => {
                out.push('&');
                rest = rest.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

/// An attribute's value (`name="value"`), by its name without a prefix.
fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attrs;
    while let Some(eq) = rest.find('=') {
        let key = rest.get(..eq).unwrap_or_default().trim();
        let key = key.rsplit(':').next().unwrap_or(key);
        let after = rest.get(eq + 1..).unwrap_or_default().trim_start();
        let quote = after.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let body = after.get(1..).unwrap_or_default();
        let close = body.find(quote)?;
        if key == name {
            return body.get(..close);
        }
        rest = body.get(close + 1..).unwrap_or_default();
    }
    None
}

/// A tag's name without its prefix (`w:p` → `p`).
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// On, for a toggle like `<w:b/>` (off when `w:val` is 0, false or off).
fn on(attrs: &str) -> bool {
    !matches!(attr(attrs, "val"), Some("0" | "false" | "off" | "none"))
}

// ----- Word -----

fn document(xml: &str) -> Option<Preview> {
    let mut blocks = Vec::new();
    // Inside a table: its rows, the row being read, the cell's text; nested tables flatten into
    // the cell of the outer one.
    let mut table: Option<(Vec<Vec<String>>, Vec<String>, String)> = None;
    let mut depth = 0usize;
    let mut para: Option<(Vec<Run>, bool, bool)> = None;
    let mut run = Run::default();
    let mut in_text = false;
    for t in tokens(xml) {
        if blocks.len() >= MAX_BLOCKS {
            break;
        }
        match t {
            Tok::Start { name, attrs, empty } => match local(name) {
                "tbl" if !empty => {
                    depth += 1;
                    if depth == 1 {
                        table = Some((Vec::new(), Vec::new(), String::new()));
                    }
                }
                "tr" if depth == 1 && !empty => {
                    if let Some(tb) = table.as_mut() {
                        tb.1.clear();
                    }
                }
                "tc" if depth == 1 && !empty => {
                    if let Some(tb) = table.as_mut() {
                        tb.2.clear();
                    }
                }
                "p" if !empty => para = Some((Vec::new(), false, false)),
                "p" => {
                    if table.is_none() {
                        blocks.push(Block::Paragraph { runs: Vec::new(), heading: false, list: false });
                    }
                }
                "pStyle" => {
                    let v = attr(attrs, "val").unwrap_or_default().to_ascii_lowercase();
                    if let Some(p) = para.as_mut() {
                        p.1 = v.starts_with("heading") || v == "title" || v.starts_with("berschrift") || v.starts_with("titre");
                    }
                }
                "numPr" => {
                    if let Some(p) = para.as_mut() {
                        p.2 = true;
                    }
                }
                "r" if !empty => run = Run::default(),
                "b" => run.bold = on(attrs),
                "i" => run.italic = on(attrs),
                "t" if !empty => in_text = true,
                "tab" => run.text.push('\t'),
                "br" | "cr" => run.text.push('\n'),
                _ => {}
            },
            Tok::Text(s) if in_text => run.text.push_str(&s),
            Tok::Text(_) => {}
            Tok::End(name) => match local(name) {
                "t" => in_text = false,
                "r" => {
                    if let Some(p) = para.as_mut()
                        && !run.text.is_empty()
                    {
                        p.0.push(std::mem::take(&mut run));
                    }
                }
                "p" => {
                    let Some((runs, heading, list)) = para.take() else { continue };
                    match table.as_mut() {
                        Some(tb) => {
                            if !tb.2.is_empty() {
                                tb.2.push('\n');
                            }
                            tb.2.extend(runs.iter().map(|r| r.text.as_str()));
                        }
                        None => blocks.push(Block::Paragraph { runs, heading, list }),
                    }
                }
                "tc" if depth == 1 => {
                    if let Some(tb) = table.as_mut()
                        && tb.1.len() < MAX_COLS
                    {
                        let cell = std::mem::take(&mut tb.2);
                        tb.1.push(cell);
                    }
                }
                "tr" if depth == 1 => {
                    if let Some(tb) = table.as_mut()
                        && tb.0.len() < MAX_ROWS
                    {
                        let row = std::mem::take(&mut tb.1);
                        tb.0.push(row);
                    }
                }
                "tbl" => {
                    depth = depth.saturating_sub(1);
                    if depth == 0
                        && let Some((rows, _, _)) = table.take()
                    {
                        blocks.push(Block::Table(rows));
                    }
                }
                _ => {}
            },
        }
    }
    // Trailing empty paragraphs say nothing.
    while matches!(blocks.last(), Some(Block::Paragraph { runs, .. }) if runs.iter().all(|r| r.text.trim().is_empty())) {
        blocks.pop();
    }
    Some(Preview::Document(blocks))
}

// ----- Excel -----

fn workbook(zip: &Zip<'_>) -> Option<Preview> {
    // Shared strings: each <si> is one string, its <t> pieces joined.
    let mut shared: Vec<String> = Vec::new();
    if let Some(x) = zip.read("xl/sharedStrings.xml") {
        let mut cur: Option<String> = None;
        let mut in_t = false;
        let mut in_phonetic = false;
        for t in tokens(&String::from_utf8_lossy(&x)) {
            match t {
                Tok::Start { name, empty, .. } => match local(name) {
                    "si" if !empty => cur = Some(String::new()),
                    "si" => shared.push(String::new()),
                    "rPh" if !empty => in_phonetic = true,
                    "t" if !empty => in_t = true,
                    _ => {}
                },
                Tok::Text(s) if in_t && !in_phonetic => {
                    if let Some(c) = cur.as_mut() {
                        c.push_str(&s);
                    }
                }
                Tok::Text(_) => {}
                Tok::End(name) => match local(name) {
                    "t" => in_t = false,
                    "rPh" => in_phonetic = false,
                    "si" => shared.push(cur.take().unwrap_or_default()),
                    _ => {}
                },
            }
            if shared.len() > 1_000_000 {
                break;
            }
        }
    }
    // The sheets' names, in order.
    let mut names = Vec::new();
    if let Some(x) = zip.read("xl/workbook.xml") {
        for t in tokens(&String::from_utf8_lossy(&x)) {
            if let Tok::Start { name, attrs, .. } = t
                && local(name) == "sheet"
            {
                names.push(attr(attrs, "name").map(unescape).unwrap_or_default());
            }
        }
    }
    let mut sheets = Vec::new();
    for n in 1..=MAX_SHEETS {
        let part = format!("xl/worksheets/sheet{n}.xml");
        if !zip.has(&part) {
            break;
        }
        let Some(x) = zip.read(&part) else { continue };
        let name = names.get(n - 1).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Sheet{n}"));
        sheets.push((name, sheet(&String::from_utf8_lossy(&x), &shared)));
    }
    Some(Preview::Workbook(sheets))
}

/// `B7` → (row 6, column 1).
fn cell_ref(r: &str) -> Option<(usize, usize)> {
    let letters: String = r.chars().take_while(char::is_ascii_alphabetic).collect();
    let digits = r.get(letters.len()..)?;
    let mut col = 0usize;
    for c in letters.chars() {
        col = col.checked_mul(26)?.checked_add(usize::from(u8::try_from(c.to_ascii_uppercase()).ok()?.checked_sub(b'A')?) + 1)?;
    }
    let row: usize = digits.parse().ok()?;
    Some((row.checked_sub(1)?, col.checked_sub(1)?))
}

fn sheet(xml: &str, shared: &[String]) -> Vec<Vec<String>> {
    let mut grid: Vec<Vec<String>> = Vec::new();
    let mut at: Option<(usize, usize, String)> = None;
    let mut next_col = 0usize;
    let mut row_no = 0usize;
    let mut value = String::new();
    let mut in_v = false;
    for t in tokens(xml) {
        match t {
            Tok::Start { name, attrs, empty } => match local(name) {
                "row" => {
                    row_no = attr(attrs, "r").and_then(|r| r.parse::<usize>().ok()).unwrap_or(row_no + 1);
                    next_col = 0;
                }
                "c" => {
                    let (r, c) = attr(attrs, "r").and_then(cell_ref).unwrap_or((row_no.saturating_sub(1), next_col));
                    next_col = c + 1;
                    at = if empty { None } else { Some((r, c, attr(attrs, "t").unwrap_or_default().to_string())) };
                    value.clear();
                }
                "v" | "t" if !empty && at.is_some() => in_v = true,
                _ => {}
            },
            Tok::Text(s) if in_v => value.push_str(&s),
            Tok::Text(_) => {}
            Tok::End(name) => match local(name) {
                "v" | "t" => in_v = false,
                "c" => {
                    let Some((r, c, kind)) = at.take() else { continue };
                    if r >= MAX_ROWS || c >= MAX_COLS {
                        continue;
                    }
                    let shown = match kind.as_str() {
                        "s" => value.trim().parse::<usize>().ok().and_then(|i| shared.get(i).cloned()).unwrap_or_default(),
                        "b" => {
                            if value.trim() == "1" {
                                "TRUE".into()
                            } else {
                                "FALSE".into()
                            }
                        }
                        "e" => value.clone(),
                        _ => value.clone(),
                    };
                    if shown.is_empty() {
                        continue;
                    }
                    while grid.len() <= r {
                        grid.push(Vec::new());
                    }
                    if let Some(row) = grid.get_mut(r) {
                        while row.len() <= c {
                            row.push(String::new());
                        }
                        if let Some(cell) = row.get_mut(c) {
                            *cell = shown;
                        }
                    }
                }
                _ => {}
            },
        }
    }
    grid
}

// ----- PowerPoint -----

fn slides(zip: &Zip<'_>) -> Option<Preview> {
    let mut out = Vec::new();
    for n in 1..=MAX_SLIDES {
        let part = format!("ppt/slides/slide{n}.xml");
        if !zip.has(&part) {
            break;
        }
        let Some(x) = zip.read(&part) else { continue };
        let mut paras = Vec::new();
        let mut cur: Option<String> = None;
        let mut in_t = false;
        for t in tokens(&String::from_utf8_lossy(&x)) {
            match t {
                Tok::Start { name, empty, .. } => match local(name) {
                    "p" if !empty => cur = Some(String::new()),
                    "t" if !empty => in_t = true,
                    "br" => {
                        if let Some(c) = cur.as_mut() {
                            c.push('\n');
                        }
                    }
                    _ => {}
                },
                Tok::Text(s) if in_t => {
                    if let Some(c) = cur.as_mut() {
                        c.push_str(&s);
                    }
                }
                Tok::Text(_) => {}
                Tok::End(name) => match local(name) {
                    "t" => in_t = false,
                    "p" => {
                        if let Some(c) = cur.take().filter(|c| !c.trim().is_empty())
                            && paras.len() < 400
                        {
                            paras.push(c);
                        }
                    }
                    _ => {}
                },
            }
        }
        out.push(paras);
    }
    (!out.is_empty()).then_some(Preview::Slides(out))
}

#[cfg(test)]
mod tests {
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
        assert_eq!(cell_ref("1"), None);
    }
}
