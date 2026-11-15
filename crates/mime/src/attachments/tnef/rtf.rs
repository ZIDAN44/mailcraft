//! Rich Text Format as Outlook keeps a body: compressed (MS-OXRTFCP), then read as the HTML or
//! the text it wraps (MS-OXRTFEX), or, when it wraps neither, as simple HTML of its own (bold,
//! italic, underline, struck-through text, links and line breaks).

use std::collections::HashMap;
use std::rc::Rc;

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

/// What RTF wraps: the first ten tokens say, when they're all groups and control words.
fn mode(rtf: &[u8]) -> Mode {
    for t in (Lexer { s: rtf, i: 0 }).take(10) {
        match t {
            Tok::Open => {}
            Tok::Word(b"fromhtml", n) if n != Some(0) => return Mode::Html,
            Tok::Word(b"fromtext", _) => return Mode::Text,
            Tok::Word(..) => {}
            _ => break,
        }
    }
    Mode::Rich
}

/// Whether RTF wraps HTML (`\fromhtml1`), so [`rtf_body`] gives that HTML back rather than
/// simple HTML of the RTF's own, which leaves out tables, pictures and objects.
pub fn rtf_wraps_html(rtf: &[u8]) -> bool {
    rtf.starts_with(b"{\\rtf") && mode(rtf) == Mode::Html
}

/// A body's RTF as the reading pane takes it.
pub fn rtf_body(rtf: &[u8]) -> TnefBody {
    if !rtf.starts_with(b"{\\rtf") {
        return TnefBody::None;
    }
    let mode = mode(rtf);
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
    /// The link a field's instruction named, and the one its result shows (shared: every token
    /// of text takes a copy of its group).
    field_link: Option<Rc<str>>,
    link: Option<Rc<str>>,
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
    open_link: Option<Rc<str>>,
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
        if self.s.len() >= RTF_MAX {
            return;
        }
        if self.mode == Mode::Rich {
            self.format(g.fmt, &g.link);
        }
        self.s.push_str(s);
    }

    /// Close what's open and open `fmt` inside `link`, which is written again only when it
    /// changes. Nothing is written once the HTML has [`RTF_MAX`] bytes: a link can be thousands
    /// of bytes long, and RTF can change its formatting every few bytes.
    fn format(&mut self, fmt: Fmt, link: &Option<Rc<str>>) {
        if (fmt == self.open && *link == self.open_link) || self.s.len() >= RTF_MAX {
            return;
        }
        self.close_tags();
        if *link != self.open_link {
            if self.open_link.is_some() {
                self.s.push_str("</a>");
            }
            if let Some(l) = link {
                self.s.push_str(&format!("<a href=\"{}\">", l.replace('"', "%22")));
            }
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

    fn close_tags(&mut self) {
        let close = [(self.open.strike, "</s>"), (self.open.underline, "</u>"), (self.open.italic, "</i>"), (self.open.bold, "</b>")];
        for (on, tag) in close {
            if on {
                self.s.push_str(tag);
            }
        }
    }

    /// Close what's still open, at the end: a few bytes, written past the cap too.
    fn close(&mut self) {
        self.close_tags();
        if self.open_link.take().is_some() {
            self.s.push_str("</a>");
        }
        self.open = Fmt::default();
    }
}

/// The address in a field's `HYPERLINK "…"` instruction (web and mail addresses only).
fn hyperlink(inst: &str) -> Option<String> {
    let rest = inst.trim_start().strip_prefix("HYPERLINK")?;
    let start = rest.find('"')? + 1;
    let url = rest.get(start..)?.split('"').next()?.trim();
    crate::markup::safe_scheme(url).then(|| url.to_string())
}

fn read(rtf: &[u8], mode: Mode) -> String {
    let mut r = Reader {
        mode,
        stack: vec![Group { uc: 1, ..Default::default() }],
        deeper: 0,
        fonts: HashMap::new(),
        default_cp: 1252,
        out: Out { mode, s: String::new(), pending: Vec::new(), pending_cp: 1252, open: Fmt::default(), open_link: None },
        fresh: false,
        starred: false,
        fallback: 0,
        high_surrogate: None,
        inst: String::new(),
    };
    for t in (Lexer { s: rtf, i: 0 }) {
        if !r.token(t) {
            break;
        }
    }
    if let Some(g) = r.stack.last().cloned() {
        r.out.flush(&g);
    }
    if mode == Mode::Rich {
        r.out.close();
    }
    r.out.s
}

/// What [`read`] keeps as it goes through the tokens.
struct Reader {
    mode: Mode,
    stack: Vec<Group>,
    /// Groups opened past [`MAX_GROUPS`] and not closed yet.
    deeper: usize,
    /// Each font's code page.
    fonts: HashMap<i32, u32>,
    default_cp: u32,
    out: Out,
    /// The first token of a group, and whether it was `\*`: what says the group's destination.
    fresh: bool,
    starred: bool,
    /// Characters after a `\uN` that stand in for it, still to skip.
    fallback: usize,
    high_surrogate: Option<u16>,
    /// A field's instruction, as far as it's read.
    inst: String,
}

impl Reader {
    /// One token; `false` once the document has ended.
    fn token(&mut self, t: Tok) -> bool {
        let Some(g) = self.stack.last().cloned() else { return false };
        if !matches!(t, Tok::Byte(_)) {
            self.out.flush(&g);
        }
        let first = std::mem::take(&mut self.fresh);
        let shows = match g.dest {
            Dest::Body => self.mode != Mode::Html || g.tag || !g.htmlrtf,
            _ => false,
        };
        match t {
            Tok::Open => {
                if self.stack.len() < MAX_GROUPS {
                    self.stack.push(g);
                } else {
                    self.deeper += 1;
                }
                self.fresh = true;
                self.starred = false;
                self.fallback = 0;
            }
            Tok::Close => return self.close(),
            Tok::Word(b"*", _) if first => {
                self.starred = true;
                self.fresh = true;
            }
            Tok::Word(name, _) if first && (self.starred || SKIPPED.contains(&name) || matches!(name, b"fonttbl" | b"fldinst" | b"fldrslt")) => {
                self.destination(name);
            }
            Tok::Word(name, n) => self.word(name, n, &g, shows),
            Tok::Byte(b) | Tok::Text(b) => {
                if self.fallback > 0 {
                    self.fallback -= 1;
                    return true;
                }
                self.high_surrogate = None;
                if shows {
                    let cp = match (g.tag, self.fonts.get(&g.font).copied().filter(|c| *c != 0)) {
                        (false, Some(c)) => c,
                        _ => self.default_cp,
                    };
                    self.out.byte(b, cp, &g);
                } else if g.dest == Dest::FieldInst && self.inst.len() < 4096 {
                    self.inst.push(char::from(b));
                }
            }
            Tok::Char(c) => {
                self.fallback = self.fallback.saturating_sub(1);
                if shows {
                    self.out.char(c, &g);
                } else if g.dest == Dest::FieldInst && self.inst.len() < 4096 {
                    self.inst.push(c);
                }
            }
        }
        true
    }

    /// A group's end; `false` at the document's own.
    fn close(&mut self) -> bool {
        if self.deeper > 0 {
            self.deeper -= 1;
        } else if self.stack.len() > 1 {
            let closed = self.stack.pop();
            if let (Some(c), Some(parent)) = (closed, self.stack.last_mut())
                && c.dest == Dest::FieldInst
                && parent.dest != Dest::FieldInst
            {
                parent.field_link = hyperlink(&self.inst).map(Rc::from);
                self.inst.clear();
            }
            // The document ends with its group: Outlook leaves bytes of an older body after it.
            if self.stack.len() == 1 {
                return false;
            }
        }
        self.fallback = 0;
        true
    }

    /// The word that starts a group and says what it holds.
    fn destination(&mut self, name: &[u8]) {
        let Some(top) = self.stack.last_mut() else { return };
        match name {
            b"htmltag" if self.mode == Mode::Html => top.tag = true,
            b"fonttbl" => top.dest = Dest::Fonts,
            b"fldinst" => {
                top.dest = Dest::FieldInst;
                self.inst.clear();
            }
            b"fldrslt" => top.link = top.field_link.clone(),
            _ => top.dest = Dest::Skip,
        }
    }

    /// A control word (`\b`, `\par`, `\u8364`…); `g` is its group as it was before it.
    fn word(&mut self, name: &[u8], n: Option<i32>, g: &Group, shows: bool) {
        let mode = self.mode;
        let Some(top) = self.stack.last_mut() else { return };
        let on = n != Some(0);
        match name {
            b"htmlrtf" => top.htmlrtf = on,
            b"f" => {
                top.font = n.unwrap_or(0);
            }
            b"fcharset" if top.dest == Dest::Fonts => {
                self.fonts.insert(top.font, charset_cp(n.unwrap_or(0)));
            }
            b"cpg" if top.dest == Dest::Fonts => {
                self.fonts.insert(top.font, n.unwrap_or(0).max(0) as u32);
            }
            b"ansicpg" => self.default_cp = n.unwrap_or(1252).max(0) as u32,
            b"mac" => self.default_cp = 10000,
            b"pc" => self.default_cp = 437,
            b"pca" => self.default_cp = 850,
            // The document's default font, until a group names another.
            b"deff" => top.font = n.unwrap_or(0),
            b"uc" => top.uc = n.unwrap_or(1).clamp(0, 8) as usize,
            b"u" => self.unicode(n, g, shows),
            b"par" | b"line" if shows => self.out.raw(if mode == Mode::Rich { "<br>\n" } else { "\r\n" }, g),
            b"tab" if shows => self.out.raw(if mode == Mode::Rich { "&nbsp;&nbsp;&nbsp;&nbsp;" } else { "\t" }, g),
            b"emdash" if shows => self.out.char('\u{2014}', g),
            b"endash" if shows => self.out.char('\u{2013}', g),
            b"bullet" if shows => self.out.char('\u{2022}', g),
            b"lquote" if shows => self.out.char('\u{2018}', g),
            b"rquote" if shows => self.out.char('\u{2019}', g),
            b"ldblquote" if shows => self.out.char('\u{201C}', g),
            b"rdblquote" if shows => self.out.char('\u{201D}', g),
            b"emspace" | b"enspace" | b"qmspace" if shows => self.out.char(' ', g),
            b"b" => top.fmt.bold = on,
            b"i" => top.fmt.italic = on,
            b"ul" => top.fmt.underline = on,
            b"ulnone" => top.fmt.underline = false,
            b"strike" => top.fmt.strike = on,
            b"plain" => top.fmt = Fmt::default(),
            _ => {}
        }
        if name != b"u" {
            self.high_surrogate = None;
        }
    }

    /// `\uN`: a UTF-16 unit (a surrogate pair takes two), then `\uc` characters standing in for it.
    fn unicode(&mut self, n: Option<i32>, g: &Group, shows: bool) {
        let unit = n.unwrap_or(0) as i16 as u16;
        self.fallback = g.uc;
        let c = match (self.high_surrogate.take(), unit) {
            (_, 0xD800..=0xDBFF) => {
                self.high_surrogate = Some(unit);
                None
            }
            (Some(h), 0xDC00..=0xDFFF) => char::decode_utf16([h, unit]).next().and_then(Result::ok),
            (_, u) => char::from_u32(u32::from(u)),
        };
        if let Some(c) = c.filter(|_| shows) {
            self.out.char(c, g);
        } else if g.dest == Dest::FieldInst
            && self.inst.len() < 4096
            && let Some(c) = c
        {
            self.inst.push(c);
        }
    }
}
