//! A shop's notice (an order shipped; issue #47), as a template compiler writes one: what was
//! found reading one beside the same message in a browser.

use super::*;

/// Every table in `blocks`, those in their cells too, outermost first.
fn tables(blocks: &[Block]) -> Vec<&Table> {
    let mut out = Vec::new();
    for b in blocks {
        if let Block::Table(t) = b {
            out.push(t);
            t.rows.iter().flatten().for_each(|c| out.extend(tables(&c.blocks)));
        }
    }
    out
}

/// The spans of `blocks` whose text is `words`.
fn spans(blocks: &[Block], words: &str) -> Vec<crate::Span> {
    let mut all = blocks.to_vec();
    let mut out = Vec::new();
    each_span(&mut all, &mut |s| {
        if s.text.trim() == words {
            out.push(s.clone());
        }
    });
    out
}

fn page(inner: &str) -> String {
    format!(r#"<table width="600" align="center" cellpadding="0" cellspacing="0"><tr><td>{inner}</td></tr></table>"#)
}

/// The steps an order goes through, drawn with cells that hold nothing: a dot as wide and tall
/// as it says, a line between two dots. They were left out, and the steps' names stood alone.
#[test]
fn empty_cells_that_set_their_size_are_kept() {
    let b = html_layout(&page(
        r#"<table width="100%" cellpadding="0" cellspacing="0" role="presentation"><tr><td style="width:20px;height:20px;background-color:#008296;border-radius:10px"></td><td style="height:4px;background-color:#008296"></td><td style="width:20px;height:20px;background-color:#e6e6e6;border-radius:10px"></td><td></td></tr><tr><td>Ordered</td><td></td><td>Shipped</td><td></td></tr></table>"#,
    ));
    let all = tables(&b);
    let steps = all.iter().find(|t| t.rows.len() == 2).unwrap_or_else(|| panic!("{b:?}"));
    let drawn: Vec<(Option<Width>, Option<u16>, Option<[u8; 3]>)> = steps.rows[0].iter().map(|c| (c.width, c.height, c.background)).collect();
    assert_eq!(
        drawn,
        [
            (Some(Width::Px(20)), Some(20), Some([0x00, 0x82, 0x96])),
            (None, Some(4), Some([0x00, 0x82, 0x96])),
            (Some(Width::Px(20)), Some(20), Some([0xe6, 0xe6, 0xe6]))
        ],
        "the dots and the line between them, not the cell that says nothing"
    );
    assert_eq!(steps.rows[1].len(), 2, "a row of names: its empty cells say nothing, and go");
}

/// Bold set on the box around the words (`font-weight` on a cell or a `<div>`), not with `<b>`:
/// a step's name and a total came out plain. A box inside that sets it back is plain.
#[test]
fn a_boxs_weight_is_its_words() {
    let b = html_layout(&page(
        r#"<table width="100%" cellpadding="0" cellspacing="0" role="presentation"><tr><td>Total</td><td align="right" style="font-weight:700">$15.98</td></tr></table><div style="font-weight:bold"><span>Shipped</span> <span style="font-weight:400">today</span></div><div style="font-weight:500">Ordered</div>"#,
    ));
    let bold = |words: &str| spans(&b, words).iter().map(|s| s.bold).collect::<Vec<bool>>();
    assert_eq!(bold("$15.98"), [true], "{b:?}");
    assert_eq!(bold("Total"), [false]);
    assert_eq!(bold("Shipped"), [true]);
    assert_eq!(bold("today"), [false]);
    assert_eq!(bold("Ordered"), [false], "a medium weight isn't bold");
}

/// A cell that puts its boxes at the right (`<td align="right">`): the table in it goes there,
/// unless it says where it sits. A header's link stood at the left of its cell.
#[test]
fn a_cells_align_moves_the_table_in_it() {
    let inner = |own: &str| {
        format!(
            r#"<table width="100%" cellpadding="0" cellspacing="0" role="presentation"><tr><td width="50%">Shop</td><td width="50%" align="right"><table {own} cellpadding="0" cellspacing="0"><tr><td>Your</td><td>Orders</td></tr></table></td></tr></table>"#
        )
    };
    let at = |own: &str| {
        let b = html_layout(&page(&inner(own)));
        let found = tables(&b).iter().find(|t| t.rows.first().is_some_and(|r| r.len() == 2 && r[0].width.is_none())).map(|t| t.align);
        found.unwrap_or_else(|| panic!("{b:?}"))
    };
    assert_eq!(at(""), Align::Right);
    assert_eq!(at(r#"align="left""#), Align::Left);
    assert_eq!(at(r#"style="margin:0 auto""#), Align::Center);
}

/// A tint written as a gradient of one see-through colour (`linear-gradient(rgba(…, 0.03),
/// rgba(…, 0.03))`): the colour it shows on a white page, behind an item's picture.
#[test]
fn a_see_through_tint_is_the_colour_it_shows() {
    let b = html_layout(&page(
        r#"<table width="100%" cellpadding="0" cellspacing="0" role="presentation"><tr><td style="background-image:linear-gradient(rgba(15,17,17,0.03),rgba(15,17,17,0.03));padding:8px">Item</td><td>Name</td></tr></table>"#,
    ));
    let tint = tables(&b).iter().flat_map(|t| t.rows.iter().flatten()).find_map(|c| c.background);
    let Some([r, g, bl]) = tint else { panic!("{b:?}") };
    assert!([r, g, bl].iter().all(|c| (246..=249).contains(c)), "{tint:?}");
}

/// A style sheet's rules reach an element however deep the template nests it (found on a
/// shipping notice: past four dozen elements none applied, and an item's picture was shown at
/// the size of its file).
#[test]
fn style_rules_reach_deeply_nested_elements() {
    const N: usize = 20;
    let open = r#"<table border="0" cellpadding="0" cellspacing="0" role="presentation" width="100%"><tbody><tr><td align="left" style="padding:0">"#;
    let deep = format!("{}<span class=\"name\">Deep</span> <span>plain</span>{}", open.repeat(N), "</td></tr></tbody></table>".repeat(N));
    let b = html_layout(&inline_css(&format!("<style>.name{{font-weight:bold}}</style>{}", page(&deep))));
    assert_eq!(spans(&b, "Deep").iter().map(|s| s.bold).collect::<Vec<bool>>(), [true], "{b:?}");
    assert_eq!(spans(&b, "plain").iter().map(|s| s.bold).collect::<Vec<bool>>(), [false]);
}

/// A column of a set width that holds only a divider: the divider, as wide as the column, not
/// a line across the whole message.
#[test]
fn a_divider_alone_in_a_column_is_as_wide_as_the_column() {
    let b = html_layout(&page(
        r#"<p>Above</p><div style="margin:0 auto;max-width:568px"><p style="border-top:solid 1px #d5d9d9;font-size:1px;margin:0 auto;width:100%"></p></div><p>Below</p>"#,
    ));
    let all = tables(&b);
    let line = all.iter().find(|t| t.rows.first().and_then(|r| r.first()).is_some_and(|c| c.borders[0].is_some())).unwrap_or_else(|| panic!("{b:?}"));
    assert_eq!(line.rows[0][0].borders[0], Some(Line { width: 1, color: [0xd5, 0xd9, 0xd9] }));
    let widths: Vec<Option<Width>> = all.iter().map(|t| t.width).collect();
    assert!(widths.contains(&Some(Width::Px(568))), "{widths:?}");
}

/// Room a box asks for above it (`margin-top`): above the first box in a cell it's room in the
/// cell (a footer's first line stood against the top of its box), and between two boxes an
/// empty line, one, where a paragraph doesn't leave its own.
#[test]
fn room_above_a_box_is_kept() {
    let b = html_layout(&page(
        r#"<table width="100%" cellpadding="0" cellspacing="0" role="presentation"><tr><td width="100">Side</td><td style="padding:0 16px 16px 16px"><div style="margin-top:20px">One</div><div style="margin-top:20px">Two</div><div style="margin:20px 0"><p>Three</p><p>Four</p></div><div style="margin-top:20px">Five</div><div style="margin-top:2px">Six</div><div>Seven</div></td></tr></table>"#,
    ));
    let all = tables(&b);
    let cell = all.iter().flat_map(|t| t.rows.iter().flatten()).find(|c| c.padding.is_some_and(|p| p[1] == 16)).unwrap_or_else(|| panic!("{b:?}"));
    assert_eq!(cell.padding, Some([20, 16, 16, 16]));
    let lines: Vec<String> = cell
        .blocks
        .iter()
        .flat_map(|x| if let Block::Text(l) = x { l.clone() } else { Vec::new() })
        .map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>())
        .collect();
    assert_eq!(lines, ["One", "", "Two", "", "Three", "", "Four", "", "Five", "Six", "Seven"]);
    // No room asked for, none given: a cell keeps its own, or its table's.
    let plain = html_layout(&page(
        r#"<table width="100%" cellpadding="3" cellspacing="0" role="presentation"><tr><td width="100">Side</td><td><div style="margin-top:8px">One</div></td><td><div>Two</div></td></tr></table>"#,
    ));
    let rooms: Vec<Option<[u16; 4]>> =
        tables(&plain).iter().flat_map(|t| t.rows.iter().flatten()).filter(|c| c.width.is_none() && c.blocks.len() == 1).map(|c| c.padding).collect();
    assert!(rooms.contains(&Some([11, 3, 3, 3])) && rooms.contains(&None), "{rooms:?}");
}

/// Found in review, in styles as programs write them out. A colour with spaces in it
/// (`rgb(34, 34, 34)`) wasn't read: the cell had no background, its line was black. The last
/// declaration of a property is the one that counts, as in CSS: a cell that says a colour twice
/// has the second (what's counted as hidden reads its styles the same way).
#[test]
fn styles_as_programs_write_them_are_read() {
    let b = html_layout(&page(
        r#"<table width="100%" cellpadding="0" cellspacing="0"><tr><td style="background-color: rgb(34, 34, 34); color: #fff; border: 2px solid rgb(204, 204, 204)">Dark</td><td style="background: rgba(0, 130, 150, 1) none">Teal</td><td style="background-color:#ffffff;background-color:#c00000">Total</td></tr></table>"#,
    ));
    let all = tables(&b);
    let row = all.iter().find_map(|t| t.rows.iter().find(|r| r.len() == 3)).unwrap_or_else(|| panic!("{b:?}"));
    assert_eq!(row[0].background, Some([34, 34, 34]));
    assert_eq!(row[0].borders[0], Some(Line { width: 2, color: [204, 204, 204] }));
    assert_eq!(row[1].background, Some([0, 130, 150]));
    assert_eq!(row[2].background, Some([0xc0, 0, 0]), "the last one said");
    // Found in a second review: the one marked `!important` counts over a later one, and a
    // value that can't be read gives way to the one before it, as in a browser.
    let b = html_layout(&page(
        r#"<table width="100%" cellpadding="0" cellspacing="0"><tr><td style="background-color:#00aa00;background-color:var(--brand)">A</td><td style="background-color:#c00000 !important;background-color:#ffffff">B</td></tr></table>"#,
    ));
    let all = tables(&b);
    let row = all.iter().find_map(|t| t.rows.iter().find(|r| r.len() == 2)).unwrap_or_else(|| panic!("{b:?}"));
    assert_eq!((row[0].background, row[1].background), (Some([0, 0xaa, 0]), Some([0xc0, 0, 0])));
}
