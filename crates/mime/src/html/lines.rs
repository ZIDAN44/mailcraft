//! A line of the text converter's rich output as spans: its runs of text formatted alike
//! merged, a heading's marks read, a list's bullet and a table's rules.

use super::*;

impl Span {
    /// Whether `other` is formatted as this is: everything but the text the same (each field
    /// named, so that one added is compared too).
    fn same_format(&self, other: &Span) -> bool {
        let Span { text: _, bold, italic, strike, code, link, image, heading, color, background, image_size, size, rise } = self;
        *bold == other.bold
            && *italic == other.italic
            && *strike == other.strike
            && *code == other.code
            && *link == other.link
            && *image == other.image
            && *heading == other.heading
            && *color == other.color
            && *background == other.background
            && *image_size == other.image_size
            && *size == other.size
            && *rise == other.rise
    }
}

/// A line of the text converter's as spans, one for each run of text formatted alike, the
/// spaces at its end left out.
pub(super) fn line_spans(line: &html2text::render::TaggedLine<Vec<html2text::render::RichAnnotation>>) -> Vec<Span> {
    use html2text::render::RichAnnotation as A;
    let mut spans: Vec<Span> = Vec::new();
    for ts in line.tagged_strings() {
        let mut sp = Span { text: ts.s.clone(), ..Default::default() };
        for a in &ts.tag {
            match a {
                A::Strong => sp.bold = true,
                A::Emphasis => sp.italic = true,
                A::Strikeout => sp.strike = true,
                A::Code | A::Preformat(_) => sp.code = true,
                A::Link(l) => sp.link = Some(l.clone()),
                A::Image(src) => sp.image = Some(src.clone()),
                A::Colour(c) => sp.color = Some([c.r, c.g, c.b]),
                A::BgColour(c) => sp.background = Some([c.r, c.g, c.b]),
                _ => {}
            }
        }
        // Merged with the one before when it's formatted alike.
        match spans.last_mut() {
            Some(last) if last.same_format(&sp) => last.text.push_str(&sp.text),
            _ => spans.push(sp),
        }
    }
    if let Some(last) = spans.last_mut() {
        let trimmed = last.text.trim_end().len();
        last.text.truncate(trimmed);
    }
    spans
}

/// A line of a heading given its level: headings come out as `## <mark>Title`, each line of them
/// (see [`mark_headings`]), and in a quote as `> ## <mark>Title`: the marks dropped, the level
/// kept. Before a title in a colour of its own they're spans of their own, dropped too (#122:
/// one in a quote kept `> ## ` as words).
pub(super) fn heading_of(spans: &mut [Span]) {
    let mut level = 0u8;
    let mut hashes = 0usize;
    let mut marked_at = 0;
    for (at, sp) in spans.iter_mut().enumerate() {
        if level > 0 {
            break;
        }
        match sp.text.find(HEADING_MARK) {
            Some(i) => {
                hashes += sp.text.get(..i).unwrap_or("").chars().filter(|c| *c == '#').count();
                level = u8::try_from(hashes.clamp(1, 6)).unwrap_or(1);
                sp.text = sp.text.get(i + HEADING_MARK.len_utf8()..).unwrap_or("").to_string();
                marked_at = at;
            }
            None => hashes += sp.text.chars().filter(|c| *c == '#').count(),
        }
    }
    if level > 0 {
        let marks_only = |sp: &Span| sp.image.is_none() && sp.text.chars().all(|c| c == '#' || c == '>' || c.is_whitespace());
        for sp in spans.iter_mut().take(marked_at.saturating_add(1)) {
            if marks_only(sp) {
                sp.text.clear();
            }
        }
        for sp in spans.iter_mut() {
            sp.heading = level;
        }
    }
}

/// Whether a line is a heading's empty line (`<br><br>` in an `<h3>`): nothing but the marks of
/// a heading of `level`, the level of the line before it, as the converter writes them on each
/// line of a heading, text or not. Its marks go, and it's an empty line; elsewhere `###` is what
/// the message says. They showed where its empty lines were (#271).
pub(super) fn heading_gap(spans: &mut [Span], level: u8) -> bool {
    if level == 0 || spans.iter().any(|s| s.image.is_some()) {
        return false;
    }
    let whole: String = spans.iter().map(|s| s.text.as_str()).collect();
    // In a quote, after its `> `.
    let marks = whole.trim().trim_start_matches(['>', ' ']);
    if marks.len() != usize::from(level) || !marks.chars().all(|c| c == '#') {
        return false;
    }
    for sp in spans.iter_mut() {
        sp.text.retain(|c| c != '#');
    }
    true
}

/// A list's bullet (`* item`, indented when nested) as a real one.
pub(super) fn bullet(spans: &mut [Span]) {
    if let Some(first) = spans.first_mut() {
        let indent = first.text.len() - first.text.trim_start_matches(' ').len();
        if first.text.get(indent..).is_some_and(|r| r.starts_with("* ")) {
            first.text = format!("{}• {}", " ".repeat(indent), first.text.get(indent + 2..).unwrap_or(""));
        }
    }
}

/// Tables come laid out with box-drawing borders. A layout table (one column) only adds rules
/// around its text: those dropped. A data table's rows line up only in a monospace font.
pub(super) fn table_rules(spans: &mut Vec<Span>) {
    let whole: String = spans.iter().map(|s| s.text.as_str()).collect();
    if !whole.trim().is_empty() && whole.chars().all(|c| matches!(c, '─' | '━' | '═' | ' ')) {
        spans.clear();
    } else if whole.contains(['│', '┼', '┬', '┴']) {
        for sp in spans.iter_mut() {
            sp.code = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same answer as the comparison it replaced (each span cloned without its text), for
    /// spans that differ in each field in turn.
    #[test]
    fn same_format_is_equal_but_for_the_text() {
        let base = Span { text: "a".into(), link: Some("https://l.example/".into()), ..Default::default() };
        let spans = [
            base.clone(),
            Span { text: "other".into(), ..base.clone() },
            Span { bold: true, ..base.clone() },
            Span { italic: true, ..base.clone() },
            Span { strike: true, ..base.clone() },
            Span { code: true, ..base.clone() },
            Span { link: None, ..base.clone() },
            Span { image: Some("cid:x".into()), ..base.clone() },
            Span { heading: 2, ..base.clone() },
            Span { color: Some([1, 2, 3]), ..base.clone() },
            Span { background: Some([1, 2, 3]), ..base.clone() },
            Span { image_size: Some(ImageSize { height: Some(4), ..Default::default() }), ..base.clone() },
            Span { size: Some(12), ..base.clone() },
            Span { rise: Rise::Sup, ..base.clone() },
        ];
        for a in &spans {
            for b in &spans {
                let old = Span { text: String::new(), ..a.clone() } == Span { text: String::new(), ..b.clone() };
                assert_eq!(a.same_format(b), old, "{a:?} {b:?}");
            }
        }
    }
}
