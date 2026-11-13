//! The layout of a message's tables: how wide its columns come out, where its cells and what's
//! in them go, and that nothing a message can say hangs it.

use super::*;
use crate::Placed;

fn faces() -> [Face<'static>; 2] {
    [Face::Helvetica { bold: false }, Face::Helvetica { bold: true }]
}

fn words(text: &str) -> Block {
    Block::Text(vec![vec![Run { text: text.into(), ..Default::default() }]])
}

fn cell(text: &str) -> Cell {
    Cell { blocks: vec![words(text)], ..Default::default() }
}

fn laid(blocks: &[Block], width: f32) -> (Vec<Op>, f32) {
    let faces = faces();
    let (mut ops, mut y) = (Vec::new(), 0.0);
    Lay { faces: &faces }.blocks(blocks, 0.0, width, Align::Left, &mut y, &mut ops, 0);
    (ops, y)
}

fn texts(ops: &[Op]) -> Vec<(&str, &Placed, f32)> {
    ops.iter()
        .filter_map(|o| match o {
            Op::Text { piece, top, .. } => Some((piece.text.trim(), piece, *top)),
            _ => None,
        })
        .collect()
}

fn fills(ops: &[Op]) -> Vec<(f32, f32, f32, f32, [u8; 3])> {
    ops.iter()
        .filter_map(|o| match o {
            Op::Fill { x, y, w, h, color, .. } => Some((*x, *y, *w, *h, *color)),
            _ => None,
        })
        .collect()
}

fn widths(t: &Table, inner: f32, stretch: bool) -> Vec<f32> {
    let faces = faces();
    Lay { faces: &faces }.columns(t, inner, stretch, 0)
}

#[test]
fn columns_share_the_room_by_what_they_hold() {
    let long = "a much longer line of words than the other column has in it";
    let t = Table { rows: vec![vec![cell("short"), cell(long)]], ..Default::default() };
    // Room for both whole: each as wide as its line, and no wider unless the table is.
    let w = widths(&t, 2000.0, false);
    assert!(w[0] < w[1] && w.iter().sum::<f32>() < 500.0, "{w:?}");
    let w = widths(&t, 2000.0, true);
    assert!((w.iter().sum::<f32>() - 2000.0).abs() < 1.0 && w[0] < w[1], "stretched to the table's width: {w:?}");
    // Too little for both whole: the long one wraps, the short one keeps its word.
    let w = widths(&t, 200.0, true);
    assert!((w.iter().sum::<f32>() - 200.0).abs() < 1.0, "{w:?}");
    assert!(w[0] >= 24.0 && w[1] > w[0], "{w:?}");

    // A width set in points is kept; a share is a share of the table.
    let mut set = t.clone();
    set.rows[0][0].width = Some(Width::Pt(120.0));
    let w = widths(&set, 400.0, true);
    assert!((w[0] - 120.0).abs() < 0.5 && (w[1] - 280.0).abs() < 0.5, "{w:?}");
    set.rows[0][0].width = Some(Width::Percent(25.0));
    let w = widths(&set, 400.0, true);
    assert!((w[0] - 100.0).abs() < 0.5 && (w[1] - 300.0).abs() < 0.5, "{w:?}");

    // A column set to the whole table leaves the one beside it its longest word (it was left a
    // letter wide, its words a letter to a line).
    let mut all = t.clone();
    all.rows[0][1].width = Some(Width::Percent(100.0));
    let w = widths(&all, 400.0, true);
    assert!(w[0] >= 24.0, "the first column keeps its word: {w:?}");
    assert!((w.iter().sum::<f32>() - 400.0).abs() < 1.0, "{w:?}");

    // Widths that don't fit give way together, and never past the room.
    let mut wide = t.clone();
    wide.rows[0][0].width = Some(Width::Pt(900.0));
    wide.rows[0][1].width = Some(Width::Pt(300.0));
    let w = widths(&wide, 400.0, true);
    assert!((w.iter().sum::<f32>() - 400.0).abs() < 1.0 && w[0] > w[1], "{w:?}");
}

#[test]
fn cells_sit_side_by_side_with_their_backgrounds() {
    let mut left = cell("Left");
    left.background = Some([1, 2, 3]);
    left.padding = Some([4.0, 6.0, 4.0, 6.0]);
    let mut right = Cell { blocks: vec![words("Right one"), words("and a second line"), words("and a third")], ..Default::default() };
    right.background = Some([4, 5, 6]);
    right.borders[2] = Some(Line { width: 1.5, color: [9, 9, 9] });
    let t = Table {
        rows: vec![vec![left, right]],
        width: Some(Width::Pt(300.0)),
        background: Some([7, 7, 7]),
        border: Some(Line { width: 1.0, color: [8, 8, 8] }),
        spacing: 2.0,
        ..Default::default()
    };
    let (ops, height) = laid(&[Block::Table(t)], 400.0);
    let t = texts(&ops);
    let (l, r) = (t.iter().find(|x| x.0 == "Left").unwrap(), t.iter().find(|x| x.0 == "Right").unwrap());
    assert!(l.1.x < r.1.x && (l.2 - r.2 - 4.0).abs() < 0.01, "side by side, the left one under its own padding");
    assert!(r.1.x > 30.0 && r.1.x < 300.0, "{}", r.1.x);
    assert!((l.1.x - 8.0).abs() < 0.1, "after the table's spacing and its own padding: {}", l.1.x);
    let f = fills(&ops);
    // The table's background first, as wide as it says; then each cell's, as tall as the row.
    assert_eq!((f[0].0, f[0].1, f[0].2, f[0].4), (0.0, 0.0, 300.0, [7, 7, 7]));
    assert!((f[0].3 - height).abs() < 0.01);
    let (a, b) = (f.iter().find(|x| x.4 == [1, 2, 3]).unwrap(), f.iter().find(|x| x.4 == [4, 5, 6]).unwrap());
    assert!((a.3 - b.3).abs() < 0.01 && a.3 > 40.0, "both as tall as the three lines: {a:?} {b:?}");
    assert!((a.0 + a.2 + 2.0 - b.0).abs() < 0.1, "the spacing between them: {a:?} {b:?}");
    // The cell's line along its bottom, and the table's four around it.
    assert!(f.iter().any(|x| x.4 == [9, 9, 9] && (x.3 - 1.5).abs() < 0.01 && (x.1 + x.3 - (b.1 + b.3)).abs() < 0.01));
    assert_eq!(f.iter().filter(|x| x.4 == [8, 8, 8]).count(), 4);
    // Everything inside the table's width.
    for o in &ops {
        let (x0, x1) = match o {
            Op::Fill { x, w, .. } | Op::Image { x, w, .. } | Op::Link { x, w, .. } => (*x, *x + *w),
            Op::Text { piece, .. } => (piece.x, piece.x + piece.width),
        };
        assert!(x0 >= -0.01 && x1 <= 300.01, "{o:?}");
    }
}

#[test]
fn content_lines_up_as_the_cell_says() {
    let row = |align, valign| {
        let tall = Cell { blocks: vec![words("one"), words("two"), words("three"), words("four")], ..Default::default() };
        let mut c = cell("Here");
        c.align = align;
        c.valign = valign;
        c.width = Some(Width::Pt(200.0));
        Table { rows: vec![vec![c, tall]], width: Some(Width::Pt(300.0)), ..Default::default() }
    };
    let at = |align, valign| {
        let (ops, _) = laid(&[Block::Table(row(align, valign))], 300.0);
        let t = texts(&ops);
        let here = t.iter().find(|x| x.0 == "Here").unwrap();
        (here.1.x, here.1.x + here.1.width, here.2)
    };
    let (left, _, top) = at(Align::Left, VAlign::Top);
    let (c0, c1, middle) = at(Align::Center, VAlign::Middle);
    let (_, right, bottom) = at(Align::Right, VAlign::Bottom);
    assert!(left.abs() < 0.1 && (right - 200.0).abs() < 0.1, "{left} {right}");
    assert!(((c0 + c1) / 2.0 - 100.0).abs() < 0.1, "{c0} {c1}");
    assert!(top < middle && middle < bottom, "{top} {middle} {bottom}");
    // A table that fits what it holds, centred in its room.
    let button =
        Table { rows: vec![vec![cell("Go")]], fit: true, align: Align::Center, padding: 10.0, background: Some([1, 1, 1]), ..Default::default() };
    let (ops, _) = laid(&[Block::Table(button)], 400.0);
    let f = fills(&ops);
    assert!(f[0].2 < 60.0 && ((f[0].0 + f[0].2 / 2.0) - 200.0).abs() < 0.1, "{:?}", f[0]);
}

#[test]
fn a_picture_and_a_link_take_their_place() {
    let line = vec![
        Run { text: "See".into(), ..Default::default() },
        Run { picture: Some((0, 40.0, 30.0)), link: Some("https://x.example/".into()), ..Default::default() },
        Run { text: "here".into(), link: Some("https://x.example/".into()), ..Default::default() },
        Run { picture: Some((HELD_BACK, 16.0, 16.0)), ..Default::default() },
    ];
    let (ops, height) = laid(&[Block::Text(vec![line.clone()])], 300.0);
    let image = ops.iter().find_map(|o| if let Op::Image { x, y, w, h, index } = o { Some((*x, *y, *w, *h, *index)) } else { None }).unwrap();
    assert_eq!((image.2, image.3, image.4), (40.0, 30.0, 0));
    assert!(image.1 >= 0.0 && image.1 + image.3 <= height, "on its line: {image:?} in {height}");
    // The picture and the word after it go to one place: one place to click, over both.
    let links: Vec<f32> = ops.iter().filter_map(|o| if let Op::Link { w, .. } = o { Some(*w) } else { None }).collect();
    assert!(links.len() == 1 && links[0] > 40.0 + 15.0, "{links:?}");
    assert_eq!(fills(&ops).len(), 4, "the room of the picture that isn't shown, a line around it");
    assert!(!ops.iter().any(|o| matches!(o, Op::Image { index, .. } if *index == HELD_BACK)));
    // In a narrow cell it's as wide as the cell, its shape kept; never taller than a page.
    let (ops, _) = laid(&[Block::Text(vec![line])], 20.0);
    let image = ops.iter().find_map(|o| if let Op::Image { w, h, .. } = o { Some((*w, *h)) } else { None }).unwrap();
    assert!((image.0 - 20.0).abs() < 0.01 && (image.1 - 15.0).abs() < 0.01, "{image:?}");
    let tall = vec![Run { picture: Some((0, 100.0, 90_000.0)), ..Default::default() }];
    let (ops, height) = laid(&[Block::Text(vec![tall])], 300.0);
    assert!(height < 700.0 && ops.len() == 1, "{height}");
}

#[test]
fn nothing_a_message_says_hangs_the_layout() {
    // Tables in tables past any message's: the deepest are left out, and it ends.
    let mut deep = Block::Table(Table { rows: vec![vec![cell("bottom")]], ..Default::default() });
    for _ in 0..200 {
        deep = Block::Table(Table { rows: vec![vec![Cell { blocks: vec![deep], ..Default::default() }]], ..Default::default() });
    }
    let (ops, _) = laid(&[deep], 300.0);
    assert!(!texts(&ops).iter().any(|t| t.0 == "bottom"));
    // More columns than are laid out, spans past the row's end, a span of nothing.
    let many = Table { rows: vec![(0..500).map(|i| Cell { span: (i % 7) as u16, ..cell("x") }).collect()], ..Default::default() };
    let (ops, _) = laid(&[Block::Table(many)], 300.0);
    assert!(!ops.is_empty());
    // Sizes that aren't sizes.
    for bad in [f32::NAN, f32::INFINITY, -50.0, 0.0, 1e30] {
        let mut c = cell("words in a cell");
        c.width = Some(Width::Pt(bad));
        c.padding = Some([bad; 4]);
        c.height = Some(bad);
        let t = Table { rows: vec![vec![c, cell("more")]], width: Some(Width::Percent(bad)), spacing: bad, padding: bad, ..Default::default() };
        let (ops, _) = laid(&[Block::Table(t.clone())], 300.0);
        let _ = (ops.len(), laid(&[Block::Table(t)], bad));
    }
    // No rows, no cells, no blocks.
    for t in [Table::default(), Table { rows: vec![vec![]], ..Default::default() }, Table { rows: vec![vec![Cell::default()]], ..Default::default() }]
    {
        let _ = laid(&[Block::Table(t)], 300.0);
    }
}
