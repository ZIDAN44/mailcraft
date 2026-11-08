use super::*;

fn text(b: &Block) -> String {
    match b {
        Block::Text(r) => r.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n"),
        Block::Table(_) => String::new(),
    }
}

#[test]
fn a_newsletter_keeps_its_columns() {
    let html = r##"<html><head><style>.hide{display:none}</style></head><body>
            <div style="display:none">Preheader text</div>
            <center><table width="600" align="center" bgcolor="#f4f4f4"><tr><td align="center"><h1>Studio news</h1></td></tr>
            <tr><td><table width="100%"><tr>
                <td width="50%" style="background-color: #ffeedd"><p>Left <b>column</b></p><img src="cid:a@x" alt="poster"></td>
                <td width="50%"><p>Right column</p><a href="https://example.com/x">Read more</a></td>
            </tr></table></td></tr></table></center>
            <p class="hide">Hidden by a rule</p><p>Footer</p></body></html>"##;
    let b = html_layout(html);
    assert!(!b.is_empty());
    // Centre wrapper → table → second row's cell → the two columns.
    let Block::Table(center) = &b[0] else { panic!("{b:?}") };
    let Block::Table(outer) = &center.rows[0][0].blocks[0] else { panic!() };
    assert_eq!(outer.width, Some(Width::Px(600)));
    assert_eq!(outer.align, Align::Center);
    assert_eq!(outer.background, Some([0xf4, 0xf4, 0xf4]));
    assert_eq!(outer.rows[0][0].align, Align::Center);
    assert!(text(&outer.rows[0][0].blocks[0]).contains("Studio news"));
    let Block::Table(cols) = &outer.rows[1][0].blocks[0] else { panic!() };
    let row = &cols.rows[0];
    assert_eq!(row.len(), 2);
    assert_eq!(row[0].width, Some(Width::Percent(50)));
    assert_eq!(row[0].background, Some([0xff, 0xee, 0xdd]));
    let Block::Text(left) = &row[0].blocks[0] else { panic!() };
    assert!(left.iter().flatten().any(|s| s.bold && s.text.contains("column")));
    assert!(left.iter().flatten().any(|s| s.image.as_deref() == Some("cid:a@x")));
    let Block::Text(right) = &row[1].blocks[0] else { panic!() };
    assert!(right.iter().flatten().any(|s| s.link.as_deref() == Some("https://example.com/x")));
    // Hidden parts stay hidden; text after the table follows it.
    let all = format!("{b:?}");
    assert!(!all.contains("Preheader") && !all.contains("Hidden by a rule"), "{all}");
    assert!(text(b.last().unwrap()).contains("Footer"));
}

#[test]
fn css_columns_are_columns_too() {
    // Flexbox, a grid and inline-block boxes, no tables.
    let flex = html_layout(
        r#"<div style="display:flex"><div style="width:30%"><p>Left</p></div><div style="background-color:#eee"><p>Right</p></div></div>"#,
    );
    let Block::Table(t) = &flex[0] else { panic!("{flex:?}") };
    assert_eq!(t.rows[0].len(), 2);
    assert_eq!(t.rows[0][0].width, Some(Width::Percent(30)));
    assert_eq!(t.rows[0][1].background, Some([0xee, 0xee, 0xee]));
    let grid = html_layout(r#"<div style="display: grid; grid-template-columns: repeat(2, 1fr)"><div>a</div><div>b</div><div>c</div></div>"#);
    let Block::Table(t) = &grid[0] else { panic!("{grid:?}") };
    assert_eq!(t.rows.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1]);
    let inline =
        html_layout(r#"<div><div style="display:inline-block;width:280px">One</div> <div style="display:inline-block;width:280px">Two</div></div>"#);
    assert!(matches!(&inline[0], Block::Table(t) if t.rows[0].len() == 2), "{inline:?}");
    let floats = html_layout(r#"<div><div style="float:left">One</div><div style="float:right">Two</div></div>"#);
    assert!(!floats.is_empty());
    // Stacked flexbox, inline text with spans: no columns.
    assert!(html_layout(r#"<div style="display:flex;flex-direction:column"><div>a</div><div>b</div></div>"#).is_empty());
    assert!(html_layout(r#"<p>Hello <span style="display:inline-block">a</span> and <span style="display:inline-block">b</span></p>"#).is_empty());
}

#[test]
fn pictures_behind_boxes_are_kept() {
    // A banner: a cell with a picture behind its heading (the address keeps its case).
    let banner = html_layout(
        r##"<table width="600"><tr><td bgcolor="#2e7d6b" background="https://img.example/Hero.JPG" style="color:#fff"><h1>Open call</h1></td></tr></table>"##,
    );
    let Block::Table(t) = &banner[0] else { panic!("{banner:?}") };
    assert_eq!(t.rows[0][0].background_image.as_deref(), Some("https://img.example/Hero.JPG"));
    assert_eq!(t.rows[0][0].background, Some([0x2e, 0x7d, 0x6b]), "the colour stays, for when the picture isn't shown");
    // CSS forms, on a `<div>`: `background-image` and `background` with `url()`.
    for css in ["background-image: url('cid:Hero@x')", "background: #000 url(cid:Hero@x) no-repeat center / cover"] {
        let b = html_layout(&format!(r#"<div style="{css}"><p>Over the picture</p></div>"#));
        let Block::Table(t) = &b[0] else { panic!("{css}: {b:?}") };
        assert_eq!(t.rows[0][0].background_image.as_deref(), Some("cid:Hero@x"), "{css}");
        assert!(text(&t.rows[0][0].blocks[0]).contains("Over the picture"));
        assert_eq!(background_pictures(&b), ["cid:Hero@x"]);
    }
    // Columns inside a box with a picture behind it stay columns.
    let cols = html_layout(r#"<div style="display:flex;background-image:url(cid:b@x)"><div>One</div><div>Two</div></div>"#);
    let Block::Table(t) = &cols[0] else { panic!("{cols:?}") };
    assert!(matches!(&t.rows[0][0].blocks[0], Block::Table(inner) if inner.rows[0].len() == 2), "{cols:?}");
    // Nothing that runs, and nothing local.
    for bad in ["javascript:alert(1)", "file:///etc/passwd", "data:text/html,x"] {
        assert!(html_layout(&format!(r#"<div style="background-image:url({bad})"><p>x</p></div>"#)).is_empty(), "{bad}");
    }
}

#[test]
fn text_takes_its_cells_colour() {
    // White on a dark band, set on the cell (or the table), as CSS inherits it.
    let colour_of = |b: &[Block], word: &str| {
        format!("{b:?}").contains(word).then(|| {
            fn spans(b: &[Block], out: &mut Vec<(String, Option<[u8; 3]>)>) {
                for x in b {
                    match x {
                        Block::Text(r) => out.extend(r.iter().flatten().map(|s| (s.text.clone(), s.color))),
                        Block::Table(t) => t.rows.iter().flatten().for_each(|c| spans(&c.blocks, out)),
                    }
                }
            }
            let mut v = Vec::new();
            spans(b, &mut v);
            v.into_iter().find(|(t, _)| t.contains(word)).and_then(|(_, c)| c)
        })
    };
    let cell = html_layout(r##"<table><tr><td style="color:#ffffff" bgcolor="#123456">Band</td><td>Other</td></tr></table>"##);
    assert_eq!(colour_of(&cell, "Band").flatten(), Some([255, 255, 255]), "{cell:?}");
    assert_eq!(colour_of(&cell, "Other").flatten(), None, "a neighbour keeps its own");
    let table = html_layout(r##"<table style="color:#00ff00"><tr><td>A</td><td>B</td></tr></table>"##);
    assert_eq!(colour_of(&table, "A").flatten(), Some([0, 255, 0]));
    // A colour can't smuggle markup in.
    let evil = html_layout(r##"<table><tr><td style="color:red&quot;><script>x</script>">A</td><td>B</td></tr></table>"##);
    assert!(!format!("{evil:?}").contains("script"), "{evil:?}");
}

#[test]
fn boxes_placed_over_others_are_kept() {
    // A label over a picture, and a note in the bottom right corner.
    let b = html_layout(
        r##"<div style="position:relative; width:300px"><img src="cid:poster@x" alt="Poster">
               <span style="position:absolute; top:8px; left:10px; background:#c62828; color:#fff">NEW</span>
               <div style="position:absolute; right:4px; bottom:6px; width:120px">Ends Friday</div></div>"##,
    );
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert_eq!(t.width, Some(Width::Px(300)));
    let cell = &t.rows[0][0];
    assert!(matches!(&cell.blocks[0], Block::Text(r) if r.iter().flatten().any(|s| s.image.as_deref() == Some("cid:poster@x"))));
    assert_eq!(cell.overlays.len(), 2);
    let (new, ends) = (&cell.overlays[0], &cell.overlays[1]);
    assert_eq!((new.x, new.y, new.from_right, new.from_bottom), (10, 8, false, false));
    assert_eq!(new.background, Some([0xc6, 0x28, 0x28]));
    assert!(text(&new.blocks[0]).contains("NEW"));
    assert_eq!((ends.x, ends.y, ends.from_right, ends.from_bottom), (4, 6, true, true));
    assert_eq!(ends.width, Some(Width::Px(120)));
    // Placed with nothing to sit over: it flows; absurd offsets are capped.
    assert!(html_layout(r#"<p>Hi <span style="position:absolute;top:3px">there</span></p>"#).is_empty());
    let far = html_layout(r#"<div style="position:relative"><p>a</p><b style="position:absolute;top:99999999px;left:-50px">b</b></div>"#);
    let Block::Table(t) = &far[0] else { panic!("{far:?}") };
    assert_eq!((t.rows[0][0].overlays[0].x, t.rows[0][0].overlays[0].y), (0, MAX_OFFSET));
}

#[test]
fn plain_mail_and_one_column_tables_have_no_layout() {
    assert!(html_layout("<p>Hello <b>there</b></p>").is_empty());
    assert!(html_layout("<table><tr><td>One</td></tr><tr><td>Two</td></tr></table>").is_empty());
    // A receipt (a data table) has columns.
    assert!(!html_layout("<table><tr><th>Item</th><th>Price</th></tr><tr><td>Poster</td><td>40</td></tr></table>").is_empty());
}

#[test]
fn hostile_markup_is_survived() {
    // Unclosed, misnested, deep, huge.
    let deep = "<table><tr><td>".repeat(500) + "x";
    let _ = html_layout(&deep);
    let _ = html_layout("<table><td>a<td>b</tr></tr></table></td><tr><td><td>");
    let _ = html_layout("<table><tr><td><script>if (a < b) { document.write('<td>') }</script>z</td><td>y</td></tr></table>");
    let many = "<table><tr>".to_string() + &"<td>c</td>".repeat(5000) + "</tr></table>";
    assert!(html_layout(&many).is_empty(), "too many cells: flows as text");
    assert_eq!(color("#abc"), Some([0xaa, 0xbb, 0xcc]));
    assert_eq!(color("rgb(1, 2, 300)"), Some([1, 2, 255]));
    assert_eq!(color("#12"), None);
}

/// A message in a centred column of a set width keeps it, one column or not: it was stretched
/// across the window (issue #10).
#[test]
fn a_centred_column_of_a_set_width_keeps_its_width() {
    // The usual table: a full-width one, a 600-pixel one centred in it.
    let b = html_layout(
        r#"<body><table width="100%"><tr><td align="center"><table width="600"><tr><td><h1>Spring sale</h1><p>All week.</p></td></tr>
        <tr><td><img src="cid:banner@x" width="600"></td></tr></table></td></tr></table></body>"#,
    );
    let Block::Table(outer) = &b[0] else { panic!("{b:?}") };
    assert_eq!(outer.rows[0][0].align, Align::Center);
    let Block::Table(inner) = &outer.rows[0][0].blocks[0] else { panic!("{b:?}") };
    assert_eq!(inner.width, Some(Width::Px(600)));
    // A box: `max-width` and auto margins, no table at all.
    let b = html_layout(r#"<div style="max-width:600px;margin:0 auto;background-color:#ffffff"><p>Hello</p><p>Thanks for your order.</p></div>"#);
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert_eq!((t.width, t.align), (Some(Width::Px(600)), Align::Center));
    assert_eq!(t.rows[0][0].background, Some([0xff, 0xff, 0xff]));
    assert!(text(&t.rows[0][0].blocks[0]).contains("Thanks for your order"));
    // Narrower than the window: `width: 100%` held to its `max-width`; `<center>` around a box.
    let b = html_layout(r#"<center><div style="width:100%;max-width:480px;text-align:left">Hi there</div></center>"#);
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    let Block::Table(t) = &t.rows[0][0].blocks[0] else { panic!("{b:?}") };
    assert_eq!(t.width, Some(Width::Px(480)));
    assert_eq!(t.rows[0][0].align, Align::Left);
    // No set width: the text flows, as before.
    assert!(html_layout(r#"<div style="width:100%"><p>Just text</p></div>"#).is_empty());
    assert!(html_layout("<p>Plain <b>mail</b></p><table><tr><td>one cell</td></tr></table>").is_empty());
}

/// Issue #41: a centred column whose width is in points (as Outlook and Word write it), in `em`
/// or `rem` (as email frameworks write it), set on the body, or given by an attribute beside a
/// style width a browser can't read, keeps its width instead of filling the window.
#[test]
fn a_centred_column_in_points_or_ems_keeps_its_width() {
    let column = |html: &str| -> Option<(Option<Width>, Align)> {
        fn first(b: &[Block]) -> Option<(Option<Width>, Align)> {
            let Some(Block::Table(t)) = b.first() else { return None };
            if matches!(t.width, Some(Width::Px(_))) {
                return Some((t.width, t.align));
            }
            first(&t.rows.first()?.first()?.blocks)
        }
        first(&html_layout(&inline_css(html)))
    };
    let word = r#"<div class=WordSection1><div align=center><table class=MsoNormalTable border=0 cellspacing=0 cellpadding=0 width=600
        style='width:450.0pt;mso-cellspacing:0cm' align=center><tr><td style='padding:0cm'><p class=MsoNormal>Hello from Outlook</p></td></tr></table></div></div>"#;
    assert_eq!(column(word), Some((Some(Width::Px(600)), Align::Center)));
    let framework = r#"<body style="background-color:#ffffff"><table align="center" width="100%" role="presentation"
        style="max-width:37.5em;margin:0 auto"><tbody><tr style="width:100%"><td><p>Your code is ready.</p></td></tr></tbody></table></body>"#;
    assert_eq!(column(framework), Some((Some(Width::Px(600)), Align::Center)));
    assert_eq!(column(r#"<div style="width:40rem;margin:0 auto"><p>rem</p></div>"#), Some((Some(Width::Px(640)), Align::Center)));
    assert_eq!(column(r#"<div style="width:6.25in;margin:0 auto"><p>inches</p></div>"#), Some((Some(Width::Px(600)), Align::Center)));
    assert_eq!(column(r#"<body style="width:600px;margin:0 auto"><p>In the body itself</p></body>"#), Some((Some(Width::Px(600)), Align::Center)));
    assert_eq!(
        column(r#"<table width="560" align="center" style="width:35vw"><tr><td>Unread unit</td></tr></table>"#).map(|c| c.0),
        Some(Some(Width::Px(560)))
    );
    // A style a browser reads still wins over the attribute, and `auto` sets no width.
    assert!(html_layout(r#"<table width="600" align="center" style="width:100%"><tr><td>Full width</td></tr></table>"#).is_empty());
    assert!(html_layout(r#"<table width="600" style="width:auto"><tr><td>Auto</td></tr></table>"#).is_empty());
    assert_eq!(length_px("12pt"), Some(16.0));
    assert_eq!(length_px("2.54cm").map(f32::round), Some(96.0));
    assert_eq!((length_px("-3px"), length_px("1e9px"), length_px("NaNpx"), length_px("10vw")), (None, Some(100_000.0), None, None));
}

/// Issue #41, as Outlook shows it: a title centred in the column (`text-align: center`) stays
/// centred, and its size, set on a span inside `<b>` as Outlook writes it, makes it a title.
#[test]
fn a_centred_title_in_a_column_stays_centred_and_big() {
    let html = r#"<table width=600 style='width:450.0pt' align=center><tr><td>
        <p class=MsoNormal style='text-align:center'><b><span style='font-size:20.0pt;color:#1F3864'>Quarterly update</span></b></p>
        <p class=MsoNormal>Hello everyone,</p></td></tr></table>"#;
    let b = html_layout(&inline_css(html));
    let Block::Table(column) = &b[0] else { panic!("{b:?}") };
    let cell = &column.rows[0][0].blocks;
    let Block::Table(title) = &cell[0] else { panic!("{cell:?}") };
    assert_eq!((title.align, title.width, title.rows[0][0].align), (Align::Center, None, Align::Center));
    let Block::Text(lines) = &title.rows[0][0].blocks[0] else { panic!() };
    let span = lines.iter().flatten().find(|s| s.text.contains("Quarterly")).unwrap();
    assert!(span.heading > 0 && span.bold, "{span:?}");
    assert_eq!(span.color, Some([0x1F, 0x38, 0x64]));
    assert!(matches!(&cell[1], Block::Text(t) if t.iter().flatten().any(|s| s.text.contains("Hello") && s.heading == 0)), "{cell:?}");
    // Text that flows (no column) keeps flowing: no layout for a centred paragraph alone.
    assert!(html_layout(r#"<p style="text-align:center">Just centred</p>"#).is_empty());
}

/// A business card (issue #11): the photo's column as wide as the photo, the details beside it.
#[test]
fn a_photo_column_is_as_wide_as_its_photo() {
    let b = html_layout(r#"<table><tr><td><img src="cid:p@x" width="80" height="80"></td><td><b>Dean Moss</b><br>Sales</td></tr></table>"#);
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert_eq!(t.rows[0].iter().map(|c| c.width).collect::<Vec<_>>(), [Some(Width::Px(96)), None]);
    // A picture of no set width, or with text beside it, leaves the column as it was.
    let b = html_layout(r#"<table><tr><td><img src="cid:p@x"> Hi</td><td>There</td></tr></table>"#);
    let Block::Table(t) = &b[0] else { panic!("{b:?}") };
    assert_eq!(t.rows[0][0].width, None);
}

/// Issue #41, the tester's newsletter: blocks of one width stacked one on another (each its own
/// table, or a table's rows) touch, as in Outlook: the tables' own room (`cellspacing="0"`,
/// `cellpadding="0"`) and each cell's padding are kept, a line break between two of them is the
/// empty line a browser leaves, and a gradient Outlook doesn't draw shows its `bgcolor`.
#[test]
fn stacked_blocks_keep_their_own_room() {
    let html = r##"<table width="100%" cellpadding="0" cellspacing="0"><tr><td align="center">
        <table width="600" cellpadding="0" cellspacing="0" bgcolor="#fff8cc"><tr><td style="padding:24px">
          <table width="100%" cellpadding="0" cellspacing="0" style="background:linear-gradient(90deg,#ffcc00,#c08000)" bgcolor="#f0b800">
            <tr><td style="padding:20px 10px">BANNER</td></tr></table></td></tr></table>
        <table width="600" cellpadding="0" cellspacing="0" bgcolor="#fff8cc"><tr><td style="padding:24px 20px 12pt">First block</td></tr></table>
        <br>
        <table width="600" cellpadding="4" cellspacing="0"><tr><td bgcolor="#cce8ff">Row one</td></tr><tr><td bgcolor="#b8dcff">Row two</td></tr></table>
        <table width="600"><tr><td>HTML's own room</td></tr></table>
        <table width="600" cellspacing="8" style="border-collapse:collapse"><tr><td>Collapsed</td></tr></table>
        </td></tr></table>"##;
    let b = html_layout(&inline_css(html));
    let Block::Table(outer) = &b[0] else { panic!("{b:?}") };
    let parts = &outer.rows[0][0].blocks;
    let tables: Vec<&Table> = parts.iter().filter_map(|p| if let Block::Table(t) = p { Some(t) } else { None }).collect();
    let room: Vec<(Option<u16>, Option<u16>)> = tables.iter().map(|t| (t.spacing, t.padding)).collect();
    assert_eq!(room, [(Some(0), Some(0)), (Some(0), Some(0)), (Some(0), Some(4)), (Some(2), Some(1)), (Some(0), Some(1))]);
    assert_eq!(tables[0].rows[0][0].padding, Some([24, 24, 24, 24]));
    assert_eq!(tables[1].rows[0][0].padding, Some([24, 20, 16, 20]), "a missing side is the second's; points as pixels");
    assert_eq!(tables[2].rows[0][0].padding, None, "its table's");
    let Block::Table(banner) = &tables[0].rows[0][0].blocks[0] else { panic!() };
    assert_eq!(banner.background, Some([0xf0, 0xb8, 0x00]), "the bgcolor under a gradient");
    assert_eq!(banner.rows[0][0].padding, Some([20, 10, 20, 10]));
    // The line break between the second block and the rows: one empty line.
    let at = parts.iter().position(|p| matches!(p, Block::Table(t) if t.padding == Some(4))).unwrap();
    assert!(matches!(&parts[at - 1], Block::Text(lines) if lines.len() == 1 && lines[0].is_empty()), "{:?}", parts[at - 1]);
    assert!(matches!(&parts[1], Block::Table(_)), "nothing between the first two: {:?}", parts[1]);
    // A gradient with no bgcolor: its first colour.
    let b = html_layout(r#"<table width="600" style="background-image:linear-gradient(#123456, #fff)"><tr><td>x</td></tr></table>"#);
    let Block::Table(t) = &b[0] else { panic!() };
    assert_eq!(t.background, Some([0x12, 0x34, 0x56]));
}

/// Issue #47: a cell with a picture behind it given as an attribute, the picture in the message
/// (`<td background="cid:…">`, a newsletter's banner): the cell carries it, listed with the
/// message's pictures, and it's at least as tall as it asks.
#[test]
fn a_cells_background_attribute_is_its_picture() {
    let blocks = html_layout(
        r#"<table width="600"><tr><td background="cid:banner@shop.example" height="200" valign="bottom"><h1>Autumn</h1></td></tr></table>"#,
    );
    let Some(Block::Table(t)) = blocks.first() else { panic!("{blocks:?}") };
    let cell = &t.rows[0][0];
    assert_eq!(cell.background_image.as_deref(), Some("cid:banner@shop.example"));
    assert_eq!((cell.height, cell.valign), (Some(200), VAlign::Bottom));
    assert_eq!(background_pictures(&blocks), ["cid:banner@shop.example"]);
}

/// Issue #47: the page's own colour (`body { background }`, `<body bgcolor>`) is the page's,
/// not a highlight behind each run of its text: a newsletter's light grey came out behind every
/// word of its white columns. A box's own colour stays.
#[test]
fn a_pages_background_is_no_highlight() {
    let html = r##"<html><head><style>body{background:#eef1f4;margin:0} h1{color:#1f2a44}</style></head><body bgcolor="#eef1f4"><table width="600" bgcolor="#ffffff"><tr><td><h1>Autumn sale</h1><p>Warm light, <span style="background:#ffff00">this week</span>.</p></td><td>b</td></tr></table></body></html>"##;
    let spans: Vec<crate::Span> = crate::html_to_rich(&inline_css(html)).into_iter().flatten().collect();
    assert!(spans.iter().all(|s| s.background != Some([0xee, 0xf1, 0xf4])), "no page grey behind the text: {spans:?}");
    assert!(spans.iter().any(|s| s.text.contains("this week") && s.background == Some([255, 255, 0])), "a highlight stays: {spans:?}");
    let blocks = html_layout(&inline_css(html));
    let flat = format!("{blocks:?}");
    assert!(!flat.contains("[238, 241, 244]"), "no grey behind the text in the layout: {flat}");
}

/// Found in review: past the depth to which the tree is written as it is, every tag was dropped,
/// and the paragraphs of a newsletter nested in two dozen tables ran together, in a message with
/// a style sheet or Outlook's copy of a table around it, as did those past the depth the tree
/// is read to. Every tag is written; in Outlook's copy, its own tables are written as blocks in
/// the text; too deep to read, the message is read as it came.
#[test]
fn blocks_nested_deep_stay_apart() {
    let message = |html: &str| {
        let raw = format!("From: a@shop.example\r\nSubject: s\r\nContent-Type: text/html\r\n\r\n{html}");
        crate::parse(raw.as_bytes()).unwrap_or_default()
    };
    let shown = |p: &crate::Parsed| p.rich.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("|");
    let body = "<p>First paragraph.</p><p>Second paragraph.</p><h2>Heading</h2><p>Last words</p>";
    for n in [24, 28, 32, 80] {
        let nest = format!("{}{body}{}", r#"<table role="presentation"><tbody><tr><td>"#.repeat(n), "</td></tr></tbody></table>".repeat(n));
        for html in [
            format!("<!--[if mso]><table width=600><tr><td><![endif]-->{nest}<!--[if mso]></td></tr></table><![endif]-->"),
            format!("<style>p{{margin:0}}</style>{nest}"),
            format!("<style>p{{margin:0}}</style>{}{body}", "<blockquote><ul><li>".repeat(n)),
        ] {
            let p = message(&html);
            let words = ["First paragraph.", "Second paragraph.", "Heading", "Last words"];
            let apart =
                |lines: Vec<&str>| words.iter().all(|w| lines.iter().any(|l| l.ends_with(w) && words.iter().filter(|o| l.contains(*o)).count() == 1));
            assert!(apart(shown(&p).split('|').collect()), "{n}: {:?}", shown(&p));
            assert!(apart(p.text.lines().collect()), "{n}: {:?}", p.text);
        }
    }
    // Deeper than it's written as it is, and than it's read: tables each in the last, in
    // Outlook's copy, and boxes in a styled message.
    for html in [
        format!("<!--[if mso]>{}<![endif]-->Tail<p>More</p>After", "<table>".repeat(100)),
        format!("<!--[if mso]>{}<![endif]-->Tail<p>More</p>After", "<table>".repeat(150_000)),
        format!("<style>p{{margin:0}}</style>{}Tail<p>More</p>After", "<div>".repeat(300)),
    ] {
        let p = message(&html);
        assert_eq!(shown(&p).split('|').filter(|l| !l.is_empty()).collect::<Vec<_>>(), ["Tail", "More", "After"], "{}", &html[..40]);
        assert_eq!(p.text.lines().filter(|l| !l.is_empty()).collect::<Vec<_>>(), ["Tail", "More", "After"], "{}", &html[..40]);
    }
    // Paragraphs too deep to read keep the empty line between them (found in review: one line
    // break ran them together), as they do at any depth.
    let paragraphs = "<p>One para</p><p>Two para</p>";
    for html in [
        format!("<style>p{{color:red}}</style>{}{paragraphs}{}", "<div>".repeat(300), "</div>".repeat(300)),
        format!("<!--[if mso]><table><tr><td><![endif]-->{}{paragraphs}", "<div>".repeat(300)),
        format!(
            "<!--[if mso]><table><tr><td><![endif]-->{}{paragraphs}{}<!--[if mso]></td></tr></table><![endif]-->",
            "<table><tr><td>".repeat(120),
            "</td></tr></table>".repeat(120)
        ),
        format!("<!--[if mso]><table><tr><td><![endif]-->{paragraphs}"),
    ] {
        let p = message(&html);
        assert_eq!(shown(&p), "One para||Two para", "{}", &html[..40]);
        assert_eq!(p.text, "One para\n\nTwo para", "{}", &html[..40]);
    }
}

/// Found in review: Outlook's copy of every newsletter was written out again from the tree to be
/// shown, which made a newsletter of 4 MB slower to read. It's shown as it came unless the tree
/// reads otherwise: `<plaintext>`, or a mark in text. One nesting more than two dozen tables
/// isn't read: the message is read as it came, without it.
#[test]
fn outlooks_copy_is_written_again_only_when_the_tree_reads_otherwise() {
    let ready_copy = |html: &str| {
        let copy = crate::html::outlook_copy(html);
        assert!(copy.tables, "{html}");
        let r = ready(html, &copy);
        assert!(r.copy, "{html}");
        (r, copy.html.into_owned())
    };
    let column = "<!--[if mso]><table><tr><td><![endif]--><div>c</div><!--[if mso]></td></tr></table><![endif]-->";
    let (r, copy) = ready_copy(&column.repeat(50));
    assert_eq!(r.styled, copy);
    assert!(!r.text.contains("<table"));
    // A table of values in tables nested no more than two dozen deep: as it came. One deeper:
    // the message as it came, without Outlook's copy.
    let nested =
        |n: usize| format!("<!--[if mso]><table><tr><td><![endif]-->{}<table><tr><td>A</td><td>B</td></tr></table>", "<table><tr><td>".repeat(n));
    let (r, copy) = ready_copy(&nested(MAX_COPY_TABLES - 2));
    assert_eq!(r.styled, copy);
    for html in [nested(MAX_COPY_TABLES - 1), format!("<!--[if mso]>{}Deep<![endif]-->Tail", "<table><tr><td>".repeat(40))] {
        assert!(!ready(&html, &crate::html::outlook_copy(&html)).copy, "{html}");
    }
    for html in [
        "<textarea><!--[if mso]><table><tr><td>A<![endif]--></textarea>".to_string(),
        "<!--[if mso]><table><tr><td>A</td></tr></table><![endif]--><plaintext>B".to_string(),
    ] {
        let (r, copy) = ready_copy(&html);
        assert_ne!(r.styled, copy, "{html}");
        assert!(!r.styled.contains(crate::html::OUTLOOK_MARK) && !r.styled.contains("plaintext"), "{}", r.styled);
    }
}

mod buttons;
