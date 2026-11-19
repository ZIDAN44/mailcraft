//! What a search found, marked in the open message (issue #173), as Outlook marks it: a copy of
//! the message whose runs of text are cut where a hit starts and ends, each hit on a highlight,
//! drawn as any formatted message is. Made once for a message and a search.

use std::sync::Arc;

use mailcraft_engine::model::search::{Field, Query, hits};

use super::*;

/// The highlight under what a search found (Outlook's is yellow too). The text on it is black
/// or white, whichever reads, as on any highlight a message sets.
pub(crate) const MARK: [u8; 3] = [255, 226, 92];

/// The open message marked, kept while it, the search and how it's shown are the same.
#[derive(Default)]
pub struct Marked {
    /// The message, the search, shown formatted, and its text's length (the message read
    /// again, from its source, after its summary).
    of: Option<(MessageKey, String, bool, usize)>,
    parsed: Option<Arc<mime::Parsed>>,
}

/// The open message with what the search on finds in its text marked: `None` with no search on,
/// or nothing of it found there. `formatted`: as it's shown (plain text is made lines of its own).
pub(crate) fn marked(s: &Session, st: &mut UiState, key: &MessageKey, p: &mime::Parsed, formatted: bool) -> Option<Arc<mime::Parsed>> {
    let query = s.search.as_ref().map(|(q, _)| q.as_str()).filter(|q| !q.trim().is_empty())?;
    let of = (key.clone(), query.to_string(), formatted, p.text.len());
    if st.marked.of.as_ref() != Some(&of) {
        let sought = Query::parse(query).sought(&Field::Body);
        st.marked.parsed = mark(p, &sought, formatted).map(Arc::new);
        st.marked.of = Some(of);
    }
    st.marked.parsed.clone()
}

/// `p` with `sought` marked, as it's shown; `None` when it isn't found.
pub(crate) fn mark(p: &mime::Parsed, sought: &[Vec<String>], formatted: bool) -> Option<mime::Parsed> {
    if sought.is_empty() {
        return None;
    }
    let mut found = false;
    let (rich, layout) = if formatted {
        (lines(&p.rich, sought, &mut found), p.layout.iter().map(|b| block(b, sought, &mut found, 0)).collect())
    } else {
        // Plain text as lines of one run each (an empty one is a blank line).
        let plain: mime::RichText = p
            .text
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .map(|l| if l.is_empty() { Vec::new() } else { vec![mime::Span { text: l.to_string(), ..Default::default() }] })
            .collect();
        (lines(&plain, sought, &mut found), Vec::new())
    };
    found.then(|| mime::Parsed { rich, layout, ..p.clone() })
}

/// A block marked, the tables in it as deep as they're drawn.
fn block(b: &mime::layout::Block, sought: &[Vec<String>], found: &mut bool, depth: usize) -> mime::layout::Block {
    use mime::layout::Block;
    match b {
        Block::Text(rich) => Block::Text(lines(rich, sought, found)),
        Block::Table(t) if depth < MAX_NESTED => {
            let mut t = t.clone();
            for cell in t.rows.iter_mut().flatten() {
                cell.blocks = cell.blocks.iter().map(|b| block(b, sought, found, depth + 1)).collect();
                for o in &mut cell.overlays {
                    o.blocks = o.blocks.iter().map(|b| block(b, sought, found, depth + 1)).collect();
                }
            }
            Block::Table(t)
        }
        Block::Table(_) => b.clone(),
    }
}

/// Lines marked: each line's text read whole (a word may be in two runs), its runs cut where a
/// hit starts and ends.
fn lines(rich: &[Vec<mime::Span>], sought: &[Vec<String>], found: &mut bool) -> mime::RichText {
    rich.iter()
        .map(|line| {
            // Pictures' descriptions aren't the text read.
            let text: String = line.iter().filter(|sp| sp.image.is_none()).map(|sp| sp.text.as_str()).collect();
            let hits = hits(&text, sought);
            if hits.is_empty() {
                return line.clone();
            }
            *found = true;
            let mut out = Vec::with_capacity(line.len() + 2 * hits.len());
            let mut at = 0usize;
            for sp in line {
                if sp.image.is_some() {
                    out.push(sp.clone());
                    continue;
                }
                let (start, end) = (at, at.saturating_add(sp.text.len()));
                at = end;
                // Where the run is cut: its ends, and each hit's that falls inside it.
                let mut cuts = vec![start];
                for h in hits.iter().filter(|h| h.start < end && h.end > start) {
                    cuts.extend([h.start.max(start), h.end.min(end)]);
                }
                cuts.push(end);
                cuts.dedup();
                for w in cuts.windows(2) {
                    let &[a, b] = w else { continue };
                    let Some(piece) = sp.text.get(a.saturating_sub(start)..b.saturating_sub(start)).filter(|s| !s.is_empty()) else { continue };
                    let hit = hits.iter().any(|h| h.start <= a && b <= h.end);
                    out.push(mime::Span { text: piece.to_string(), background: if hit { Some(MARK) } else { sp.background }, ..sp.clone() });
                }
            }
            out
        })
        .collect()
}

/// Where the search on found something in a subject (issue #173): none with no search on.
pub(crate) fn subject_marks(s: &Session, subject: &str) -> Vec<std::ops::Range<usize>> {
    marks_in(s, subject, &Field::Subject)
}

/// Where the search on found something in a message's text (the list's line of it).
pub(crate) fn text_marks(s: &Session, text: &str) -> Vec<std::ops::Range<usize>> {
    marks_in(s, text, &Field::Body)
}

fn marks_in(s: &Session, text: &str, field: &Field) -> Vec<std::ops::Range<usize>> {
    match s.search.as_ref().map(|(q, _)| q.as_str()).filter(|q| !q.trim().is_empty()) {
        Some(q) => hits(text, &Query::parse(q).sought(field)),
        None => Vec::new(),
    }
}

/// A text laid out in `font` and `color`, with `marks` (byte ranges, in order) on the highlight.
pub(crate) fn marked_job(text: &str, marks: &[std::ops::Range<usize>], font: egui::FontId, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let plain = egui::TextFormat { font_id: font.clone(), color, ..Default::default() };
    let [r, g, b] = MARK;
    let hit = egui::TextFormat { font_id: font, color: Color32::BLACK, background: Color32::from_rgb(r, g, b), ..Default::default() };
    let mut at = 0usize;
    for m in marks {
        if let (Some(before), Some(found)) = (text.get(at..m.start).filter(|_| m.start >= at), text.get(m.clone())) {
            job.append(before, 0.0, plain.clone());
            job.append(found, 0.0, hit.clone());
            at = m.end;
        }
    }
    job.append(text.get(at..).unwrap_or_default(), 0.0, plain);
    job
}
