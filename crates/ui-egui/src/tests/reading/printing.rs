//! UI tests: a message printed (issue #291), again while the PDF viewer still has the first, and
//! with what the PDF's font doesn't have drawn by the window's own fallback fonts.

use super::*;

/// A message with these words in it, opened.
fn open_saying(a: &mut MailApp, uid: u32, html: &str) -> MessageKey {
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    let raw = format!("From: Ann <ann@news.example>\r\nSubject: Summit\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n").into_bytes();
    inbox.messages.insert(0, mailcraft_engine::mime::summary_from_full(&raw, key.clone(), Default::default(), 0, NOW).into());
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
    key
}

/// Issue #291: printing again while the PDF viewer still had the first failed with "The process
/// cannot access the file because it is being used by another process" (Print writes the same
/// name each time). The new one goes in a numbered folder beside it, under the same name, and is
/// opened; the one held is left alone. Windows alone: elsewhere a file a viewer has open can be
/// written.
#[cfg(windows)]
#[test]
fn printing_again_while_the_viewer_has_the_first_open() {
    use std::os::windows::fs::OpenOptionsExt;
    let (mut a, _) = app("print-held");
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    a.services = Box::new(attachments::Opens(opened.clone()));
    open_saying(&mut a, 9982, "<p>Hello</p>");
    a.act(crate::Action::Print);
    let first = opened.borrow().last().cloned().expect("opened");
    let before = std::fs::read(&first).unwrap();
    // Held as a PDF viewer holds the file it shows.
    let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&first).unwrap();
    a.act(crate::Action::Print);
    assert!(a.session.notices.iter().all(|n| !n.error), "{:?}", a.session.notices);
    let second = opened.borrow().last().cloned().unwrap();
    assert!(second != first && second.file_name() == first.file_name(), "{second:?}");
    assert!(std::fs::read(&second).unwrap().starts_with(b"%PDF"));
    drop(held);
    assert_eq!(std::fs::read(&first).unwrap(), before, "the one held is left alone");
}

/// Issue #291: icons in a message (a calendar, a clock, an arrow) printed as empty boxes: the
/// PDF's font hasn't them, and it had no other. The window's fallback fonts draw them, as on
/// screen: one is embedded only when the message has something for it.
#[test]
fn what_the_pdfs_font_lacks_is_drawn_with_the_windows_fallbacks() {
    let (mut a, _) = app("print-fallbacks");
    // egui's own text font, which has letters and not emoji: the same on every system.
    let defaults = egui::FontDefinitions::default();
    let text_font = defaults.font_data.get("Ubuntu-Light").map(|d| (*d.font).as_ref().to_vec()).expect("egui's text font");
    a.session.print_fonts = Some((text_font, None));
    assert!(!a.session.print_fallbacks.is_empty(), "the window gives its fallbacks");
    // A font past the regular and bold ones (`/F1`, `/F2`) named on the pages: a fallback.
    let fallback_in_pdf = |a: &MailApp| {
        let pdf = String::from_utf8_lossy(&a.session.message_pdf().unwrap()).into_owned();
        (3..40).any(|n| pdf.contains(&format!("/F{n} ")))
    };
    open_saying(&mut a, 9983, "<p>\u{1F4C5} Thursday \u{1F552} 10:00</p>");
    assert!(fallback_in_pdf(&a), "a fallback is embedded and named");
    open_saying(&mut a, 9984, "<p>Thursday 10:00</p>");
    assert!(!fallback_in_pdf(&a), "none is, with nothing for it");
    // Without fallbacks, as before: none.
    a.session.print_fallbacks.clear();
    open_saying(&mut a, 9985, "<p>\u{1F4C5} Thursday</p>");
    assert!(!fallback_in_pdf(&a));
}
