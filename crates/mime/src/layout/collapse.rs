//! A table whose borders collapse (`border-collapse: collapse`): two cells side by side, or one
//! over the other, share the line between them, as a browser draws it. Each cell kept its own,
//! and the line between two was twice as thick as the table's others.

use super::{Cell, Line};

const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;

/// A cell's lines: top, right, bottom, left.
type Lines = [Option<Line>; 4];

/// Leave one line where two cells meet ([`shared_lines`]).
pub(super) fn share_lines(rows: &mut [Vec<Cell>]) {
    let shared = shared_lines(rows);
    for (row, lines) in rows.iter_mut().zip(shared) {
        for (cell, lines) in row.iter_mut().zip(lines) {
            cell.borders = lines;
        }
    }
}

/// Each cell's lines with one left where two cells meet: the wider, or the one above or to the
/// left when they're as wide. A side with no line of its own keeps its neighbour's.
pub(super) fn shared_lines(rows: &[Vec<Cell>]) -> Vec<Vec<Lines>> {
    let mut grid: Vec<Vec<Lines>> = rows.iter().map(|row| row.iter().map(|c| c.borders).collect()).collect();
    // The columns each cell covers, by the columns those before it in its row span.
    let columns: Vec<Vec<(usize, usize)>> = rows
        .iter()
        .map(|row| {
            let mut at = 0usize;
            row.iter()
                .map(|c| {
                    let to = at.saturating_add(usize::from(c.span.max(1)));
                    let covers = (at, to);
                    at = to;
                    covers
                })
                .collect()
        })
        .collect();
    for row in &mut grid {
        for i in 1..row.len() {
            let (before, after) = row.split_at_mut(i);
            if let (Some(left), Some(right)) = (before.last_mut(), after.first_mut()) {
                let (mut a, mut b) = (left[RIGHT], right[LEFT]);
                share(&mut a, &mut b);
                (left[RIGHT], right[LEFT]) = (a, b);
            }
        }
    }
    for r in 1..grid.len() {
        let (above, below) = grid.split_at_mut(r);
        let (Some(above), Some(below)) = (above.last_mut(), below.first_mut()) else { continue };
        let (Some(over_columns), Some(under_columns)) = (columns.get(r - 1), columns.get(r)) else { continue };
        for (cell, &(from, to)) in below.iter_mut().zip(under_columns) {
            let Some(own) = cell[TOP] else { continue };
            // The cells over it.
            let over: Vec<usize> = over_columns.iter().enumerate().filter(|(_, (a, b))| *a < to && *b > from).map(|(j, _)| j).collect();
            if over.is_empty() {
                continue;
            }
            let lines: Vec<Option<Line>> = over.iter().map(|&j| above.get(j).and_then(|c| c[BOTTOM])).collect();
            let spans = |j: usize| over_columns.get(j).copied();
            let covered = over.first().and_then(|&j| spans(j)).is_some_and(|(a, _)| a <= from)
                && over.last().and_then(|&j| spans(j)).is_some_and(|(_, b)| b >= to);
            if covered && lines.iter().all(|l| l.is_some_and(|l| l.width >= own.width)) {
                // Theirs along all of it: its own goes.
                cell[TOP] = None;
            } else if lines.iter().all(|l| l.is_none_or(|l| l.width < own.width))
                && over.iter().all(|&j| spans(j).is_some_and(|(a, b)| a >= from && b <= to))
            {
                // Its own is the wider, and under all of theirs: theirs go.
                for &j in &over {
                    if let Some(c) = above.get_mut(j) {
                        c[BOTTOM] = None;
                    }
                }
            }
        }
    }
    grid
}

/// Two lines along one edge, `first` the one above or to the left: the wider stays, `first` when
/// they're as wide.
fn share(first: &mut Option<Line>, second: &mut Option<Line>) {
    let (Some(a), Some(b)) = (*first, *second) else { return };
    if b.width > a.width {
        *first = None;
    } else {
        *second = None;
    }
}
