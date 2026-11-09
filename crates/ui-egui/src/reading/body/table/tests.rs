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
    let (w, stack) = cell_widths(&row, &column_widths(std::slice::from_ref(&row)), &[], 592.0, 4.0);
    assert!(!stack);
    assert!(close(&w, &[194.67, 194.67, 194.67]), "{w:?}");
    // A share, a cell that asks for nothing and one of 150 pixels: 150, 300, 150 in Chrome.
    let row =
        first_row(r#"<table width="600" cellspacing="0" cellpadding="0"><tr><td width="25%">a</td><td>b</td><td width="150">c</td></tr></table>"#);
    let (w, _) = cell_widths(&row, &column_widths(std::slice::from_ref(&row)), &[], 600.0, 0.0);
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
    let (w, stack) = cell_widths(&cols, &column_widths(std::slice::from_ref(&cols)), &[], 552.0, 0.0);
    assert!(!stack && w.first().is_some_and(|w| (w - 180.0).abs() < 0.5), "{w:?}");
    let (_, stack) = cell_widths(&cols, &column_widths(std::slice::from_ref(&cols)), &[], 300.0, 0.0);
    assert!(stack, "too narrow: one under the other");
}

/// `f` given a `Ui` of a frame, its fonts ready.
fn with_ui<R>(f: impl FnOnce(&Ui) -> R) -> R {
    let ctx = egui::Context::default();
    let mut f = Some(f);
    let mut out = None;
    let mut done = ctx.run_ui(egui::RawInput::default(), |ui| {
        if let Some(f) = f.take() {
            out = Some(f(ui));
        }
    });
    // No renderer here: the font atlas's uploads let go deliberately.
    done.textures_delta.clear();
    out.expect("a frame")
}

/// A cell of pictures side by side, each `widths` pixels wide: content of a width known
/// without a font, so its columns can be held against Chrome's.
fn pictures(widths: &[u32]) -> Cell {
    let line = widths
        .iter()
        .map(|w| mime::Span {
            image: Some("cid:p@x".into()),
            image_size: Some(mime::ImageSize { width: Some(Width::Px(*w)), ..Default::default() }),
            ..Default::default()
        })
        .collect();
    Cell { blocks: vec![Block::Text(vec![line])], span: 1, ..Default::default() }
}

fn key() -> MessageKey {
    MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid: 1 }
}

/// Columns that set no width, sized by what they hold as Chrome sizes them (CSS 2.1 §17.5.2.2):
/// pictures of 50, four of 100 and two of 150 side by side, a column each, `cellspacing=0`:
/// - in 600, more than their widest picture and less than all unwrapped: 50, 300, 250 (each its
///   widest, the rest in proportion to how much more it would take);
/// - one picture each in 600, all fit: 100, 200, 300 (the room left in proportion to each);
/// - a table that sets no width: as wide as they are, 300 (50, 100, 150);
/// - `cellspacing=4` in 600: 50, 289.33, 244.67.
///
/// They were 200 each, shared equally.
#[test]
fn columns_without_widths_are_sized_by_what_they_hold_as_in_chrome() {
    let wrapped = vec![pictures(&[50]), pictures(&[100; 4]), pictures(&[150; 2])];
    let one_each = vec![pictures(&[50]), pictures(&[100]), pictures(&[150])];
    let table = |row: &Vec<Cell>, spacing: u16, fit: bool| mime::layout::Table {
        rows: vec![row.clone()],
        width: (!fit).then_some(Width::Px(600)),
        spacing: Some(spacing),
        padding: Some(0),
        fit,
        ..Default::default()
    };
    let widths = |t: &mime::layout::Table, inner: f32| {
        let m = with_ui(|ui| measured(ui, &key(), t));
        cell_widths(&t.rows[0], &column_widths(&t.rows), &m.columns, inner, f32::from(t.spacing.unwrap_or(0)))
    };
    let (w, stack) = widths(&table(&wrapped, 0, false), 600.0);
    assert!(!stack && close(&w, &[50.0, 300.0, 250.0]), "{w:?}");
    let (w, _) = widths(&table(&one_each, 0, false), 600.0);
    assert!(close(&w, &[100.0, 200.0, 300.0]), "{w:?}");
    let t = table(&one_each, 0, true);
    let m = with_ui(|ui| measured(ui, &key(), &t));
    assert_eq!(m.whole.map(|f| f.max), Some(300.0));
    let (w, _) = widths(&t, 300.0);
    assert!(close(&w, &[50.0, 100.0, 150.0]), "{w:?}");
    // The row inside the table's own spacing (600 less 4 on each side).
    let (w, _) = widths(&table(&wrapped, 4, false), 592.0);
    assert!(close(&w, &[50.0, 289.33, 244.67]), "{w:?}");
    // Unmeasured, as before: shared equally.
    let (w, _) = cell_widths(&wrapped, &[], &[], 600.0, 0.0);
    assert!(close(&w, &[200.0, 200.0, 200.0]), "{w:?}");
}

/// Text measured in the fonts it's drawn in: a short column narrow, a long one wide, all of the
/// row used, none narrower than its widest word; side by side in 600, one under the other in a
/// narrow pane, as before. A table that sets no width is as wide as its words.
#[test]
fn text_columns_are_as_wide_as_their_words_ask() {
    let blocks = html_layout(
        r#"<table width="600" cellspacing="0" cellpadding="8"><tr><td>Qty</td><td>Our customers love this lamp: brass, hand-finished, with a linen shade, a dimmer and a two-year warranty on every part of it, shipped in two days.</td><td>Unit price</td></tr></table>"#,
    );
    let Some(Block::Table(t)) = blocks.first() else { panic!("a table: {blocks:?}") };
    let m = with_ui(|ui| measured(ui, &key(), t));
    let (w, stack) = cell_widths(&t.rows[0], &column_widths(&t.rows), &m.columns, 600.0, 0.0);
    assert!(!stack, "{w:?}");
    let [qty, text, price] = w.as_slice() else { panic!("{w:?}") };
    assert!(qty < price && price < text, "{w:?}");
    assert!((qty + text + price - 600.0).abs() < 0.5, "{w:?}");
    for (w, f) in w.iter().zip(&m.columns) {
        assert!(f.is_some_and(|f| *w + 0.5 >= f.min), "{w} {f:?}");
    }
    let (_, stack) = cell_widths(&t.rows[0], &column_widths(&t.rows), &m.columns, 300.0, 0.0);
    assert!(stack, "too narrow: one under the other");
    let blocks = html_layout(r#"<table cellspacing="0"><tr><td>Hello</td><td>there</td></tr></table><table width="600"><tr><td>a</td></tr></table>"#);
    let Some(Block::Table(t)) = blocks.first() else { panic!("a table: {blocks:?}") };
    assert!(t.fit);
    let m = with_ui(|ui| measured(ui, &key(), t));
    assert!(m.whole.is_some_and(|f| f.max > 20.0 && f.max < 150.0), "{m:?}");
}

/// A table of more words than are measured is left unmeasured, its room shared as before, and
/// quickly.
#[test]
fn a_huge_table_is_left_unmeasured() {
    let html = format!("<table width=\"600\"><tr><td>{}</td><td>b</td></tr></table>", "word ".repeat(30_000));
    let blocks = html_layout(&html);
    let Some(Block::Table(t)) = blocks.first() else { panic!("a table") };
    let started = std::time::Instant::now();
    let m = with_ui(|ui| measured(ui, &key(), t));
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    assert!(m.whole.is_none() && m.columns.first().is_some_and(Option::is_none), "{m:?}");
}

/// A table of values whose columns set their widths reads exactly as before: what's measured
/// changes nothing where the cells ask.
#[test]
fn a_table_of_values_with_widths_is_unchanged() {
    for html in [
        r#"<table width="600" cellspacing="0"><tr><td width="300">Item</td><td width="100">Qty</td><td width="200">Price</td></tr><tr><td>Brass lamp with a linen shade</td><td>2</td><td>$120.00</td></tr></table>"#,
        r#"<table width="600" cellspacing="2" cellpadding="4"><tr><td width="50%">Item</td><td width="20%">Qty</td><td width="30%">Price</td></tr><tr><td>Brass lamp</td><td>2</td><td>$120.00</td></tr></table>"#,
    ] {
        let blocks = html_layout(html);
        let Some(Block::Table(t)) = blocks.first() else { panic!("a table: {blocks:?}") };
        let m = with_ui(|ui| measured(ui, &key(), t));
        let columns = column_widths(&t.rows);
        for row in &t.rows {
            assert_eq!(cell_widths(row, &columns, &m.columns, 596.0, 2.0), cell_widths(row, &columns, &[], 596.0, 2.0), "{html}");
        }
    }
}

/// Round corners as asked, no more than half the box's shorter side (a pill), as CSS draws them.
#[test]
fn corners_are_no_rounder_than_half_the_shorter_side() {
    let rect = |w: f32, h: f32| egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(w, h));
    assert_eq!(corner(Some(4), rect(200.0, 44.0)), egui::CornerRadius::same(4));
    assert_eq!(corner(Some(200), rect(200.0, 44.0)), egui::CornerRadius::same(22));
    assert_eq!(corner(Some(30), rect(40.0, 100.0)), egui::CornerRadius::same(20));
    assert_eq!(corner(None, rect(40.0, 100.0)), egui::CornerRadius::ZERO);
    assert_eq!(corner(Some(9), rect(0.0, 0.0)), egui::CornerRadius::ZERO);
}

/// A row's cells for `widths`: a width each asks for (or none), one picture of 10 in each.
fn asking(widths: &[Option<Width>]) -> Vec<Cell> {
    widths.iter().map(|w| Cell { width: *w, ..pictures(&[10]) }).collect()
}

/// Found on a shipping notice (issue #47): a progress step of a bar, a dot of 18 pixels and a
/// bar, 50% each, came to more than the column has. The dot keeps its 18 and the shares split
/// the rest, side by side, as browsers do; it was squeezed, and the row stacked.
#[test]
fn shares_of_a_row_leave_the_others_what_they_ask_for() {
    let row = asking(&[Some(Width::Percent(50)), Some(Width::Px(18)), Some(Width::Percent(50))]);
    let (w, stack) = cell_widths(&row, &[], &[], 142.0, 0.0);
    assert!(!stack && close(&w, &[62.0, 18.0, 62.0]), "{w:?} {stack}");
    // Narrower than a column is read at: still a row, the bars holding nothing to read.
    let row = asking(&[Some(Width::Percent(50)), Some(Width::Px(18)), Some(Width::Percent(50))]);
    let (w, stack) = cell_widths(&row, &[], &[], 60.0, 0.0);
    assert!(!stack && close(&w, &[21.0, 18.0, 21.0]), "{w:?} {stack}");
}

/// A logo beside a cell of 100% (the same notice's header): the logo keeps its width, measured,
/// and the rest is the other cell's; unmeasured (its picture not here yet), a column's least.
#[test]
fn a_cell_of_the_whole_row_leaves_the_others_their_content() {
    let row = vec![pictures(&[290]), Cell { width: Some(Width::Percent(100)), ..pictures(&[10]) }];
    let fit = |min: f32| Some(Fit { min, max: min });
    let (w, stack) = cell_widths(&row, &[], &[fit(290.0), fit(10.0)], 600.0, 0.0);
    assert!(!stack && close(&w, &[290.0, 310.0]), "{w:?} {stack}");
    let (w, stack) = cell_widths(&row, &[], &[], 600.0, 0.0);
    assert!(!stack && w[0] < 200.0 && close(&[w[0] + w[1]], &[600.0]), "{w:?} {stack}");
}

/// A picture that sets no width is measured by its own size, once here; by its description
/// while it can't be shown; and measures aren't kept while one's still coming.
#[test]
fn pictures_without_a_width_are_measured_when_known() {
    let sp = mime::Span { image: Some("https://x.example/logo.png".into()), text: "Logo".into(), ..Default::default() };
    let t = mime::layout::Table {
        rows: vec![vec![Cell { blocks: vec![Block::Text(vec![vec![sp]])], span: 1, ..Default::default() }]],
        fit: true,
        ..Default::default()
    };
    let here = with_ui(|ui| super::measure::measured_with(ui, &key(), &t, true, &mut |_| PictureWidth::Is(290.0)));
    assert_eq!(here.whole.map(|f| f.min), Some(290.0));
    let coming = with_ui(|ui| super::measure::measured_with(ui, &key(), &t, false, &mut |_| PictureWidth::Coming));
    assert_eq!(coming.whole, None);
    let alt = with_ui(|ui| {
        let first = super::measure::measured_with(ui, &key(), &t, false, &mut |_| PictureWidth::Coming);
        assert_eq!(first.whole, None);
        super::measure::measured_with(ui, &key(), &t, false, &mut |_| PictureWidth::Alt)
    });
    assert!(alt.whole.is_some_and(|f| f.min > 10.0), "{alt:?}");
}

/// A table that doesn't say where it sits goes where the cell's content goes (the notice's
/// "Your Orders", right-aligned in its cell, sat at the left of it).
#[test]
fn a_table_sits_where_its_cell_puts_it() {
    use mime::layout::Align as A;
    assert_eq!(table_place(A::Left, A::Right), A::Right);
    assert_eq!(table_place(A::Left, A::Center), A::Center);
    assert_eq!(table_place(A::Center, A::Right), A::Center);
}

/// A picture held back keeps the size it says, so the message doesn't move when it's shown, and
/// an icon's description doesn't wrap a letter to a line; one that doesn't say is its description,
/// cut short.
#[test]
fn a_picture_held_back_keeps_its_size() {
    use crate::reading::body::{held_back_box, held_back_words};
    let size = |w: Option<Width>, h: Option<u32>| Some(mime::ImageSize { width: w, height: h, ..Default::default() });
    assert_eq!(held_back_box(size(Some(Width::Px(18)), Some(18)), 500.0), Some(egui::vec2(18.0, 18.0)));
    assert_eq!(held_back_box(size(Some(Width::Px(18)), None), 500.0), Some(egui::vec2(18.0, 18.0)), "an icon, square");
    assert_eq!(held_back_box(size(Some(Width::Px(600)), Some(300)), 300.0), Some(egui::vec2(300.0, 150.0)), "its shape kept in less room");
    assert_eq!(held_back_box(size(Some(Width::Px(300)), None), 500.0), None, "no height to keep");
    assert_eq!(held_back_box(size(Some(Width::Percent(100)), Some(20)), 500.0), None);
    assert_eq!(held_back_box(None, 500.0), None);
    assert_eq!(held_back_words(""), "[picture]");
    assert_eq!(held_back_words(" Logo "), "[Logo]");
    let long = held_back_words(&"é".repeat(300));
    assert!(long.ends_with("…]") && long.chars().count() == 51, "{long}");
}
