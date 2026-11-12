//! The long sheet a laid-out message is put on, and its cut into pages: between lines of text
//! and pictures, never through one; a background or a line that crosses a cut goes on on the
//! next page.

use super::Placed;

/// Something on the sheet: `x` from its left edge, `y` down from its top, in points.
#[derive(Debug)]
pub(super) enum Op {
    /// A filled box (a background, or a line along a box's side), its corners rounded by `radius`.
    Fill { x: f32, y: f32, w: f32, h: f32, color: [u8; 3], radius: f32 },
    /// A piece of text on its line (`top` to `bottom`); `piece.y` is its baseline.
    Text { top: f32, bottom: f32, piece: Placed },
    /// The `index`th picture.
    Image { x: f32, y: f32, w: f32, h: f32, index: usize },
    /// Where a click goes to `url`.
    Link { x: f32, y: f32, w: f32, h: f32, url: String },
}

impl Op {
    /// The same, `down` further down the sheet.
    pub(super) fn moved(self, down: f32) -> Op {
        match self {
            Op::Fill { x, y, w, h, color, radius } => Op::Fill { x, y: y + down, w, h, color, radius },
            Op::Text { top, bottom, mut piece } => {
                piece.y += down;
                Op::Text { top: top + down, bottom: bottom + down, piece }
            }
            Op::Image { x, y, w, h, index } => Op::Image { x, y: y + down, w, h, index },
            Op::Link { x, y, w, h, url } => Op::Link { x, y: y + down, w, h, url },
        }
    }

    fn span(&self) -> (f32, f32) {
        match self {
            Op::Fill { y, h, .. } | Op::Image { y, h, .. } | Op::Link { y, h, .. } => (*y, *y + *h),
            Op::Text { top, bottom, .. } => (*top, *bottom),
        }
    }
}

/// Where the sheet is cut: the tops of its pages, the first at 0. The first page has `first`
/// room down it, the others `room`; at most `most` pages (whether there was more is said too).
pub(super) fn cuts(ops: &[Op], first: f32, room: f32, most: usize) -> (Vec<f32>, bool) {
    let room = room.max(40.0);
    let total = ops.iter().map(|o| o.span().1).fold(0.0f32, f32::max);
    // What a cut must not go through: a line of text, a picture that fits a page.
    let whole: Vec<(f32, f32)> = ops
        .iter()
        .filter(|o| matches!(o, Op::Text { .. } | Op::Image { .. }))
        .map(Op::span)
        .filter(|(top, bottom)| bottom - top <= room && bottom > top)
        .collect();
    let mut tops = vec![0.0f32];
    let mut start = 0.0f32;
    let mut cap = first.max(0.0);
    // (Written so a size that isn't a number ends it.)
    while start + cap < total {
        if tops.len() >= most.max(1) {
            return (tops, true);
        }
        let limit = start + cap;
        let mut cut = limit;
        // Up to the top of whatever the cut would go through, until it goes through nothing.
        for _ in 0..64 {
            let through = whole.iter().filter(|(top, bottom)| *top < cut && *bottom > cut).map(|(top, _)| *top).fold(f32::INFINITY, f32::min);
            if !through.is_finite() {
                break;
            }
            cut = through;
        }
        if cut <= start {
            // What's left of the first page holds nothing whole: the sheet starts on the next.
            if tops.len() == 1 && cap < room {
                tops.push(start);
                cap = room;
                continue;
            }
            // One thing after another taller than the page: cut where the page ends.
            cut = limit;
        }
        tops.push(cut);
        start = cut;
        cap = room;
    }
    (tops, false)
}

/// The sheet's things page by page (`tops` from [`cuts`]): each with `y` down from its page's
/// top. A box crossing a cut is on both pages, as much of it as each has.
pub(super) fn pages(ops: Vec<Op>, tops: &[f32]) -> Vec<Vec<Op>> {
    let mut out: Vec<Vec<Op>> = tops.iter().map(|_| Vec::new()).collect();
    // The page a point of the sheet is on: the last one that starts at or above it.
    let page_of = |y: f32| tops.iter().rposition(|t| *t <= y).unwrap_or(0);
    let end_of = |page: usize| tops.get(page + 1).copied().unwrap_or(f32::INFINITY);
    for op in ops {
        match op {
            Op::Fill { x, y, w, h, color, radius } => {
                let (first, last) = (page_of(y), page_of((y + h).max(y)));
                for page in first..=last {
                    let Some(top) = tops.get(page).copied() else { continue };
                    let (from, to) = (y.max(top), (y + h).min(end_of(page)));
                    if to - from <= 0.0 {
                        continue;
                    }
                    // Cut, its corners are square.
                    let radius = if first == last { radius } else { 0.0 };
                    if let Some(p) = out.get_mut(page) {
                        p.push(Op::Fill { x, y: from - top, w, h: to - from, color, radius });
                    }
                }
            }
            other => {
                let page = page_of(other.span().0);
                let top = tops.get(page).copied().unwrap_or(0.0);
                if let Some(p) = out.get_mut(page) {
                    p.push(other.moved(-top));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(top: f32) -> Op {
        Op::Text { top, bottom: top + 10.0, piece: Placed { y: top + 8.0, ..Default::default() } }
    }

    #[test]
    fn pages_are_cut_between_lines() {
        // Lines every 10 points down 250 of them; 95 on the first page, 100 on the others.
        let ops: Vec<Op> = (0..25).map(|i| line(i as f32 * 10.0)).collect();
        let (tops, more) = cuts(&ops, 95.0, 100.0, 500);
        assert_eq!(tops, [0.0, 90.0, 190.0], "above the line each cut would go through");
        assert!(!more);
        let pages = pages(ops, &tops);
        assert_eq!(pages.iter().map(Vec::len).collect::<Vec<_>>(), [9, 10, 6]);
        // Each line whole on its page, from the page's top.
        for page in &pages {
            let spans: Vec<(f32, f32)> = page.iter().map(Op::span).collect();
            assert!(spans.iter().all(|(top, bottom)| *top >= 0.0 && *bottom <= 100.0), "{spans:?}");
        }
        // No more pages than asked for, and it says there was more.
        let ops: Vec<Op> = (0..25).map(|i| line(i as f32 * 10.0)).collect();
        let (tops, more) = cuts(&ops, 95.0, 100.0, 2);
        assert_eq!((tops.len(), more), (2, true));
    }

    #[test]
    fn a_background_goes_on_across_a_cut() {
        let mut ops = vec![Op::Fill { x: 0.0, y: 50.0, w: 100.0, h: 100.0, color: [1, 2, 3], radius: 6.0 }];
        ops.extend((0..15).map(|i| line(i as f32 * 10.0)));
        let (tops, _) = cuts(&ops, 100.0, 100.0, 500);
        assert_eq!(tops, [0.0, 100.0]);
        let pages = pages(ops, &tops);
        let fills: Vec<(f32, f32, f32)> = pages
            .iter()
            .flat_map(|p| p.iter())
            .filter_map(|o| match o {
                Op::Fill { y, h, radius, .. } => Some((*y, *h, *radius)),
                _ => None,
            })
            .collect();
        assert_eq!(fills, [(50.0, 50.0, 0.0), (0.0, 50.0, 0.0)], "half on each page, square where it's cut");
    }

    #[test]
    fn what_is_taller_than_a_page_is_cut_and_nothing_loops() {
        // A picture taller than any page, lines too tall for the first page's room, no room at all.
        let ops = vec![Op::Image { x: 0.0, y: 0.0, w: 10.0, h: 5000.0, index: 0 }, line(5000.0)];
        let (tops, more) = cuts(&ops, 100.0, 100.0, 500);
        assert!(tops.len() > 40 && !more, "{}", tops.len());
        assert!(tops.windows(2).all(|w| w[1] > w[0]), "each page further down");
        let (tops, _) = cuts(&[line(0.0), line(10.0)], 0.0, 100.0, 500);
        assert_eq!(tops, [0.0, 0.0], "nothing fits what's left of the first page: it starts on the next");
        for (first, room) in [(0.0, 0.0), (-5.0, -5.0), (f32::NAN, 100.0), (100.0, f32::NAN), (f32::INFINITY, 1.0)] {
            let ops: Vec<Op> = (0..300).map(|i| line(i as f32 * 10.0)).collect();
            let (tops, _) = cuts(&ops, first, room, 500);
            assert!(!tops.is_empty() && tops.len() <= 500);
        }
        assert_eq!(cuts(&[], 100.0, 100.0, 500), (vec![0.0], false));
    }
}
