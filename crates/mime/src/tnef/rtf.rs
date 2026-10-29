//! Rich Text Format as Outlook keeps a body: compressed (MS-OXRTFCP), then read as the HTML or
//! the text it wraps (MS-OXRTFEX), or, when it wraps neither, as simple HTML of its own (bold,
//! italic, underline, struck-through text, links and line breaks).

use std::collections::HashMap;

use super::{TnefBody, decode};

/// Most bytes decompressed RTF may have.
const RTF_MAX: usize = 16 << 20;
/// Deepest group nesting followed; deeper groups count as the one they're in.
const MAX_GROUPS: usize = 512;

/// What the compression's dictionary starts with (MS-OXRTFCP 2.1.2.1).
const DICTIONARY: &[u8; 207] = b"{\\rtf1\\ansi\\mac\\deff0\\deftab720{\\fonttbl;}{\\f0\\fnil \\froman \\fswiss \\fmodern \\fscript \\fdecor MS Sans SerifSymbolArialTimes New RomanCourier{\\colortbl\\red0\\green0\\blue0\r\n\\par \\pard\\plain\\f0\\fs20\\b\\i\\u\\tab\\tx";
const COMPRESSED: u32 = 0x7546_5A4C;
const UNCOMPRESSED: u32 = 0x414C_454D;

/// `PidTagRtfCompressed` as RTF; `None` for bytes that aren't compressed RTF. Cut-short input
/// gives what was there.
pub fn decompress_rtf(data: &[u8]) -> Option<Vec<u8>> {
    let word = |i: usize| data.get(i..i + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let size = word(0)? as usize;
    let raw = word(4)? as usize;
    let kind = word(8)?;
    let rest = data.get(16..)?;
    // The size counts the 12 header bytes after it.
    let contents = rest.get(..size.saturating_sub(12).min(rest.len())).unwrap_or(rest);
    match kind {
        UNCOMPRESSED => Some(contents.get(..contents.len().min(RTF_MAX)).unwrap_or_default().to_vec()),
        COMPRESSED => Some(lzfu(contents, raw.min(RTF_MAX))),
        _ => None,
    }
}

/// The runs of literals and dictionary references (MS-OXRTFCP 2.2.3.2), up to `cap` bytes.
fn lzfu(input: &[u8], cap: usize) -> Vec<u8> {
    let mut dict = [0u8; 4096];
    if let Some(start) = dict.get_mut(..DICTIONARY.len()) {
        start.copy_from_slice(DICTIONARY);
    }
    let mut write = DICTIONARY.len();
    let mut out = Vec::with_capacity(cap.min(input.len().saturating_mul(4)));
    let add = |b: u8, out: &mut Vec<u8>, dict: &mut [u8; 4096], write: &mut usize| {
        out.push(b);
        if let Some(slot) = dict.get_mut(*write) {
            *slot = b;
        }
        *write = (*write + 1) % 4096;
    };
    let mut bytes = input.iter().copied();
    while let Some(control) = bytes.next() {
        for bit in 0..8 {
            if out.len() >= cap {
                return out;
            }
            if control & (1 << bit) == 0 {
                let Some(b) = bytes.next() else { return out };
                add(b, &mut out, &mut dict, &mut write);
            } else {
                let (Some(hi), Some(lo)) = (bytes.next(), bytes.next()) else { return out };
                let reference = u16::from_be_bytes([hi, lo]);
                let offset = usize::from(reference >> 4);
                // The reference to where the next byte goes ends the data.
                if offset == write {
                    return out;
                }
                for k in 0..usize::from(reference & 0xF) + 2 {
                    let b = dict.get((offset + k) % 4096).copied().unwrap_or(0);
                    add(b, &mut out, &mut dict, &mut write);
                }
            }
        }
    }
    out
}

/// What RTF wraps (MS-OXRTFEX 2.2.3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// HTML (`\fromhtml1`): the HTML is taken back out.
    Html,
    /// Plain text (`\fromtext`).
    Text,
    /// RTF of its own, made into simple HTML.
    Rich,
}

/// A body's RTF as the reading pane takes it.
pub fn rtf_body(rtf: &[u8]) -> TnefBody {
    if !rtf.starts_with(b"{\\rtf") {
        return TnefBody::None;
    }
    let mut mode = Mode::Rich;
    // The first ten tokens say, when they're all groups and control words.
    for t in (Lexer { s: rtf, i: 0 }).take(10) {
        match t {
            Tok::Open => {}
            Tok::Word(b"fromhtml", n) if n != Some(0) => {
                mode = Mode::Html;
                break;
            }
            Tok::Word(b"fromtext", _) => {
                mode = Mode::Text;
                break;
            }
            Tok::Word(..) => {}
            _ => break,
        }
    }
    let out = read(rtf, mode);
    match mode {
        Mode::Text => TnefBody::Text(out),
        _ if out.trim().is_empty() => TnefBody::None,
        _ => TnefBody::Html(out),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok<'a> {
    Open,
    Close,
    Word(&'a [u8], Option<i32>),
    /// `\'hh`: a byte in the current code page.
    Byte(u8),
    /// A byte of text.
    Text(u8),
    /// An escaped character (`\{`, `\~`…).
    Char(char),
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Tok<'a>;

    fn next(&mut self) -> Option<Tok<'a>> {
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'{' => return Some(Tok::Open),
                b'}' => return Some(Tok::Close),
                b'\r' | b'\n' | 0 => continue,
                b'\\' => {}
                _ => return Some(Tok::Text(c)),
            }
            let n = *self.s.get(self.i)?;
            self.i += 1;
            match n {
                b'a'..=b'z' | b'A'..=b'Z' => {
                    let start = self.i - 1;
                    while self.i - start < 32 && self.s.get(self.i).is_some_and(u8::is_ascii_alphabetic) {
                        self.i += 1;
                    }
                    let name = self.s.get(start..self.i).unwrap_or_default();
                    let num_start = self.i;
                    if self.s.get(self.i) == Some(&b'-') {
                        self.i += 1;
                    }
                    while self.i - num_start < 11 && self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                        self.i += 1;
                    }
                    let param = std::str::from_utf8(self.s.get(num_start..self.i).unwrap_or_default())
                        .ok()
                        .and_then(|p| p.parse::<i64>().ok())
                        .map(|p| p.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32);
                    if self.s.get(self.i) == Some(&b' ') {
                        self.i += 1;
                    }
                    // Binary data: skipped whole.
                    if name == b"bin" {
                        self.i = self.i.saturating_add(param.unwrap_or(0).max(0) as usize).min(self.s.len());
                        continue;
                    }
                    return Some(Tok::Word(name, param));
                }
                b'\'' => {
                    let hex = self.s.get(self.i..self.i + 2).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                    if let Some(b) = hex {
                        self.i += 2;
                        return Some(Tok::Byte(b));
                    }
                }
                b'{' | b'}' | b'\\' => return Some(Tok::Char(char::from(n))),
                b'~' => return Some(Tok::Char('\u{a0}')),
                b'_' => return Some(Tok::Char('\u{2011}')),
                b'*' => return Some(Tok::Word(b"*", None)),
                b'\r' | b'\n' => return Some(Tok::Word(b"par", None)),
                _ => {}
            }
        }
    }
}

/// Text formatting, for [`Mode::Rich`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Fmt {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Dest {
    #[default]
    Body,
    /// Shown nowhere (`\colortbl`, `\*\…`…).
    Skip,
    /// The font table: read for each font's code page.
    Fonts,
    /// A field's instruction (`HYPERLINK "…"`): read, not shown.
    FieldInst,
}

#[derive(Clone, Debug, Default)]
struct Group {
    dest: Dest,
    /// Inside `\*\htmltag`.
    tag: bool,
    /// After `\htmlrtf`: RTF that wasn't in the HTML.
    htmlrtf: bool,
    font: i32,
    /// Characters after `\uN` that stand in for it.
    uc: usize,
    fmt: Fmt,
    /// The link a field's instruction named, and the one its result shows.
    field_link: Option<String>,
    link: Option<String>,
}

/// Destinations never shown.
const SKIPPED: [&[u8]; 30] = [
    b"colortbl",
    b"stylesheet",
    b"info",
    b"pict",
    b"header",
    b"headerl",
    b"headerr",
    b"headerf",
    b"footer",
    b"footerl",
    b"footerr",
    b"footerf",
    b"footnote",
    b"xmlnstbl",
    b"listtable",
    b"listoverridetable",
    b"revtbl",
    b"rsidtbl",
    b"generator",
    b"themedata",
    b"colorschememapping",
    b"latentstyles",
    b"datastore",
    b"filetbl",
    b"pntxta",
    b"pntxtb",
    b"mhtmltag",
    b"nonshppict",
    b"objdata",
    b"comment",
];

/// A font's character set as a code page (0: the document's own).
fn charset_cp(n: i32) -> u32 {
    match n {
        77 => 10000,
        128 => 932,
        129 => 949,
        134 => 936,
        136 => 950,
        161 => 1253,
        162 => 1254,
        163 => 1258,
        177 => 1255,
        178 => 1256,
        186 => 1257,
        204 => 1251,
        222 => 874,
        238 => 1250,
        255 => 437,
        _ => 0,
    }
}

struct Out {
    mode: Mode,
    s: String,
    /// Bytes waiting to be read together (a lead byte and its trail byte), in this code page.
    pending: Vec<u8>,
    pending_cp: u32,
    /// What's open in the HTML ([`Mode::Rich`]).
    open: Fmt,
    open_link: Option<String>,
}

impl Out {
    fn byte(&mut self, b: u8, cp: u32, g: &Group) {
        if cp != self.pending_cp {
            self.flush(g);
        }
        self.pending_cp = cp;
        self.pending.push(b);
    }

    fn flush(&mut self, g: &Group) {
        if self.pending.is_empty() {
            return;
        }
        let text = decode(&self.pending, self.pending_cp);
        self.pending.clear();
        for c in text.chars() {
            self.char(c, g);
        }
    }

    /// One character of text (escaped, and formatted, in [`Mode::Rich`]).
    fn char(&mut self, c: char, g: &Group) {
        if self.s.len() >= RTF_MAX {
            return;
        }
        if self.mode != Mode::Rich {
            self.s.push(c);
            return;
        }
        self.format(g.fmt, &g.link);
        match c {
            '&' => self.s.push_str("&amp;"),
            '<' => self.s.push_str("&lt;"),
            '>' => self.s.push_str("&gt;"),
            _ => self.s.push(c),
        }
    }

    fn raw(&mut self, s: &str, g: &Group) {
        self.flush(g);
        if self.mode == Mode::Rich {
            self.format(g.fmt, &g.link);
        }
        if self.s.len() < RTF_MAX {
            self.s.push_str(s);
        }
    }

    fn format(&mut self, fmt: Fmt, link: &Option<String>) {
        if fmt == self.open && *link == self.open_link {
            return;
        }
        let close = [(self.open.strike, "</s>"), (self.open.underline, "</u>"), (self.open.italic, "</i>"), (self.open.bold, "</b>")];
        for (on, tag) in close {
            if on {
                self.s.push_str(tag);
            }
        }
        if self.open_link.is_some() {
            self.s.push_str("</a>");
        }
        if let Some(l) = link {
            self.s.push_str(&format!("<a href=\"{}\">", l.replace('"', "%22")));
        }
        let open = [(fmt.bold, "<b>"), (fmt.italic, "<i>"), (fmt.underline, "<u>"), (fmt.strike, "<s>")];
        for (on, tag) in open {
            if on {
                self.s.push_str(tag);
            }
        }
        self.open = fmt;
        self.open_link = link.clone();
    }
}

/// The address in a field's `HYPERLINK "…"` instruction (web and mail addresses only).
fn hyperlink(inst: &str) -> Option<String> {
    let rest = inst.trim_start().strip_prefix("HYPERLINK")?;
    let start = rest.find('"')? + 1;
    let url = rest.get(start..)?.split('"').next()?.trim();
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")).then(|| url.to_string())
}

fn read(rtf: &[u8], mode: Mode) -> String {
    let mut stack = vec![Group { uc: 1, ..Default::default() }];
    let mut deeper = 0usize;
    let mut fonts: HashMap<i32, u32> = HashMap::new();
    let mut default_cp = 1252u32;
    let mut out = Out { mode, s: String::new(), pending: Vec::new(), pending_cp: 1252, open: Fmt::default(), open_link: None };
    // The first token of a group, and whether it was `\*`: what says the group's destination.
    let mut fresh = false;
    let mut starred = false;
    let mut fallback = 0usize;
    let mut high_surrogate: Option<u16> = None;
    let mut inst = String::new();
    for t in (Lexer { s: rtf, i: 0 }) {
        let Some(g) = stack.last().cloned() else { break };
        if !matches!(t, Tok::Byte(_)) {
            out.flush(&g);
        }
        let first = std::mem::take(&mut fresh);
        let shows = match g.dest {
            Dest::Body => mode != Mode::Html || g.tag || !g.htmlrtf,
            _ => false,
        };
        let cp = match (g.tag, fonts.get(&g.font).copied().filter(|c| *c != 0)) {
            (false, Some(c)) => c,
            _ => default_cp,
        };
        match t {
            Tok::Open => {
                if stack.len() < MAX_GROUPS {
                    stack.push(g);
                } else {
                    deeper += 1;
                }
                fresh = true;
                starred = false;
                fallback = 0;
            }
            Tok::Close => {
                if deeper > 0 {
                    deeper -= 1;
                } else if stack.len() > 1 {
                    let closed = stack.pop();
                    if let (Some(c), Some(parent)) = (closed, stack.last_mut())
                        && c.dest == Dest::FieldInst
                        && parent.dest != Dest::FieldInst
                    {
                        parent.field_link = hyperlink(&inst);
                        inst.clear();
                    }
                    // The document ends with its group: Outlook leaves bytes of an older body after it.
                    if stack.len() == 1 {
                        break;
                    }
                }
                fallback = 0;
            }
            Tok::Word(b"*", _) if first => {
                starred = true;
                fresh = true;
            }
            Tok::Word(name, _) if first && (starred || SKIPPED.contains(&name) || matches!(name, b"fonttbl" | b"fldinst" | b"fldrslt")) => {
                let Some(top) = stack.last_mut() else { break };
                match name {
                    b"htmltag" if mode == Mode::Html => top.tag = true,
                    b"fonttbl" => top.dest = Dest::Fonts,
                    b"fldinst" => {
                        top.dest = Dest::FieldInst;
                        inst.clear();
                    }
                    b"fldrslt" => top.link = top.field_link.clone(),
                    _ => top.dest = Dest::Skip,
                }
            }
            Tok::Word(name, n) => {
                let Some(top) = stack.last_mut() else { break };
                let on = n != Some(0);
                match name {
                    b"htmlrtf" => top.htmlrtf = on,
                    b"f" => {
                        top.font = n.unwrap_or(0);
                    }
                    b"fcharset" if top.dest == Dest::Fonts => {
                        fonts.insert(top.font, charset_cp(n.unwrap_or(0)));
                    }
                    b"cpg" if top.dest == Dest::Fonts => {
                        fonts.insert(top.font, n.unwrap_or(0).max(0) as u32);
                    }
                    b"ansicpg" => default_cp = n.unwrap_or(1252).max(0) as u32,
                    b"mac" => default_cp = 10000,
                    b"pc" => default_cp = 437,
                    b"pca" => default_cp = 850,
                    // The document's default font, until a group names another.
                    b"deff" => top.font = n.unwrap_or(0),
                    b"uc" => top.uc = n.unwrap_or(1).clamp(0, 8) as usize,
                    b"u" => {
                        let unit = n.unwrap_or(0) as i16 as u16;
                        fallback = g.uc;
                        let c = match (high_surrogate.take(), unit) {
                            (_, 0xD800..=0xDBFF) => {
                                high_surrogate = Some(unit);
                                None
                            }
                            (Some(h), 0xDC00..=0xDFFF) => char::decode_utf16([h, unit]).next().and_then(Result::ok),
                            (_, u) => char::from_u32(u32::from(u)),
                        };
                        if let Some(c) = c.filter(|_| shows) {
                            out.char(c, &g);
                        } else if g.dest == Dest::FieldInst
                            && let Some(c) = c
                        {
                            inst.push(c);
                        }
                    }
                    b"par" | b"line" if shows => out.raw(if mode == Mode::Rich { "<br>\n" } else { "\r\n" }, &g),
                    b"tab" if shows => out.raw(if mode == Mode::Rich { "&nbsp;&nbsp;&nbsp;&nbsp;" } else { "\t" }, &g),
                    b"emdash" if shows => out.char('\u{2014}', &g),
                    b"endash" if shows => out.char('\u{2013}', &g),
                    b"bullet" if shows => out.char('\u{2022}', &g),
                    b"lquote" if shows => out.char('\u{2018}', &g),
                    b"rquote" if shows => out.char('\u{2019}', &g),
                    b"ldblquote" if shows => out.char('\u{201C}', &g),
                    b"rdblquote" if shows => out.char('\u{201D}', &g),
                    b"emspace" | b"enspace" | b"qmspace" if shows => out.char(' ', &g),
                    b"b" => top.fmt.bold = on,
                    b"i" => top.fmt.italic = on,
                    b"ul" => top.fmt.underline = on,
                    b"ulnone" => top.fmt.underline = false,
                    b"strike" => top.fmt.strike = on,
                    b"plain" => top.fmt = Fmt::default(),
                    _ => {}
                }
                if name != b"u" {
                    high_surrogate = None;
                }
            }
            Tok::Byte(b) | Tok::Text(b) => {
                if fallback > 0 {
                    fallback -= 1;
                    continue;
                }
                high_surrogate = None;
                if shows {
                    out.byte(b, cp, &g);
                } else if g.dest == Dest::FieldInst && inst.len() < 4096 {
                    inst.push(char::from(b));
                }
            }
            Tok::Char(c) => {
                fallback = fallback.saturating_sub(1);
                if shows {
                    out.char(c, &g);
                } else if g.dest == Dest::FieldInst && inst.len() < 4096 {
                    inst.push(c);
                }
            }
        }
    }
    if let Some(g) = stack.last().cloned() {
        out.flush(&g);
    }
    if mode == Mode::Rich {
        out.format(Fmt::default(), &None);
    }
    out.s
}
