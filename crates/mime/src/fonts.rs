//! Font sizes and typefaces of pasted HTML (a signature copied from Outlook or Word): the HTML
//! converter keeps bold, colours and links but not sizes or fonts, so where each element with
//! a `font-size` or `font-family` (or `<font size face>`) starts and ends is marked in its text
//! first, and the marks become the characters' sizes and fonts afterwards.

use crate::rich::{DEFAULT_SIZE, Style, Styles};

/// Starts a marked element; the next character (from [`INDEX_BASE`]) says which.
const START: char = '\u{E020}';
/// Ends the innermost marked element.
const END: char = '\u{E021}';
const INDEX_BASE: u32 = 0xE100;
/// Most marked elements (the rest keep the size around them).
const MAX_MARKS: usize = 3000;

/// What a marked element sets: its size in points and its typeface.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Font {
    pub size: Option<u8>,
    pub family: Option<String>,
}

/// Elements that never hold anything (no end tag).
fn void(tag: &str) -> bool {
    matches!(tag, "br" | "img" | "hr" | "meta" | "link" | "input" | "col" | "area" | "base" | "wbr" | "source" | "track" | "embed" | "param")
}

/// An attribute of a start tag (`name="v"`, `name='v'`, `name=v`), case-insensitive.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find(name)) {
        let at = from + i;
        from = at + name.len();
        let before_ok = lower.get(..at).and_then(|b| b.chars().last()).is_some_and(|c| c.is_whitespace());
        let rest = lower.get(at + name.len()..).unwrap_or("").trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let off = tag.len() - rest.len() + 1;
        let v = tag.get(off..).unwrap_or("").trim_start();
        return Some(match v.chars().next() {
            Some(q @ ('"' | '\'')) => v.get(1..).and_then(|x| x.split(q).next()).unwrap_or("").to_string(),
            _ => v.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").to_string(),
        });
    }
    None
}

/// A typeface's name from a CSS `font-family` list: the first, unquoted, or `None` for a
/// generic one or MailCraft's own (Aptos).
fn family(v: &str) -> Option<String> {
    let first = v.split(',').next()?.trim().trim_matches(['"', '\'']).trim();
    let clean: String = first.chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')).take(64).collect();
    let generic = matches!(
        clean.to_ascii_lowercase().as_str(),
        "" | "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" | "system-ui" | "inherit" | "aptos"
    );
    (!generic).then_some(clean)
}

/// Points from pixels, as a whole size (7.5 is taken as 8).
fn points(px: f32) -> Option<u8> {
    let pt = (px * 0.75).round();
    (pt.is_finite() && pt >= 1.0).then(|| u8::try_from(pt.min(96.0) as u32).unwrap_or(DEFAULT_SIZE))
}

/// What a start tag sets, if it sets a size or typeface.
fn font_of(tag: &str, name: &str) -> Option<Font> {
    let mut f = Font::default();
    if let Some(style) = attr(tag, "style") {
        for decl in style.split(';') {
            let Some((k, v)) = decl.split_once(':') else { continue };
            let v = v.trim().trim_end_matches("!important").trim();
            match k.trim().to_ascii_lowercase().as_str() {
                "font-size" => f.size = crate::layout::font_px(v).and_then(points),
                "font-family" => f.family = family(v),
                "font" => {
                    // The shorthand: `[style] [weight] size[/line] family`.
                    for word in v.split_whitespace() {
                        if let Some(px) = crate::layout::font_px(word.split('/').next().unwrap_or("")) {
                            f.size = points(px);
                            let rest = v.split_once(word).map(|(_, r)| r).unwrap_or("");
                            f.family = family(rest.trim_start_matches(|c: char| c != ' ' && c != ',').trim());
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if name == "font" {
        if let Some(size) = attr(tag, "size") {
            f.size = crate::layout::font_attr_px(&size).and_then(points).or(f.size);
        }
        if let Some(face) = attr(tag, "face") {
            f.family = family(&face).or(f.family);
        }
    }
    (f != Font::default()).then_some(f)
}

/// Mark where each element with a size or typeface starts and ends; the fonts, by mark.
pub(crate) fn mark(html: &str) -> (String, Vec<Font>) {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 64);
    let mut fonts: Vec<Font> = Vec::new();
    // Open elements: (name, marked).
    let mut open: Vec<(String, bool)> = Vec::new();
    let mut raw: Option<String> = None;
    let mut i = 0;
    while let Some(off) = lower.get(i..).and_then(|r| r.find('<')) {
        let at = i + off;
        out.push_str(html.get(i..at).unwrap_or(""));
        let Some(close) = lower.get(at..).and_then(|r| r.find('>')).map(|c| at + c) else {
            out.push_str(html.get(at..).unwrap_or(""));
            return (out, fonts);
        };
        let tag = html.get(at..=close).unwrap_or("");
        let inner = lower.get(at + 1..close).unwrap_or("");
        let end_tag = inner.starts_with('/');
        let name: String = inner.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric() || *c == ':').collect();
        if let Some(r) = &raw {
            // Inside <style> or <script>: only its end matters.
            if end_tag && name == *r {
                raw = None;
            }
            out.push_str(tag);
            i = close + 1;
            continue;
        }
        if name.is_empty() || inner.starts_with('!') || inner.starts_with('?') {
            out.push_str(tag);
        } else if end_tag {
            if let Some(pos) = open.iter().rposition(|(n, _)| *n == name) {
                for (_, marked) in open.drain(pos..).rev() {
                    if marked {
                        out.push(END);
                    }
                }
            }
            out.push_str(tag);
        } else {
            out.push_str(tag);
            if matches!(name.as_str(), "style" | "script" | "title" | "head" | "xml") {
                raw = Some(name);
            } else if !void(&name) && !inner.trim_end().ends_with('/') {
                let font = font_of(tag, &name).filter(|_| fonts.len() < MAX_MARKS);
                let marked = font.is_some();
                if let Some(f) = font
                    && let Some(index) = char::from_u32(INDEX_BASE + u32::try_from(fonts.len()).unwrap_or(0))
                {
                    out.push(START);
                    out.push(index);
                    fonts.push(f);
                }
                if open.len() < 500 {
                    open.push((name, marked));
                }
            }
        }
        i = close + 1;
    }
    out.push_str(html.get(i..).unwrap_or(""));
    (out, fonts)
}

/// The marks in a converted text as its characters' sizes and typefaces: the text and styles
/// without them (a line left holding only marks goes too).
pub(crate) fn apply(text: &str, styles: &Styles, fonts: &[Font]) -> (String, Styles) {
    let mut stack: Vec<usize> = Vec::new();
    let mut lines: Vec<(Vec<(char, Style)>, bool)> = vec![(Vec::new(), false)];
    let mut chars = text.chars().enumerate().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            START => {
                if let Some((_, idx)) = chars.next() {
                    stack.push((idx as u32).saturating_sub(INDEX_BASE) as usize);
                }
                if let Some(l) = lines.last_mut() {
                    l.1 = true;
                }
            }
            END => {
                stack.pop();
                if let Some(l) = lines.last_mut() {
                    l.1 = true;
                }
            }
            '\n' => lines.push((Vec::new(), false)),
            c => {
                let mut st = styles.style(i);
                if let Some(size) = stack.iter().rev().find_map(|k| fonts.get(*k).and_then(|f| f.size)) {
                    st.size = (size != DEFAULT_SIZE).then_some(size);
                }
                if let Some(family) = stack.iter().rev().find_map(|k| fonts.get(*k).and_then(|f| f.family.clone())) {
                    st.font = Some(family);
                }
                if let Some(l) = lines.last_mut() {
                    l.0.push((c, st));
                }
            }
        }
    }
    lines.retain(|(l, marked)| !(l.is_empty() && *marked));
    let blank = |l: &(Vec<(char, Style)>, bool)| l.0.iter().all(|(c, _)| c.is_whitespace());
    while lines.last().is_some_and(blank) {
        lines.pop();
    }
    let first = lines.iter().position(|l| !blank(l)).unwrap_or(lines.len());
    lines.drain(..first);
    let mut out = String::new();
    let mut st = Styles::plain("");
    st.chars.clear();
    st.paras.clear();
    for (n, (line, _)) in lines.iter().enumerate() {
        if n > 0 {
            out.push('\n');
            st.chars.push(0);
        }
        st.paras.push(Default::default());
        for (c, s) in line {
            let id = st.intern(s.clone());
            st.chars.push(id);
            out.push(*c);
        }
    }
    st.fit(&out);
    (out, st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_follow_the_elements() {
        let (m, f) = mark(
            "<p style='font-size:7.5pt;font-family:\"Tahoma\",sans-serif'>a<br>b</p><span>c</span><font size=5 face=Arial>d</font><style>p{font-size:9pt}</style>",
        );
        assert_eq!(f, vec![Font { size: Some(8), family: Some("Tahoma".into()) }, Font { size: Some(18), family: Some("Arial".into()) }]);
        assert_eq!(m.matches(START).count(), 2);
        assert_eq!(m.matches(END).count(), 2);
        assert!(m.find(END) < m.find("<span"), "closed with its element");
        assert_eq!(family("Aptos, sans-serif"), None);
        assert_eq!(family("'Bahnschrift Light'"), Some("Bahnschrift Light".into()));
        for junk in ["<", "<p style=", "<p style='font-size:", "</p></p><p>", "<font size=x>", "<p style='font:bold 12pt/1.2 Georgia'>x"] {
            let _ = mark(junk);
        }
        assert_eq!(mark("<p style='font:bold 12pt/1.2 Georgia, serif'>x").1, vec![Font { size: Some(12), family: Some("Georgia".into()) }]);
    }
}
