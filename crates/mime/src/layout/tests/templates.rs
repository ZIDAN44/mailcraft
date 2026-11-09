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

/// The size of the words `word` is in.
fn size_of(blocks: &[Block], word: &str) -> Option<u16> {
    let mut found = None;
    let mut b = blocks.to_vec();
    each_span(&mut b, &mut |s| {
        if s.text.contains(word) {
            found = s.size;
        }
    });
    found
}

/// Text at the size its sender set, as CSS works it out: inherited, relative to what's around
/// it, and a template's `font-size: 0` on a column passed over.
#[test]
fn text_keeps_the_size_its_sender_set() {
    let b = html_layout(&page(
        r#"<div style="font-size:12px">Twelve <b>inherited</b><div style="font-size:150%">Eighteen<span style="font-size:0.5em">Nine</span></div></div>
           <table width="100%"><tr><td style="font-size:0px"><div style="font-size:15px">Fifteen</div></td><td><font size="1">Ten</font></td></tr></table>
           <p>Unset</p><p style="font-size:11pt">Points</p><p style="font-size:1px">Tiny</p><p style="font-size:900px">Huge</p>"#,
    ));
    for (word, px) in [
        ("Twelve", Some(12)),
        ("inherited", Some(12)),
        ("Eighteen", Some(18)),
        ("Nine", Some(9)),
        ("Fifteen", Some(15)),
        ("Ten", Some(10)),
        ("Unset", None),
        ("Points", Some(15)),
        ("Tiny", Some(9)),
        ("Huge", Some(96)),
    ] {
        assert_eq!(size_of(&b, word), px, "{word}: {b:?}");
    }
    let shown = words(&b);
    assert!(shown.contains("Twelve inherited"), "the space between them kept: {shown}");
    // A space around the words of a size is no part of them: it doesn't start the line.
    let b = html_layout(&page("<p style=\"font-size:12px\">\n   Leading</p>"));
    assert!(words(&b).lines().any(|l| l == "Leading"), "{:?}", words(&b));
    assert!(!shown.chars().any(|c| ('\u{F0000}'..='\u{F0100}').contains(&c)), "{shown:?}");
}

/// Flowing text too (no layout), a long paragraph all of it, what's in a table of values left as
/// it was (a mark would count for a letter in its columns).
#[test]
fn flowing_text_keeps_its_size() {
    let raw = format!(
        "From: a@shop.example\r\nSubject: s\r\nContent-Type: text/html\r\n\r\n<p style=\"font-size:10px\">Small {}</p><p>Plain</p>",
        "word ".repeat(1000)
    );
    let p = crate::parse(raw.as_bytes()).unwrap();
    assert!(p.layout.is_empty());
    let sized: Vec<&crate::Span> = p.rich.iter().flatten().filter(|s| s.text.contains("word")).collect();
    assert!(sized.len() > 1 && sized.iter().all(|s| s.size == Some(10)), "every line of it: {} {:?}", sized.len(), sized.first());
    assert!(p.rich.iter().flatten().filter(|s| s.text.contains("Plain")).all(|s| s.size.is_none()), "{:?}", p.rich);
    let table = sized_html(r#"<p style="font-size:10px">Words</p><table><tr><td style="font-size:10px">Cell</td></tr></table>"#).unwrap_or_default();
    assert!(table.contains('\u{F000A}') && table.contains(">Cell<"), "{table:?}");
    assert!(!p.text.chars().any(|c| ('\u{F0000}'..='\u{F0100}').contains(&c)), "the plain text has no marks");
}

/// Found on the shipping notice: a progress step's dot is a round box of colour around a white
/// check mark, and only a box of a set width kept its colour, so the mark was white on white. A
/// box that draws something keeps it, whatever its width; one that draws nothing is no box.
#[test]
fn a_box_of_colour_keeps_it() {
    let b = html_layout(&page(
        r#"<table width="18"><tr><td width="18"><div style="border-radius:18px;background:#1196AB"><img src="https://x.example/check.png" width="18" height="18" alt="Done"></div></td></tr></table>
           <div style="background:#f0f2f2;padding:8px;border-left:2px solid #336699;border-radius:6px">A note</div>
           <div style="font-family:Arial">Plain</div>"#,
    ));
    let Block::Table(page) = &b[0] else { panic!("{b:?}") };
    let inside = &page.rows[0][0].blocks;
    let Block::Table(step) = &inside[0] else { panic!("{inside:?}") };
    let Block::Table(dot) = &step.rows[0][0].blocks[0] else { panic!("{step:?}") };
    assert_eq!((dot.rows[0][0].background, dot.rows[0][0].radius), (Some([0x11, 0x96, 0xAB]), Some(18)), "{dot:?}");
    let Block::Table(note) = &inside[1] else { panic!("{inside:?}") };
    let cell = &note.rows[0][0];
    assert_eq!((cell.background, cell.padding, cell.radius), (Some([0xf0, 0xf2, 0xf2]), Some([8; 4]), Some(6)));
    assert_eq!(cell.borders[3], Some(Line { width: 2, color: [0x33, 0x66, 0x99] }));
    assert!(matches!(&inside[2], Block::Text(_)), "{:?}", inside[2]);
    // The colour it's on already draws nothing.
    let b = html_layout(
        r##"<table width="600" bgcolor="#ffffff"><tr><td bgcolor="#ffffff"><div style="background:#ffffff">Same</div></td></tr></table>"##,
    );
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert!(matches!(&t.rows[0][0].blocks[0], Block::Text(_)), "{b:?}");
}
