//! The lines between a table's cells: where its borders collapse (`border-collapse: collapse`),
//! two cells side by side or one over the other share one line, as a browser draws it. Each
//! drew its own, and a line between them was twice as thick as the table's others.

use super::*;

/// The grid of a table's cells' lines: `[top, right, bottom, left]` widths, 0 for none.
fn grid(html: &str) -> Vec<Vec<[u8; 4]>> {
    let b = html_layout(html);
    let t = tables(&b).into_iter().find(|t| t.rows.iter().flatten().any(|c| c.borders.iter().any(Option::is_some)));
    let t = t.unwrap_or_else(|| panic!("a table with lines: {b:?}"));
    t.rows.iter().map(|r| r.iter().map(|c| c.borders.map(|l| l.map_or(0, |l| l.width))).collect()).collect()
}

const CELL: &str = "border:1px solid #444";

/// Every cell with a line all round, the borders collapsing (a table copied in a browser, or one
/// MailCraft writes): one line where two meet, the one above or to the left.
#[test]
fn cells_in_a_collapsed_table_share_their_lines() {
    let html = format!(
        r#"<table style="border-collapse:collapse"><tr><td style="{CELL}">A</td><td style="{CELL}">B</td></tr><tr><td style="{CELL}">C</td><td style="{CELL}">D</td></tr></table>"#
    );
    assert_eq!(grid(&html), vec![vec![[1, 1, 1, 1], [1, 1, 1, 0]], vec![[0, 1, 1, 1], [0, 1, 1, 0]]]);
}

/// Cells apart (`cellspacing`, or none said): each keeps its lines, as a browser draws them, even
/// with no room between (LibreOffice's tables: their lines are two wide in a browser too).
#[test]
fn cells_apart_keep_their_own_lines() {
    for table in [r#"<table cellspacing="0">"#, "<table>"] {
        let html = format!(r#"{table}<tr><td style="{CELL}">A</td><td style="{CELL}">B</td></tr></table>"#);
        assert_eq!(grid(&html), vec![vec![[1, 1, 1, 1], [1, 1, 1, 1]]], "{table}");
    }
}

/// Of two lines that meet, the wider is the one drawn; a cell's side with no line of its own
/// keeps its neighbour's.
#[test]
fn the_wider_of_two_lines_is_drawn() {
    let html = r#"<table style="border-collapse:collapse"><tr>
        <td style="border:1px solid #444">A</td><td style="border:3px solid #000">B</td><td style="border-right:1px solid #444">C</td>
    </tr></table>"#;
    assert_eq!(grid(html), vec![vec![[1, 0, 1, 1], [3, 3, 3, 3], [0, 1, 0, 0]]]);
}

/// A cell across two columns under two cells: its top is theirs when both have a line along their
/// bottom; when only one does, it keeps its own (a line can't be drawn in part).
#[test]
fn a_merged_cell_shares_its_top_with_the_cells_over_it() {
    let row = |cells: &str| format!("<tr>{cells}</tr>");
    let two = format!(r#"<td style="{CELL}">A</td><td style="{CELL}">B</td>"#);
    let merged = format!(r#"<td colspan="2" style="{CELL}">Total</td>"#);
    let html = format!(r#"<table style="border-collapse:collapse">{}{}</table>"#, row(&two), row(&merged));
    assert_eq!(grid(&html)[1], vec![[0, 1, 1, 1]]);
    let half = format!(r#"<td style="{CELL}">A</td><td>B</td>"#);
    let html = format!(r#"<table style="border-collapse:collapse">{}{}</table>"#, row(&half), row(&merged));
    assert_eq!(grid(&html)[1], vec![[1, 1, 1, 1]]);
}
