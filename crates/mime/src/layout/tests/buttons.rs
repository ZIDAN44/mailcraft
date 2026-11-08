//! Buttons (issue #47's gap): a link drawn as a button is a box of its own, with its colour,
//! room, round corners and the size its VML gives it, its words still a link.

use super::*;

fn message(html: &str) -> crate::Parsed {
    let raw = format!("From: a@shop.example\r\nSubject: s\r\nContent-Type: text/html\r\n\r\n{html}");
    crate::parse(raw.as_bytes()).unwrap_or_default()
}

/// Every one-cell table of a colour in `b`, at any depth: the buttons.
fn buttons(b: &[Block]) -> Vec<&Table> {
    fn walk<'a>(b: &'a [Block], out: &mut Vec<&'a Table>) {
        for x in b {
            if let Block::Table(t) = x {
                if let [row] = t.rows.as_slice()
                    && let [c] = row.as_slice()
                    && c.background.is_some()
                    && c.radius.is_some()
                {
                    out.push(t);
                }
                for c in t.rows.iter().flatten() {
                    walk(&c.blocks, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(b, &mut out);
    out
}

fn link_text(c: &Cell) -> Vec<(String, Option<String>)> {
    c.blocks
        .iter()
        .filter_map(|b| if let Block::Text(t) = b { Some(t) } else { None })
        .flatten()
        .flatten()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| (s.text.trim().to_string(), s.link.clone()))
        .collect()
}

/// A newsletter's button as Outlook draws it, from its VML (`v:roundrect` 200 by 44, `arcsize`
/// 10%): a box 200 wide and 44 tall at least, its corners 4.4 round (rounded to 4), its words in
/// the middle, white on its colour, still a link; the copy for the others, hidden from Outlook,
/// isn't shown.
#[test]
fn a_vml_button_is_its_size_with_round_corners() {
    let html = r##"<html xmlns:v="urn:schemas-microsoft-com:vml"><body><table width="600" align="center"><tr><td align="center" style="padding:20px">
        <!--[if mso]><v:roundrect xmlns:v="urn:schemas-microsoft-com:vml" href="https://shop.example/sale" style="height:44px;v-text-anchor:middle;width:200px" arcsize="10%" stroke="f" fillcolor="#d6336c"><w:anchorlock/><center style="color:#ffffff;font-family:sans-serif;font-size:13px;font-weight:bold">Shop the sale</center></v:roundrect><![endif]-->
        <!--[if !mso]><!--><a href="https://shop.example/sale" style="background-color:#d6336c;border-radius:4px;color:#ffffff;display:inline-block;line-height:44px;text-align:center;width:200px;mso-hide:all">Shop the sale</a><!--<![endif]-->
        </td></tr></table></body></html>"##;
    let p = message(html);
    let found = buttons(&p.layout);
    let [t] = found.as_slice() else { panic!("one button: {:#?}", p.layout) };
    assert_eq!(t.width, Some(Width::Px(200)));
    assert!(!t.fit);
    let c = &t.rows[0][0];
    assert_eq!((c.background, c.radius, c.height), (Some([0xd6, 0x33, 0x6c]), Some(4), Some(44)));
    assert_eq!((c.align, c.valign), (Align::Center, VAlign::Middle));
    assert_eq!(link_text(c), [("Shop the sale".to_string(), Some("https://shop.example/sale".to_string()))]);
    assert!(c.blocks.iter().flat_map(|b| if let Block::Text(t) = b { t.iter().flatten().collect() } else { vec![] }).all(|s| s.background.is_none()));
    // Its words once, in what's shown and in its text.
    assert_eq!(format!("{:?}", p.layout).matches("Shop the sale").count(), 1);
    assert_eq!(p.text.matches("Shop the sale").count(), 1, "{}", p.text);
    // `arcsize` in each of VML's forms, and VML's own when it says none: a share of the shorter
    // side, at most half of it (a pill).
    for (arc, r) in [(r#"arcsize="50%""#, 22), (r#"arcsize="0.25""#, 11), (r#"arcsize="6554f""#, 4), ("", 9), (r#"arcsize="90%""#, 22)] {
        let html = format!(
            r##"<table width="600"><tr><td><!--[if mso]><v:roundrect href="https://x.example/" style="width:200px;height:44px" {arc} fillcolor="#123456"><center>Go</center></v:roundrect><![endif]--></td></tr></table>"##
        );
        let p = message(&html);
        let found = buttons(&p.layout);
        assert_eq!(found.first().map(|t| t.rows[0][0].radius), Some(Some(r)), "{arc}: {:#?}", p.layout);
    }
}

/// A button of CSS (a link `inline-block` with room and round corners, as email frameworks write
/// them, in a cell of the same colour or alone in a paragraph): a box as wide as its words and
/// room, its corners round; one `display:block` fills its room, one with a `line-height` of
/// pixels is that tall.
#[test]
fn a_css_button_has_its_room_and_round_corners() {
    let html = r##"<table width="600"><tr><td>
        <table border="0" cellpadding="0" cellspacing="0" role="presentation"><tr><td align="center" bgcolor="#414141" style="border-radius:3px;background:#414141" valign="middle">
        <a href="https://shop.example/buy" style="display:inline-block;background:#414141;color:#ffffff;font-size:13px;padding:10px 25px;border-radius:3px;text-decoration:none" target="_blank">Buy now</a>
        </td></tr></table>
        <p style="text-align:center"><a class="btn" href="https://shop.example/more" style="display:block;background-color:#0b6e4f;color:#fff;border-radius:8px 8px 0 0;line-height:40px">See more</a></p>
        </td></tr></table>"##;
    let b = html_layout(html);
    let found = buttons(&b);
    // The cell of the framework's table, and the two links.
    assert_eq!(found.len(), 3, "{b:#?}");
    let framework = found[0];
    assert!(framework.fit, "a table that sets no width is as wide as what it holds");
    assert_eq!(framework.rows[0][0].radius, Some(3));
    let buy = found[1];
    let c = &buy.rows[0][0];
    assert!(buy.fit && buy.width.is_none());
    assert_eq!((c.padding, c.radius, c.background), (Some([10, 25, 10, 25]), Some(3), Some([0x41, 0x41, 0x41])));
    assert_eq!(link_text(c), [("Buy now".to_string(), Some("https://shop.example/buy".to_string()))]);
    let more = found[2];
    let c = &more.rows[0][0];
    assert!(!more.fit, "a block fills its room");
    assert_eq!(more.align, Align::Center);
    assert_eq!((c.radius, c.height), (Some(8), Some(40)));
    assert_eq!(link_text(c), [("See more".to_string(), Some("https://shop.example/more".to_string()))]);
    // Alone in a message of paragraphs, a button is reason enough for a layout.
    let b = html_layout(
        r##"<p>Hello</p><p><a href="https://x.example/" style="display:inline-block;padding:8px 16px;background:#06c;color:#fff;border-radius:4px">Go</a></p><p>Bye</p>"##,
    );
    assert_eq!(buttons(&b).len(), 1, "{b:#?}");
    assert_eq!(b.len(), 3);
    // Not buttons: a link in a sentence, one with no colour of its own, one with nothing a button
    // has.
    for html in [
        r##"<p>Read <a href="https://x.example/" style="display:inline-block;padding:4px;background:#ff0">this</a> now</p>"##,
        r##"<p><a href="https://x.example/" style="display:inline-block;padding:4px;border-radius:4px">Go</a></p>"##,
        r##"<p><a href="https://x.example/" style="display:inline-block;background:#ff0">Go</a></p>"##,
    ] {
        assert!(buttons(&html_layout(html)).is_empty(), "{html}");
    }
}

/// Hostile buttons: thousands of them stay quick, past the most drawn as their words on their
/// colour; sizes and corners that would be huge are held to what's drawn.
#[test]
fn hostile_buttons_are_survived() {
    let one = r##"<a href="https://x.example/" style="display:inline-block;padding:9999px;background:#06c;border-radius:1e30px;width:99999px;height:1e9px;line-height:-4px">Go</a><br>"##;
    let html = format!("<table width=600><tr><td>{}</td></tr></table>", one.repeat(5_000));
    let t = std::time::Instant::now();
    let b = html_layout(&html);
    assert!(t.elapsed().as_secs() < 5, "{:?}", t.elapsed());
    let found = buttons(&b);
    assert!(!found.is_empty() && found.len() <= MAX_BUTTONS, "{}", found.len());
    let c = &found[0].rows[0][0];
    assert_eq!((c.radius, c.height, c.padding), (Some(MAX_ROOM), Some(MAX_CELL_HEIGHT), Some([MAX_ROOM; 4])));
    assert_eq!(found[0].width, Some(Width::Px(4000)));
    for vml in [
        r##"<!--[if mso]><v:roundrect href="https://x.example/" style="width:1e40px;height:NaNpx" arcsize="-5%" fillcolor="#123"><center>Go</center></v:roundrect><![endif]-->"##,
        r##"<!--[if mso]><v:roundrect href="https://x.example/" style="width:;height" arcsize="inf" fillcolor="#123">Go</v:roundrect><![endif]-->"##,
    ] {
        let p = message(&format!("<table width=600><tr><td>{vml}</td></tr></table>"));
        assert!(p.text.contains("Go"), "{vml}");
    }
}
