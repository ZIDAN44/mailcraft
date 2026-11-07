//! Table rows against what a browser makes of the same HTML (issue #47): the widths and heights
//! below are Chrome's, measured on the HTML each test quotes.

use mime::layout::{Block, Cell, VAlign, Width, html_layout};

use super::*;

/// The first row of the first table in `html`, as the layout reads it.
fn first_row(html: &str) -> Vec<Cell> {
    let blocks = html_layout(html);
    let Some(Block::Table(t)) = blocks.first() else { panic!("a table: {blocks:?}") };
    t.rows.first().cloned().unwrap_or_default()
}

fn close(got: &[f32], want: &[f32]) -> bool {
    got.len() == want.len() && got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.5)
}

/// `<table width=600 cellspacing=4 cellpadding=6>` of three 33% cells: Chrome makes each 194.67
/// wide, the 1% left over shared out; it went to the last cell (192.7, 192.7, 198.6).
#[test]
fn percentage_cells_share_the_row_as_a_browser_does() {
    let row = first_row(
        r#"<table width="600" cellspacing="4" cellpadding="6"><tr><td width="33%">a</td><td width="33%">b</td><td width="33%">c</td></tr></table>"#,
    );
    // The row inside the table's own spacing (600 less 4 on each side).
    let (w, stack) = cell_widths(&row, &column_widths(std::slice::from_ref(&row)), 592.0, 4.0);
    assert!(!stack);
    assert!(close(&w, &[194.67, 194.67, 194.67]), "{w:?}");
    // A share, a cell that asks for nothing and one of 150 pixels: 150, 300, 150 in Chrome.
    let row =
        first_row(r#"<table width="600" cellspacing="0" cellpadding="0"><tr><td width="25%">a</td><td>b</td><td width="150">c</td></tr></table>"#);
    let (w, _) = cell_widths(&row, &column_widths(std::slice::from_ref(&row)), 600.0, 0.0);
    assert!(close(&w, &[150.0, 300.0, 150.0]), "{w:?}");
}

/// `<td height=100 valign=top>` beside a cell of two lines (middle, as HTML has it unsaid) and
/// one `valign=bottom`, lines 18 pixels tall: the row is 100 tall, and their text starts 0, 32
/// and 82 down, as in Chrome. A row of `height=80` with `cellpadding=10`: text 31 down.
#[test]
fn a_rows_height_is_a_minimum_and_its_cells_line_up_as_they_say() {
    let row = first_row(
        r#"<table width="600" cellspacing="0" cellpadding="0"><tr><td height="100" valign="top">top</td><td>middle<br>by default</td><td valign="bottom">bottom</td></tr></table>"#,
    );
    assert_eq!(
        row.iter().map(|c| (c.valign, c.height)).collect::<Vec<_>>(),
        [(VAlign::Top, Some(100)), (VAlign::Middle, None), (VAlign::Bottom, None)]
    );
    let asked: Vec<f32> = row.iter().map(|c| c.height.map_or(0.0, f32::from)).collect();
    let (h, down) = row_heights(&[18.0, 36.0, 18.0], &asked, &[VAlign::Top, VAlign::Middle, VAlign::Bottom]);
    assert_eq!(h, 100.0);
    assert!(close(&down, &[0.0, 32.0, 82.0]), "{down:?}");
    // Taller content than asked: the row grows to it.
    let (h, _) = row_heights(&[18.0, 140.0, 18.0], &asked, &[VAlign::Top; 3]);
    assert_eq!(h, 140.0);
    let row = first_row(r#"<table width="600" cellspacing="0" cellpadding="10"><tr height="80"><td>a</td><td valign="middle">b</td></tr></table>"#);
    assert!(row.iter().all(|c| c.height == Some(80) && c.valign == VAlign::Middle), "{row:?}");
    // Content 18 tall and padding 10 above and below; the content starts 10 + 21 = 31 down.
    let (h, down) = row_heights(&[38.0, 38.0], &[80.0, 80.0], &[VAlign::Middle; 2]);
    assert_eq!(h, 80.0);
    assert!(close(&down, &[21.0, 21.0]), "{down:?}");
}

/// Boxes side by side with `inline-block` straight in a cell (a newsletter's notes under its
/// products): one row of columns, side by side when they fit (three of 180 in 552, as in
/// Chrome), one under the other when they don't (in 300).
#[test]
fn inline_blocks_in_a_cell_are_columns_when_they_fit() {
    let html = r#"<table width="600"><tr><td style="padding:16px 24px"><div style="display:inline-block;vertical-align:top;width:180px"><h3>Free returns</h3><p>30 days.</p></div><div style="display:inline-block;vertical-align:top;width:180px"><h3>Two-year warranty</h3><p>On every lamp.</p></div><div style="display:inline-block;vertical-align:top;width:180px"><h3>Bulbs included</h3><p>Warm white.</p></div></td></tr></table>"#;
    let row = first_row(html);
    let Some(Block::Table(inner)) = row.first().and_then(|c| c.blocks.first()) else { panic!("columns in the cell: {row:?}") };
    let cols = inner.rows.first().cloned().unwrap_or_default();
    assert_eq!(cols.iter().map(|c| c.width).collect::<Vec<_>>(), [Some(Width::Px(180)); 3]);
    let (w, stack) = cell_widths(&cols, &column_widths(std::slice::from_ref(&cols)), 552.0, 0.0);
    assert!(!stack && w.first().is_some_and(|w| (w - 180.0).abs() < 0.5), "{w:?}");
    let (_, stack) = cell_widths(&cols, &column_widths(std::slice::from_ref(&cols)), 300.0, 0.0);
    assert!(stack, "too narrow: one under the other");
}
