//! UI tests: windows of their own, where they open, fonts and the browser.

use super::*;

#[test]
fn in_the_browser_a_popped_out_message_opens_in_a_window_of_its_own() {
    let (a, dir) = app("tabs");
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut a = MailApp::new(a.session, Box::new(NullBackend), Box::new(PageServices(opened.clone())));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("message.popOut", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(*opened.borrow(), [k.to_id()], "a browser window, not one drawn in the page");
    assert!(a.session.windows.is_empty());
    // The new window shows only that message; done with, it closes.
    let mut tab = MailApp::new(a.session, Box::new(NullBackend), Box::new(PageServices(opened.clone())));
    tab.show_message_page(&k.to_id());
    frame(&ctx, &mut tab, vec![]);
    assert_eq!(tab.ui.page_message.as_ref(), Some(&k));
    assert_eq!(tab.session.windows.len(), 1, "shown in place, not opened again");
    tab.act(crate::Action::Run("message.closeWindow", json!({"key": k.to_id()})));
    frame(&ctx, &mut tab, vec![]);
    assert_eq!(opened.borrow().last().map(String::as_str), Some("closed"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn double_clicked_mail_opens_in_its_own_window() {
    let (mut a, dir) = app("windows");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("message.popOut", json!({"key": k.to_id()}));
    // Without a native backend the window is drawn inside the main one.
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.windows.len(), 1);
    // Its toolbar acts on its own message: Reply opens the reply in the main window and closes it.
    a.act(crate::Action::Open(k.clone()));
    a.act(crate::Action::Run("mail.reply", json!({})));
    a.act(crate::Action::Run("message.closeWindow", json!({"key": k.to_id()})));
    assert!(a.session.compose.is_some() && a.session.windows.is_empty());
    // Escape closes a window.
    a.run("compose.discard", json!({}));
    a.run("message.popOut", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.windows.is_empty(), "Escape closes the window");
    let _ = std::fs::remove_dir_all(dir);
}

/// A message's own window: its attachment buttons work on its message, not the one in the
/// reading pane (they used to act on the reading pane's, and fail).
#[test]
fn a_message_windows_attachments_are_its_own() {
    let (mut a, dir) = app("window-attachments");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.services = Box::new(Recorder(Default::default()));
    a.ui.can_drag_out = true;
    // The reading pane shows a message without attachments; a window, one with.
    let plain = a.session.list.iter().find(|m| !m.has_attachments).map(|m| m.key.clone()).unwrap();
    let with = a.session.list.iter().find(|m| m.has_attachments).map(|m| m.key.clone()).unwrap();
    a.run("message.popOut", json!({"key": with.to_id()}));
    a.run("list.open", json!({"key": plain.to_id()}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(plain.clone()));
    let w = a.session.windows.iter().find(|w| w.key == with).cloned().expect("the window");
    let index = w.parsed.as_ref().and_then(|p| p.attachments.first()).map(|x| x.index).unwrap();
    a.act(crate::Action::InWindow(with.clone(), Box::new(crate::Action::DragOut(index))));
    let path = a.take_drag_out().expect("the window's attachment, as a file");
    let name = w.parsed.as_ref().and_then(|p| p.attachments.first()).map(|x| x.name.clone()).unwrap();
    assert!(path.is_file() && path.file_name().is_some_and(|n| n.to_string_lossy() == name), "{path:?}");
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(plain), "the reading pane is as it was");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn in_the_browser_files_download_and_attachments_come_from_the_picker() {
    let store = Store::memory();
    sample::seed_if_empty(&store, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let web = WebLike::default();
    let (downloads, asked) = (web.downloads.clone(), web.asked.clone());
    let mut a = MailApp::new(s, Box::new(NullBackend), Box::new(web));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.has_attachments).map(|m| m.key.clone()).unwrap();
    a.session.open_message(&key);
    frame(&ctx, &mut a, vec![]);
    let first =
        a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).and_then(|p| p.attachments.iter().find(|x| !x.inline)).map(|x| x.index).unwrap();
    for act in [crate::Action::SaveMessage, crate::Action::ExportPdf, crate::Action::SaveAttachment(first), crate::Action::SaveAll] {
        a.act(act);
    }
    let got = downloads.borrow().clone();
    assert!(got.len() >= 4, "{got:?}");
    assert!(got[0].0.ends_with(".eml") && got[1].0.ends_with(".pdf") && got.iter().all(|(_, n)| *n > 0), "{got:?}");

    // Only JMAP accounts: the form asks for the session address, and nothing else is accepted.
    a.session.jmap_only = true;
    a.ui.dialog = Some(Dialog::add_account_with("zoe@fastmail.example", false));
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::AddAccount(f)) if f.jmap && f.advanced));
    a.ui.dialog = None;
    assert!(a.session.run("account.add", &json!({"email": "zoe@example.com", "password": "pw", "imapHost": "imap.example.com"})).is_err());

    a.run("mail.new", json!({}));
    a.act(crate::Action::AttachFiles);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(asked.borrow().as_slice(), ["attach"]);
    assert!(a.session.compose.as_ref().is_some_and(|c| c.attachments.iter().any(|x| x.name == "notes.txt")));
}

#[test]
fn new_mail_writes_in_its_own_window_and_replies_pop_out() {
    let (mut a, dir) = app("compose-window");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.as_ref().is_some_and(|c| c.window));
    a.run("compose.update", json!({"to": "zoe@studio.example", "subject": "Poster sizes"}));
    frame(&ctx, &mut a, vec![]);
    // Back into the reading pane, out again, then the window closes into Drafts.
    a.act(crate::Action::Run("compose.popIn", json!({})));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Run("compose.popOut", json!({})));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Run("compose.close", json!({})));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.is_none());
    a.run("folder.open", json!({"account": "sample", "folder": "Drafts"}));
    assert!(a.session.list.iter().any(|m| m.subject == "Poster sizes"));
    // A reply starts inline, with the original under a rule.
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("mail.reply", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.as_ref().unwrap();
    assert!(!c.window && c.body.contains("\nSent: "));
    let _ = std::fs::remove_dir_all(dir);
}

/// Fonts built in from craft-fonts (`CRAFT_FONTS_DIR`) are the last fallbacks, with their
/// licences; without them nothing changes.
#[test]
fn craft_fonts_are_the_last_fallbacks() {
    let ctx = egui::Context::default();
    // (Not install_fonts: that notes the main font's coverage, which other tests share.)
    let mut defs = egui::FontDefinitions::default();
    crate::theme::add_craft_fallbacks(&mut defs);
    ctx.set_fonts(defs);
    ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    let fonts = crate::craft_fonts::CRAFT_FONTS;
    for f in fonts {
        assert!(!f.bytes.is_empty() && f.licence_text.len() > 100, "{} comes with its licence", f.family);
    }
    // Arabic is drawn (from Noto Sans Arabic) when the build has it; nothing breaks when it hasn't.
    let arabic = ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(14.0), "مرحبا"));
    assert_eq!(arabic, crate::craft_fonts::has_script("Arab"));
}

/// A remembered window place is used only when it's sensible: a real size, roughly on a screen.
#[test]
fn window_places_open_only_where_they_make_sense() {
    use crate::WindowPlace;
    let p = |x: f32, y: f32, w: f32, h: f32| WindowPlace { x, y, w, h, maximized: false, monitor: None };
    assert!(p(213.0, 157.0, 1011.0, 707.0).usable().is_some());
    assert!(p(-1920.0, 40.0, 900.0, 700.0).usable().is_some(), "a screen to the left");
    assert!(p(100.0, 100.0, 50.0, 40.0).usable().is_none(), "too small to use");
    assert!(p(-32000.0, -32000.0, 900.0, 700.0).usable().is_none(), "Windows' minimized spot");
    assert!(p(f32::NAN, 0.0, 900.0, 700.0).usable().is_none());
    let v: WindowPlace = serde_json::from_value(json!({"x": 1.0, "y": 2.0, "w": 800.0, "h": 600.0})).unwrap();
    assert!(!v.maximized, "saved before maximized was kept");
}

/// A window restored onto a monitor that went away is brought back, centred on the primary one;
/// one on the monitor it was saved on stays put; a maximized one fills whichever monitor it gets.
#[test]
fn a_window_whose_monitor_went_comes_back_on_screen() {
    use crate::WindowPlace;
    let saved = WindowPlace { x: 2100.0, y: 50.0, w: 1200.0, h: 800.0, maximized: false, monitor: Some([1920.0, 1080.0]) };
    let info = |monitor: egui::Vec2| egui::ViewportInfo { monitor_size: Some(monitor), ..Default::default() };
    assert_eq!(saved.rescue(&info(egui::vec2(1600.0, 900.0))), Some(egui::pos2(200.0, 50.0)));
    assert_eq!(saved.rescue(&info(egui::vec2(1920.0, 1080.0))), None, "the same monitor");
    assert_eq!(saved.rescue(&egui::ViewportInfo::default()), None, "not known yet");
    assert_eq!(WindowPlace { maximized: true, ..saved }.rescue(&info(egui::vec2(1600.0, 900.0))), None);
    assert_eq!(WindowPlace { monitor: None, ..saved }.rescue(&info(egui::vec2(1600.0, 900.0))), None, "saved before monitors were kept");
}

/// A window too big for its screen, frame included, shrinks to fit and is centred; one that fits
/// is left alone; the system's own placement counts its position, a restored one only its size.
#[test]
fn a_window_too_big_for_its_screen_shrinks_to_fit() {
    use crate::WindowPlace;
    let rect = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
    let info = |outer: egui::Rect, inner: egui::Rect, monitor: egui::Vec2| egui::ViewportInfo {
        outer_rect: Some(outer),
        inner_rect: Some(inner),
        monitor_size: Some(monitor),
        ..Default::default()
    };
    let laptop = egui::vec2(1366.0, 768.0);
    // 1320 x 820 with a 31-point title bar on a 1366 x 768 screen.
    let big = info(rect(26.0, 26.0, 1336.0, 859.0), rect(34.0, 57.0, 1320.0, 820.0), laptop);
    let (pos, size) = WindowPlace::fit(&big, true).unwrap();
    assert!(size.x + 16.0 <= laptop.x - 96.0 + 0.5 && size.y + 39.0 <= laptop.y - 96.0 + 0.5, "{size:?}");
    assert!(pos.x >= 0.0 && pos.y >= 0.0);
    // Fits: nothing to do.
    let fits = info(rect(100.0, 100.0, 1016.0, 739.0), rect(108.0, 131.0, 1000.0, 700.0), egui::vec2(1920.0, 1080.0));
    assert_eq!(WindowPlace::fit(&fits, true), None);
    // Placed by the system past the screen's edge (not too big): moved; restored there: left.
    let past = info(rect(900.0, 300.0, 1016.0, 600.0), rect(908.0, 331.0, 1000.0, 561.0), egui::vec2(1920.0, 1080.0));
    assert!(WindowPlace::fit(&past, false).is_some());
    assert_eq!(WindowPlace::fit(&past, true), None);
    // Maximized: fills its monitor already.
    let max = egui::ViewportInfo { maximized: Some(true), ..big };
    assert_eq!(WindowPlace::fit(&max, true), None);
}

/// A damaged system font (or a file that isn't a font, or a collection index past its end)
/// panicked in egui's `set_fonts` at start-up before egui 0.37 (GridCraft found); egui now skips
/// such a face. This keeps it so: the window still draws with them installed.
#[test]
fn a_damaged_system_font_doesnt_stop_the_window() {
    let real = crate::craft_fonts::fallbacks().next().map(|f| f.bytes.to_vec());
    let garbage = b"not a font at all, just text".to_vec();
    let truncated = real.as_ref().map(|r| r[..r.len().min(200)].to_vec()).unwrap_or_else(|| b"\x00\x01\x00\x00\x00\x0c".to_vec());
    // Installed as install_fonts_with does, without noting the main font's coverage (which other
    // tests share): first in every family, a collection index past its end among them.
    let mut defs = egui::FontDefinitions::default();
    for (name, bytes, index) in [("garbage", garbage.clone(), 0), ("truncated", truncated, 0), ("past-end", garbage, 3)] {
        let mut data = egui::FontData::from_owned(bytes);
        data.index = index;
        defs.font_data.insert(name.into(), std::sync::Arc::new(data));
        for family in defs.families.values_mut() {
            family.insert(0, name.into());
        }
    }
    let ctx = egui::Context::default();
    ctx.set_fonts(defs);
    let (mut a, dir) = app("bad-fonts");
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}
