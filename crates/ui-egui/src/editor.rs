//! The message editor's formatting: the marks (`**bold**`, `- list`, `[link](…)`) styled as
//! they are typed, and the toolbar's buttons that add or remove them around the selection.

use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Stroke};
use mailcraft_engine::mime::markup::{Block, Inline, block, inlines};

use crate::theme::{self, Tokens};

/// A formatting button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fmt {
    Bold,
    Italic,
    Strike,
    Code,
    Link,
    Bullet,
    Numbered,
    Quote,
    Heading,
}

impl Fmt {
    pub const ALL: [Fmt; 9] = [Fmt::Bold, Fmt::Italic, Fmt::Strike, Fmt::Code, Fmt::Link, Fmt::Bullet, Fmt::Numbered, Fmt::Quote, Fmt::Heading];

    pub fn label(self) -> &'static str {
        match self {
            Fmt::Bold => "B",
            Fmt::Italic => "I",
            Fmt::Strike => "S",
            Fmt::Code => "</>",
            Fmt::Link => "Link",
            Fmt::Bullet => "• List",
            Fmt::Numbered => "1. List",
            Fmt::Quote => "Quote",
            Fmt::Heading => "Heading",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Fmt::Bold => "Bold (Ctrl+B)",
            Fmt::Italic => "Italic (Ctrl+I)",
            Fmt::Strike => "Strikethrough",
            Fmt::Code => "Code",
            Fmt::Link => "Link (Ctrl+K)",
            Fmt::Bullet => "Bulleted list",
            Fmt::Numbered => "Numbered list",
            Fmt::Quote => "Quote",
            Fmt::Heading => "Heading",
        }
    }
}

fn byte_of(s: &str, ch: usize) -> usize {
    s.char_indices().nth(ch).map_or(s.len(), |(b, _)| b)
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// Apply a format to `body` over the character range `sel`; returns the new selection.
pub fn apply(body: &mut String, sel: Range<usize>, f: Fmt) -> Range<usize> {
    let (a, b) = (byte_of(body, sel.start.min(sel.end)), byte_of(body, sel.start.max(sel.end)));
    let wrap = |body: &mut String, open: &str, close: &str| -> Range<usize> {
        let picked = body.get(a..b).unwrap_or("").to_string();
        // Already wrapped: unwrap.
        let before = body.get(..a).unwrap_or("");
        let after = body.get(b..).unwrap_or("");
        if before.ends_with(open) && after.starts_with(close) && !picked.is_empty() {
            let start = a - open.len();
            body.replace_range(start..b + close.len(), &picked);
            let s = chars(body.get(..start).unwrap_or(""));
            return s..s + chars(&picked);
        }
        body.replace_range(a..b, &format!("{open}{picked}{close}"));
        let s = chars(body.get(..a).unwrap_or("")) + chars(open);
        s..s + chars(&picked)
    };
    match f {
        Fmt::Bold => wrap(body, "**", "**"),
        Fmt::Italic => wrap(body, "*", "*"),
        Fmt::Strike => wrap(body, "~~", "~~"),
        Fmt::Code => wrap(body, "`", "`"),
        Fmt::Link => {
            let picked = body.get(a..b).unwrap_or("").to_string();
            let (text, url) = if picked.starts_with("https://") || picked.starts_with("http://") {
                (picked.clone(), picked.clone())
            } else {
                (picked.clone(), "https://".to_string())
            };
            let text = if text.is_empty() { "link".to_string() } else { text };
            body.replace_range(a..b, &format!("[{text}]({url})"));
            // Select the address, ready to type over.
            let s = chars(body.get(..a).unwrap_or("")) + chars(&text) + 3;
            s..s + chars(&url)
        }
        Fmt::Bullet | Fmt::Numbered | Fmt::Quote | Fmt::Heading => {
            let line_start = body.get(..a).and_then(|p| p.rfind('\n')).map_or(0, |i| i + 1);
            let line_end = body.get(b..).and_then(|r| r.find('\n')).map_or(body.len(), |i| b + i);
            let lines: Vec<String> = body.get(line_start..line_end).unwrap_or("").split('\n').map(str::to_string).collect();
            let want = |l: &str| match f {
                Fmt::Bullet => block(l).0 == Block::Bullet,
                Fmt::Numbered => block(l).0 == Block::Numbered,
                Fmt::Quote => block(l).0 == Block::Quote,
                _ => matches!(block(l).0, Block::Heading(_)),
            };
            // Toggle: take the mark off when every line has it, else put it on.
            let all = lines.iter().all(|l| want(l));
            let out: Vec<String> = lines
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let (kind, start) = block(l);
                    let content = if matches!(kind, Block::Bullet | Block::Numbered | Block::Quote | Block::Heading(_)) {
                        l.get(start..).unwrap_or("")
                    } else {
                        l.as_str()
                    };
                    if all {
                        content.to_string()
                    } else {
                        match f {
                            Fmt::Bullet => format!("- {content}"),
                            Fmt::Numbered => format!("{}. {content}", i + 1),
                            Fmt::Quote => format!("> {content}"),
                            _ => format!("## {content}"),
                        }
                    }
                })
                .collect();
            let joined = out.join("\n");
            body.replace_range(line_start..line_end, &joined);
            let s = chars(body.get(..line_start).unwrap_or(""));
            s..s + chars(&joined)
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Normal,
    Mark,
    Bold,
    Italic,
    Strike,
    Code,
    Link,
    Picture,
    Heading(u8),
    Quote,
}

/// Lay the editor's text out with its formatting shown: marks dimmed, content styled.
pub fn layout(text: &str, t: &Tokens, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let size = 14.5;
    let fmt = |s: Style| -> TextFormat {
        let base = TextFormat { font_id: theme::regular(size), color: t.text, ..Default::default() };
        match s {
            Style::Normal => base,
            Style::Mark => TextFormat { color: t.faint, ..base },
            Style::Bold => TextFormat { font_id: theme::bold(size), ..base },
            Style::Italic => TextFormat { italics: true, ..base },
            Style::Strike => TextFormat { strikethrough: Stroke::new(1.0, t.text), ..base },
            Style::Code => TextFormat { font_id: FontId::monospace(13.5), background: t.chrome, ..base },
            Style::Link => TextFormat { color: t.accent, underline: Stroke::new(1.0, t.accent), ..base },
            Style::Picture => TextFormat { color: t.accent, italics: true, ..base },
            Style::Heading(n) => TextFormat {
                font_id: theme::bold(match n {
                    1 => 20.0,
                    2 => 17.5,
                    _ => 16.0,
                }),
                ..base
            },
            Style::Quote => TextFormat { color: t.dim, ..base },
        }
    };
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            job.append("\n", 0.0, fmt(Style::Normal));
        }
        let (kind, start) = block(line);
        let base = match kind {
            Block::Heading(l) => Style::Heading(l),
            Block::Quote => Style::Quote,
            _ => Style::Normal,
        };
        let mut styles: Vec<Style> = vec![base; line.len()];
        for st in styles.iter_mut().take(start) {
            *st = Style::Mark;
        }
        let content = line.get(start..).unwrap_or("");
        // Every byte inside a piece takes the piece's style; the rest of the content is marks.
        let origin = line.as_ptr() as usize;
        let mut covered = vec![false; line.len()];
        for p in inlines(content) {
            let (inner, style) = match p {
                Inline::Text(s) => (s, base),
                Inline::Bold(s) => (s, if base == Style::Normal { Style::Bold } else { base }),
                Inline::Italic(s) => (s, Style::Italic),
                Inline::Strike(s) => (s, Style::Strike),
                Inline::Code(s) => (s, Style::Code),
                Inline::Link { text, .. } => (text, Style::Link),
                Inline::Url(s) => (s, Style::Link),
                Inline::Picture { alt, .. } => (alt, Style::Picture),
            };
            let from = (inner.as_ptr() as usize).saturating_sub(origin);
            for i in from..(from + inner.len()).min(line.len()) {
                if let Some(s) = styles.get_mut(i) {
                    *s = style;
                }
                if let Some(c) = covered.get_mut(i) {
                    *c = true;
                }
            }
        }
        for i in start..line.len() {
            if !covered.get(i).copied().unwrap_or(true)
                && let Some(s) = styles.get_mut(i)
            {
                *s = Style::Mark;
            }
        }
        // Emit runs of one style (on character boundaries).
        let mut run_start = 0;
        let mut cur = styles.first().copied().unwrap_or(base);
        for (i, _) in line.char_indices().skip(1) {
            let s = styles.get(i).copied().unwrap_or(base);
            if s != cur {
                job.append(line.get(run_start..i).unwrap_or(""), 0.0, fmt(cur));
                run_start = i;
                cur = s;
            }
        }
        job.append(line.get(run_start..).unwrap_or(""), 0.0, fmt(cur));
    }
    if text.is_empty() {
        job.append("", 0.0, fmt(Style::Normal));
    }
    let _ = Color32::TRANSPARENT;
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(body: &str, sel: Range<usize>, f: Fmt) -> (String, Range<usize>) {
        let mut b = body.to_string();
        let r = apply(&mut b, sel, f);
        (b, r)
    }

    #[test]
    fn wrap_and_unwrap() {
        assert_eq!(run("say hello", 4..9, Fmt::Bold), ("say **hello**".into(), 6..11));
        assert_eq!(run("say **hello**", 6..11, Fmt::Bold), ("say hello".into(), 4..9), "a second press undoes it");
        assert_eq!(run("é x", 2..3, Fmt::Italic), ("é *x*".into(), 3..4), "character, not byte, positions");
        assert_eq!(run("", 0..0, Fmt::Code), ("``".into(), 1..1), "nothing selected: the cursor goes between the marks");
        let (b, r) = run("see site", 4..8, Fmt::Link);
        assert_eq!(b, "see [site](https://)");
        assert_eq!(b.chars().skip(r.start).take(r.end - r.start).collect::<String>(), "https://");
    }

    #[test]
    fn line_marks_toggle() {
        assert_eq!(run("one\ntwo\nthree", 1..6, Fmt::Bullet).0, "- one\n- two\nthree");
        assert_eq!(run("- one\n- two", 0..9, Fmt::Bullet).0, "one\ntwo");
        assert_eq!(run("a\nb", 0..3, Fmt::Numbered).0, "1. a\n2. b");
        assert_eq!(run("- a", 0..0, Fmt::Quote).0, "> a", "one mark replaces another");
        assert_eq!(run("Title", 0..0, Fmt::Heading).0, "## Title");
    }

    #[test]
    fn layout_covers_every_character() {
        let t = Tokens::light();
        for text in ["", "plain", "# H\n- **b** *i* [l](https://x.example) `c`\n> q ![p](cid:1@x) é", "**unclosed"] {
            let job = layout(text, &t, 400.0);
            assert_eq!(job.text, text, "the layout shows the text exactly");
        }
    }
}
