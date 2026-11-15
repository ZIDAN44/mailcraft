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

/// A mailto link from elsewhere (a quoted message, a pasted page) opens as RFC 6068 reads it:
/// field names in any case, addresses in `to=`, no `#fragment`, the subject on one line.
#[test]
fn a_mailto_link_opens_as_the_rfc_reads_it() {
    let open = |link: &str| {
        let mut c = Compose { body: "write".into(), ..Default::default() };
        c.styles = Styles::plain(&c.body);
        c.styles.restyle(&c.body, 0..5, |s| Style { link: Some(link.into()), ..s.clone() });
        let f = LinkForm::open(&c, 0..5);
        (f.email, f.to, f.subject)
    };
    let mail = |to: &str, subject: &str| (true, to.to_string(), subject.to_string());
    assert_eq!(open("MAILTO:ann@x.example?SUBJECT=Hi%20there"), mail("ann@x.example", "Hi there"));
    assert_eq!(open("mailto:?to=bob@x.example&Subject=Plans"), mail("bob@x.example", "Plans"));
    assert_eq!(open("mailto:ann@x.example?subject=Hi#top"), mail("ann@x.example", "Hi"));
    assert_eq!(open("mailto:ann@x.example?subject=Hi%0D%0ABcc:%20b@x.example"), mail("ann@x.example", "Hi Bcc: b@x.example"));
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
            let g = layout::layout(ui, text, Lay { st: &Styles::plain(text), plain, t: &t, wrap, own: &HashMap::new(), card: None });
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
            let g = layout::layout(ui, body, Lay { st: &st, plain: false, t: &t, wrap, own: &own, card: None });
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

/// A long message, some of it formatted: `lines` paragraphs of about 90 characters, some not
/// ASCII.
fn long_message(lines: usize) -> Compose {
    let body: Vec<String> =
        (0..lines).map(|k| format!("Paragraph {k}: the quick brown fox jumps over the lazy dog, naïve café 東京 — done.")).collect();
    let mut c = Compose { body: body.join("\n"), ..Default::default() };
    c.styles = Styles::plain(&c.body);
    for k in (0..lines).step_by(7) {
        let at = c.body.split('\n').take(k).map(|l| l.chars().count() + 1).sum::<usize>();
        c.styles.restyle(&c.body, at..at + 9, |s| Style { bold: true, ..s.clone() });
    }
    c
}

/// MEASURE: the room Undo keeps and the time it takes, a long message and 200 steps (the
/// steps' looks undone and redone as they were).
#[test]
#[ignore = "a measurement"]
fn measure_undo_with_a_long_message() {
    let ctx = egui::Context::default();
    let mut c = long_message(3_400);
    let n = c.body.chars().count();
    let mut ed = Editor::new(c.id);
    let t = std::time::Instant::now();
    for k in 0..200 {
        let at = (k * 1_543) % n;
        if k % 5 == 4 {
            ed.sel = at..at + 20;
            apply(&mut ed, &mut c, Op::Bold);
        } else {
            ed.sel = at..at;
            apply(&mut ed, &mut c, Op::Insert("word ".into()));
        }
    }
    let steps = t.elapsed();
    // The room the steps take: text, a look per character, the looks, paragraphs.
    let kept = ed.undo.bytes();
    let last = (c.body.clone(), c.styles.clone());
    let t = std::time::Instant::now();
    for _ in 0..200 {
        undo(&mut ed, &mut c, &ctx, false);
    }
    let back = t.elapsed();
    let first = c.body.clone();
    let t = std::time::Instant::now();
    for _ in 0..200 {
        undo(&mut ed, &mut c, &ctx, true);
    }
    let forward = t.elapsed();
    assert_eq!((&c.body, &c.styles), (&last.0, &last.1));
    assert_eq!(first, long_message(3_400).body);
    println!(
        "MEASURE undo, {} KB message, 200 steps: kept {:.1} MB; 200 steps {:?}, 200 undos {:?}, 200 redos {:?}",
        c.body.len() / 1024,
        kept as f64 / 1_048_576.0,
        steps,
        back,
        forward
    );
}

/// A repeatable stream of numbers below `m`, for the tests' scripted changes.
fn numbers(mut seed: u64) -> impl FnMut(usize) -> usize {
    move |m: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        usize::try_from(seed % u64::try_from(m.max(1)).unwrap_or(1)).unwrap_or(0)
    }
}

/// Undo's steps come back as they were: the newest is kept whole and the older ones as what
/// differs, and each is put back as a list of whole copies (as Undo kept them) has it, through
/// changes to text that isn't ASCII, to looks (past 64 of them, when unused ones are dropped and
/// the rest numbered again) and to paragraphs, more steps than are kept, and undos.
#[test]
fn undo_steps_come_back_as_they_were() {
    let mut rand = numbers(0x9E37_79B9_7F4A_7C15);
    let mut history = undo::History::default();
    let mut whole: Vec<undo::Snap> = Vec::new();
    let mut s = undo::Snap { text: "Hello".into(), styles: Styles::plain("Hello"), cursor: 0 };
    let pieces = ["a", "é", "東京", "🙂", "\n", "x\ny", "", "naïve café "];
    let (mut undos, mut trimmed) = (0, false);
    for _ in 0..3_000 {
        let n = s.text.chars().count();
        let a = rand(n + 1);
        let b = (a + rand(5)).min(n);
        match rand(5) {
            0..=2 => {
                let old = s.text.clone();
                let byte = |at: usize| old.char_indices().nth(at).map_or(old.len(), |(i, _)| i);
                let piece = pieces[rand(pieces.len())];
                s.text.replace_range(byte(a)..byte(b), piece);
                let look = Style { bold: rand(2) == 0, ..Style::default() };
                s.styles.edit(&old, &s.text, &look);
            }
            3 => {
                let size = u8::try_from(6 + rand(80)).unwrap_or(9);
                let color = [0, 0, u8::try_from(rand(3)).unwrap_or(0)];
                s.styles.restyle(&s.text, a..b, |st| Style { size: Some(size), color: Some(color), ..st.clone() });
            }
            _ => {
                let indent = u8::try_from(rand(4)).unwrap_or(0);
                s.styles.relayout(&s.text, a..b, |p| p.indent = indent);
            }
        }
        s.cursor = a;
        if rand(4) == 0 {
            let back = whole.pop();
            assert_eq!(history.pop(), back);
            if let Some(back) = back {
                s = back;
                undos += 1;
            }
        } else {
            history.push(s.clone());
            whole.push(s.clone());
            if whole.len() > undo::UNDO_MAX {
                whole.remove(0);
                trimmed = true;
            }
        }
        assert_eq!(history.len(), whole.len());
    }
    assert!(undos > 500 && trimmed && s.styles.palette.len() > 1, "the script went where it should: {undos} undos");
    // All the way back.
    while let Some(back) = whole.pop() {
        assert_eq!(history.pop(), Some(back));
    }
    assert!(history.pop().is_none() && history.is_empty() && history.bytes() == 0);
}

/// Undo keeps its steps within its room: the oldest go first, and the newest is kept however
/// big (a long message's steps were each a whole copy of it, 0.9 MB a step at 300 KB).
#[test]
fn undo_keeps_to_its_room() {
    let mut history = undo::History::default();
    let step = |k: usize| undo::Snap { text: ["a", "b"][k % 2].repeat((3 << 20) + k), styles: Styles::default(), cursor: k };
    for k in 0..30 {
        history.push(step(k));
        assert!(history.bytes() <= undo::UNDO_BYTES, "{} bytes after {k}", history.bytes());
    }
    assert!(history.len() < 15 && history.len() > 5, "{} steps", history.len());
    let kept = history.len();
    for k in (30 - kept..30).rev() {
        assert_eq!(history.pop(), Some(step(k)));
    }
    assert!(history.is_empty());
    let huge = undo::Snap { text: "x".repeat(undo::UNDO_BYTES + 1), ..Default::default() };
    history.push(step(0));
    history.push(huge.clone());
    assert_eq!(history.len(), 1);
    assert_eq!(history.pop(), Some(huge));
}

/// The selection drawn as it was, one cursor looked up from the first row for each end of each
/// character (what `selection_rects` was).
fn selection_rects_one_by_one(g: &Galley, sel: Range<usize>) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    for i in sel.start..sel.end.min(sel.start + 20_000) {
        let a = g.pos_from_cursor(CCursor::new(i));
        let b = g.pos_from_cursor(CCursor::new(i + 1));
        let right = if (b.min.y - a.min.y).abs() < 0.5 { b.min.x } else { a.min.x + 5.0 };
        let r = Rect::from_min_max(egui::pos2(a.min.x, a.min.y), egui::pos2(right.max(a.min.x), a.max.y));
        match out.last_mut() {
            Some(last) if (last.min.y - r.min.y).abs() < 0.5 => *last = last.union(r),
            _ => out.push(r),
        }
    }
    out
}

/// A selection drawn while the text doesn't have the keyboard (a menu or the ribbon has it)
/// is where it was drawn, now with the rows walked once: in wrapped paragraphs, empty and
/// indented lines, lists, right-to-left and mixed paragraphs, pictures, a trailing line break,
/// and selections that run past the text's end or past the most drawn. Each cursor is where
/// egui puts it.
#[test]
fn a_selection_is_drawn_where_it_was() {
    let ctx = egui::Context::default();
    let t = Tokens::light();
    let long = "a long paragraph that wraps over several rows of the editor ".repeat(40);
    let texts = [
        String::new(),
        "one line".to_string(),
        format!("{long}\n\nshort\n\u{FFFC} after a picture\nשלום עולם, and English\nEnglish then עברית here\n{long}\n"),
        format!("first\n\n\n{long}\nזוהי פסקה ארוכה מאוד בעברית שתיכנס לכמה שורות כשהעורך צר {long}"),
        "x\n".repeat(600),
    ];
    for text in &texts {
        let mut st = Styles::plain(text);
        st.relayout(text, 0..text.chars().count().min(40), |p| p.list = List::Bullet);
        st.relayout(text, 9..12, |p| p.indent = 2);
        st.restyle(text, 3..30, |s| Style { bold: true, size: Some(20), ..s.clone() });
        for (i, ch) in text.chars().enumerate() {
            if ch == PICTURE {
                st.restyle(text, i..i + 1, |s| Style { picture: Some("cid".into()), ..s.clone() });
            }
        }
        let own = HashMap::from([("cid".to_string(), vec2(120.0, 60.0))]);
        let n = text.chars().count();
        for wrap in [250.0, 700.0] {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let g = layout::layout(ui, text, Lay { st: &st, plain: false, t: &t, wrap, own: &own, card: None });
                let mut carets = carets::Carets::new(&g);
                for i in 0..n + 3 {
                    assert_eq!(carets.at(i), g.pos_from_cursor(CCursor::new(i)), "{wrap}: cursor {i} of {n}");
                }
                assert_eq!(carets.at(0), g.pos_from_cursor(CCursor::new(0)), "asked again from the start");
                let sels = [0..n, 0..n + 4, n..n + 2, n / 3..n / 2, 1..2, 5..5, 2..25_000];
                for sel in sels {
                    assert_eq!(carets::selection_rects(&g, sel.clone()), selection_rects_one_by_one(&g, sel.clone()), "{wrap}: {sel:?} of {n}");
                }
            });
            out.textures_delta.clear();
        }
    }
}

/// One frame of the message window's editor: its ribbon, then the text in a scroll area as the
/// window has it. What was drawn.
fn editor_frame(ctx: &egui::Context, ed: &mut Editor, c: &mut Compose) -> Vec<egui::epaint::ClippedShape> {
    let t = Tokens::light();
    let clock = Clock::default();
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1000.0, 800.0))), ..Default::default() };
    let mut acts = Vec::new();
    let mut out = ctx.run_ui(raw, |ui| {
        ribbon(ui, &t, ed, c, &[], &clock, &mut acts);
        egui::ScrollArea::vertical().auto_shrink(false).id_salt("compose-body").show(ui, |ui| body(ui, &t, ed, c, None, &mut acts));
    });
    out.textures_delta.clear();
    out.shapes
}

/// What was drawn, hashed (without writing it all out).
fn drawn(frames: &[Vec<egui::epaint::ClippedShape>]) -> u64 {
    use std::hash::{Hash, Hasher};
    struct Feed<'a>(&'a mut std::collections::hash_map::DefaultHasher);
    impl std::fmt::Write for Feed<'_> {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            s.hash(self.0);
            Ok(())
        }
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let _ = std::fmt::write(&mut Feed(&mut h), format_args!("{frames:?}"));
    h.finish()
}

/// A long message with every kind of paragraph: bulleted, numbered and indented (some empty),
/// centred and right-aligned, right-to-left, a rule, and pictures in lines.
fn formatted_message(lines: usize) -> Compose {
    let body: Vec<String> = (0..lines)
        .map(|k| match k % 9 {
            3 => "שלום עולם, זוהי פסקה בעברית שנכתבה מימין לשמאל".to_string(),
            6 => String::new(),
            7 => format!("A picture \u{FFFC} in line {k}, then more text after it."),
            _ => format!("Paragraph {k}: the quick brown fox jumps over the lazy dog, naïve café 東京 — done."),
        })
        .collect();
    let mut c = Compose { body: body.join("\n"), ..Default::default() };
    c.styles = Styles::plain(&c.body);
    let mut at = 0;
    for (k, line) in body.iter().enumerate() {
        let para = at..at;
        match k % 10 {
            0..=2 => c.styles.relayout(&c.body, para, |p| p.list = List::Bullet),
            4 | 5 => c.styles.relayout(&c.body, para, |p| (p.list, p.indent) = (List::Number, 1)),
            6 => c.styles.relayout(&c.body, para, |p| p.indent = 2),
            8 => c.styles.relayout(&c.body, para, |p| p.align = if k % 20 == 8 { Align::Center } else { Align::Right }),
            _ => {}
        }
        if k % 9 == 7 {
            c.styles.restyle(&c.body, at + 10..at + 11, |s| Style { picture: Some("cid".into()), ..s.clone() });
        }
        if k == 50 {
            c.styles.relayout(&c.body, at..at, |p| p.rule = true);
        }
        at += line.chars().count() + 1;
    }
    c
}

/// MEASURE: what a frame of the editor did with a long message and no longer does, each against
/// what it does now (the median of 31 runs).
#[test]
#[ignore = "a measurement"]
fn measure_editor_frame_pieces() {
    use std::hint::black_box;
    fn median(mut f: impl FnMut()) -> std::time::Duration {
        let mut times: Vec<_> = (0..31)
            .map(|_| {
                let t = std::time::Instant::now();
                f();
                t.elapsed()
            })
            .collect();
        times.sort();
        times[15]
    }
    let ctx = egui::Context::default();
    let t = Tokens::light();
    for (name, c) in [("plain paragraphs", long_message(3_400)), ("every kind of paragraph", formatted_message(3_400))] {
        let n = c.body.chars().count();
        let at = n / 2;
        let starts: Vec<usize> = std::iter::once(0).chain(c.body.chars().enumerate().filter(|(_, ch)| *ch == '\n').map(|(i, _)| i + 1)).collect();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let own = HashMap::new();
            let copy = median(|| {
                black_box((c.body.clone(), c.styles.clone()));
            });
            let look_was = median(|| {
                black_box(typing_style_was(&c.body, &c.styles, at));
            });
            let look = median(|| {
                black_box(typing_style(&c.body, &c.styles, at));
            });
            let line_was = median(|| {
                let line = rich::lines_of(&c.body, at..at).0;
                let start = c.body.split('\n').take(line).map(|l| l.chars().count() + 1).sum::<usize>();
                black_box((line, start, c.body.split('\n').nth(line).is_some_and(str::is_empty)));
            });
            let line = median(|| {
                black_box(line_at(&c.body, at));
            });
            let chosen = median(|| {
                black_box(c.body.chars().take(n).collect::<String>());
            });
            let fresh = median(|| {
                black_box(layout::layout(ui, &c.body, Lay { st: &c.styles, plain: false, t: &t, wrap: 900.0, own: &own, card: None }));
            });
            let mut laid = layout::Laid::default();
            let kept = median(|| {
                black_box(layout::layout_kept(
                    &mut laid,
                    ui,
                    &c.body,
                    Lay { st: &c.styles, plain: false, t: &t, wrap: 900.0, own: &own, card: None },
                ));
            });
            let g = layout::layout(ui, &c.body, Lay { st: &c.styles, plain: false, t: &t, wrap: 900.0, own: &own, card: None });
            let lines_was = median(|| {
                for &i in &starts {
                    black_box(g.pos_from_cursor(CCursor::new(i)));
                }
            });
            let lines = median(|| {
                let mut carets = carets::Carets::new(&g);
                for &i in &starts {
                    black_box(carets.at(i));
                }
            });
            println!(
                "MEASURE frame pieces, {} KB, {name}: text and looks copied {copy:?} -> none; typing look {look_was:?} -> {look:?}; \
                 cursor's line {line_was:?} -> {line:?}; all selected copied {chosen:?} -> none; layout {fresh:?} -> {kept:?}; \
                 lines' places {lines_was:?} -> {lines:?}",
                c.body.len() / 1024
            );
        });
        out.textures_delta.clear();
    }
}

/// MEASURE: a frame of the editor with a long message: the cursor in it, everything selected
/// in it, and a selection drawn while the text doesn't have the keyboard; and the cursor in a
/// long message with every kind of paragraph. What's drawn hashed, to show it's the same.
#[test]
#[ignore = "a measurement"]
fn measure_editor_frames_with_a_long_message() {
    let mut c = long_message(3_400);
    c.id = 7;
    let n = c.body.chars().count();
    let id = body_id(&c);
    let cases = [
        ("cursor", n / 2..n / 2, true, false),
        ("all selected", 0..n, true, false),
        ("selection drawn", 1_000..21_000, false, false),
        ("cursor, formatted", 0..0, true, true),
    ];
    for (case, sel, focus, formatted) in cases {
        let mut c = if formatted { formatted_message(3_400) } else { c.clone() };
        c.id = 7;
        let sel = if formatted { c.body.chars().count() / 2..c.body.chars().count() / 2 } else { sel };
        let ctx = egui::Context::default();
        let mut ed = Editor::new(c.id);
        set_selection(&ctx, id, sel.clone());
        ed.sel = sel;
        if focus {
            ctx.memory_mut(|m| m.request_focus(id));
        }
        let mut frames: Vec<_> = (0..3).map(|_| editor_frame(&ctx, &mut ed, &mut c)).collect();
        let mut times = Vec::new();
        for _ in 0..20 {
            let t = std::time::Instant::now();
            frames.push(editor_frame(&ctx, &mut ed, &mut c));
            times.push(t.elapsed());
        }
        times.sort();
        let (least, middle) = (times[0], times[times.len() / 2]);
        println!(
            "MEASURE editor frame, {} KB message, {case}: least {least:?}, median {middle:?} a frame; drawn {:016x}",
            c.body.len() / 1024,
            drawn(&frames)
        );
    }
}

/// As typing_style was: the text's characters copied into a list first.
fn typing_style_was(text: &str, styles: &Styles, at: usize) -> Style {
    let chars: Vec<char> = text.chars().collect();
    let before = at.checked_sub(1).filter(|i| chars.get(*i).is_some_and(|c| *c != '\n'));
    let i = before.or_else(|| chars.get(at).filter(|c| **c != '\n').map(|_| at));
    let Some(i) = i else { return Style::default() };
    let mut s = styles.style(i).for_typing();
    if s.link.is_some() && styles.style(at).link != s.link {
        s.link = None;
    }
    s
}

/// The look typed text takes and the line the cursor is in are what they were, now found
/// without copying the text out (the ribbon and the shortcuts ask every frame): at every place
/// in texts with empty lines, a line break first or last, letters that aren't ASCII, and places
/// past the end.
#[test]
fn typing_look_and_line_are_as_they_were() {
    for text in ["", "\n", "a", "ab\n\ncd", "\nnaïve\n東京🙂\n", "x\n\n\ny", "line one\nline two"] {
        let mut st = Styles::plain(text);
        let n = text.chars().count();
        for i in 0..n {
            st.restyle(text, i..i + 1, |s| Style { size: Some(8 + u8::try_from(i % 5).unwrap_or(0)), ..s.clone() });
        }
        st.restyle(text, 1..3, |s| Style { link: Some("https://example.com".into()), ..s.clone() });
        for at in 0..n + 3 {
            assert_eq!(typing_style(text, &st, at), typing_style_was(text, &st, at), "{text:?} at {at}");
            let line = rich::lines_of(text, at..at).0;
            let start = text.split('\n').take(line).map(|l| l.chars().count() + 1).sum::<usize>();
            let empty = text.split('\n').nth(line).is_some_and(str::is_empty);
            assert_eq!(line_at(text, at), (line, start, empty), "{text:?} at {at}");
        }
    }
}

/// The text box's text knows what it was only when the box changed it.
#[test]
fn the_text_box_text_knows_what_it_was() {
    use egui::TextBuffer;
    use egui::text::CharIndex;
    let mut text = "naïve café".to_string();
    assert_eq!(Tracked::new(&mut text).before(), None);
    let mut t = Tracked::new(&mut text);
    assert_eq!(t.insert_text("très ", CharIndex(6)), 5);
    t.delete_char_range(CharIndex(0)..CharIndex(1));
    assert_eq!(t.as_str(), "aïve très café");
    assert_eq!(t.before().as_deref(), Some("naïve café"));
    assert_eq!(text, "aïve très café");
    // Changed and changed back: as it was.
    let mut t = Tracked::new(&mut text);
    t.insert_text("x", CharIndex(0));
    t.delete_char_range(CharIndex(0)..CharIndex(1));
    assert_eq!(t.before(), None);
    let mut t = Tracked::new(&mut text);
    t.replace_with("new");
    assert_eq!(t.before().as_deref(), Some("aïve très café"));
    assert_eq!(text, "new");
}

/// The text laid out last is given again only while it would be laid out the same: as laid out
/// afresh after its paragraphs' alignment, its width (by less than egui rounds to) or plain text
/// change.
#[test]
fn the_text_laid_out_is_kept_only_while_the_same() {
    let ctx = egui::Context::default();
    let t = Tokens::light();
    let c = formatted_message(60);
    let mut laid = layout::Laid::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let own = HashMap::new();
        let fresh =
            |st: &Styles, plain: bool, wrap: f32| format!("{:?}", layout::layout(ui, &c.body, Lay { st, plain, t: &t, wrap, own: &own, card: None }));
        let mut kept =
            |st: &Styles, plain: bool, wrap: f32| layout::layout_kept(&mut laid, ui, &c.body, Lay { st, plain, t: &t, wrap, own: &own, card: None });
        let a = kept(&c.styles, false, 900.4);
        let b = kept(&c.styles, false, 900.4);
        assert!(Arc::ptr_eq(&a, &b), "given again");
        assert!(format!("{a:?}") == fresh(&c.styles, false, 900.4), "as laid out afresh");
        let mut right = c.styles.clone();
        right.relayout(&c.body, 0..1, |p| p.align = Align::Right);
        assert!(fresh(&right, false, 900.4) != fresh(&c.styles, false, 900.4), "the first line moved right");
        assert!(format!("{:?}", kept(&right, false, 900.4)) == fresh(&right, false, 900.4), "aligned again");
        assert!(fresh(&right, false, 899.6) != fresh(&right, false, 900.4), "it moved with the width");
        assert!(format!("{:?}", kept(&right, false, 899.6)) == fresh(&right, false, 899.6), "a little narrower");
        assert!(format!("{:?}", kept(&right, true, 899.6)) == fresh(&right, true, 899.6), "plain");
    });
    out.textures_delta.clear();
}

/// The pictures from the web the editors are given are those the reader has, as they change:
/// kept from frame to frame while the same (not copied each frame), made again when one is
/// loaded, loaded again or let go.
#[test]
fn the_web_pictures_given_follow_the_readers() {
    use mailcraft_engine::Picture;
    let ctx = egui::Context::default();
    let given = || -> pictures::WebPictures { ctx.data(|d| d.get_temp(pictures::web_id())).unwrap_or_default() };
    let bytes = |b: u8| Arc::new(vec![b; 4]);
    let mut session: HashMap<String, Picture> = HashMap::new();
    session.insert("https://x.example/a.png".into(), Picture::Ready(bytes(1)));
    session.insert("https://x.example/b.png".into(), Picture::Loading);
    pictures::web_loaded(&ctx, &session);
    let first = given();
    assert_eq!(first.keys().collect::<Vec<_>>(), ["https://x.example/a.png"]);
    pictures::web_loaded(&ctx, &session);
    assert!(Arc::ptr_eq(&first, &given()), "the same: kept");
    // Loaded: there; loaded again (new bytes, same address): the new ones; let go: gone.
    session.insert("https://x.example/b.png".into(), Picture::Ready(bytes(2)));
    pictures::web_loaded(&ctx, &session);
    assert_eq!(given().get("https://x.example/b.png").map(|d| d[0]), Some(2));
    session.insert("https://x.example/a.png".into(), Picture::Ready(bytes(3)));
    pictures::web_loaded(&ctx, &session);
    assert_eq!(given().get("https://x.example/a.png").map(|d| d[0]), Some(3));
    session.remove("https://x.example/a.png");
    pictures::web_loaded(&ctx, &session);
    assert_eq!(given().keys().collect::<Vec<_>>(), ["https://x.example/b.png"]);
}
