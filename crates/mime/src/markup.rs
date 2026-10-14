//! The formatting marks of the message editor, and what they become when sent.
//!
//! The editor keeps plain text with light marks, styled as they are typed; sending turns them
//! into HTML (and a clean plain-text twin):
//!
//! | Mark | Means |
//! |---|---|
//! | `**bold**`, `*italic*`, `~~struck~~`, `` `code` `` | inline emphasis |
//! | `[text](https://…)` | a link (`http(s):` and `mailto:` only) |
//! | `![name](cid:…)` | a picture sent with the message |
//! | `# `, `## `, `### ` at the start of a line | headings |
//! | `- ` or `* `, `1. ` | bulleted and numbered lists |
//! | `> ` | quoted text (replies quote this way) |
//!
//! Bare web addresses become links. Everything else is text: `<`, `&` and quotes are escaped, so
//! nothing typed can turn into markup the recipient's client would run.

/// One piece of a line, for the editor's styling and the converters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline<'a> {
    Text(&'a str),
    Bold(&'a str),
    Italic(&'a str),
    Strike(&'a str),
    Code(&'a str),
    Link {
        text: &'a str,
        url: &'a str,
    },
    Picture {
        alt: &'a str,
        src: &'a str,
    },
    /// A bare web address.
    Url(&'a str),
}

/// What a line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Para,
    Heading(u8),
    Bullet,
    Numbered,
    Quote,
    /// The `-- ` line before a signature.
    SigDash,
    Blank,
}

/// The kind of a line and where its content starts (after the mark).
pub fn block(line: &str) -> (Block, usize) {
    let t = line.trim_end();
    if t.is_empty() {
        return (Block::Blank, 0);
    }
    if t == "--" {
        return (Block::SigDash, 0);
    }
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=3).contains(&hashes) && line.get(hashes..).is_some_and(|r| r.starts_with(' ')) {
        return (Block::Heading(u8::try_from(hashes).unwrap_or(3)), hashes + 1);
    }
    if line.starts_with("- ") || line.starts_with("* ") {
        return (Block::Bullet, 2);
    }
    if line.starts_with("> ") || line == ">" {
        return (Block::Quote, line.len().min(2));
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if (1..=3).contains(&digits) && line.get(digits..).is_some_and(|r| r.starts_with(". ")) {
        return (Block::Numbered, digits + 2);
    }
    (Block::Para, 0)
}

fn is_url_start(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://")
}

fn safe_url(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    (l.starts_with("https://") || l.starts_with("http://") || l.starts_with("mailto:")) && !u.contains(['"', '<', '>', ' '])
}

/// Split a line into inline pieces. Unclosed marks stay as text.
pub fn inlines<'a>(line: &'a str) -> Vec<Inline<'a>> {
    let mut out = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    let bytes = line.as_bytes();
    let flush = |out: &mut Vec<Inline<'a>>, from: usize, to: usize| {
        if to > from
            && let Some(t) = line.get(from..to)
        {
            out.push(Inline::Text(t));
        }
    };
    while i < bytes.len() {
        let rest = line.get(i..).unwrap_or("");
        // Delimited spans: (open, close, constructor).
        let pair = |open: &str, close: &str| -> Option<(usize, usize)> {
            let body = rest.strip_prefix(open)?;
            if body.starts_with(' ') || body.is_empty() {
                return None;
            }
            let end = body.find(close)?;
            (end > 0).then_some((open.len(), end))
        };
        let matched: Option<(Inline<'a>, usize)> = if let Some((o, e)) = pair("**", "**") {
            Some((Inline::Bold(&rest[o..o + e]), o + e + 2))
        } else if let Some((o, e)) = pair("~~", "~~") {
            Some((Inline::Strike(&rest[o..o + e]), o + e + 2))
        } else if let Some((o, e)) = pair("`", "`") {
            Some((Inline::Code(&rest[o..o + e]), o + e + 1))
        } else if let Some((o, e)) = pair("*", "*").filter(|_| !rest.starts_with("**")) {
            // Not inside a word (`a*b*c` stays text).
            let before_ok = i == 0 || line[..i].chars().last().is_some_and(|c| !c.is_alphanumeric());
            before_ok.then(|| (Inline::Italic(&rest[o..o + e]), o + e + 1))
        } else if rest.starts_with("![") || rest.starts_with('[') {
            let pic = rest.starts_with("![");
            let open = if pic { 2 } else { 1 };
            rest.get(open..).and_then(|r| r.find("](").map(|t| (r, t))).and_then(|(r, t)| {
                let text = &r[..t];
                let after = &r[t + 2..];
                let end = after.find(')')?;
                let url = &after[..end];
                let used = open + t + 2 + end + 1;
                if text.contains(']') {
                    return None;
                }
                if pic {
                    url.starts_with("cid:").then_some((Inline::Picture { alt: text, src: url }, used))
                } else {
                    safe_url(url).then_some((Inline::Link { text, url }, used))
                }
            })
        } else if is_url_start(rest) && (i == 0 || line[..i].ends_with([' ', '(', '\t'])) {
            let end = rest.find(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == '"').unwrap_or(rest.len());
            let url = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
            (url.len() > 8).then_some((Inline::Url(url), url.len()))
        } else {
            None
        };
        match matched {
            Some((piece, used)) => {
                flush(&mut out, text_start, i);
                out.push(piece);
                i += used;
                text_start = i;
            }
            None => {
                // Next character (UTF-8 safe).
                i += rest.chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    flush(&mut out, text_start, bytes.len());
    out
}

/// Whether the text uses any formatting (else it can go as plain text alone).
pub fn has_formatting(text: &str) -> bool {
    text.lines().any(|l| {
        let (b, start) = block(l);
        matches!(b, Block::Heading(_) | Block::Bullet | Block::Numbered)
            || inlines(l.get(start..).unwrap_or("")).iter().any(|p| !matches!(p, Inline::Text(_) | Inline::Url(_)))
    })
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn inline_html(line: &str) -> String {
    let mut s = String::new();
    for p in inlines(line) {
        match p {
            Inline::Text(t) => s.push_str(&esc(t)),
            Inline::Bold(t) => s.push_str(&format!("<b>{}</b>", inline_html(t))),
            Inline::Italic(t) => s.push_str(&format!("<i>{}</i>", inline_html(t))),
            Inline::Strike(t) => s.push_str(&format!("<s>{}</s>", inline_html(t))),
            Inline::Code(t) => s.push_str(&format!("<code>{}</code>", esc(t))),
            Inline::Link { text, url } => s.push_str(&format!("<a href=\"{}\">{}</a>", esc(url), inline_html(text))),
            Inline::Url(u) => s.push_str(&format!("<a href=\"{0}\">{0}</a>", esc(u))),
            Inline::Picture { alt, src } => s.push_str(&format!("<img src=\"{}\" alt=\"{}\" style=\"max-width:100%\">", esc(src), esc(alt))),
        }
    }
    s
}

fn inline_text(line: &str) -> String {
    let mut s = String::new();
    for p in inlines(line) {
        match p {
            Inline::Text(t) | Inline::Code(t) | Inline::Url(t) => s.push_str(t),
            Inline::Bold(t) | Inline::Italic(t) | Inline::Strike(t) => s.push_str(&inline_text(t)),
            Inline::Link { text, url } if text == url || url.strip_prefix("mailto:") == Some(text) => s.push_str(text),
            Inline::Link { text, url } => s.push_str(&format!("{} <{url}>", inline_text(text))),
            Inline::Picture { alt, .. } => s.push_str(&format!("[{}]", if alt.is_empty() { "picture" } else { alt })),
        }
    }
    s
}

/// The editor's text as HTML.
pub fn to_html(text: &str) -> String {
    let mut out = String::from("<div style=\"font-family: -apple-system, 'Segoe UI', Roboto, Arial, sans-serif; font-size: 14px\">\n");
    let mut open: Option<&str> = None; // "ul", "ol", "blockquote", "p"
    let close = |out: &mut String, open: &mut Option<&str>| {
        if let Some(tag) = open.take() {
            out.push_str(&format!("</{tag}>\n"));
        }
    };
    let mut para_lines = 0;
    for line in text.replace("\r\n", "\n").split('\n') {
        let (b, start) = block(line);
        let content = line.get(start..).unwrap_or("");
        let want = match b {
            Block::Bullet => Some("ul"),
            Block::Numbered => Some("ol"),
            Block::Quote => Some("blockquote"),
            Block::Para | Block::SigDash => Some("p"),
            Block::Heading(_) | Block::Blank => None,
        };
        if want != open {
            close(&mut out, &mut open);
            if let Some(tag) = want {
                out.push_str(&match tag {
                    "blockquote" => "<blockquote style=\"margin:0 0 0 0.8ex; border-left:2px solid #ccc; padding-left:1ex; color:#555\">".to_string(),
                    "p" => "<p style=\"margin:0 0 1em 0\">".to_string(),
                    t => format!("<{t}>"),
                });
                open = Some(tag);
                para_lines = 0;
            }
        }
        match b {
            Block::Blank => {}
            Block::Heading(n) => out.push_str(&format!("<h{n}>{}</h{n}>\n", inline_html(content))),
            Block::Bullet | Block::Numbered => out.push_str(&format!("<li>{}</li>\n", inline_html(content))),
            Block::Quote | Block::Para | Block::SigDash => {
                if para_lines > 0 {
                    out.push_str("<br>\n");
                }
                out.push_str(&if b == Block::SigDash { "-- ".to_string() } else { inline_html(content) });
                para_lines += 1;
            }
        }
    }
    close(&mut out, &mut open);
    out.push_str("</div>\n");
    out
}

/// The editor's text as plain text: the marks dropped, links written out.
pub fn to_text(text: &str) -> String {
    let mut out = Vec::new();
    for line in text.replace("\r\n", "\n").split('\n') {
        let (b, start) = block(line);
        let content = line.get(start..).unwrap_or("");
        out.push(match b {
            Block::Heading(_) => inline_text(content),
            Block::Bullet => format!("- {}", inline_text(content)),
            Block::Numbered | Block::Quote => format!("{}{}", line.get(..start).unwrap_or(""), inline_text(content)),
            Block::SigDash => "-- ".to_string(),
            Block::Para => inline_text(content),
            Block::Blank => String::new(),
        });
    }
    out.join("\n")
}

/// The `cid:` pictures a text refers to.
pub fn pictures(text: &str) -> Vec<String> {
    let mut v = Vec::new();
    for line in text.lines() {
        for p in inlines(line) {
            if let Inline::Picture { src, .. } = p
                && let Some(cid) = src.strip_prefix("cid:")
                && !v.iter().any(|x| x == cid)
            {
                v.push(cid.to_string());
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_marks() {
        assert_eq!(inlines("a **b** c"), vec![Inline::Text("a "), Inline::Bold("b"), Inline::Text(" c")]);
        assert_eq!(
            inlines("*it* and ~~gone~~ and `x<y`"),
            vec![Inline::Italic("it"), Inline::Text(" and "), Inline::Strike("gone"), Inline::Text(" and "), Inline::Code("x<y")]
        );
        assert_eq!(
            inlines("see [the site](https://a.example/x) now"),
            vec![Inline::Text("see "), Inline::Link { text: "the site", url: "https://a.example/x" }, Inline::Text(" now")]
        );
        assert_eq!(inlines("go to https://a.example/x."), vec![Inline::Text("go to "), Inline::Url("https://a.example/x"), Inline::Text(".")]);
        assert_eq!(inlines("![map](cid:m1@x)"), vec![Inline::Picture { alt: "map", src: "cid:m1@x" }]);
        // Not marks: unclosed, spaced, inside words, unsafe links.
        for plain in ["2 * 3 * 4", "a*b*c", "**open", "[x](javascript:alert(1))", "* not italic*"] {
            assert!(inlines(plain).iter().all(|p| matches!(p, Inline::Text(_))), "{plain}: {:?}", inlines(plain));
        }
        assert_eq!(inlines("é **ü** ß").len(), 3, "UTF-8 safe");
        // Only pictures sent with the message: a web address is at most a link.
        assert!(!inlines("![x](https://t.example/p.gif)").iter().any(|p| matches!(p, Inline::Picture { .. })));
    }

    #[test]
    fn blocks_html_and_text() {
        let src = "# Plan\nHi **Ann**,\nsee [this](https://x.example).\n\n- one\n- two <b>\n1. first\n> quoted\n-- \nRiley";
        let html = to_html(src);
        assert!(html.contains("<h1>Plan</h1>"), "{html}");
        assert!(html.contains("Hi <b>Ann</b>,<br>\nsee <a href=\"https://x.example\">this</a>."), "{html}");
        assert!(html.contains("<ul><li>one</li>\n<li>two &lt;b&gt;</li>\n</ul>"), "{html}");
        assert!(html.contains("<ol><li>first</li>"), "{html}");
        assert!(html.contains("<blockquote") && html.contains("quoted"), "{html}");
        assert!(!html.contains("<b>\n"), "typed markup is escaped");
        let text = to_text(src);
        assert_eq!(text, "Plan\nHi Ann,\nsee this <https://x.example>.\n\n- one\n- two <b>\n1. first\n> quoted\n-- \nRiley");
        assert!(has_formatting(src));
        assert!(!has_formatting("Just text with https://a.example and 2*3=6.\n> quoted reply"));
        assert_eq!(pictures("a ![p](cid:1@x) ![q](cid:1@x) ![r](cid:2@x)"), vec!["1@x".to_string(), "2@x".to_string()]);
    }
}
