//! What an Office attachment says, for the reading pane's preview, as Outlook previews Word,
//! Excel and PowerPoint files: a document's paragraphs (bold, italic, headings) and tables, a
//! workbook's sheets as grids, a presentation's slides as their text.
//!
//! The files are zip archives of XML (ECMA-376 / ISO 29500). A small zip reader (stored and
//! deflated entries) and a tag scanner read just the parts a preview needs. Everything here comes
//! from mail, so sizes, counts and nesting are capped and nothing can panic.

use std::borrow::Cow;
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
/// Relationships of a workbook or presentation followed (a sheet's or a slide's among them).
const MAX_RELATIONSHIPS: usize = 10_000;
/// Most paragraphs read from one slide.
const MAX_SLIDE_PARAS: usize = 400;

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

/// The kinds of Office file this previews.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A Word document (`.docx`, `.docm`).
    Word,
    /// An Excel workbook (`.xlsx`, `.xlsm`).
    Excel,
    /// A PowerPoint presentation (`.pptx`, `.pptm`).
    PowerPoint,
}

/// The kind of Office file a name (or type) is, when it's one this can preview.
pub fn kind(name: &str, mime: &str) -> Option<Kind> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let m = mime.to_ascii_lowercase();
    if ext == "docx" || ext == "docm" || m.contains("wordprocessingml") {
        Some(Kind::Word)
    } else if ext == "xlsx" || ext == "xlsm" || m.contains("spreadsheetml") {
        Some(Kind::Excel)
    } else if ext == "pptx" || ext == "pptm" || m.contains("presentationml") {
        Some(Kind::PowerPoint)
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
        Kind::Word => document(&String::from_utf8_lossy(&zip.read("word/document.xml")?)),
        Kind::Excel => workbook(&zip),
        Kind::PowerPoint => slides(&zip),
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
    let b = d.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*b.first()?, *b.get(1)?]))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]))
}

impl<'a> Zip<'a> {
    fn new(data: &'a [u8]) -> Option<Zip<'a>> {
        // The end of central directory record: within the last 64 KiB (its comment), last first.
        let from = data.len().saturating_sub(22 + 65_535);
        let eocd = (from..=data.len().saturating_sub(22)).rev().find(|&i| data.get(i..i + 4) == Some(&b"PK\x05\x06"[..]))?;
        // Offsets from the file's 32-bit fields, added checked (a `usize` is 32 bits on wasm).
        let count = usize::from(u16_at(data, eocd.checked_add(10)?)?);
        let mut at = usize::try_from(u32_at(data, eocd.checked_add(16)?)?).ok()?;
        let mut entries = Vec::with_capacity(count.min(4096));
        for _ in 0..count.min(4096) {
            let field = |n: usize| at.checked_add(n);
            if data.get(at..field(4)?) != Some(&b"PK\x01\x02"[..]) {
                break;
            }
            let method = u16_at(data, field(10)?)?;
            let compressed = usize::try_from(u32_at(data, field(20)?)?).ok()?;
            let size = u64::from(u32_at(data, field(24)?)?);
            let name_len = usize::from(u16_at(data, field(28)?)?);
            let extra = usize::from(u16_at(data, field(30)?)?);
            let comment = usize::from(u16_at(data, field(32)?)?);
            let local = usize::try_from(u32_at(data, field(42)?)?).ok()?;
            let name = String::from_utf8_lossy(data.get(field(46)?..field(46)?.checked_add(name_len)?)?).replace('\\', "/");
            entries.push(Entry { name, method, compressed, size, local });
            at = at.checked_add(46)?.checked_add(name_len)?.checked_add(extra)?.checked_add(comment)?;
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
        if d.get(e.local..e.local.checked_add(4)?) != Some(&b"PK\x03\x04"[..]) {
            return None;
        }
        let name_len = usize::from(u16_at(d, e.local.checked_add(26)?)?);
        let extra = usize::from(u16_at(d, e.local.checked_add(28)?)?);
        let start = e.local.checked_add(30)?.checked_add(name_len)?.checked_add(extra)?;
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
/// text (entities decoded; borrowed when it has none).
#[derive(Debug, PartialEq, Eq)]
enum Tok<'a> {
    Start { name: &'a str, attrs: &'a str, empty: bool },
    End(&'a str),
    Text(Cow<'a, str>),
}

/// Most pieces read from one part.
const MAX_TOKENS: usize = 2_000_000;

/// The XML split into tags and text, a piece at a time, so that a reader that has what it shows
/// stops there; comments, declarations and processing instructions left out.
struct Tokens<'a> {
    rest: &'a str,
    given: usize,
}

fn tokens(xml: &str) -> Tokens<'_> {
    Tokens { rest: xml, given: 0 }
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Tok<'a>;

    fn next(&mut self) -> Option<Tok<'a>> {
        while !self.rest.is_empty() && self.given < MAX_TOKENS {
            let rest = self.rest;
            let tok = match rest.find('<') {
                Some(0) => {
                    let Some(close) = rest.find('>') else {
                        self.rest = "";
                        return None;
                    };
                    let tag = rest.get(1..close).unwrap_or_default();
                    self.rest = rest.get(close + 1..).unwrap_or_default();
                    if tag.starts_with('?') || tag.starts_with('!') {
                        continue;
                    }
                    match tag.strip_prefix('/') {
                        Some(name) => Tok::End(name.trim()),
                        None => {
                            let empty = tag.ends_with('/');
                            let tag = tag.trim_end_matches('/');
                            let (name, attrs) = tag.split_once(|c: char| c.is_whitespace()).unwrap_or((tag, ""));
                            Tok::Start { name, attrs, empty }
                        }
                    }
                }
                Some(i) => {
                    self.rest = rest.get(i..).unwrap_or_default();
                    Tok::Text(unescape(rest.get(..i).unwrap_or_default()))
                }
                None => {
                    self.rest = "";
                    Tok::Text(unescape(rest))
                }
            };
            self.given += 1;
            return Some(tok);
        }
        None
    }
}

fn unescape(s: &str) -> Cow<'_, str> {
    crate::html::decode_xml_refs(s)
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

/// The parts the package's `main` part lists in its `element`s, in order, each by its `r:id`
/// followed through `rels` (a target under `dir`, or from the package's root with a `/`), with
/// the element's `name` when it has one; the first `most` of them. Empty when there are no
/// relationships to follow: the format ties a sheet or a slide to its part so, not by the
/// part's number.
fn in_order(zip: &Zip<'_>, main: &str, rels: &str, element: &str, dir: &str, most: usize) -> Vec<(Option<String>, String)> {
    let Some(rels) = zip.read(rels) else { return Vec::new() };
    // Each relationship's part by its id (the first of an id, as it was found), looked up once
    // for each element: looked for through all of them, a file of thousands of each was read
    // through thousands of times.
    let mut targets: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for t in tokens(&String::from_utf8_lossy(&rels)) {
        if targets.len() >= MAX_RELATIONSHIPS {
            break;
        }
        if let Tok::Start { name, attrs, .. } = t
            && local(name) == "Relationship"
            && let (Some(id), Some(target)) = (attr(attrs, "Id"), attr(attrs, "Target"))
        {
            let target = unescape(target);
            let part = match target.strip_prefix('/') {
                Some(from_root) => from_root.to_string(),
                None => format!("{dir}{target}"),
            };
            targets.entry(id.to_string()).or_insert(part);
        }
    }
    let Some(x) = zip.read(main) else { return Vec::new() };
    let mut out = Vec::new();
    for t in tokens(&String::from_utf8_lossy(&x)) {
        if out.len() >= most {
            break;
        }
        if let Tok::Start { name, attrs, .. } = t
            && local(name) == element
            && let Some(id) = prefixed_attr(attrs, "id")
            && let Some(part) = targets.get(id)
        {
            out.push((attr(attrs, "name").map(|n| unescape(n).into_owned()), part.clone()));
        }
    }
    out
}

/// [`attr`] for a name with a prefix only (`r:id`, not a plain `id` beside it).
fn prefixed_attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attrs;
    while let Some(eq) = rest.find('=') {
        let key = rest.get(..eq).unwrap_or_default().trim();
        let key = key.rsplit(char::is_whitespace).next().unwrap_or(key);
        let after = rest.get(eq + 1..).unwrap_or_default().trim_start();
        let quote = after.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let body = after.get(1..).unwrap_or_default();
        let close = body.find(quote)?;
        if key.split_once(':').is_some_and(|(_, local)| local == name) {
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

/// A Word table being read: nested tables flatten into the cell of the outer one.
#[derive(Default)]
struct OpenTable {
    rows: Vec<Vec<String>>,
    /// The row being read.
    row: Vec<String>,
    /// The cell's text so far.
    cell: String,
}

/// A Word paragraph being read.
#[derive(Default)]
struct OpenPara {
    runs: Vec<Run>,
    heading: bool,
    list: bool,
}

fn document(xml: &str) -> Option<Preview> {
    let mut blocks = Vec::new();
    let mut table: Option<OpenTable> = None;
    let mut depth = 0usize;
    let mut para: Option<OpenPara> = None;
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
                        table = Some(OpenTable::default());
                    }
                }
                "tr" if depth == 1 && !empty => {
                    if let Some(tb) = table.as_mut() {
                        tb.row.clear();
                    }
                }
                "tc" if depth == 1 && !empty => {
                    if let Some(tb) = table.as_mut() {
                        tb.cell.clear();
                    }
                }
                "p" if !empty => para = Some(OpenPara::default()),
                "p" => {
                    if table.is_none() {
                        blocks.push(Block::Paragraph { runs: Vec::new(), heading: false, list: false });
                    }
                }
                "pStyle" => {
                    let v = attr(attrs, "val").unwrap_or_default().to_ascii_lowercase();
                    if let Some(p) = para.as_mut() {
                        p.heading = v.starts_with("heading") || v == "title" || v.starts_with("berschrift") || v.starts_with("titre");
                    }
                }
                "numPr" => {
                    if let Some(p) = para.as_mut() {
                        p.list = true;
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
                        p.runs.push(std::mem::take(&mut run));
                    }
                }
                "p" => {
                    let Some(OpenPara { runs, heading, list }) = para.take() else { continue };
                    match table.as_mut() {
                        Some(tb) => {
                            if !tb.cell.is_empty() {
                                tb.cell.push('\n');
                            }
                            tb.cell.extend(runs.iter().map(|r| r.text.as_str()));
                        }
                        None => blocks.push(Block::Paragraph { runs, heading, list }),
                    }
                }
                "tc" if depth == 1 => {
                    if let Some(tb) = table.as_mut()
                        && tb.row.len() < MAX_COLS
                    {
                        let cell = std::mem::take(&mut tb.cell);
                        tb.row.push(cell);
                    }
                }
                "tr" if depth == 1 => {
                    if let Some(tb) = table.as_mut()
                        && tb.rows.len() < MAX_ROWS
                    {
                        let row = std::mem::take(&mut tb.row);
                        tb.rows.push(row);
                    }
                }
                "tbl" => {
                    depth = depth.saturating_sub(1);
                    if depth == 0
                        && let Some(OpenTable { rows, .. }) = table.take()
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
    // The sheets as the workbook lists them, each followed to its part.
    let listed = in_order(zip, "xl/workbook.xml", "xl/_rels/workbook.xml.rels", "sheet", "xl/", MAX_SHEETS);
    let mut sheets = Vec::new();
    for (n, (name, part)) in listed.into_iter().enumerate() {
        let Some(x) = zip.read(&part) else { continue };
        let name = name.filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Sheet{}", n + 1));
        sheets.push((name, sheet(&String::from_utf8_lossy(&x), &shared)));
    }
    if !sheets.is_empty() {
        return Some(Preview::Workbook(sheets));
    }
    // A package without its relationships, or whose relationships lead to no part that reads:
    // its parts by number, its names in order.
    let mut names = Vec::new();
    if let Some(x) = zip.read("xl/workbook.xml") {
        for t in tokens(&String::from_utf8_lossy(&x)) {
            if let Tok::Start { name, attrs, .. } = t
                && local(name) == "sheet"
            {
                names.push(attr(attrs, "name").map(|n| unescape(n).into_owned()).unwrap_or_default());
            }
        }
    }
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
                    row_no = attr(attrs, "r").and_then(|r| r.parse::<usize>().ok()).unwrap_or_else(|| row_no.saturating_add(1));
                    // Rows come in order (Excel takes a sheet whose rows don't for damaged): the rest are
                    // past what's shown.
                    if row_no > MAX_ROWS {
                        break;
                    }
                    next_col = 0;
                }
                "c" => {
                    let (r, c) = attr(attrs, "r").and_then(cell_ref).unwrap_or((row_no.saturating_sub(1), next_col));
                    next_col = c.saturating_add(1);
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
    // As the presentation lists them, each followed to its part; without its relationships, or
    // when they lead to no part there is, by number.
    let mut parts: Vec<String> = in_order(zip, "ppt/presentation.xml", "ppt/_rels/presentation.xml.rels", "sldId", "ppt/", MAX_SLIDES)
        .into_iter()
        .map(|(_, p)| p)
        .filter(|p| zip.has(p))
        .collect();
    if parts.is_empty() {
        parts = (1..=MAX_SLIDES).map(|n| format!("ppt/slides/slide{n}.xml")).take_while(|p| zip.has(p)).collect();
    }
    for part in parts.into_iter().take(MAX_SLIDES) {
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
                        if let Some(c) = cur.take().filter(|c| !c.trim().is_empty()) {
                            paras.push(c);
                            if paras.len() >= MAX_SLIDE_PARAS {
                                break;
                            }
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
mod tests;
