//! UI tests: attachments previewed in the reading pane, and opened.

use super::*;

#[test]
fn every_kind_of_attachment_previews_in_the_reading_pane() {
    let (mut a, dir) = app("attachment-preview");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let n = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map_or(0, |p| p.attachments.len());
    assert_eq!(n, 4, "notes, a PDF, a document and a workbook");
    for i in 0..n {
        a.ui.attachment_view = Some((k.clone(), i));
        // A PDF draws a page a frame.
        for _ in 0..3 {
            frame(&ctx, &mut a, vec![]);
        }
        assert_eq!(a.ui.attachment_view, Some((k.clone(), i)));
    }
    // Escape: back to the message.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_view.is_none());
    // One that isn't there any more (another message's index) is just the message.
    a.ui.attachment_view = Some((k.clone(), 99));
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Back from a previewed attachment, its chip isn't left selected.
#[test]
fn leaving_an_attachment_preview_lets_go_of_its_chip() {
    let (mut a, dir) = app("attach-back");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.contains("festival poster")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let index = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments[0].index).unwrap();
    a.ui.attachment_view = Some((k.clone(), index));
    a.ui.attachment_selected = Some((k.clone(), vec![index]));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_view.is_none() && a.ui.attachment_selected.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Frames at a time of the test's own, so a double-click's two clicks can come closer together
/// than the double-click time, and the time can run out.
struct Clocked(f64);

impl Clocked {
    /// A frame `step` seconds after the last; each chip's ▾ (the chip's body is to its left), in
    /// reading order.
    fn frame(&mut self, ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>, step: f64) -> Vec<egui::Rect> {
        self.0 += step;
        let input = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            time: Some(self.0),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        let mut mores: Vec<egui::Rect> = nodes
            .iter()
            .filter(|(_, n)| n.label() == Some("More options"))
            .filter_map(|(_, n)| n.bounds())
            .map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
            .collect();
        mores.sort_by(|p, q| p.top().total_cmp(&q.top()).then(p.left().total_cmp(&q.left())));
        mores
    }

    /// A double-click on the chip whose ▾ is `more`, then a click alone: the double-click opens
    /// the file (the test's services say it can't) and leaves no preview, then or after; the click
    /// previews it once the double-click time is out. What it previews, if anything.
    fn double_then_single(&mut self, ctx: &egui::Context, a: &mut MailApp, more: egui::Rect, what: &str) -> Option<(MessageKey, usize)> {
        let body = egui::pos2(more.left() - 60.0, more.center().y);
        let press = |pressed| Event::PointerButton { pos: body, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let notices = a.session.notices.len();
        self.frame(ctx, a, vec![Event::PointerMoved(body)], 0.05);
        self.frame(ctx, a, vec![press(true)], 0.05);
        self.frame(ctx, a, vec![press(false)], 0.05);
        assert!(a.ui.attachment_view.is_none(), "{what}: a click waits out the double-click time");
        self.frame(ctx, a, vec![press(true)], 0.05);
        self.frame(ctx, a, vec![press(false)], 0.05);
        for _ in 0..3 {
            self.frame(ctx, a, vec![], 0.4);
        }
        assert!(a.ui.attachment_view.is_none(), "{what}: a double-click left a preview");
        assert!(a.session.notices.len() > notices, "{what}: a double-click opens the file");
        self.frame(ctx, a, vec![press(true)], 0.05);
        self.frame(ctx, a, vec![press(false)], 0.05);
        assert!(a.ui.attachment_view.is_none(), "{what}: a click waits out the double-click time");
        for _ in 0..2 {
            self.frame(ctx, a, vec![], 0.4);
        }
        a.ui.attachment_view.clone()
    }
}

/// A double-click on an attachment only opens it, as Outlook's: its first click showed the file
/// in the reading pane, left there behind the app it opened in. A click alone still previews it,
/// once the double-click time is out. Every kind: text, PDF, Office files, pictures, an attached
/// message and a file of no known kind.
#[test]
fn a_double_clicked_attachment_only_opens() {
    let (mut a, dir) = app("attach-double");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let raw = b"From: Sam <sam@studio.example>\r\nSubject: Forwarded with a file\r\nMIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain\r\n\r\nSee below.\r\n\
        --b1\r\nContent-Type: message/rfc822\r\nContent-Disposition: attachment; filename=\"earlier.eml\"\r\n\r\n\
        From: Zoe <zoe@studio.example>\r\nSubject: Earlier\r\n\r\nThe first one.\r\n\
        --b1\r\nContent-Type: application/x-unknown\r\nContent-Disposition: attachment; filename=\"data.qqq\"\r\n\r\nxyz\r\n--b1--\r\n";
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let own = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9901 };
    inbox.messages.insert(0, mailcraft_engine::mime::summary_from_full(raw, own.clone(), Default::default(), 0, NOW).into());
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&own, raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    let mut clock = Clocked(1.0);
    for subject in ["Your proof", "festival poster", "Forwarded with a file"] {
        let k = a.session.list.iter().find(|m| m.subject.contains(subject)).map(|m| m.key.clone()).unwrap();
        a.run("list.open", json!({"key": k.to_id()}));
        for _ in 0..3 {
            clock.frame(&ctx, &mut a, vec![], 0.5);
        }
        let indices: Vec<usize> =
            a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments.iter().map(|x| x.index).collect()).unwrap_or_default();
        assert!(indices.len() >= 2, "{subject}: {indices:?}");
        for (i, index) in indices.iter().enumerate() {
            let mut chips = Vec::new();
            for _ in 0..3 {
                chips = clock.frame(&ctx, &mut a, vec![], 0.5);
            }
            assert_eq!(chips.len(), indices.len(), "{subject}");
            let shown = clock.double_then_single(&ctx, &mut a, chips[i], &format!("{subject} #{i}"));
            assert_eq!(shown, Some((k.clone(), *index)), "{subject} #{i}: a click previews it");
            clock.frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)], 0.5);
            assert!(a.ui.attachment_view.is_none());
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The same in a message's own window: a double-click only opens, a click previews.
#[test]
fn a_double_clicked_attachment_only_opens_in_a_message_window() {
    let (mut a, dir) = app("attach-double-window");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut clock = Clocked(1.0);
    clock.frame(&ctx, &mut a, vec![], 0.5);
    // The reading pane shows a message without attachments: every chip is the window's.
    let plain = a.session.list.iter().find(|m| !m.has_attachments).map(|m| m.key.clone()).unwrap();
    let with = a.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
    a.run("message.popOut", json!({"key": with.to_id()}));
    a.run("list.open", json!({"key": plain.to_id()}));
    let mut chips = Vec::new();
    for _ in 0..4 {
        chips = clock.frame(&ctx, &mut a, vec![], 0.5);
    }
    assert_eq!(chips.len(), 4, "the window's four attachments");
    let w = a.session.windows.iter().find(|w| w.key == with).cloned().expect("the window");
    let index = w.parsed.as_ref().and_then(|p| p.attachments.get(1)).map(|x| x.index).unwrap();
    let shown = clock.double_then_single(&ctx, &mut a, chips[1], "the window's PDF");
    assert_eq!(shown, Some((with, index)), "a click previews it in the window");
    let _ = std::fs::remove_dir_all(dir);
}
