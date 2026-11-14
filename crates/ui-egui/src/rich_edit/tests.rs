//! The message editor's tests: links, right-to-left paragraphs, pictures in the text.

use super::*;

#[test]
fn link_addresses() {
    let web = |a: &str| LinkForm { address: a.into(), ..Default::default() }.url();
    assert_eq!(web("example.com").as_deref(), Some("https://example.com"));
    assert_eq!(web("http://x.example/a").as_deref(), Some("http://x.example/a"));
    assert_eq!(web("C:/Users/me/plan.docx").as_deref(), Some("file:///C:/Users/me/plan.docx"));
    assert_eq!(web("https://"), None);
    assert_eq!(web("  "), None);
    let mail = LinkForm { email: true, to: "zoe@studio.example".into(), subject: "Poster & sizes".into(), ..Default::default() };
    assert_eq!(mail.url().as_deref(), Some("mailto:zoe@studio.example?subject=Poster%20%26%20sizes"));
    assert_eq!(LinkForm { email: true, ..Default::default() }.url(), None);
    // An existing mailto link opens as an email address with its subject.
    let mut c = Compose { body: "write".into(), ..Default::default() };
    c.styles = Styles::plain(&c.body);
    c.styles.restyle(&c.body, 0..5, |s| Style { link: Some("mailto:ann@x.example?subject=Hi%20there".into()), ..s.clone() });
    let f = LinkForm::open(&c, 0..5);
    assert!(f.email && f.editing && !f.multi);
    assert_eq!((f.to.as_str(), f.subject.as_str(), f.text.as_str()), ("ann@x.example", "Hi there", "write"));
}

/// Right-to-left paragraphs in a message start against the editor's right edge, a short one
/// and every row of a wrapped one, in rich and plain text; a left-to-right one stays left. (egui
/// put them against the widest line, so a short Hebrew line sat in the middle.)
#[test]
fn right_to_left_paragraphs_start_on_the_right() {
    let text = "שלום עולם
English stays left.
זוהי פסקה ארוכה מאוד בעברית שתיכנס לכמה שורות כשהעורך צר, וכל שורה שלה צריכה להתחיל בצד ימין של העורך.";
    let wrap = 300.0;
    let ctx = egui::Context::default();
    let t = Tokens::light();
    for plain in [false, true] {
        let mut rows = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let g = layout::layout(ui, text, &Styles::plain(text), plain, &t, wrap, &HashMap::new(), None);
            rows = g
                .rows
                .iter()
                .map(|r| {
                    // Where its letters are (a row's rect takes in its leading space).
                    let ink =
                        |f: fn(f32, f32) -> f32, init: f32| r.row.glyphs.iter().filter(|g| !g.chr.is_whitespace()).map(|g| g.pos.x).fold(init, f);
                    let right = r.row.glyphs.iter().map(|g| g.max_x()).fold(0.0, f32::max);
                    (r.pos.x + ink(f32::min, f32::INFINITY), r.pos.x + right, r.ends_with_newline)
                })
                .collect();
        });
        out.textures_delta.clear();
        assert!(rows.len() > 3, "the long paragraph wraps: {rows:?}");
        // Row 0: the short Hebrew line; row 1: English; then the wrapped Hebrew paragraph.
        let (min0, max0, _) = rows[0];
        assert!((wrap - max0).abs() < 2.0 && min0 > 100.0, "plain={plain}: the short line against the right edge: {rows:?}");
        assert!(rows[1].0 < 2.0, "plain={plain}: English on the left: {rows:?}");
        for r in &rows[2..] {
            assert!((wrap - r.1).abs() < 2.0, "plain={plain}: each wrapped row against the right edge: {rows:?}");
        }
    }
}

/// A picture has room as wide as it is, in a row as tall: after its character, or before it
/// when it's last in its line (letter spacing put it some 60 points right of its place after
/// text, and none at a line's start; room after the last one wasn't the row's, so a picture
/// that didn't fit stayed on its row, squeezed). One that doesn't fit starts the next row.
#[test]
fn a_picture_sits_where_it_is_in_the_text() {
    let ctx = egui::Context::default();
    let t = Tokens::light();
    let own = HashMap::from([("cid".to_string(), vec2(300.0, 150.0))]);
    // (text, picture, editor width, it starts a row of its own)
    let cases = [
        ("\u{FFFC}", 0, 600.0, false),
        ("ab\u{FFFC}cd", 2, 600.0, false),
        ("x\n\u{FFFC}\u{FFFC}", 2, 900.0, false),
        ("x\n\u{FFFC}\u{FFFC}", 3, 900.0, false),
        ("x\n\u{FFFC}\u{FFFC}", 3, 500.0, true),
    ];
    for (body, at, wrap, alone) in cases {
        let mut st = Styles::plain(body);
        for (i, ch) in body.chars().enumerate() {
            if ch == PICTURE {
                st.restyle(body, i..i + 1, |s| Style { picture: Some("cid".into()), ..s.clone() });
            }
        }
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let g = layout::layout(ui, body, &st, false, &t, wrap, &own, None);
            let (a, b) = (g.pos_from_cursor(CCursor::new(at)), g.pos_from_cursor(CCursor::new(at + 1)));
            let last = body.chars().nth(at + 1).is_none_or(|c| c == '\n');
            let place = pictures::Place::of(&g, egui::Pos2::ZERO, at, last);
            assert_eq!(place.wrapped, alone, "{body:?} at {at}, {wrap} wide");
            if last && !alone {
                // Room before it: after what's before it in its line.
                let before =
                    if at == 0 || body.chars().nth(at - 1) == Some('\n') { 0.0 } else { g.pos_from_cursor(CCursor::new(at - 1)).min.x + 7.0 };
                assert!(place.glyph.min.x - 300.0 >= before - 0.5, "{body:?} at {at}: room before it from {}", place.glyph.min.x - 300.0);
            } else if !last {
                assert!(b.min.x - a.min.x >= 300.0, "{body:?} at {at}: a room {} wide", b.min.x - a.min.x);
            }
            assert!(place.glyph.height() >= 150.0, "{body:?} at {at}: its row as tall as the picture ({})", place.glyph.height());
        });
        out.textures_delta.clear();
    }
}

/// A look picked with nothing selected (Bold, a size, a colour, a link) is only how what's typed
/// next looks: the text is as it was, so it's no undo step. It was one: the next Undo put back
/// the same text (it seemed to do nothing), and Redo was gone. A paragraph's look (a list) with
/// nothing selected changes the text's, and is one.
#[test]
fn a_look_with_nothing_selected_is_no_undo_step() {
    let ctx = egui::Context::default();
    let mut c = Compose { body: "Hello".into(), ..Default::default() };
    c.styles = Styles::plain(&c.body);
    let mut ed = Editor::new(c.id);
    ed.sel = 5..5;
    apply(&mut ed, &mut c, Op::Insert(" world".into()));
    let looks = [
        Op::Bold,
        Op::Italic,
        Op::Underline,
        Op::Strike,
        Op::Size(14),
        Op::Grow,
        Op::Shrink,
        Op::Color(Some(COLORS[0])),
        Op::Highlight(Some(HIGHLIGHTS[0])),
        Op::Link("https://example.com".into()),
        Op::Unlink,
    ];
    for op in looks.clone() {
        apply(&mut ed, &mut c, op);
    }
    let typing = ed.typing.clone().unwrap_or_default();
    assert!(typing.bold && typing.italic && typing.underline && typing.strike && typing.size == Some(14), "{typing:?}");
    assert_eq!((typing.color, typing.highlight, typing.link), (Some(COLORS[0]), Some(HIGHLIGHTS[0]), None));
    // The first Undo takes out what was typed.
    undo(&mut ed, &mut c, &ctx, false);
    assert_eq!(c.body, "Hello");
    // A look picked with nothing selected leaves Redo as it was.
    for op in looks {
        apply(&mut ed, &mut c, op);
    }
    undo(&mut ed, &mut c, &ctx, true);
    assert_eq!(c.body, "Hello world");
    // A list with nothing selected is an undo step, as is a look on a selection.
    apply(&mut ed, &mut c, Op::Bullets);
    assert_eq!(c.styles.para(0).list, List::Bullet);
    undo(&mut ed, &mut c, &ctx, false);
    assert_eq!(c.styles.para(0).list, List::None);
    ed.sel = 0..5;
    apply(&mut ed, &mut c, Op::Bold);
    assert!(c.styles.style(0).bold);
    undo(&mut ed, &mut c, &ctx, false);
    assert!(!c.styles.style(0).bold);
    assert_eq!(c.body, "Hello world");
}
