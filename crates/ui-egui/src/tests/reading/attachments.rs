//! UI tests: attachments previewed in the reading pane, and opened.

use super::*;

#[test]
fn every_kind_of_attachment_previews_in_the_reading_pane() {
    let (mut a, _) = app("attachment-preview");
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
}

/// Back from a previewed attachment, its chip isn't left selected.
#[test]
fn leaving_an_attachment_preview_lets_go_of_its_chip() {
    let (mut a, _) = app("attach-back");
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
        let mut mores: Vec<egui::Rect> =
            named_in(&out).into_iter().filter(|n| n.label.as_deref() == Some("More options")).filter_map(|n| n.rect).collect();
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
    let (mut a, _) = app("attach-double");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let raw = b"From: Sam <sam@studio.example>\r\nSubject: Forwarded with a file\r\nMIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain\r\n\r\nSee below.\r\n\
        --b1\r\nContent-Type: message/rfc822\r\nContent-Disposition: attachment; filename=\"earlier.eml\"\r\n\r\n\
        From: Zoe <zoe@studio.example>\r\nSubject: Earlier\r\n\r\nThe first one.\r\n\
        --b1\r\nContent-Type: application/x-unknown\r\nContent-Disposition: attachment; filename=\"data.qqq\"\r\n\r\nxyz\r\n--b1--\r\n";
    with_raw(&mut a, 9901, raw);
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
}

/// The same in a message's own window: a double-click only opens, a click previews.
#[test]
fn a_double_clicked_attachment_only_opens_in_a_message_window() {
    let (mut a, _) = app("attach-double-window");
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
}

/// The text read out of a frame (AccessKit's labels and values).
fn read_out(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    named(ctx, a, egui::vec2(1280.0, 800.0)).iter().filter_map(Named::name).map(str::to_string).collect()
}

/// How many attachments' previews are kept, read and waiting to be shown again.
fn previews_kept(ctx: &egui::Context, a: &MailApp) -> usize {
    ctx.data(|d| d.count::<std::sync::Arc<crate::reading::Shown>>()) + usize::from(a.ui.attachment_shown.is_some())
}

/// A file previewed while its message's text had come first (issue #24) has no preview yet; once
/// the whole message is here, the same preview shows the file. It said "no preview" for good.
#[test]
fn a_file_previewed_before_it_came_shows_once_it_does() {
    let (mut a, _) = app("attach-came");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| !m.has_attachments).map(|m| m.key.clone()).unwrap();
    a.session.open_message(&key);
    let d = mailcraft_engine::mime::Draft { subject: "Notes".into(), body: "The notes are attached.".into(), ..Default::default() };
    let file = mailcraft_engine::mime::PendingFile { name: "notes.txt".into(), mime: "text/plain".into(), size: 40, ..Default::default() };
    let from = mailcraft_engine::model::EmailAddress::new("Sam", "sam@studio.example");
    let first = mailcraft_engine::mime::text_first(&d, &from, NOW, "<n@x>", &[file]).unwrap();
    a.session.apply(MailEvent::Body { key: key.clone(), raw: first });
    frame(&ctx, &mut a, vec![]);
    let notes = |a: &MailApp| {
        a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).and_then(|p| p.attachments.iter().find(|x| x.name == "notes.txt")).map(|x| x.index)
    };
    let index = notes(&a).expect("the file named");
    a.ui.attachment_view = Some((key.clone(), index));
    let shown = read_out(&ctx, &mut a);
    assert!(shown.iter().any(|l| l.contains("There's no preview for this file.")), "{shown:?}");
    // The whole message.
    let whole = b"From: Sam <sam@studio.example>\r\nSubject: Notes\r\nMessage-ID: <n@x>\r\nMIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
        The notes are attached.\r\n--b1\r\nContent-Type: text/plain; charset=utf-8\r\n\
        Content-Disposition: attachment; filename=\"notes.txt\"\r\n\r\nThe first line of the notes.\r\n--b1--\r\n";
    a.session.store.as_ref().unwrap().write_body(&key, whole).unwrap();
    a.session.apply(MailEvent::Body { key: key.clone(), raw: whole.to_vec() });
    frame(&ctx, &mut a, vec![]);
    assert!(!a.session.reading.as_ref().is_some_and(|r| r.text_first), "the whole message shown");
    assert_eq!(notes(&a), Some(index), "the same file");
    assert_eq!(a.ui.attachment_view, Some((key, index)));
    let shown = read_out(&ctx, &mut a);
    assert!(shown.iter().any(|l| l.contains("The first line of the notes.")), "{shown:?}");
    assert!(!shown.iter().any(|l| l.contains("There's no preview for this file.")), "{shown:?}");
}

/// One attachment after another previewed keeps one preview at most (each was kept for the
/// session, a whole Office file's or 300,000 characters' worth), none once back at the
/// message; and the one shown isn't read again frame after frame.
#[test]
fn previews_of_many_attachments_are_not_all_kept() {
    let (mut a, _) = app("attach-kept");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let with: Vec<MessageKey> = a.session.list.iter().filter(|m| m.has_attachments).map(|m| m.key.clone()).collect();
    let mut previewed = 0;
    for k in &with {
        a.run("list.open", json!({"key": k.to_id()}));
        frame(&ctx, &mut a, vec![]);
        let indices: Vec<usize> =
            a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments.iter().map(|x| x.index).collect()).unwrap_or_default();
        for i in indices {
            a.ui.attachment_view = Some((k.clone(), i));
            frame(&ctx, &mut a, vec![]);
            previewed += 1;
            assert!(previews_kept(&ctx, &a) <= 1, "{} kept after {previewed} previewed", previews_kept(&ctx, &a));
        }
    }
    assert!(previewed >= 5, "{previewed} previewed");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_view.is_none());
    assert_eq!(previews_kept(&ctx, &a), 0, "let go of with the preview");
    // A text file shown for a while: read once.
    let k = a.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let notes = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).and_then(|p| p.attachments.first()).map(|x| x.index).unwrap();
    a.ui.attachment_view = Some((k, notes));
    frame(&ctx, &mut a, vec![]);
    let reads = a.session.store.as_ref().map_or(0, |s| s.reads());
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(a.session.store.as_ref().map_or(0, |s| s.reads()), reads, "read again while shown");
}

/// Issue #120: an attachment dragged out of a message's own window was dropped as a file called
/// "attachment": the window's message, opened in the main window as the drag began, was still
/// being read there, on another thread, and the name was looked for in that. It keeps its name.
#[test]
fn an_attachment_dragged_out_of_a_message_window_keeps_its_name() {
    let (mut a, _) = app("attach-drag-window");
    a.services = Box::new(Recorder(Default::default()));
    a.ui.can_drag_out = true;
    let with = a.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
    a.run("message.popOut", json!({"key": with.to_id()}));
    let w = a.session.windows.iter().find(|w| w.key == with).cloned().expect("the window");
    let pdf = w.parsed.as_ref().and_then(|p| p.attachments.get(1)).cloned().expect("its second attachment");
    // As the window's drag goes: its message opened in the main window, read on another thread,
    // then the drag.
    a.session.defer_bodies(true);
    a.act(crate::Action::Open(with.clone()));
    assert!(a.session.reading.as_ref().is_some_and(|r| r.key == with && r.parsed.is_none()), "still being read");
    a.act(crate::Action::InWindow(with.clone(), Box::new(crate::Action::DragOut(vec![pdf.index]))));
    let files = a.take_drag_out();
    let names: Vec<String> = files.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).collect();
    assert_eq!(names, std::slice::from_ref(&pdf.name));
    assert!(files.iter().all(|p| p.is_file()), "{files:?}");
}

/// Issue #120: attachments selected together drag out together, as Save Selected saves them (only
/// the one under the pointer went); a drag on one not selected takes it alone. Two of the same
/// name both go, under that name, neither written over the other.
#[test]
fn selected_attachments_drag_out_together() {
    let (mut a, _) = app("attach-drag-several");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.services = Box::new(Recorder(Default::default()));
    a.ui.can_drag_out = true;
    let mut clock = Clocked(1.0);
    let k = a.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let mut chips = Vec::new();
    for _ in 0..3 {
        chips = clock.frame(&ctx, &mut a, vec![], 0.5);
    }
    assert_eq!(chips.len(), 4);
    let attachments = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments.clone()).unwrap_or_default();
    let name = |p: &std::path::PathBuf| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let body = |more: egui::Rect| egui::pos2(more.left() - 60.0, more.center().y);
    // The first and the third, picked with Ctrl.
    for chip in [chips[0], chips[2]] {
        let at = body(chip);
        let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::COMMAND };
        clock.frame(&ctx, &mut a, vec![Event::ModifiersChanged(Modifiers::COMMAND), Event::PointerMoved(at)], 0.05);
        clock.frame(&ctx, &mut a, vec![press(true)], 0.05);
        clock.frame(&ctx, &mut a, vec![press(false)], 0.05);
    }
    clock.frame(&ctx, &mut a, vec![Event::ModifiersChanged(Modifiers::NONE)], 0.5);
    assert_eq!(a.ui.attachment_selected.as_ref().map(|(_, v)| v.len()), Some(2));
    // A drag from `chip`: the files it hands the system.
    let mut drag_from = |a: &mut MailApp, chip: egui::Rect| {
        let at = body(chip);
        let to = at + egui::vec2(-40.0, 30.0);
        let press = |pressed, pos| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        clock.frame(&ctx, a, vec![Event::PointerMoved(at)], 0.05);
        clock.frame(&ctx, a, vec![press(true, at)], 0.05);
        clock.frame(&ctx, a, vec![Event::PointerMoved(at + egui::vec2(-20.0, 15.0))], 0.05);
        clock.frame(&ctx, a, vec![Event::PointerMoved(to)], 0.05);
        let files = a.take_drag_out();
        clock.frame(&ctx, a, vec![press(false, to)], 0.05);
        clock.frame(&ctx, a, vec![], 0.5);
        files
    };
    let both = drag_from(&mut a, chips[2]);
    assert_eq!(both.iter().map(name).collect::<Vec<_>>(), [attachments[0].name.clone(), attachments[2].name.clone()]);
    assert!(both.iter().all(|p| p.is_file()), "{both:?}");
    let alone = drag_from(&mut a, chips[1]);
    assert_eq!(alone.iter().map(name).collect::<Vec<_>>(), [attachments[1].name.clone()], "one not selected goes alone");
    // The same name twice: two files of that name, in folders of their own.
    a.act(crate::Action::DragOut(vec![attachments[0].index, attachments[0].index]));
    let twice = a.take_drag_out();
    assert_eq!(twice.len(), 2);
    assert!(twice[0] != twice[1] && twice.iter().all(|p| p.is_file() && name(p) == attachments[0].name), "{twice:?}");
}

/// Services that remember the files handed to their app.
#[cfg(windows)]
struct Opens(std::rc::Rc<std::cell::RefCell<Vec<std::path::PathBuf>>>);

#[cfg(windows)]
impl crate::Services for Opens {
    stub_services!(clock, pick_files, save_path);
    fn open_path(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.0.borrow_mut().push(path.to_path_buf());
        Ok(())
    }
}

/// Issue #260: an attachment opened again while its app still had it open failed with "The
/// process cannot access the file because it is being used by another process": Word keeps the
/// file it shows, and Windows won't let it be written. The file there is that attachment, so it's
/// opened as it is (Word comes to it). One of the same name that isn't the attachment goes in a
/// folder of its own, the other left alone. Windows alone: elsewhere a file open in an app can be
/// written.
#[cfg(windows)]
#[test]
fn an_attachment_its_app_still_has_open_opens_again() {
    use std::os::windows::fs::OpenOptionsExt;
    let (mut a, _) = app("attach-open-held");
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    a.services = Box::new(Opens(opened.clone()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let doc = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).and_then(|p| p.attachments.get(2)).cloned().expect("the document");
    let bytes = a.session.attachment_bytes(doc.index).unwrap();
    let errors = |a: &MailApp| a.session.notices.iter().filter(|n| n.error).map(|n| n.text.clone()).collect::<Vec<_>>();
    // Held as Word holds a document it shows: no one else may write it.
    let hold = |p: &std::path::Path| std::fs::OpenOptions::new().read(true).share_mode(0).open(p).unwrap();

    a.act(crate::Action::OpenAttachment(doc.index));
    let first = opened.borrow().last().cloned().expect("opened");
    let held = hold(&first);
    a.act(crate::Action::OpenAttachment(doc.index));
    assert_eq!(errors(&a), Vec::<String>::new());
    assert_eq!(*opened.borrow(), [first.clone(), first.clone()], "the same file, opened again");
    drop(held);

    // The file there now isn't this attachment (another of its name, or an earlier MailCraft's).
    std::fs::write(&first, b"another file of the same name").unwrap();
    a.ui.opened_files.clear();
    let held = hold(&first);
    a.act(crate::Action::OpenAttachment(doc.index));
    assert_eq!(errors(&a), Vec::<String>::new());
    let beside = opened.borrow().last().cloned().unwrap();
    assert!(beside != first && beside.file_name() == first.file_name(), "{beside:?}");
    assert_eq!(std::fs::read(&beside).unwrap(), bytes);
    drop(held);
    assert_eq!(std::fs::read(&first).unwrap(), b"another file of the same name", "left alone");
}

/// A note's file opened again while its app still has it open: the same as an attachment's (issue
/// #260), as they're handed over from the same folder.
#[cfg(windows)]
#[test]
fn a_note_s_file_its_app_still_has_open_opens_again() {
    use std::os::windows::fs::OpenOptionsExt;
    let (mut a, _) = app("note-file-open-held");
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    a.services = Box::new(Opens(opened.clone()));
    a.run("view.module", json!({"name": "notes"}));
    let uid = a.session.notes("")[0].note.uid.clone();
    let line = "ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"swatch.pdf\":JVBERi0xLjc=".to_string();
    for p in a.session.pim.values_mut() {
        for n in p.notes.iter_mut().filter(|n| n.note.uid == uid) {
            n.note.files = vec![line.clone()];
        }
    }
    a.act(crate::Action::NoteFile { uid: uid.clone(), index: 0, open: true });
    let first = opened.borrow().last().cloned().expect("opened");
    let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&first).unwrap();
    a.act(crate::Action::NoteFile { uid, index: 0, open: true });
    assert!(a.session.notices.iter().all(|n| !n.error), "{:?}", a.session.notices);
    assert_eq!(*opened.borrow(), [first.clone(), first], "the same file, opened again");
    drop(held);
}
