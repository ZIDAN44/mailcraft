//! UI tests: links in formatted mail say where they open, pictures' addresses are told apart
//! (in the message or on the web) by how they start, and lines are as wide as they measure.

use super::*;

/// A formatted message with one tracked link, opened.
fn open_with_link(a: &mut MailApp) -> MessageKey {
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9970 };
    let raw = b"From: Shop <news@shop.example>\r\nSubject: Fall\r\nContent-Type: text/html\r\n\r\n<p>See <a href=\"https://shop.example/coat?utm_source=news&id=7\">the winter coat</a> today.</p>";
    inbox.messages.insert(0, mailcraft_engine::mime::summary_from_full(raw, key.clone(), Default::default(), 0, NOW).into());
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
    key
}

/// Hovered, a link says where it opens, cleaned of its tracking tags (or as it is, when links
/// aren't cleaned); a screen reader is told the same of it. Each is written only then.
#[test]
fn a_link_says_where_it_opens_when_hovered() {
    let size = egui::vec2(1280.0, 800.0);
    for clean in [true, false] {
        let (mut a, _) = app(&format!("link-hover-{clean}"));
        a.session.prefs.clean_links = clean;
        let ctx = egui::Context::default();
        frame(&ctx, &mut a, vec![]);
        open_with_link(&mut a);
        let drawn = drawn_text(&ctx, &mut a, size);
        let shown = if clean { "https://shop.example/coat?id=7" } else { "https://shop.example/coat?utm_source=news&id=7" };
        assert!(!drawn.iter().any(|d| d.text.contains("shop.example/coat")), "not before it's hovered");
        // Its note drawn as when it's hovered (egui's way to show every widget's note at once).
        ctx.memory_mut(|m| m.set_everything_is_visible(true));
        let link = drawn.iter().find(|d| d.text == "the winter coat").expect("the link").rect.center();
        sized_frame(&ctx, &mut a, size, vec![Event::PointerMoved(link)]);
        sized_frame(&ctx, &mut a, size, vec![]);
        let texts = texts_in(&sized_frame(&ctx, &mut a, size, vec![]).shapes);
        ctx.memory_mut(|m| m.set_everything_is_visible(false));
        assert!(texts.iter().any(|d| d.text == shown), "hovered, it says {shown}");
        // A screen reader is told it too.
        ctx.enable_accesskit();
        let out = sized_frame(&ctx, &mut a, size, vec![]);
        let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
        assert!(tree.nodes.iter().any(|(_, n)| n.description() == Some(shown)), "described as {shown}");
    }
}

/// A picture's address is on the web by its first letters, in any case, as when the whole
/// address was lowered to look.
#[test]
fn a_pictures_address_is_on_the_web_as_before() {
    let lowered = |src: &str| {
        let lower = src.to_ascii_lowercase();
        lower.starts_with("http://") || lower.starts_with("https://")
    };
    let cases = [
        "https://x.example/a.png",
        "HTTP://X.EXAMPLE/A.PNG",
        "Https://x",
        "http://",
        "https:/",
        "http:",
        "cid:logo@x",
        "",
        "h",
        "httpś://x",
        "ĥttps://x",
        "https:/\u{e9}",
        "https:\u{e9}//",
        " https://x",
        "file:///c:/x.png",
        "logo.png",
    ];
    for src in cases {
        assert_eq!(crate::reading::body::on_the_web(src), lowered(src), "{src:?}");
    }
    // Every string of up to four letters from a few that matter, and each address cut short.
    let letters = ['h', 'H', 't', 'T', 'p', 's', 'S', ':', '/', 'é', 'x'];
    let mut n = 0;
    for len in 0..=4u32 {
        for i in 0..letters.len().pow(len) {
            let mut s = String::from("http");
            let mut i = i;
            for _ in 0..len {
                s.push(letters[i % letters.len()]);
                i /= letters.len();
            }
            for cut in 0..=s.len() {
                if let Some(part) = s.get(cut..) {
                    assert_eq!(crate::reading::body::on_the_web(part), lowered(part), "{part:?}");
                    n += 1;
                }
            }
        }
    }
    assert!(n > 50_000, "{n}");
}

/// A line's width, kept from frame to frame, is what measuring its spans says, the first time
/// and after; a line of other words or fonts, or at another zoom, has its own.
#[test]
fn a_lines_width_is_as_measured() {
    use mailcraft_engine::mime::{Rise, Span};
    let ctx = egui::Context::default();
    let words = ["Read", " the story ", "Fall", "save 20%", "", " ", "\u{e9}t\u{e9}", "\u{65e5}\u{672c}"];
    let mut lines: Vec<Vec<Span>> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        for style in 0..7u8 {
            let sp = Span {
                text: (*w).to_string(),
                bold: style == 1,
                code: style == 2,
                heading: if style == 3 { 2 } else { 0 },
                size: (style == 4).then_some(20),
                rise: if style == 5 {
                    Rise::Sup
                } else if style == 6 {
                    Rise::Sub
                } else {
                    Rise::Normal
                },
                ..Default::default()
            };
            lines.push(vec![sp.clone()]);
            lines.push(vec![sp, Span { text: words[(i + 1) % words.len()].to_string(), ..Default::default() }]);
        }
    }
    for zoom in [1.0, 1.5] {
        ctx.set_pixels_per_point(zoom);
        let mut out = ctx.run_ui(RawInput::default(), |ui| {
            for _ in 0..2 {
                for line in &lines {
                    let measured: f32 = line
                        .iter()
                        .map(|sp| {
                            ui.fonts_mut(|f| f.layout_no_wrap(sp.text.clone(), crate::reading::body::span_font(sp), egui::Color32::WHITE).size().x)
                        })
                        .sum();
                    assert_eq!(crate::reading::body::line_width(ui, line), measured, "{zoom} {line:?}");
                }
            }
        });
        out.textures_delta.clear();
    }
}

/// Issue #225: an Apple Mail rich link card (a link around a table holding a picture written into
/// the HTML as `data:image/png;base64,…`) shows its picture, which is part of the message (no
/// asking, as for one on the web), and the picture opens the link it's in. Only an empty box was
/// drawn, and nothing about it could be clicked.
#[test]
fn a_linked_picture_written_into_the_message_shows_and_opens_its_link() {
    use egui::{Pos2, RawInput, Rect, Shape};
    let (mut a, _) = app("rich-link-card");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9972 };
    let png = mailcraft_engine::model::encoding::base64::encode(&super::super::pictures::png(275, 160));
    let html = format!(
        "<html><body dir=\"auto\"><div dir=\"ltr\">Open my shared album:<br><div>\
         <a style=\"border-radius:10px;display:block;width:275px;overflow:hidden;text-decoration:none;\" class=\"lp-rich-link\" rel=\"nofollow\" \
         href=\"https://photos.example/shared/album/EXAMPLE\" dir=\"ltr\" role=\"button\" draggable=\"false\" width=\"275\">\
         <table style=\"table-layout:fixed;border-collapse:collapse;width:275px;background-color:#E9E9EB;\" class=\"lp-rich-link-emailBaseTable\" \
         cellpadding=\"0\" cellspacing=\"0\" border=\"0\" width=\"275\"><tbody><tr><td align=\"center\">\
         <img src=\"data:image/png;base64,{png}\" /></td></tr></tbody></table></a></div></div></body></html>"
    );
    let raw = format!("From: Ana <ana@studio.example>\r\nSubject: My album\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}").into_bytes();
    inbox.messages.insert(0, mailcraft_engine::mime::summary_from_full(&raw, key.clone(), Default::default(), 0, NOW).into());
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
    // Where the frame paints a picture (a rectangle filled with a texture).
    let paint = |a: &mut MailApp, events: Vec<Event>| -> Option<Rect> {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        fn walk(s: &Shape, found: &mut Option<Rect>) {
            match s {
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, found)),
                Shape::Rect(r) if r.fill_texture_id() != egui::TextureId::default() && r.rect.width() > 100.0 => *found = Some(r.rect),
                _ => {}
            }
        }
        let mut found = None;
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut found));
        found
    };
    let mut at = None;
    for _ in 0..30 {
        at = at.or(paint(&mut a, vec![]));
    }
    let at = at.expect("the card's picture is drawn");
    a.session.notices.clear();
    click(&ctx, &mut a, at.center());
    frame(&ctx, &mut a, vec![]);
    // The test's services open nothing: that they were asked to is the notice they give.
    assert!(a.session.notices.iter().any(|n| n.text.contains("The link couldn't be opened")), "{:?}", a.session.notices);
}
