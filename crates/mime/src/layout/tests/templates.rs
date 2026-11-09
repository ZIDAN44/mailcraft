//! Mail built by a template compiler (MJML and the like, issue #47): a table in a box in a table
//! for every part of it, read as Outlook reads it.

use super::*;

fn all_text(blocks: &[Block], out: &mut String) {
    for b in blocks {
        match b {
            Block::Text(lines) => {
                for l in lines {
                    out.extend(l.iter().map(|s| s.text.as_str()));
                    out.push('\n');
                }
            }
            Block::Table(t) => t.rows.iter().flatten().for_each(|c| all_text(&c.blocks, out)),
        }
    }
}

fn words(blocks: &[Block]) -> String {
    let mut out = String::new();
    all_text(blocks, &mut out);
    out
}

/// How many tables deep `blocks` go.
fn nesting(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|b| match b {
            Block::Text(_) => 0,
            Block::Table(t) => 1 + t.rows.iter().flatten().map(|c| nesting(&c.blocks)).max().unwrap_or(0),
        })
        .max()
        .unwrap_or(0)
}

/// `inner` in `n` tables that draw nothing, each as wide as its room.
fn wrapped(n: usize, inner: &str) -> String {
    let open = r#"<table border="0" cellpadding="0" cellspacing="0" role="presentation" width="100%"><tbody><tr><td align="left" style="padding:0">"#;
    format!("{}{inner}{}", open.repeat(n), "</td></tr></tbody></table>".repeat(n))
}

fn page(inner: &str) -> String {
    format!(r#"<table width="600" align="center" cellpadding="0" cellspacing="0"><tr><td>{inner}</td></tr></table>"#)
}

const TOTAL: &str =
    r#"<table width="100%" cellpadding="0" cellspacing="0" role="presentation"><tr><td>Total</td><td align="right">$15.98</td></tr></table>"#;

/// Found on a shipping notice (issue #47): tables nested past what's laid out came out as text
/// tables, of box-drawing lines in a monospace font. Tables that only wrap what they hold don't
/// count, so what they hold is laid out however many there are.
#[test]
fn tables_that_only_wrap_are_no_depth() {
    let b = html_layout(&page(&wrapped(20, TOTAL)));
    assert_eq!(nesting(&b), 2, "{b:?}");
    let shown = words(&b);
    assert!(shown.contains("Total") && shown.contains("$15.98") && !shown.contains(['─', '│']), "{shown}");
    let Block::Table(outer) = &b[0] else { panic!("{b:?}") };
    let Block::Table(row) = &outer.rows[0][0].blocks[0] else { panic!("{b:?}") };
    assert_eq!(row.rows[0].len(), 2, "side by side: {row:?}");
    assert_eq!(row.rows[0][1].align, Align::Right);
}

/// What draws something stays: a table with a colour of its own, room, a line, or narrower than
/// its room isn't a wrapper.
#[test]
fn a_table_that_draws_something_stays() {
    for (what, open) in [
        ("colour", r##"<table width="100%" bgcolor="#336699" cellpadding="0" cellspacing="0"><tr><td>"##),
        ("room", r#"<table width="100%" cellpadding="0" cellspacing="0"><tr><td style="padding:8px">"#),
        ("line", r#"<table width="100%" cellpadding="0" cellspacing="0"><tr><td style="border:1px solid #ddd">"#),
        ("narrower", r#"<table width="300" cellpadding="0" cellspacing="0"><tr><td>"#),
        ("height", r#"<table width="100%" cellpadding="0" cellspacing="0"><tr><td height="90">"#),
        ("centred", r#"<table width="100%" cellpadding="0" cellspacing="0"><tr><td align="center">"#),
    ] {
        let b = html_layout(&page(&format!("{open}{TOTAL}</td></tr></table>")));
        assert_eq!(nesting(&b), 3, "{what}: {b:?}");
    }
    // A colour it's on already adds nothing.
    let b = html_layout(&format!(
        r##"<table width="600" bgcolor="#ffffff" cellpadding="0" cellspacing="0"><tr><td bgcolor="#ffffff"><table width="100%" bgcolor="#ffffff" cellpadding="0" cellspacing="0"><tr><td>{TOTAL}</td></tr></table></td></tr></table>"##
    ));
    assert_eq!(nesting(&b), 2, "{b:?}");
}

/// Past what's laid out, however a table is nested: its cells one after another, never a text
/// table (of those lines, in that font, in a layout).
#[test]
fn tables_past_the_depth_are_read_as_blocks() {
    let open = r#"<table width="100%" cellpadding="0" cellspacing="0"><tr><td style="padding:2px">"#;
    let html = format!("{}{TOTAL}{}", open.repeat(16), "</td></tr></table>".repeat(16));
    let b = html_layout(&page(&html));
    let shown = words(&b);
    assert!(shown.contains("Total") && shown.contains("$15.98"), "{shown}");
    assert!(!shown.contains(['─', '│', '┬']), "{shown}");
    let mut code = false;
    let mut blocks = b;
    each_span(&mut blocks, &mut |s| code |= s.code);
    assert!(!code, "no monospace");
}

/// Hostile: wrappers as deep as a message can nest, on a thread with the smallest stack a window
/// has, reading what a message does: no overflow, and it ends.
#[test]
fn wrappers_as_deep_as_can_be_read_are_safe() {
    let html = page(&wrapped(400, "x"));
    let done = std::thread::Builder::new().stack_size(1 << 20).spawn(move || html_layout(&html).len()).unwrap().join();
    assert!(done.is_ok());
}

/// A divider as MJML writes one: a line for the others and a table for Outlook, one drawn.
#[test]
fn a_divider_is_a_line_drawn_once() {
    let html = page(
        r#"<p style="border-top:solid 2px #336699;font-size:1px;margin:0 auto;width:100%"></p><table style="border-top:solid 2px #336699;font-size:1px;margin:0 auto;width:550px" role="presentation" width="550px"><tr><td style="height:0;line-height:0">&nbsp;</td></tr></table><p>After</p>"#,
    );
    let b = html_layout(&html);
    let Block::Table(outer) = &b[0] else { panic!("{b:?}") };
    let inside = &outer.rows[0][0].blocks;
    let lines: Vec<&Table> = inside.iter().filter_map(|x| if let Block::Table(t) = x { Some(t) } else { None }).collect();
    assert_eq!(lines.len(), 1, "{inside:?}");
    assert_eq!(lines[0].rows[0][0].borders[0], Some(Line { width: 2, color: [0x33, 0x66, 0x99] }));
}

/// A spacer as MJML writes one, and a bar of colour: as tall as they say.
#[test]
fn a_spacer_keeps_its_height() {
    let b = html_layout(&page(r#"<p>Before</p><div style="height:30px;line-height:30px">&#8202;</div><p>After</p>"#));
    let Block::Table(outer) = &b[0] else { panic!("{b:?}") };
    let spacers: Vec<Option<u16>> =
        outer.rows[0][0].blocks.iter().filter_map(|x| if let Block::Table(t) = x { Some(t.rows[0][0].height) } else { None }).collect();
    assert_eq!(spacers, [Some(30)]);
}

/// A box with nothing in it but a picture behind it, or a colour, is no spacer or divider.
#[test]
fn a_blank_box_with_a_picture_behind_it_stays_one() {
    let b = html_layout(r#"<div style="width:300px;height:90px;background-image:url(https://x.example/a.png)"></div><p>After</p>"#);
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert_eq!(t.rows[0][0].background_image.as_deref(), Some("https://x.example/a.png"), "{b:?}");
    let b = html_layout(
        r##"<table width="600"><tr><td><div style="height:4px;background:#336699;border-top:1px solid #000000"></div></td></tr></table>"##,
    );
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    let Block::Table(bar) = &t.rows[0][0].blocks[0] else { panic!("{b:?}") };
    assert_eq!((bar.rows[0][0].background, bar.rows[0][0].height), (Some([0x33, 0x66, 0x99]), Some(4)), "{b:?}");
}

/// Lines: a cell's own, its row's, a table's, the attribute's; `none`, and none said, draw none.
#[test]
fn lines_around_cells_and_tables_are_kept() {
    let b = html_layout(
        r#"<table width="600"><tr style="border-bottom:1px solid #eeeeee"><td style="border-left:3px dashed red">A</td><td style="border:none">B</td></tr></table>"#,
    );
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    let (a, c) = (&t.rows[0][0], &t.rows[0][1]);
    assert_eq!(a.borders[3], Some(Line { width: 3, color: [255, 0, 0] }));
    assert_eq!(a.borders[2], Some(Line { width: 1, color: [0xee, 0xee, 0xee] }), "the row's");
    assert_eq!(c.borders[0], None);
    let b = html_layout(r#"<table border="1" width="600"><tr><td>A</td><td>B</td></tr></table>"#);
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert!(t.border.is_some() && t.rows[0][0].borders.iter().all(Option::is_some), "{t:?}");
    let b = html_layout(r#"<table border="0" width="600" style="border-top:0"><tr><td style="border:1px #ccc">A</td><td>B</td></tr></table>"#);
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert!(t.border.is_none() && t.rows[0][0].borders.iter().all(Option::is_none), "no style, no line: {t:?}");
}

/// A heading cell: bold and in the middle unless it says otherwise; a row's alignment its cells'.
#[test]
fn heading_cells_and_rows_read_as_a_browser_reads_them() {
    let b = html_layout(
        r#"<table width="600"><tr><th>A</th><th style="text-align:left;font-weight:normal">B</th></tr><tr align="right"><td>C</td><td>D</td></tr></table>"#,
    );
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    let bold = |c: &Cell| matches!(&c.blocks[0], Block::Text(l) if l[0][0].bold);
    assert_eq!((t.rows[0][0].align, bold(&t.rows[0][0])), (Align::Center, true));
    assert_eq!((t.rows[0][1].align, bold(&t.rows[0][1])), (Align::Left, false));
    assert_eq!(t.rows[1][0].align, Align::Right);
}

/// Outlook's copy of a picture placed behind a box (a hero's) isn't drawn again above it; a
/// tracking pixel there still counts.
#[test]
fn a_picture_behind_a_box_is_not_drawn_again() {
    let read = crate::as_outlook_reads(
        r#"<!--[if mso]><v:image style="height:300px;position:absolute;top:0;width:600px;z-index:-3" src="https://x.example/hero.jpg"/><![endif]-->"#,
    );
    assert!(!read.contains("<img"), "{read}");
    let read = crate::as_outlook_reads(
        r#"<!--[if mso]><v:image style="position:absolute;width:1px;height:1px" src="https://x.example/o.gif"/><![endif]-->"#,
    );
    assert!(read.contains("<img"), "{read}");
    let read = crate::as_outlook_reads(r#"<!--[if mso]><v:image style="width:100px;height:40px" src="https://x.example/a.png"/><![endif]-->"#);
    assert!(read.contains("<img"), "{read}");
}
