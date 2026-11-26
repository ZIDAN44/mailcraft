//! Cells spanning rows (`rowspan`, discussion #275), laid out as a browser lays them out: a cell
//! spanning the rows below it stays one cell beside them, and those rows' other cells stand in a
//! table of their own in its place, one row under another. Read as one row each, a signature's
//! logo beside its lines (each line a row) stood above them, every line but the first under it.

use super::{Block, Cell, Table, VAlign};

/// Most rows one cell is taken to span.
const MOST_ROWS: usize = 1_000;
/// How deep spans inside spans are folded; past it, a row's cells are taken as they come.
const MOST_DEPTH: usize = 4;

/// How many rows a cell's `rowspan` says it spans: one for none, zero, below zero or nonsense.
pub(super) fn rows_spanned(rowspan: Option<&str>) -> usize {
    rowspan.and_then(|s| s.trim().parse::<usize>().ok()).filter(|n| *n >= 1).unwrap_or(1).min(MOST_ROWS)
}

/// A table's rows, each cell with the rows it spans, as rows of cells: the rows a cell spans made
/// one, with what stands beside it in tables of their own (with the table's `spacing` and
/// `padding`).
pub(super) fn fold(rows: Vec<Vec<(Cell, usize)>>, spacing: Option<u16>, padding: Option<u16>) -> Vec<Vec<Cell>> {
    fold_at(rows, spacing, padding, 0)
}

fn fold_at(rows: Vec<Vec<(Cell, usize)>>, spacing: Option<u16>, padding: Option<u16>, depth: usize) -> Vec<Vec<Cell>> {
    let mut rows = rows.into_iter();
    let mut out = Vec::new();
    while let Some(first) = rows.next() {
        let spans = first.iter().map(|(_, s)| *s).max().unwrap_or(1);
        // The rows it spans, as far as the table goes.
        let rest: Vec<Vec<(Cell, usize)>> = if depth < MOST_DEPTH { rows.by_ref().take(spans.saturating_sub(1)).collect() } else { Vec::new() };
        if rest.is_empty() {
            out.push(first.into_iter().map(|(c, _)| c).collect());
        } else {
            out.push(fold_group(first, rest, spacing, padding, depth));
        }
    }
    out
}

/// What the first row of a group has, in order: a cell spanning the whole group, or cells that
/// don't, with how many columns they take.
enum Part {
    Spanning(Cell),
    Run(Vec<(Cell, usize)>, u16),
}

/// One row for `first` and the rows it spans (`rest`): its cells spanning them all as they are,
/// and in place of each run of the others a table of those rows' cells in those columns.
fn fold_group(first: Vec<(Cell, usize)>, rest: Vec<Vec<(Cell, usize)>>, spacing: Option<u16>, padding: Option<u16>, depth: usize) -> Vec<Cell> {
    let group = rest.len() + 1;
    let mut parts: Vec<Part> = Vec::new();
    for (c, s) in first {
        if s >= group {
            parts.push(Part::Spanning(c));
            continue;
        }
        let span = c.span.max(1);
        match parts.last_mut() {
            Some(Part::Run(cells, columns)) => {
                *columns = columns.saturating_add(span);
                cells.push((c, s));
            }
            _ => parts.push(Part::Run(vec![(c, s)], span)),
        }
    }
    // Spanning cells alone in the first row: the other rows' cells stand after them.
    if !parts.iter().any(|p| matches!(p, Part::Run(..))) {
        parts.push(Part::Run(Vec::new(), 1));
    }
    let widths: Vec<u16> = parts.iter().filter_map(|p| if let Part::Run(_, w) = p { Some(*w) } else { None }).collect();
    // Each run's rows: its cells of the first row, then of each row below, the cells of a row
    // filling the runs in order, as many columns as each takes (the last takes what's left).
    let mut below: Vec<Vec<Vec<(Cell, usize)>>> = vec![Vec::new(); widths.len()];
    for row in rest {
        let mut cells = row.into_iter().peekable();
        for (k, width) in widths.iter().enumerate() {
            let last = k + 1 == widths.len();
            let mut taken = Vec::new();
            let mut used = 0u16;
            while let Some((c, _)) = cells.peek() {
                if !last && used >= *width {
                    break;
                }
                used = used.saturating_add(c.span.max(1));
                taken.extend(cells.next());
            }
            if let Some(rows) = below.get_mut(k) {
                rows.push(taken);
            }
        }
    }
    let mut below = below.into_iter();
    let mut out = Vec::new();
    for part in parts {
        match part {
            Part::Spanning(c) => out.push(c),
            Part::Run(cells, columns) => {
                let mut rows: Vec<Vec<(Cell, usize)>> = vec![cells];
                rows.extend(below.next().unwrap_or_default());
                rows.retain(|r| !r.is_empty());
                match rows.len() {
                    0 => {}
                    // Nothing below it: its cells, as they are.
                    1 => out.extend(rows.into_iter().flatten().map(|(c, _)| c)),
                    _ => out.push(Cell {
                        blocks: vec![Block::Table(Table {
                            rows: fold_at(rows, spacing, padding, depth + 1),
                            spacing,
                            padding,
                            fit: true,
                            ..Default::default()
                        })],
                        span: columns.max(1),
                        padding: Some([0; 4]),
                        valign: VAlign::Middle,
                        ..Default::default()
                    }),
                }
            }
        }
    }
    out
}
