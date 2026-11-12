//! A formatted message laid out as it's read on screen: its tables side by side in columns,
//! their backgrounds, lines and rounded corners, its pictures and its links, on one long sheet
//! that [`sheet::pages`](super::sheet) then cuts into pages.
//!
//! Column widths follow CSS's automatic table layout: a column is at least as wide as its
//! longest word and wants to be as wide as its longest line; widths a message sets are kept,
//! and what's left is shared by how much each column wants.

use super::Face;
use super::rich::{self, Run};
use super::sheet::Op;

/// Tables nested deeper than this are left out (a message's own are cut at fewer).
const MAX_DEPTH: usize = 24;
/// Columns laid out in one table.
const MAX_COLUMNS: usize = 64;
/// Things put on the sheet for one message: past it, the rest is left out.
pub(super) const MAX_OPS: usize = 400_000;
/// The narrowest a column is made.
const NARROWEST: f32 = 6.0;
/// In place of a picture's number ([`Run::picture`]): one that isn't shown, whose room is kept.
pub const HELD_BACK: usize = usize::MAX;

/// A part of a laid-out message.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// Formatted lines.
    Text(Vec<Vec<Run>>),
    Table(Table),
}

/// How wide something wants to be: in points, or as a share of the room it's in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Width {
    Pt(f32),
    Percent(f32),
}

/// Where content sits across its room.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// Where a cell's content sits down its row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

/// A line along a box's side: how thick, in points, and its colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub width: f32,
    pub color: [u8; 3],
}

/// A table: rows of cells. Lengths are in points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub rows: Vec<Vec<Cell>>,
    pub width: Option<Width>,
    /// With no width: as wide as what it holds (a `<table>`, a button) rather than its room.
    pub fit: bool,
    pub align: Align,
    pub background: Option<[u8; 3]>,
    /// Room between its cells and around them.
    pub spacing: f32,
    /// Room inside each cell that sets none of its own.
    pub padding: f32,
    pub radius: f32,
    pub border: Option<Line>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub blocks: Vec<Block>,
    pub width: Option<Width>,
    pub background: Option<[u8; 3]>,
    pub align: Align,
    /// Columns it spans (0 is 1).
    pub span: u16,
    /// Room inside it (top, right, bottom, left); none: its table's.
    pub padding: Option<[f32; 4]>,
    pub valign: VAlign,
    /// How tall it is at least.
    pub height: Option<f32>,
    pub radius: f32,
    /// The lines along its sides: top, right, bottom, left.
    pub borders: [Option<Line>; 4],
}

/// What something needs across: at least `min` (its longest word), and `max` to be on one line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Fit {
    min: f32,
    max: f32,
}

/// The layout of one message.
pub(super) struct Lay<'a, 'f> {
    pub(super) faces: &'a [Face<'f>],
}

impl Lay<'_, '_> {
    /// Lay `blocks` out `width` wide from `x`, down from `*y` (which moves to under them).
    pub(super) fn blocks(&self, blocks: &[Block], x: f32, width: f32, align: Align, y: &mut f32, ops: &mut Vec<Op>, depth: usize) {
        for b in blocks {
            if ops.len() >= MAX_OPS {
                return;
            }
            match b {
                Block::Text(lines) => self.text(lines, x, width, align, y, ops),
                Block::Table(t) if depth < MAX_DEPTH => self.table(t, x, width, y, ops, depth),
                Block::Table(_) => {}
            }
        }
    }

    fn text(&self, lines: &[Vec<Run>], x: f32, width: f32, align: Align, y: &mut f32, ops: &mut Vec<Op>) {
        rich::lay(self.faces, lines, width.max(NARROWEST), |lead, row| {
            let used = row.iter().map(|p| p.x + p.width).fold(0.0f32, f32::max);
            let shift = match align {
                Align::Left => 0.0,
                Align::Center => (width - used) / 2.0,
                Align::Right => width - used,
            }
            .max(0.0);
            let top = *y;
            *y += lead;
            // The baseline: a line of text's descenders under it, a picture standing on it.
            let base = top + lead - rich::LEAD * 0.22;
            // Where the line's last link is, to go on with: a link's words are one place to click.
            let mut linked: Option<usize> = None;
            for mut p in row {
                p.x += x + shift;
                match p.link.take() {
                    Some(url) => match linked.and_then(|i| ops.get_mut(i)) {
                        Some(Op::Link { x: from, w, url: same, .. }) if *same == url => *w = p.x + p.width - *from,
                        _ => {
                            linked = Some(ops.len());
                            ops.push(Op::Link { x: p.x, y: top, w: p.width, h: lead, url });
                        }
                    },
                    None => linked = None,
                }
                match p.picture {
                    // A picture that isn't shown: its room, with a faint line around it.
                    Some((HELD_BACK, height)) => {
                        let (bx, by, bw, bh, line) = (p.x, base - height, p.width, height, 0.4);
                        if bw > 3.0 && bh > 3.0 {
                            for (fx, fy, fw, fh) in
                                [(bx, by, bw, line), (bx + bw - line, by, line, bh), (bx, by + bh - line, bw, line), (bx, by, line, bh)]
                            {
                                ops.push(Op::Fill { x: fx, y: fy, w: fw, h: fh, color: [0xC8, 0xC8, 0xC8], radius: 0.0 });
                            }
                        }
                    }
                    Some((index, height)) => ops.push(Op::Image { x: p.x, y: base - height, w: p.width, h: height, index }),
                    None => {
                        p.y = base;
                        ops.push(Op::Text { top, bottom: top + lead, piece: p });
                    }
                }
            }
            ops.len() < MAX_OPS
        });
    }

    /// What `blocks` need across.
    fn fit(&self, blocks: &[Block], depth: usize) -> Fit {
        let mut fit = Fit::default();
        for b in blocks {
            let f = match b {
                Block::Text(lines) => {
                    let mut f = Fit::default();
                    for line in lines {
                        let (min, max) = rich::fit(self.faces, line);
                        f.min = f.min.max(min);
                        f.max = f.max.max(max);
                    }
                    f
                }
                Block::Table(t) if depth < MAX_DEPTH => self.table_fit(t, depth),
                Block::Table(_) => Fit::default(),
            };
            fit.min = fit.min.max(f.min);
            fit.max = fit.max.max(f.max);
        }
        fit
    }

    fn table_fit(&self, t: &Table, depth: usize) -> Fit {
        let cols = self.columns_fit(t, depth);
        let room = t.spacing * (cols.len() as f32 + 1.0);
        let min = cols.iter().map(|c| c.fit.min).sum::<f32>() + room;
        let max = cols.iter().map(|c| c.fixed.unwrap_or(c.fit.max).max(c.fit.min)).sum::<f32>() + room;
        match t.width {
            Some(Width::Pt(w)) => Fit { min: min.min(w), max: w },
            _ => Fit { min, max: max.max(min) },
        }
    }

    /// Each column's needs and what a cell of it asks to be.
    fn columns_fit(&self, t: &Table, depth: usize) -> Vec<Column> {
        let n = t.rows.iter().map(|r| r.iter().map(|c| usize::from(c.span.max(1))).sum::<usize>()).max().unwrap_or(0).clamp(1, MAX_COLUMNS);
        let mut cols = vec![Column::default(); n];
        let mut spanning: Vec<(usize, usize, Fit)> = Vec::new();
        for row in &t.rows {
            let mut at = 0usize;
            for cell in row {
                let span = usize::from(cell.span.max(1)).min(n.saturating_sub(at));
                if span == 0 {
                    break;
                }
                let pad = cell.padding.map_or(t.padding * 2.0, |p| p[1] + p[3]);
                let inner = self.fit(&cell.blocks, depth + 1);
                let fit = Fit { min: inner.min + pad, max: inner.max + pad };
                if span == 1 {
                    if let Some(c) = cols.get_mut(at) {
                        c.fit.min = c.fit.min.max(fit.min);
                        c.fit.max = c.fit.max.max(fit.max);
                        match cell.width {
                            Some(Width::Pt(w)) if c.fixed.is_none() && c.percent.is_none() => c.fixed = Some(w),
                            Some(Width::Percent(p)) if c.fixed.is_none() && c.percent.is_none() => c.percent = Some(p.clamp(0.0, 100.0)),
                            _ => {}
                        }
                    }
                } else {
                    spanning.push((at, span, fit));
                }
                at += span;
            }
        }
        // A cell over several columns: what it needs more than they have is shared among them.
        for (at, span, fit) in spanning {
            let Some(over) = cols.get_mut(at..at + span) else { continue };
            let room = t.spacing * (span as f32 - 1.0);
            let (have_min, have_max) = (over.iter().map(|c| c.fit.min).sum::<f32>() + room, over.iter().map(|c| c.fit.max).sum::<f32>() + room);
            for c in over {
                c.fit.min += ((fit.min - have_min) / span as f32).max(0.0);
                c.fit.max += ((fit.max - have_max) / span as f32).max(0.0);
            }
        }
        for c in &mut cols {
            c.fit.max = c.fit.max.max(c.fit.min);
        }
        cols
    }

    /// The columns' widths in `inner` (the table's width less the room between its cells);
    /// `stretch`: the table sets its width, so what's left over is shared too.
    fn columns(&self, t: &Table, inner: f32, stretch: bool, depth: usize) -> Vec<f32> {
        let cols = self.columns_fit(t, depth);
        let inner = inner.max(NARROWEST);
        let mut w: Vec<f32> =
            cols.iter().map(|c| c.fixed.map(|f| f.max(c.fit.min).min(inner)).or(c.percent.map(|p| inner * p / 100.0)).unwrap_or(-1.0)).collect();
        let taken: f32 = w.iter().filter(|x| **x >= 0.0).sum();
        let rest = (inner - taken).max(0.0);
        let auto: Vec<usize> = (0..cols.len()).filter(|i| w.get(*i).is_some_and(|x| *x < 0.0)).collect();
        let (sum_min, sum_max) = auto.iter().filter_map(|i| cols.get(*i)).fold((0.0f32, 0.0f32), |(a, b), c| (a + c.fit.min, b + c.fit.max));
        for i in &auto {
            let (Some(c), Some(slot)) = (cols.get(*i), w.get_mut(*i)) else { continue };
            *slot = if rest >= sum_max {
                // Room for every line whole; the rest, when the table is that wide, by their share.
                let extra = if !stretch {
                    0.0
                } else if sum_max > 0.0 {
                    (rest - sum_max) * c.fit.max / sum_max
                } else {
                    (rest - sum_max) / auto.len() as f32
                };
                c.fit.max + extra
            } else if rest >= sum_min && sum_max > sum_min {
                c.fit.min + (rest - sum_min) * (c.fit.max - c.fit.min) / (sum_max - sum_min)
            } else if sum_min > 0.0 {
                c.fit.min * rest / sum_min
            } else {
                rest / auto.len() as f32
            };
        }
        // No column narrower than its longest word while the others have room to give: a
        // column set to all of the table's width left the one beside it a letter wide.
        let least: f32 = cols.iter().map(|c| c.fit.min).sum();
        if least <= inner {
            let mut short = 0.0f32;
            for (x, c) in w.iter_mut().zip(&cols) {
                if *x < c.fit.min {
                    short += c.fit.min - *x;
                    *x = c.fit.min;
                }
            }
            let spare: f32 = w.iter().zip(&cols).map(|(x, c)| (*x - c.fit.min).max(0.0)).sum();
            if short > 0.0 && spare > 0.0 {
                let take = short.min(spare);
                for (x, c) in w.iter_mut().zip(&cols) {
                    *x -= take * (*x - c.fit.min).max(0.0) / spare;
                }
            }
        }
        // Widths set that don't fit the room give way together.
        let total: f32 = w.iter().sum();
        // So do widths set that leave room over, in a table as wide as it says.
        let over = total > inner + 0.5;
        let under = stretch && auto.is_empty() && total < inner - 0.5;
        if total > 0.0 && (over || under) {
            for x in &mut w {
                *x *= inner / total;
            }
        }
        for x in &mut w {
            *x = x.max(NARROWEST);
        }
        w
    }

    fn table(&self, t: &Table, x: f32, room: f32, y: &mut f32, ops: &mut Vec<Op>, depth: usize) {
        let n = t.rows.iter().map(|r| r.iter().map(|c| usize::from(c.span.max(1))).sum::<usize>()).max().unwrap_or(0).clamp(1, MAX_COLUMNS);
        let between = t.spacing * (n as f32 + 1.0);
        let wanted = match t.width {
            Some(Width::Pt(w)) => w.min(room),
            Some(Width::Percent(p)) => room * p.clamp(0.0, 100.0) / 100.0,
            None if t.fit => self.table_fit(t, depth).max.min(room),
            None => room,
        };
        let stretch = t.width.is_some() || !t.fit;
        let widths = self.columns(t, wanted - between, stretch, depth);
        let outer = (widths.iter().sum::<f32>() + between).min(room.max(NARROWEST));
        let left = x + match t.align {
            Align::Left => 0.0,
            Align::Center => (room - outer) / 2.0,
            Align::Right => room - outer,
        }
        .max(0.0);
        let top = *y;
        let under = ops.len();
        let mut row_top = top + t.spacing;
        for row in &t.rows {
            if ops.len() >= MAX_OPS {
                break;
            }
            // Each cell's content, laid out from 0 down; then the row as tall as its tallest.
            let mut cells: Vec<(f32, f32, &Cell, Vec<Op>, f32, [f32; 4])> = Vec::new();
            let mut at = 0usize;
            for cell in row {
                let span = usize::from(cell.span.max(1)).min(n.saturating_sub(at));
                if span == 0 {
                    break;
                }
                let cx = left + t.spacing + widths.iter().take(at).sum::<f32>() + t.spacing * at as f32;
                let cw = widths.iter().skip(at).take(span).sum::<f32>() + t.spacing * (span as f32 - 1.0);
                let pad = cell.padding.unwrap_or([t.padding; 4]);
                let mut content = Vec::new();
                let mut h = 0.0f32;
                self.blocks(&cell.blocks, cx + pad[3], (cw - pad[1] - pad[3]).max(NARROWEST), cell.align, &mut h, &mut content, depth + 1);
                cells.push((cx, cw, cell, content, h, pad));
                at += span;
            }
            let row_h = cells.iter().map(|(_, _, c, _, h, pad)| (h + pad[0] + pad[2]).max(c.height.unwrap_or(0.0))).fold(0.0f32, f32::max);
            for (cx, cw, cell, content, h, pad) in cells {
                if let Some(color) = cell.background {
                    ops.push(Op::Fill { x: cx, y: row_top, w: cw, h: row_h, color, radius: cell.radius });
                }
                let free = (row_h - pad[0] - pad[2] - h).max(0.0);
                let down = row_top
                    + pad[0]
                    + match cell.valign {
                        VAlign::Top => 0.0,
                        VAlign::Middle => free / 2.0,
                        VAlign::Bottom => free,
                    };
                ops.extend(content.into_iter().map(|op| op.moved(down)));
                // Its lines, over its background: top, right, bottom, left.
                for (side, line) in cell.borders.iter().enumerate() {
                    let Some(l) = line else { continue };
                    let (lx, ly, lw, lh) = match side {
                        0 => (cx, row_top, cw, l.width),
                        1 => (cx + cw - l.width, row_top, l.width, row_h),
                        2 => (cx, row_top + row_h - l.width, cw, l.width),
                        _ => (cx, row_top, l.width, row_h),
                    };
                    ops.push(Op::Fill { x: lx, y: ly, w: lw, h: lh, color: l.color, radius: 0.0 });
                }
            }
            row_top += row_h + t.spacing;
        }
        let height = row_top - top;
        // Behind its rows: its background; around them, its line.
        if let Some(color) = t.background {
            ops.insert(under.min(ops.len()), Op::Fill { x: left, y: top, w: outer, h: height, color, radius: t.radius });
        }
        if let Some(l) = t.border {
            for (lx, ly, lw, lh) in [
                (left, top, outer, l.width),
                (left + outer - l.width, top, l.width, height),
                (left, top + height - l.width, outer, l.width),
                (left, top, l.width, height),
            ] {
                ops.push(Op::Fill { x: lx, y: ly, w: lw, h: lh, color: l.color, radius: 0.0 });
            }
        }
        *y = top + height;
    }
}

/// A column of a table being measured.
#[derive(Clone, Copy, Debug, Default)]
struct Column {
    fit: Fit,
    fixed: Option<f32>,
    percent: Option<f32>,
}

#[cfg(test)]
mod tests;
