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
    // Timed only in a release build: a debug build on a busy machine measures the machine.
    if !cfg!(debug_assertions) {
        assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    }
    assert!(m.whole.is_none() && m.columns.first().is_some_and(Option::is_none), "{m:?}");
}

/// A table of values whose columns set their widths has them, shared in proportion when there's
/// a little less room: what's measured changes nothing where the cells ask, but that a column a
/// little narrower than it asks stays beside the others while it holds its widest word (#152:
/// this one, 600 wide, went one cell to a line in 596).
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
            let (measured, stack) = cell_widths(row, &columns, &m.columns, 596.0, 2.0);
            assert_eq!(measured, cell_widths(row, &columns, &[], 596.0, 2.0).0, "{html}");
            assert!(!stack, "{html}");
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
    let here = with_ui(|ui| super::measure::measured_with(ui, &key(), None, &t, true, &mut |_| PictureWidth::Is(290.0)));
    assert_eq!(here.whole.map(|f| f.min), Some(290.0));
    let coming = with_ui(|ui| super::measure::measured_with(ui, &key(), None, &t, false, &mut |_| PictureWidth::Coming));
    assert_eq!(coming.whole, None);
    let alt = with_ui(|ui| {
        let first = super::measure::measured_with(ui, &key(), None, &t, false, &mut |_| PictureWidth::Coming);
        assert_eq!(first.whole, None);
        super::measure::measured_with(ui, &key(), None, &t, false, &mut |_| PictureWidth::Alt)
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

/// Issue #90: a signature in three columns (a logo, a name and numbers, an address after a
/// dividing line), the first two as wide as they say and the address taking the rest. In a
/// reading pane of the usual width the address has less than a column's usual room, and its
/// lines are short: it wraps a little and stays beside the others, as in a browser (Chrome keeps
/// the three down to the widest word). The row went one under the other, its line gone. Too
/// narrow for the address's widest word, it still does.
#[test]
fn a_signature_in_columns_stays_in_columns_in_a_pane_of_the_usual_width() {
    let html = r#"<table width="720" style="width:720px;max-width:100%" cellspacing="0" cellpadding="0"><tr><td style="padding:8px 0"><table width="720" cellspacing="0" cellpadding="0" style="width:720px"><tr>
<td width="180" valign="middle" style="width:180px;padding:0"><img src="cid:logo@x" width="180" height="60" style="display:block;width:180px;height:60px"></td>
<td width="220" valign="middle" style="width:220px;padding:0 16px 0 12px"><span style="font-size:16px;font-weight:700;white-space:nowrap">Ms. Riley Moreno</span><br><span style="font-size:12px"><a href="mailto:riley@studio.example" style="white-space:nowrap">riley@studio.example</a><br><a href="https://studio.example" style="white-space:nowrap">www.studio.example</a><br><a href="tel:+15550100" style="white-space:nowrap">+1 555 0100 200</a></span></td>
<td valign="middle" style="padding:0 0 0 16px;border-left:1px solid #E30613;font-size:11px;line-height:16px">12 Harbour Road<br>General Quay Street<br>Sail Loft Lane No: 35<br>The Old Sail Loft Centre, Floor: 6, Unit: 18<br>Portside / Harbourtown</td></tr></table></td></tr></table>"#;
    let blocks = html_layout(html);
    let Some(Block::Table(outer)) = blocks.first() else { panic!("a table: {blocks:?}") };
    let inner =
        outer.rows.first().and_then(|r| r.first()).and_then(|c| c.blocks.iter().find_map(|b| if let Block::Table(t) = b { Some(t) } else { None }));
    let inner = inner.expect("the columns' table");
    let m = with_ui(|ui| measured(ui, &key(), inner));
    let address = m.columns.get(2).copied().flatten().expect("the address, measured");
    assert!(address.min < 120.0 && address.max < PARAGRAPH, "short lines: {address:?}");
    let widths = |room: f32| cell_widths(&inner.rows[0], &column_widths(&inner.rows), &m.columns, room, 0.0);
    for room in [720.0, 560.0, 530.0, 500.0] {
        let (w, stack) = widths(room);
        assert!(!stack && close(&w, &[180.0, 220.0, room - 400.0]), "{room}: {w:?} {stack}");
    }
    // Too narrow for the address's widest word beside the others as wide as they ask: the name's
    // column, 220 wide, gives up what its name doesn't need, as in a browser.
    let (w, stack) = widths(400.0 + address.min - 20.0);
    assert!(!stack && w.iter().zip(&m.columns).all(|(w, f)| f.is_some_and(|f| *w + 0.5 >= f.min)), "{w:?}");
    let least: f32 = m.columns.iter().flatten().map(|f| f.min).sum();
    assert!(widths(least - 20.0).1, "no room for their widest words: one under the other");
}

/// Tables as Outlook writes them (Word's HTML: `MsoTableGrid`, widths in points beside their
/// pixels, `<o:p>`) and as Excel pastes them, in a reading pane narrower than they ask: their
/// columns stay side by side, each narrower but no narrower than its widest word. They went one
/// cell to a line in any pane narrower than they are (#152). Too narrow even for those words,
/// they're drawn that wide, scrolled sideways, not one cell to a line.
#[test]
fn tables_from_outlook_and_excel_keep_their_columns_in_a_narrow_pane() {
    let cell = |w: u32, pt: &str, text: &str| {
        format!(
            "<td width={w} valign=top style='width:{pt}pt;border:solid windowtext 1.0pt;mso-border-alt:solid windowtext .5pt;padding:0in 5.4pt 0in 5.4pt'><p class=MsoNormal>{text}<o:p></o:p></p></td>"
        )
    };
    let row =
        |w: u32, pt: &str, texts: &[&str]| format!("<tr style='mso-yfti-irow:0'>{}</tr>", texts.iter().map(|t| cell(w, pt, t)).collect::<String>());
    let word = format!(
        "<div class=WordSection1><table class=MsoTableGrid border=1 cellspacing=0 cellpadding=0 style='border-collapse:collapse;border:none;mso-yfti-tbllook:1184'>{}{}</table><p class=MsoNormal><o:p>&nbsp;</o:p></p></div>",
        row(124, "93.5", &["Name", "Department", "Start date", "Hours", "Status"]),
        row(124, "93.5", &["Riley Moreno", "Accounts", "2026-03-01", "37.5", "Open"])
    );
    let excel = format!(
        "<table class=MsoNormalTable border=0 cellspacing=0 cellpadding=0 width=448 style='width:336.0pt;border-collapse:collapse'>{}{}</table>",
        row(64, "48.0", &["Region", "Q1", "Q2", "Q3", "Q4", "Total", "Change"]),
        row(64, "48.0", &["North", "1,204", "1,310", "998", "1,450", "4,962", "+4.1%"])
    );
    for (html, rooms) in [(word, [600.0, 500.0, 420.0]), (excel, [440.0, 420.0, 400.0])] {
        let blocks = html_layout(&html);
        let t = all_tables(&blocks).first().copied().expect("a table");
        let m = with_ui(|ui| measured(ui, &key(), t));
        let columns = column_widths(&t.rows);
        for room in rooms {
            for row in &t.rows {
                let (w, stack) = cell_widths(row, &columns, &m.columns, room, 0.0);
                assert!(!stack, "{room}: {w:?} {html}");
                assert!((w.iter().sum::<f32>() - room).abs() < 0.5, "all the room: {room} {w:?}");
                assert!(w.iter().zip(&m.columns).all(|(w, f)| f.is_some_and(|f| *w + 0.5 >= f.min)), "{room}: {w:?} {:?}", m.columns);
            }
            assert_eq!(sideways(t, m.whole, room), None, "it fits in {room}");
        }
        // Narrower than its widest words: as wide as those, scrolled, and side by side there.
        let least = m.whole.map(|f| f.min).expect("measured");
        assert_eq!(sideways(t, m.whole, least - 30.0), Some(least));
        assert!(t.rows.iter().all(|row| !cell_widths(row, &columns, &m.columns, least, 0.0).1));
    }
    // A page's columns, a row of one cell over them, aren't a grid: they go one under the other.
    let page =
        html_layout(r#"<table width="600"><tr><td colspan="2">Header</td></tr><tr><td width="300">Left</td><td width="300">Right</td></tr></table>"#);
    let t = all_tables(&page).first().copied().expect("a table");
    let m = with_ui(|ui| measured(ui, &key(), t));
    assert_eq!(sideways(t, m.whole, 10.0), None);
}

/// Every table in `blocks`, those inside others too (in their cells and the boxes over them).
fn all_tables(blocks: &[Block]) -> Vec<&mime::layout::Table> {
    let mut out = Vec::new();
    for b in blocks {
        if let Block::Table(t) = b {
            out.push(t);
            for c in t.rows.iter().flatten() {
                out.extend(all_tables(&c.blocks));
                for o in &c.overlays {
                    out.extend(all_tables(&o.blocks));
                }
            }
        }
    }
    out
}

/// What each table of `blocks` that's in no other holds, as it's made when one is drawn.
fn outermost_held(ui: &Ui, blocks: &[Block]) -> Vec<Held> {
    blocks.iter().filter_map(|b| if let Block::Table(t) = b { Some(Held::of(ui, t)) } else { None }).collect()
}

/// Tables measured on this thread so far.
fn measured_count() -> usize {
    super::measure::MEASURED.with(std::cell::Cell::get)
}

/// Tables inside tables `levels` deep (more than the layout makes of HTML), each with a few
/// words beside the next.
fn deep_tables(levels: usize) -> Vec<Block> {
    let words = |text: String| Block::Text(vec![vec![mime::Span { text, ..Default::default() }]]);
    let mut inner = vec![words("the bottom".into())];
    for i in (0..levels).rev() {
        let cells = vec![
            Cell { blocks: vec![words(format!("level {i} {}", "a word ".repeat(i % 3)))], span: 1, ..Default::default() },
            Cell { blocks: inner, span: 1, ..Default::default() },
        ];
        inner = vec![Block::Table(mime::layout::Table { rows: vec![cells], ..Default::default() })];
    }
    inner
}

/// Mail laid out in tables inside tables: some too deep to measure whole, some of too many
/// words, one after a table that's a share of its room, with pictures that set their width and
/// some that don't.
fn nested_samples() -> Vec<Vec<Block>> {
    let words = "Our fall collection is here with coats and boots ".repeat(6);
    let huge = format!(
        "<table><tr><td><table><tr><td>{words}</td></tr></table></td><td>{}</td></tr><tr><td><table><tr><td>{}</td></tr></table></td></tr></table>",
        "word ".repeat(19_990),
        "more words ".repeat(20)
    );
    let mut shop = String::from("<table width=\"100%\"><tr><td>");
    for i in 0..6 {
        shop.push_str(&format!(
            "<table><tr><td><table cellpadding=\"4\"><tr><td>{words}</td><td><img src=\"https://cdn.shop.example/{i}.png\" alt=\"Coat {i}\"></td></tr></table></td><td><img src=\"cid:p@x\" width=\"120\"> {words}</td></tr></table>"
        ));
    }
    shop.push_str("</td></tr></table>");
    let percent = format!(
        "<table><tr><td>Fall</td><td><table><tr><td>{words}</td><td>a</td></tr></table><table width=\"50%\"><tr><td>{words}</td><td>b</td></tr></table><table><tr><td>{words}</td><td>c</td></tr></table></td></tr></table>"
    );
    vec![deep_tables(22), html_layout(&huge), html_layout(&shop), html_layout(&percent), deep_tables(10)]
}

/// Each table inside another is measured with it, as it would be measured alone (unless a limit
/// cut it short there: then it's measured alone when drawn), and isn't measured again when
/// drawn. Found in audit (#479): each was measured once for itself and again for every table
/// it's inside.
#[test]
fn tables_inside_are_measured_with_the_one_they_are_in() {
    for (n, blocks) in nested_samples().into_iter().enumerate() {
        let tables = all_tables(&blocks);
        assert!(tables.len() > 2, "{n}: {} tables", tables.len());
        for picture in [PictureWidth::Is(90.0), PictureWidth::Alt] {
            let answer = |p: &PictureWidth| match p {
                PictureWidth::Is(w) => PictureWidth::Is(*w),
                PictureWidth::Alt => PictureWidth::Alt,
                PictureWidth::Coming => PictureWidth::Coming,
            };
            // Each alone, in a frame of its own.
            let alone: Vec<String> =
                tables.iter().map(|t| with_ui(|ui| format!("{:?}", measured_with(ui, &key(), None, t, true, &mut |_| answer(&picture))))).collect();
            // As drawn: the outermost first, then each inside it.
            let (drawn, measures) = with_ui(|ui| {
                let held = outermost_held(ui, &blocks);
                let before = measured_count();
                let drawn: Vec<String> = tables
                    .iter()
                    .map(|t| format!("{:?}", measured_with(ui, &key(), held.iter().find(|h| h.has(t)), t, true, &mut |_| answer(&picture))))
                    .collect();
                (drawn, measured_count() - before)
            });
            assert_eq!(drawn, alone, "measured as alone");
            let outermost = blocks.iter().filter(|b| matches!(b, Block::Table(_))).count();
            match n {
                // Cut short by the nesting limit, past the words measured, or after a share of
                // the room: what's inside is measured alone when drawn, the rest with it.
                0 | 1 | 3 => assert!(measures > outermost && measures < tables.len(), "{n}: {measures} of {} tables measured", tables.len()),
                _ => assert_eq!(measures, outermost, "only the outermost measured, of {}", tables.len()),
            }
        }
    }
}

/// A table with a picture still coming is measured once and kept until it comes, the tables
/// inside it too; then measured again, as it would be in a frame of its own. Found in audit
/// (#479): it was measured again every frame while the picture came.
#[test]
fn a_table_is_measured_again_only_once_a_picture_comes() {
    let blocks = nested_samples().swap_remove(2);
    let tables = all_tables(&blocks);
    let web = |sp: &mime::Span| sp.image.as_deref().is_some_and(|s| s.starts_with("https:"));
    let came = with_ui(|ui| {
        let held = outermost_held(ui, &blocks);
        let draw = |come: &dyn Fn(&mime::Span) -> bool| -> (Vec<String>, usize) {
            let before = measured_count();
            let m = tables
                .iter()
                .map(|t| {
                    format!(
                        "{:?}",
                        measured_with(ui, &key(), held.iter().find(|h| h.has(t)), t, true, &mut |sp| if web(sp) && !come(sp) {
                            PictureWidth::Coming
                        } else {
                            PictureWidth::Is(90.0)
                        })
                    )
                })
                .collect();
            (m, measured_count() - before)
        };
        let (coming, first) = draw(&|_| false);
        assert!(coming.iter().any(|m| m.contains("None")), "unmeasured while coming");
        assert!(first > 0 && first < tables.len(), "{first} of {}", tables.len());
        for _ in 0..5 {
            assert_eq!(draw(&|_| false), (coming.clone(), 0), "kept while they're coming");
        }
        // One comes: what it's in is measured again (as little as that), and kept.
        let one = |sp: &mime::Span| sp.image.as_deref() == Some("https://cdn.shop.example/3.png");
        let (_, again) = draw(&one);
        assert!(again > 0 && again <= first, "measured again with it: {again}");
        assert_eq!(draw(&one).1, 0, "and kept");
        let (all, last) = draw(&|_| true);
        assert!(last > 0);
        assert_eq!(draw(&|_| true).1, 0);
        all
    });
    let alone: Vec<String> =
        tables.iter().map(|t| with_ui(|ui| format!("{:?}", measured_with(ui, &key(), None, t, true, &mut |_| PictureWidth::Is(90.0))))).collect();
    assert_eq!(came, alone, "as measured alone once they're all here");
    assert!(!came.iter().any(|m| m.contains("None")), "{came:?}");
}

/// A table laid out again where another of the same shape was (a signature changed, a message
/// read again) has its own measures; the same table, wherever it is, the same. Found in audit
/// (#480): the measures were kept by the table's place in memory and its shape.
#[test]
fn a_table_where_another_was_has_its_own_measures() {
    let table = |words: &str| {
        let blocks = html_layout(&format!("<table><tr><td>{words}</td><td>Riley Moreno</td></tr></table>"));
        let Some(Block::Table(t)) = blocks.into_iter().next() else { panic!("a table") };
        t
    };
    with_ui(|ui| {
        let mut t = table("Short");
        let short = measured(ui, &key(), &t);
        // Another table, in the same place (the same shape).
        t = table("A much longer line of words than the first one");
        let long = measured(ui, &key(), &t);
        assert_eq!(format!("{long:?}"), format!("{:?}", with_ui(|ui| measured(ui, &key(), &t))), "as measured alone");
        assert!(long.columns[0].unwrap().max > short.columns[0].unwrap().max + 100.0, "its own: {short:?} {long:?}");
        // The first again, somewhere else: as it was.
        let again = Box::new(table("Short"));
        assert_eq!(format!("{:?}", measured(ui, &key(), &again)), format!("{short:?}"));
    });
}

/// A frame hashes what a table holds once, however deep the tables in it are, and their measures
/// are still kept from frame to frame. Every table hashed all it holds on every frame: what's in
/// a table fourteen tables down, fifteen times.
#[test]
fn a_frame_hashes_what_a_table_holds_once() {
    let hashed = || super::measure::HASHED.with(std::cell::Cell::get);
    let blocks = deep_tables(14);
    let Some(Block::Table(outer)) = blocks.first() else { panic!("a table") };
    let session = Session::open(None, mailcraft_engine::model::dates::Clock::new(0, 0));
    let (key, parsed) = (key(), mime::Parsed::default());
    let ctx = egui::Context::default();
    let mut st = UiState::default();
    // What a frame hashes, how many tables it measures, and what hashing the outer table takes.
    let mut frame = || {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 700.0))), ..Default::default() };
        let (mut frame, mut once) = ((0, 0), 0);
        let mut out = ctx.run_ui(input, |ui| {
            let (mut shown, mut acts) = (Vec::new(), Vec::new());
            let mut body = Body { s: &session, key: &key, p: &parsed, allowed: false, shown: &mut shown, acts: &mut acts, cells: None, held: None };
            let before = (hashed(), measured_count());
            body.blocks(&mut st, ui, &blocks, 800.0, [255; 3], mime::layout::Align::Left, 0);
            frame = (hashed() - before.0, measured_count() - before.1);
            let before = hashed();
            let _ = Held::of(ui, outer);
            once = hashed() - before;
        });
        out.textures_delta.clear();
        (frame, once)
    };
    let ((_, measured), once) = frame();
    assert_eq!(measured, 1, "those inside measured with the outer one, and found when drawn");
    assert!(once > 100, "{once}");
    for _ in 0..3 {
        assert_eq!(frame(), ((once, 0), once), "hashed once, and nothing measured again");
    }
}

/// Fonts installed again (another language's, one that came for Chinese) measure a line's width
/// and a table's columns again: both were kept as measured in the fonts before, a centred line
/// off-centre and columns the wrong width until MailCraft was started again.
#[test]
fn what_is_kept_is_measured_again_in_other_fonts() {
    let line = vec![mime::Span { text: "Read the story of the fall collection".into(), ..Default::default() }];
    let blocks = html_layout("<table><tr><td>Read the story of the fall collection</td><td>Riley Moreno</td></tr></table>");
    let Some(Block::Table(t)) = blocks.first() else { panic!("a table") };
    // The words' font changed for the monospace one, as wide again.
    let mut other = egui::FontDefinitions::default();
    if let Some(family) = other.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(0, "Hack".into());
    }
    // The line as kept, as laid out now, and the table's measures.
    let look = |ctx: &egui::Context| {
        let mut got = None;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let now: f32 = line.iter().map(|sp| ui.fonts_mut(|f| f.layout_no_wrap(sp.text.clone(), span_font(sp), Color32::WHITE).size().x)).sum();
            got = Some((line_width(ui, &line), now, format!("{:?}", measured(ui, &key(), t))));
        });
        out.textures_delta.clear();
        got.expect("a frame")
    };
    let ctx = egui::Context::default();
    let first = look(&ctx);
    assert_eq!((first.0, look(&ctx)), (first.1, first.clone()), "kept in the same fonts");
    ctx.set_fonts(other.clone());
    let (kept, now, columns) = look(&ctx);
    assert!((now - first.1).abs() > 20.0, "another width in the other font: {now} {}", first.1);
    assert_eq!(kept, now, "the line measured again");
    // As in a window that only ever had those fonts.
    let fresh = egui::Context::default();
    fresh.set_fonts(other);
    assert_eq!(columns, look(&fresh).2, "the table measured again");
    assert_ne!(columns, first.2);
}
