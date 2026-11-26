//! Cells spanning rows (`rowspan`, discussion #275): a signature's logo, and a line beside it,
//! next to its details, each line a row of its own. Read as one row each, every row but the
//! first fell under the logo, and the logo stood above the name.

use super::*;

/// The words in `blocks`, each line on a line of its own.
fn words(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Text(lines) => {
                for l in lines {
                    out.extend(l.iter().map(|s| s.text.as_str()));
                    out.push('\n');
                }
            }
            Block::Table(t) => t.rows.iter().flatten().for_each(|c| out.push_str(&words(&c.blocks))),
        }
    }
    out
}

/// The rows of the table `cell` holds, by their words.
fn inner_rows(cell: &Cell) -> Vec<String> {
    let Some(Block::Table(t)) = cell.blocks.first() else { panic!("no table in {cell:?}") };
    t.rows.iter().map(|r| r.iter().map(|c| words(&c.blocks).trim().to_string()).collect::<Vec<_>>().join(" | ")).collect()
}

const SIGNATURE: &str = r#"<table cellpadding="0" cellspacing="0">
    <tr><td rowspan="3"><img src="cid:logo@x" width="80" height="80"></td>
        <td rowspan="3" style="border-left:2px solid #555555"></td>
        <td><b>Dean Moss</b></td></tr>
    <tr><td>Sales</td></tr>
    <tr><td>Phone 000</td></tr>
    <tr><td colspan="3">Printed on recycled pixels</td></tr></table>"#;

#[test]
fn a_cell_spanning_rows_stays_beside_them() {
    let b = html_layout(SIGNATURE);
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert_eq!(t.rows.len(), 2, "the three rows one, the last its own: {t:#?}");
    let row = &t.rows[0];
    assert_eq!(row.len(), 3, "{row:#?}");
    assert_eq!(row[0].width, Some(Width::Px(96)), "the logo's column, as wide as it");
    assert!(row[1].borders.iter().any(Option::is_some), "the line beside it: {:?}", row[1].borders);
    assert_eq!(inner_rows(&row[2]), ["Dean Moss", "Sales", "Phone 000"], "the details, one under another, beside the logo");
    assert_eq!(words(&t.rows[1][0].blocks).trim(), "Printed on recycled pixels");
}

/// Spanning cells on both sides, and rows whose cells fill the columns between; a span past the
/// table's last row goes as far as it does, one of none or nonsense is one row.
#[test]
fn spans_on_both_sides_and_odd_spans() {
    let b = html_layout(
        r#"<table><tr><td rowspan="2">Left</td><td>A1</td><td>B1</td><td rowspan="2">Right</td></tr>
           <tr><td>A2</td><td>B2</td></tr></table>"#,
    );
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert_eq!(t.rows.len(), 1, "{t:#?}");
    let row = &t.rows[0];
    assert_eq!(words(&row[0].blocks).trim(), "Left");
    assert_eq!(inner_rows(&row[1]), ["A1 | B1", "A2 | B2"]);
    assert_eq!(words(&row[2].blocks).trim(), "Right");

    // Past the last row: as far as it goes. Zero, below zero, or not a number: one row.
    let b = html_layout(r#"<table><tr><td rowspan="9">Logo</td><td>Name</td></tr><tr><td>Title</td></tr></table>"#);
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert_eq!((t.rows.len(), inner_rows(&t.rows[0][1])), (1, vec!["Name".to_string(), "Title".to_string()]));
    for odd in ["0", "-3", "two"] {
        let b = html_layout(&format!(r#"<table><tr><td rowspan="{odd}">Logo</td><td>Name</td></tr><tr><td>Title</td><td>X</td></tr></table>"#));
        let Block::Table(t) = &b[0] else { panic!("{b:?}") };
        let rows: Vec<String> =
            t.rows.iter().map(|r| r.iter().map(|c| words(&c.blocks).trim().to_string()).collect::<Vec<_>>().join(" | ")).collect();
        assert_eq!(rows, ["Logo | Name", "Title | X"], "{odd}");
    }
}
