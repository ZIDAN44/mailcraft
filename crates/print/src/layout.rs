//! A message laid out on pages: its subject, its header, a line under them, then its text
//! (plain, formatted, or as it's laid out on screen), each piece placed where it's printed.

use super::{Align, Doc, Face, MARGIN, MAX_PAGES, PAGE_H, PAGE_W, Placed, blocks, objects, rich, sheet, wrap};

/// The lowest a line of text goes on a page (under it: the page's number).
const BOTTOM: f32 = MARGIN + 20.0;

/// A message laid out.
pub(super) struct Laid {
    /// The text each page holds.
    pub(super) pages: Vec<Vec<Placed>>,
    /// What each page holds beside its text (a laid-out message's boxes, pictures and links).
    pub(super) extras: Vec<objects::Extra>,
    /// The line under the header: its page, and how high on it.
    pub(super) rule: (usize, f32),
}

/// Lay `doc` out under `title`.
pub(super) fn layout(doc: &Doc, title: &str, faces: &[Face<'_>; 2]) -> Laid {
    let width = PAGE_W - 2.0 * MARGIN;
    let mut pages: Vec<Vec<Placed>> = vec![Vec::new()];
    let mut y = PAGE_H - MARGIN;
    for line in wrap(&faces[1], title, 16.0, width) {
        place(&mut pages, &mut y, 1, 16.0, 21.0, MARGIN, line);
    }
    y -= 6.0;
    let label_w = doc.header.iter().map(|(k, _)| faces[1].text_width(&format!("{k}:"), 9.5)).fold(0.0f32, f32::max) + 6.0;
    for (k, v) in &doc.header {
        let lines = wrap(&faces[0], v, 9.5, width - label_w);
        for (i, line) in lines.into_iter().enumerate() {
            if i == 0 {
                let ok = place(&mut pages, &mut y, 1, 9.5, 13.0, MARGIN, format!("{k}:"));
                if ok && let Some(p) = pages.last_mut() {
                    p.push(Placed { font: 0, size: 9.5, x: MARGIN + label_w, y, text: line, ..Default::default() });
                }
            } else {
                place(&mut pages, &mut y, 0, 9.5, 13.0, MARGIN + label_w, line);
            }
        }
    }
    if !doc.note.trim().is_empty() {
        y -= 3.0;
        for line in wrap(&faces[0], doc.note.trim(), 9.0, width) {
            place(&mut pages, &mut y, 0, 9.0, 12.0, MARGIN, line);
        }
    }
    y -= 8.0;
    let rule = (pages.len().saturating_sub(1), y);
    y -= 6.0;
    let mut extras: Vec<objects::Extra> = Vec::new();
    let truncated = if !doc.blocks.is_empty() {
        laid_out(doc, faces, width, &mut pages, &mut extras, &mut y)
    } else if doc.rich.is_empty() {
        // (The lines after one with no page left aren't placed.)
        wrap(&faces[0], &doc.body, rich::SIZE, width).into_iter().any(|line| !place(&mut pages, &mut y, 0, rich::SIZE, rich::LEAD, MARGIN, line))
    } else {
        // A line of formatted pieces, on one baseline.
        let fitted = rich::lay(faces, &doc.rich, width, |lead, row| {
            if !room_for(&mut pages, &mut y, lead) {
                return false;
            }
            y -= lead;
            if let Some(p) = pages.last_mut() {
                p.extend(row.into_iter().map(|piece| Placed { x: MARGIN + piece.x, y, ..piece }));
            }
            true
        });
        !fitted
    };
    if truncated {
        let cut = if doc.cut.trim().is_empty() { "(The rest of the message didn't fit in this document.)" } else { doc.cut.trim() };
        place(&mut pages, &mut y, 1, 11.0, 18.0, MARGIN, cut.into());
    }
    Laid { pages, extras, rule }
}

/// A message as it's laid out on screen, from `*y` down (which moves to under it): laid out on
/// one long sheet, then cut into pages between its lines. True when it didn't all fit.
fn laid_out(doc: &Doc, faces: &[Face<'_>; 2], width: f32, pages: &mut Vec<Vec<Placed>>, extras: &mut Vec<objects::Extra>, y: &mut f32) -> bool {
    let mut ops = Vec::new();
    let mut down = 0.0f32;
    blocks::Lay { faces }.blocks(&doc.blocks, 0.0, width, Align::Left, &mut down, &mut ops, 0);
    let mut truncated = ops.len() >= blocks::MAX_OPS;
    let (tops, more) = sheet::cuts(&ops, *y - BOTTOM, PAGE_H - MARGIN - BOTTOM, MAX_PAGES.saturating_sub(pages.len().saturating_sub(1)));
    truncated |= more;
    for (k, on_page) in sheet::pages(ops, &tops).into_iter().enumerate() {
        if k > 0 {
            pages.push(Vec::new());
        }
        let top = if k == 0 { *y } else { PAGE_H - MARGIN };
        extras.resize_with(pages.len(), Default::default);
        let (Some(page), Some(extra)) = (pages.last_mut(), extras.last_mut()) else { break };
        let mut low = top;
        for op in on_page {
            match op {
                sheet::Op::Fill { x, y: from, w, h, color, radius } => {
                    extra.fills.push(objects::Fill { x: MARGIN + x, y: top - from - h, w, h, color, radius });
                }
                sheet::Op::Text { bottom, mut piece, .. } => {
                    piece.x += MARGIN;
                    piece.y = top - piece.y;
                    low = low.min(top - bottom);
                    page.push(piece);
                }
                sheet::Op::Image { x, y: from, w, h, index } => {
                    low = low.min(top - from - h);
                    if doc.pictures.get(index).is_some() {
                        extra.images.push(objects::Image { index, x: MARGIN + x, y: top - from - h, w, h });
                    }
                }
                sheet::Op::Link { x, y: from, w, h, url } => extra.links.push(objects::Link { x: MARGIN + x, y: top - from - h, w, h, url }),
            }
        }
        *y = low;
    }
    truncated
}

/// Room on the page under `*y` for a line `lead` tall: a new page when it doesn't fit above the
/// bottom of this one (false when there are as many pages as are written).
fn room_for(pages: &mut Vec<Vec<Placed>>, y: &mut f32, lead: f32) -> bool {
    if *y - lead < BOTTOM {
        if pages.len() >= MAX_PAGES {
            return false;
        }
        pages.push(Vec::new());
        *y = PAGE_H - MARGIN;
    }
    true
}

/// A line of `text` placed under `*y` (which moves to it), on a new page when it doesn't fit
/// on this one; false when there's no page for it.
fn place(pages: &mut Vec<Vec<Placed>>, y: &mut f32, font: usize, size: f32, lead: f32, x: f32, text: String) -> bool {
    if !room_for(pages, y, lead) {
        return false;
    }
    *y -= lead;
    if let Some(p) = pages.last_mut() {
        p.push(Placed { font, size, x, y: *y, text, ..Default::default() });
    }
    true
}
