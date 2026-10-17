//! A message being written, formatted the way the person sees it: text with a style for every
//! character (bold, italic, underline, strikethrough, size, colour, highlight, link, picture)
//! and one for every paragraph (bullets or numbers, indent, alignment, or a rule).
//!
//! The text is plain (`\n` between paragraphs, U+FFFC where a picture sits), so the editor can
//! use an ordinary text box; [`Styles`] follows the text by diffing it after each change
//! ([`Styles::edit`]). Sending turns it into HTML and a plain-text twin.

use serde::{Deserialize, Serialize};

use crate::{RichText, Span};

/// Where a picture sits in the text.
pub const PICTURE: char = '\u{FFFC}';

/// Font sizes offered, in points.
pub const SIZES: [u8; 12] = [8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 36];
/// The size text has unless set.
pub const DEFAULT_SIZE: u8 = 11;
/// Most indent steps.
const MAX_INDENT: u8 = 8;

/// How a run of characters looks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub underline: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub strike: bool,
    /// Monospace (code).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mono: bool,
    /// Points; `None` is [`DEFAULT_SIZE`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// The `cid:` of the picture this character stands for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
}

impl Style {
    pub fn size(&self) -> u8 {
        self.size.unwrap_or(DEFAULT_SIZE)
    }

    /// The same look without what belongs to one spot only (a picture).
    pub fn for_typing(&self) -> Style {
        Style { picture: None, ..self.clone() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum List {
    #[default]
    None,
    Bullet,
    Number,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// How a paragraph is laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Para {
    pub list: List,
    pub indent: u8,
    pub align: Align,
    /// A horizontal rule (the line above the original under a reply); its text is empty.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub rule: bool,
}

/// The styles of a text: an index into `palette` per character, and a [`Para`] per paragraph.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Styles {
    pub chars: Vec<u16>,
    pub palette: Vec<Style>,
    pub paras: Vec<Para>,
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

impl Styles {
    /// Styles for `text` with nothing set.
    pub fn plain(text: &str) -> Styles {
        let mut s = Styles::default();
        s.fit(text);
        s
    }

    /// Make the styles match `text` (lengths), whatever happened to either.
    pub fn fit(&mut self, text: &str) {
        if self.palette.is_empty() {
            self.palette.push(Style::default());
        }
        let n = char_len(text);
        self.chars.resize(n, 0);
        let max = u16::try_from(self.palette.len().saturating_sub(1)).unwrap_or(u16::MAX);
        for c in &mut self.chars {
            if *c > max {
                *c = 0;
            }
        }
        let lines = text.split('\n').count();
        self.paras.resize(lines, Para::default());
    }

    /// The palette index of a style (added when new).
    pub fn intern(&mut self, s: Style) -> u16 {
        if self.palette.is_empty() {
            self.palette.push(Style::default());
        }
        if let Some(i) = self.palette.iter().position(|p| *p == s) {
            return u16::try_from(i).unwrap_or(0);
        }
        // A long-lived message can collect many: drop the unused ones first.
        if self.palette.len() >= usize::from(u16::MAX) {
            return 0;
        }
        self.palette.push(s);
        u16::try_from(self.palette.len() - 1).unwrap_or(0)
    }

    pub fn style(&self, i: usize) -> Style {
        self.chars.get(i).and_then(|c| self.palette.get(usize::from(*c))).cloned().unwrap_or_default()
    }

    pub fn para(&self, line: usize) -> Para {
        self.paras.get(line).copied().unwrap_or_default()
    }

    /// Whether anything is styled (else the text alone says it all).
    pub fn is_plain(&self) -> bool {
        let used_plain = self.chars.iter().all(|c| self.palette.get(usize::from(*c)).is_none_or(|s| *s == Style::default()));
        used_plain && self.paras.iter().all(|p| *p == Para::default())
    }

    /// Follow a change of the text from `old` to `new`: the characters typed take `typing`, and
    /// new paragraphs the layout of the one they were split from.
    pub fn edit(&mut self, old: &str, new: &str, typing: &Style) {
        self.edit_at(old, new, typing, None);
    }

    /// [`Styles::edit`] knowing where the cursor ended up: typing the same character as its
    /// neighbour (a second Enter) is then put where it was typed rather than guessed.
    pub fn edit_at(&mut self, old: &str, new: &str, typing: &Style, cursor: Option<usize>) {
        self.fit(old);
        let a: Vec<char> = old.chars().collect();
        let b: Vec<char> = new.chars().collect();
        let mut prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let max_suffix = a.len().min(b.len()) - prefix;
        let mut suffix = a.iter().rev().zip(b.iter().rev()).take(max_suffix).take_while(|(x, y)| x == y).count();
        // An insertion that ends at the cursor.
        if let Some(c) = cursor
            && b.len() > a.len()
            && let Some(start) = c.checked_sub(b.len() - a.len())
            && b.get(..start) == a.get(..start)
            && b.get(c..) == a.get(start..)
        {
            prefix = start;
            suffix = a.len() - start;
        }
        let removed = prefix..a.len() - suffix;
        let inserted = &b[prefix..b.len() - suffix];
        let id = self.intern(typing.for_typing());
        let ids = std::iter::repeat_n(id, inserted.len());
        self.chars.splice(removed.clone(), ids);
        // Paragraphs: the line the change starts on, the breaks it removed and added.
        let line = a.get(..prefix).map_or(0, |s| s.iter().filter(|c| **c == '\n').count());
        let gone = a.get(removed).map_or(0, |s| s.iter().filter(|c| **c == '\n').count());
        let added = inserted.iter().filter(|c| **c == '\n').count();
        let base = Para { rule: false, ..self.para(line) };
        let at = (line + 1).min(self.paras.len());
        let end = (at + gone).min(self.paras.len());
        self.paras.splice(at..end, std::iter::repeat_n(base, added));
        self.fit(new);
    }

    /// Set (or with `f` returning the new style) the style of characters `range`.
    pub fn restyle(&mut self, text: &str, range: std::ops::Range<usize>, f: impl Fn(&Style) -> Style) {
        self.fit(text);
        let end = range.end.min(self.chars.len());
        for i in range.start.min(end)..end {
            let next = f(&self.style(i));
            let id = self.intern(next);
            if let Some(c) = self.chars.get_mut(i) {
                *c = id;
            }
        }
        self.compact();
    }

    /// Change the paragraphs that characters `range` touch.
    pub fn relayout(&mut self, text: &str, range: std::ops::Range<usize>, f: impl Fn(&mut Para)) {
        self.fit(text);
        let (first, last) = lines_of(text, range);
        for p in self.paras.iter_mut().take(last + 1).skip(first) {
            f(p);
            p.indent = p.indent.min(MAX_INDENT);
        }
    }

    /// Forget palette entries no character uses.
    fn compact(&mut self) {
        if self.palette.len() < 64 {
            return;
        }
        let mut used = vec![false; self.palette.len()];
        used[0] = true;
        for c in &self.chars {
            if let Some(u) = used.get_mut(usize::from(*c)) {
                *u = true;
            }
        }
        let mut map = vec![0u16; self.palette.len()];
        let mut kept = Vec::new();
        for (i, s) in self.palette.iter().enumerate() {
            if used.get(i).copied().unwrap_or(false) {
                map[i] = u16::try_from(kept.len()).unwrap_or(0);
                kept.push(s.clone());
            }
        }
        for c in &mut self.chars {
            *c = map.get(usize::from(*c)).copied().unwrap_or(0);
        }
        self.palette = kept;
    }
}

/// The first and last paragraph (line) indices characters `range` touch.
pub fn lines_of(text: &str, range: std::ops::Range<usize>) -> (usize, usize) {
    let mut line = 0;
    let (mut first, mut last) = (None, 0);
    for (i, c) in text.chars().enumerate() {
        if i >= range.start && first.is_none() {
            first = Some(line);
        }
        if i < range.end.max(range.start + 1) {
            last = line;
        }
        if c == '\n' {
            line += 1;
        }
    }
    let first = first.unwrap_or(line);
    (first, last.max(first))
}

/// Whether every character in `range` passes `f` (for toggles: all bold → unbold).
pub fn all(text: &str, styles: &Styles, range: std::ops::Range<usize>, f: impl Fn(&Style) -> bool) -> bool {
    let n = char_len(text);
    let range = range.start.min(n)..range.end.min(n);
    !range.is_empty() && range.clone().all(|i| f(&styles.style(i)))
}

/// The next size up (or down) from `size` in [`SIZES`].
pub fn step_size(size: u8, up: bool) -> u8 {
    if up { SIZES.iter().copied().find(|s| *s > size).unwrap_or(size) } else { SIZES.iter().rev().copied().find(|s| *s < size).unwrap_or(size) }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn safe_link(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    (l.starts_with("https://") || l.starts_with("http://") || l.starts_with("mailto:")) && !u.contains(['"', '<', '>'])
}

/// Paragraphs as (text, first char index).
fn paragraphs(text: &str) -> Vec<(&str, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    for line in text.split('\n') {
        out.push((line, at));
        at += char_len(line) + 1;
    }
    out
}

/// One paragraph's runs as HTML.
fn para_html(line: &str, start: usize, styles: &Styles) -> String {
    let mut out = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let st = styles.style(start + i);
        let mut j = i + 1;
        while j < chars.len() && styles.style(start + j) == st && chars.get(j) != Some(&PICTURE) && chars.get(i) != Some(&PICTURE) {
            j += 1;
        }
        let run: String = chars.get(i..j).map(|c| c.iter().collect()).unwrap_or_default();
        if let Some(cid) = st.picture.as_ref().filter(|_| run.starts_with(PICTURE)) {
            out.push_str(&format!("<img src=\"cid:{}\" style=\"max-width:100%\">", esc(cid)));
            i = j;
            continue;
        }
        let mut h = esc(&run.replace(PICTURE, ""));
        if st.mono {
            h = format!("<code>{h}</code>");
        }
        if st.strike {
            h = format!("<s>{h}</s>");
        }
        if st.underline {
            h = format!("<u>{h}</u>");
        }
        if st.italic {
            h = format!("<i>{h}</i>");
        }
        if st.bold {
            h = format!("<b>{h}</b>");
        }
        let mut css = Vec::new();
        if let Some(sz) = st.size.filter(|s| *s != DEFAULT_SIZE) {
            css.push(format!("font-size:{sz}pt"));
        }
        if let Some(c) = st.color {
            css.push(format!("color:{}", hex(c)));
        }
        if let Some(c) = st.highlight {
            css.push(format!("background-color:{}", hex(c)));
        }
        if !css.is_empty() {
            h = format!("<span style=\"{}\">{h}</span>", css.join("; "));
        }
        if let Some(l) = st.link.as_ref().filter(|l| safe_link(l)) {
            h = format!("<a href=\"{}\">{h}</a>", esc(l.trim()));
        }
        out.push_str(&h);
        i = j;
    }
    out
}

fn para_css(p: &Para) -> String {
    let mut css = Vec::new();
    if p.indent > 0 {
        css.push(format!("margin-left:{}em", u32::from(p.indent) * 2));
    }
    match p.align {
        Align::Left => {}
        Align::Center => css.push("text-align:center".into()),
        Align::Right => css.push("text-align:right".into()),
    }
    if css.is_empty() { String::new() } else { format!(" style=\"{}\"", css.join("; ")) }
}

/// The message as HTML: a `<div>` per paragraph (as Outlook writes them), lists grouped, rules
/// as `<hr>`, pictures by `cid:`.
pub fn to_html(text: &str, styles: &Styles) -> String {
    let mut st = styles.clone();
    st.fit(text);
    let mut out = format!("<div style=\"font-family: Aptos, Calibri, 'Segoe UI', Arial, sans-serif; font-size: {DEFAULT_SIZE}pt\">\n");
    let mut open: Option<(List, u8)> = None;
    for (n, (line, start)) in paragraphs(text).into_iter().enumerate() {
        let p = st.para(n);
        let want = (p.list != List::None).then_some((p.list, p.indent));
        if want != open {
            if let Some((l, _)) = open.take() {
                out.push_str(if l == List::Number { "</ol>\n" } else { "</ul>\n" });
            }
            if let Some((l, ind)) = want {
                let margin = if ind > 0 { format!(" style=\"margin-left:{}em\"", u32::from(ind) * 2) } else { String::new() };
                out.push_str(&format!("{}{margin}>\n", if l == List::Number { "<ol" } else { "<ul" }));
                open = want;
            }
        }
        let body = para_html(line, start, &st);
        if p.rule {
            out.push_str("<hr style=\"border:none; border-top:1px solid #e1e1e1; margin:0.5em 0\">\n");
        } else if open.is_some() {
            out.push_str(&format!("<li{}>{}</li>\n", para_css(&Para { indent: 0, ..p }), if body.is_empty() { "<br>".into() } else { body }));
        } else {
            out.push_str(&format!("<div{}>{}</div>\n", para_css(&p), if body.is_empty() { "<br>".into() } else { body }));
        }
    }
    if let Some((l, _)) = open {
        out.push_str(if l == List::Number { "</ol>\n" } else { "</ul>\n" });
    }
    out.push_str("</div>\n");
    out
}

/// The message as plain text: bullets and numbers written out, indents as spaces, rules as a
/// line of underscores, links after their text, pictures by name.
pub fn to_text(text: &str, styles: &Styles) -> String {
    let mut st = styles.clone();
    st.fit(text);
    let mut out = Vec::new();
    let mut number = 0u32;
    for (n, (line, start)) in paragraphs(text).into_iter().enumerate() {
        let p = st.para(n);
        if p.rule {
            out.push(crate::markup::RULE.to_string());
            continue;
        }
        number = if p.list == List::Number { number + 1 } else { 0 };
        let indent = "    ".repeat(usize::from(p.indent));
        let mark = match p.list {
            List::None => String::new(),
            List::Bullet => "• ".into(),
            List::Number => format!("{number}. "),
        };
        let mut s = String::new();
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let style = st.style(start + i);
            let mut j = i + 1;
            while j < chars.len() && st.style(start + j) == style {
                j += 1;
            }
            let run: String = chars.get(i..j).map(|c| c.iter().collect()).unwrap_or_default();
            s.push_str(&run.replace(PICTURE, "[picture]"));
            if let Some(l) = style.link.as_ref().filter(|l| safe_link(l) && l.trim() != run.trim() && l.strip_prefix("mailto:") != Some(run.trim())) {
                s.push_str(&format!(" <{}>", l.trim()));
            }
            i = j;
        }
        out.push(format!("{indent}{mark}{s}"));
    }
    out.join("\n")
}

/// A received message's formatted body as editable text and styles (to reply or carry on a
/// draft with its formatting).
pub fn from_rich(lines: &RichText) -> (String, Styles) {
    let mut text = String::new();
    let mut styles = Styles::plain("");
    styles.chars.clear();
    styles.paras.clear();
    for (n, line) in lines.iter().enumerate() {
        if n > 0 {
            text.push('\n');
            styles.chars.push(0);
        }
        styles.paras.push(Para::default());
        for sp in line {
            let st = span_style(sp);
            let id = styles.intern(st);
            let t = if sp.image.is_some() && sp.text.trim().is_empty() { "[picture]".to_string() } else { sp.text.replace(['\n', PICTURE], " ") };
            for _ in t.chars() {
                styles.chars.push(id);
            }
            text.push_str(&t);
        }
    }
    styles.fit(&text);
    (text, styles)
}

fn span_style(sp: &Span) -> Style {
    let heading = match sp.heading {
        1 => Some(20),
        2 => Some(16),
        3 => Some(14),
        4..=6 => Some(12),
        _ => None,
    };
    Style {
        bold: sp.bold || sp.heading > 0,
        italic: sp.italic,
        strike: sp.strike,
        mono: sp.code,
        size: heading,
        color: sp.color,
        highlight: sp.background,
        link: sp.link.clone().filter(|l| safe_link(l)),
        ..Default::default()
    }
}

/// Text written with light marks (`**bold**`, `*italic*`, `~~struck~~`, `` `code` ``,
/// `[text](url)`, `![name](cid:…)`, `# ` headings, `- ` bullets, `1. ` numbers, `> ` quotes, a
/// line of underscores for a rule), as text and styles: for agents and the command line.
pub fn from_markup(marked: &str) -> (String, Styles) {
    use crate::markup::{Block, Inline, block, inlines};
    let mut text = String::new();
    let mut styles = Styles::plain("");
    styles.chars.clear();
    styles.paras.clear();
    let push = |text: &mut String, styles: &mut Styles, s: &str, st: Style| {
        let id = styles.intern(st);
        for _ in s.chars() {
            styles.chars.push(id);
        }
        text.push_str(s);
    };
    for (n, line) in marked.replace("\r\n", "\n").split('\n').enumerate() {
        if n > 0 {
            push(&mut text, &mut styles, "\n", Style::default());
        }
        let (b, start) = block(line);
        let mut para = Para::default();
        let mut base = Style::default();
        match b {
            Block::Heading(h) => {
                base.bold = true;
                base.size = Some(match h {
                    1 => 20,
                    2 => 16,
                    _ => 14,
                });
            }
            Block::Bullet => para.list = List::Bullet,
            Block::Numbered => para.list = List::Number,
            Block::Quote => {
                para.indent = 1;
                base.color = Some([0x55, 0x55, 0x55]);
            }
            Block::Rule => para.rule = true,
            Block::Para | Block::SigDash | Block::Blank => {}
        }
        styles.paras.push(para);
        if para.rule {
            continue;
        }
        let content = if b == Block::SigDash { line } else { line.get(start..).unwrap_or("") };
        for piece in inlines(content) {
            let (s, st) = match piece {
                Inline::Text(s) => (s.to_string(), base.clone()),
                Inline::Bold(s) => (s.to_string(), Style { bold: true, ..base.clone() }),
                Inline::Italic(s) => (s.to_string(), Style { italic: true, ..base.clone() }),
                Inline::Strike(s) => (s.to_string(), Style { strike: true, ..base.clone() }),
                Inline::Code(s) => (s.to_string(), Style { mono: true, ..base.clone() }),
                Inline::Link { text, url } => (text.to_string(), Style { link: Some(url.to_string()), ..base.clone() }),
                Inline::Url(u) => (u.to_string(), Style { link: Some(u.to_string()), ..base.clone() }),
                Inline::Picture { src, .. } => match src.strip_prefix("cid:") {
                    Some(cid) => (PICTURE.to_string(), Style { picture: Some(cid.to_string()), ..base.clone() }),
                    None => (String::new(), base.clone()),
                },
            };
            push(&mut text, &mut styles, &s, st);
        }
    }
    styles.fit(&text);
    (text, styles)
}

/// The `cid:`s of the pictures in a text.
pub fn pictures(text: &str, styles: &Styles) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for (i, c) in text.chars().enumerate() {
        if c == PICTURE
            && let Some(cid) = styles.style(i).picture
            && !v.contains(&cid)
        {
            v.push(cid);
        }
    }
    v
}

/// Characters `range` of a styled text, with their styles and their paragraphs' layout.
pub fn slice(text: &str, styles: &Styles, range: std::ops::Range<usize>) -> (String, Styles) {
    let n = char_len(text);
    let range = range.start.min(n)..range.end.min(n).max(range.start.min(n));
    let part: String = text.chars().skip(range.start).take(range.end - range.start).collect();
    let mut out = Styles { chars: Vec::new(), palette: styles.palette.clone(), paras: Vec::new() };
    let mut full = styles.clone();
    full.fit(text);
    out.chars = full.chars.get(range.clone()).map(<[u16]>::to_vec).unwrap_or_default();
    let (first, _) = lines_of(text, range.start..range.start);
    let lines = part.split('\n').count();
    out.paras = (first..first + lines).map(|l| full.para(l)).collect();
    out.fit(&part);
    (part, out)
}

/// Put `add` (text and styles) in place of characters `range` of `text`: the paragraph it goes
/// into keeps its layout, as text pasted into a line does.
pub fn splice(text: &mut String, styles: &mut Styles, range: std::ops::Range<usize>, add: &str, add_styles: &Styles) {
    let n = char_len(text);
    let (start, end) = (range.start.min(n), range.end.min(n).max(range.start.min(n)));
    let (mut out, mut out_styles) = slice(text, styles, 0..start);
    let (rest, rest_styles) = slice(text, styles, end..n);
    append(&mut out, &mut out_styles, add, add_styles);
    append(&mut out, &mut out_styles, &rest, &rest_styles);
    *text = out;
    *styles = out_styles;
}

/// Append `add` (text and styles) to `text` and `styles`.
pub fn append(text: &mut String, styles: &mut Styles, add: &str, add_styles: &Styles) {
    styles.fit(text);
    let mut other = add_styles.clone();
    other.fit(add);
    let map: Vec<u16> = other.palette.iter().map(|s| styles.intern(s.clone())).collect();
    // The paragraph `text` ends in continues with `add`'s first: an empty, unformatted one takes
    // its layout, any other keeps its own.
    let mut paras = other.paras.into_iter();
    if let (Some(first), Some(last)) = (paras.next(), styles.paras.last_mut())
        && (text.ends_with('\n') || text.is_empty())
        && *last == Para::default()
    {
        *last = first;
    }
    styles.paras.extend(paras);
    styles.chars.extend(other.chars.iter().map(|c| map.get(usize::from(*c)).copied().unwrap_or(0)));
    text.push_str(add);
    styles.fit(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> Style {
        Style { bold: true, ..Default::default() }
    }

    #[test]
    fn typing_follows_the_text() {
        let mut s = Styles::plain("Hello world");
        s.restyle("Hello world", 0..5, |st| Style { bold: true, ..st.clone() });
        // Typed inside the bold word: bold; deleted across: gone.
        s.edit("Hello world", "Helllo world", &bold());
        assert_eq!(s.chars.len(), 12);
        assert!(s.style(3).bold && !s.style(7).bold);
        s.edit("Helllo world", "Hd", &Style::default());
        assert_eq!((s.chars.len(), s.style(0).bold, s.style(1).bold), (2, true, false));
        // A new paragraph takes the layout of the one it split.
        let mut s = Styles::plain("one\ntwo");
        s.relayout("one\ntwo", 0..1, |p| p.list = List::Bullet);
        // Enter at the end of "one", then "new": typed where the cursor is.
        s.edit_at("one\ntwo", "one\nnew\ntwo", &Style::default(), Some(7));
        assert_eq!(s.paras.iter().map(|p| p.list).collect::<Vec<_>>(), [List::Bullet, List::Bullet, List::None]);
        s.edit("one\nnew\ntwo", "one two", &Style::default());
        assert_eq!(s.paras.len(), 1);
        // Junk lengths are fixed rather than trusted.
        let mut j = Styles { chars: vec![9, 9, 9, 9, 9, 9], palette: vec![], paras: vec![] };
        j.fit("ab");
        assert_eq!((j.chars.as_slice(), j.paras.len()), (&[0u16, 0][..], 1));
    }

    #[test]
    fn html_and_text() {
        let text = "Hi team\nfirst\nsecond\n\nsee site\nx\u{FFFC}";
        let mut s = Styles::plain(text);
        s.restyle(text, 0..2, |st| Style { bold: true, color: Some([200, 0, 0]), ..st.clone() });
        s.relayout(text, 8..10, |p| p.list = List::Number);
        s.relayout(text, 14..16, |p| p.list = List::Number);
        s.restyle(text, 26..30, |st| Style { link: Some("https://a.example/".into()), ..st.clone() });
        s.relayout(text, 22..23, |p| p.align = Align::Center);
        s.restyle(text, 32..33, |st| Style { picture: Some("pic@x".into()), ..st.clone() });
        s.restyle(text, 3..7, |st| Style { size: Some(18), highlight: Some([255, 255, 0]), italic: true, underline: true, ..st.clone() });
        let html = to_html(text, &s);
        assert!(html.contains("<span style=\"color:#c80000\"><b>Hi</b></span>"), "{html}");
        assert!(html.contains("<span style=\"font-size:18pt; background-color:#ffff00\"><i><u>team</u></i></span>"), "{html}");
        assert!(html.contains("<ol>\n<li>first</li>\n<li>second</li>\n</ol>"), "{html}");
        assert!(html.contains("<div style=\"text-align:center\">see <a href=\"https://a.example/\">site</a></div>"), "{html}");
        assert!(html.contains("<div><br></div>") && html.contains("x<img src=\"cid:pic@x\""), "{html}");
        let t = to_text(text, &s);
        assert_eq!(t, "Hi team\n1. first\n2. second\n\nsee site <https://a.example/>\nx[picture]");
        // Nothing typed can become markup.
        let evil = "<script>x</script>";
        assert!(!to_html(evil, &Styles::plain(evil)).contains("<script>"));
        let mut s = Styles::plain("a");
        s.restyle("a", 0..1, |st| Style { link: Some("javascript:alert(1)".into()), ..st.clone() });
        assert!(!to_html("a", &s).contains("javascript"));
    }

    #[test]
    fn rules_received_formatting_and_appending() {
        let mut text = "Thanks".to_string();
        let mut styles = Styles::plain(&text);
        let (orig, ost) = from_rich(&vec![
            vec![Span { text: "Big".into(), heading: 1, ..Default::default() }],
            vec![Span { text: "plain ".into(), ..Default::default() }, Span { text: "red".into(), color: Some([255, 0, 0]), ..Default::default() }],
        ]);
        assert_eq!(orig, "Big\nplain red");
        assert!(ost.style(0).bold && ost.style(0).size == Some(20) && ost.style(12).color == Some([255, 0, 0]));
        let mut rule = Styles::plain("\n\n");
        rule.paras[1].rule = true;
        append(&mut text, &mut styles, "\n\n", &rule);
        append(&mut text, &mut styles, &orig, &ost);
        assert_eq!(text, "Thanks\n\nBig\nplain red");
        assert!(styles.paras[1].rule && !styles.paras[2].rule);
        assert!(styles.style(8).bold);
        assert!(to_html(&text, &styles).contains("<hr"));
        assert_eq!(to_text(&text, &styles).lines().nth(1), Some(crate::markup::RULE));
        assert_eq!((step_size(11, true), step_size(11, false), step_size(36, true)), (12, 10, 36));
        assert_eq!(lines_of("ab\ncd\nef", 1..4), (0, 1));
        assert!(all("abc", &styles_bold("abc"), 0..3, |s| s.bold) && !all("abc", &styles_bold("abc"), 0..0, |s| s.bold));
    }

    fn styles_bold(t: &str) -> Styles {
        let mut s = Styles::plain(t);
        s.restyle(t, 0..t.len(), |_| bold());
        s
    }
}
