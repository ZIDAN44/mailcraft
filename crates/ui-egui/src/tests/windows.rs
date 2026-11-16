//! UI tests: windows of their own, where they open, fonts and the browser, a message's source.

use super::*;

pub(super) mod layout;
mod source;

#[test]
fn in_the_browser_a_popped_out_message_opens_in_a_window_of_its_own() {
    let (a, _) = app("tabs");
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
}

/// A reply written in a message's own page (the browser) has the address book as the main
/// window has: Select Names lists the people mail was exchanged with (it listed no one there,
/// the page's frame never filling it in).
#[test]
fn a_message_page_s_reply_has_the_address_book() {
    let (a, _) = app("page-names");
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut tab = MailApp::new(a.session, Box::new(NullBackend), Box::new(PageServices(opened)));
    let ctx = egui::Context::default();
    let k = tab.session.list[0].key.clone();
    tab.show_message_page(&k.to_id());
    frame(&ctx, &mut tab, vec![]);
    tab.run("list.open", json!({"key": k.to_id()}));
    tab.run("mail.reply", json!({}));
    frame(&ctx, &mut tab, vec![]);
    let c = tab.session.compose.clone().unwrap();
    tab.ui.dialog = Some(Dialog::select_names(&c, crate::dialogs::Recipients::To));
    frame(&ctx, &mut tab, vec![]);
    assert_eq!(tab.ui.page_message.as_ref(), Some(&k), "still the page");
    let Some(Dialog::SelectNames(f)) = &tab.ui.dialog else { panic!("no dialog") };
    assert!(f.list.iter().any(|x| x.email == "zoe@studio.example"), "everyone mail was exchanged with: {:?}", f.list);
}

#[test]
fn double_clicked_mail_opens_in_its_own_window() {
    let (mut a, _) = app("windows");
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
}

/// A message's own window: its attachment buttons work on its message, not the one in the
/// reading pane (they used to act on the reading pane's, and fail).
#[test]
fn a_message_windows_attachments_are_its_own() {
    let (mut a, _) = app("window-attachments");
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
    a.act(crate::Action::InWindow(with.clone(), Box::new(crate::Action::DragOut(vec![index]))));
    let path = a.take_drag_out().pop().expect("the window's attachment, as a file");
    let name = w.parsed.as_ref().and_then(|p| p.attachments.first()).map(|x| x.name.clone()).unwrap();
    assert!(path.is_file() && path.file_name().is_some_and(|n| n.to_string_lossy() == name), "{path:?}");
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(plain), "the reading pane is as it was");
}

/// Save all in the browser says which attachment couldn't be downloaded, rather than leaving it
/// out unseen (the others came, and it looked like all of them).
#[test]
fn in_the_browser_save_all_says_what_it_couldnt_download() {
    let store = Store::memory();
    sample::seed_if_empty(&store, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let web = WebLike::default();
    let downloads = web.downloads.clone();
    let mut a = MailApp::new(s, Box::new(NullBackend), Box::new(web));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.has_attachments).map(|m| m.key.clone()).unwrap();
    a.session.open_message(&key);
    frame(&ctx, &mut a, vec![]);
    // One more the message doesn't have: its bytes can't be had.
    let r = a.session.reading.as_mut().unwrap();
    let mut parsed = (**r.parsed.as_ref().unwrap()).clone();
    let mut gone = parsed.attachments.iter().find(|x| !x.inline).cloned().unwrap();
    (gone.index, gone.name) = (999, "Missing.pdf".into());
    parsed.attachments.push(gone);
    let real = parsed.attachments.iter().filter(|x| !x.inline).count() - 1;
    r.parsed = Some(std::sync::Arc::new(parsed));
    a.act(crate::Action::SaveAll);
    assert_eq!(downloads.borrow().len(), real, "the others came");
    let said: Vec<&str> = a.session.notices.iter().filter(|n| n.error).map(|n| n.text.as_str()).collect();
    assert!(said.iter().any(|t| t.starts_with("Missing.pdf couldn't be downloaded: ")), "{said:?}");
}

#[test]
fn in_the_browser_files_download_and_attachments_come_from_the_picker() {
    let store = Store::memory();
    sample::seed_if_empty(&store, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let web = WebLike::default();
    let (downloads, asked, hands) = (web.downloads.clone(), web.asked.clone(), web.hands.clone());
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
    assert!(matches!(&a.ui.dialog, Some(Dialog::AddAccount(f)) if f.kind == crate::dialogs::AccountKind::Jmap && f.advanced));
    a.ui.dialog = None;
    assert!(a.session.run("account.add", &json!({"email": "zoe@example.com", "password": "pw", "imapHost": "imap.example.com"})).is_err());

    a.run("mail.new", json!({}));
    a.act(crate::Action::AttachFiles);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(asked.borrow().as_slice(), [crate::Picked::Attach]);
    // Its bytes handed over as they are.
    assert!(a.session.compose.as_ref().is_some_and(|c| c.attachments.iter().any(|x| x.name == "notes.txt" && x.data == b"remember")));
    a.run("compose.discard", json!({}));

    // Rules and categories go to a file as downloads, and come back from the picker.
    a.run(
        "rules.add",
        json!({"rule": {"name": "From Zoe", "conditions": [{"if": "from", "value": "zoe@example.com"}], "actions": [{"do": "flag"}]}}),
    );
    let (rules, categories) = (a.session.rules.len(), a.session.prefs.categories.len());
    assert!(rules == 1 && categories > 0);
    let before = downloads.borrow().len();
    a.act(crate::Action::ExportRules);
    a.act(crate::Action::ExportCategories);
    let got = downloads.borrow().get(before..).unwrap_or_default().to_vec();
    assert_eq!(got.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["MailCraft rules.json", "MailCraft categories.json"]);
    assert!(got.iter().all(|(_, n)| *n > 0), "{got:?}");
    *hands.borrow_mut() = Some(a.session.rules_file().unwrap());
    a.run("rules.delete", json!({"index": 0}));
    assert!(a.session.rules.is_empty());
    a.act(crate::Action::ImportRules);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.rules.len(), rules, "the file's rule is back");
    let kept = a.session.categories_file().unwrap();
    a.session.prefs.categories.clear();
    *hands.borrow_mut() = Some(kept);
    a.act(crate::Action::ImportCategories);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.prefs.categories.len(), categories);
    assert_eq!(asked.borrow().get(1..), Some(&[crate::Picked::Rules, crate::Picked::Categories][..]));
    // A file that isn't one is said so, and nothing changes.
    a.act(crate::Action::ImportRules);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.rules.len(), rules);
    assert!(a.session.notices.last().is_some_and(|n| n.error && n.text.contains("rules file")), "{:?}", a.session.notices.last().map(|n| &n.text));

    // File › Open & Export has no data files to open or write there: the backup and the rules.
    let size = egui::vec2(1280.0, 800.0);
    let page = |a: &mut MailApp| -> Vec<String> {
        a.act(crate::Action::Backstage(crate::backstage::Page::Open));
        drawn_text(&ctx, a, size);
        drawn_strings(&ctx, a, size)
    };
    let there = page(&mut a);
    for card in ["Backup", "Rules"] {
        assert!(there.iter().any(|t| t == card), "{card} in {there:?}");
    }
    for card in ["Open a data file", "Open a mail file", "Export to Outlook data file"] {
        assert!(!there.iter().any(|t| t == card), "{card} in {there:?}");
    }
    a.session.jmap_only = false;
    let there = page(&mut a);
    for card in ["Open a data file", "Open a mail file", "Export to Outlook data file"] {
        assert!(there.iter().any(|t| t == card), "{card} in {there:?}");
    }
}

#[test]
fn new_mail_writes_in_its_own_window_and_replies_pop_out() {
    let (mut a, _) = app("compose-window");
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
    assert_eq!(saved.rescue(&info(egui::vec2(1600.0, 900.0)), None), Some(egui::pos2(200.0, 50.0)));
    assert_eq!(saved.rescue(&info(egui::vec2(1920.0, 1080.0)), None), None, "the same monitor");
    assert_eq!(saved.rescue(&egui::ViewportInfo::default(), None), None, "not known yet");
    assert_eq!(WindowPlace { maximized: true, ..saved }.rescue(&info(egui::vec2(1600.0, 900.0)), None), None);
    assert_eq!(WindowPlace { monitor: None, ..saved }.rescue(&info(egui::vec2(1600.0, 900.0)), None), None, "saved before monitors were kept");
}

/// A window restored where no monitor is any more, though one of the same size took the place of
/// its own (two alike, one unplugged), comes back on screen: the system's monitors say so, not
/// their sizes. Centred on the primary monitor, wherever that is; one that can be dragged back by
/// its top edge stays where it is (WordCraft's fix).
#[test]
fn a_window_on_no_monitor_comes_back_though_the_sizes_match() {
    use crate::{Monitors, WindowPlace};
    let left = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0));
    let right = egui::Rect::from_min_size(egui::pos2(1920.0, 0.0), egui::vec2(1920.0, 1080.0));
    let saved = WindowPlace { x: 2100.0, y: 50.0, w: 1200.0, h: 800.0, maximized: false, monitor: Some([1920.0, 1080.0]) };
    let at = |x: f32, y: f32| egui::ViewportInfo {
        monitor_size: Some(egui::vec2(1920.0, 1080.0)),
        outer_rect: Some(egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(1200.0, 830.0))),
        native_pixels_per_point: Some(1.0),
        ..Default::default()
    };
    let one = Monitors { all: vec![left], primary: Some(left) };
    let both = Monitors { all: vec![left, right], primary: Some(left) };
    assert_eq!(saved.rescue(&at(2100.0, 50.0), Some(&one)), Some(egui::pos2(360.0, 140.0)), "its monitor went");
    assert_eq!(saved.rescue(&at(2100.0, 50.0), Some(&both)), None, "still there");
    assert_eq!(saved.rescue(&at(2100.0, 50.0), None), None, "the system lists none: sizes only");
    assert_eq!(saved.rescue(&at(1920.0 - 96.0, 50.0), Some(&one)), None, "enough of its top edge to drag by");
    assert_eq!(saved.rescue(&at(1920.0 - 95.0, 50.0), Some(&one)), Some(egui::pos2(360.0, 140.0)));
    assert_eq!(saved.rescue(&at(100.0, -900.0), Some(&one)), Some(egui::pos2(360.0, 140.0)), "its top edge above every monitor");
    assert_eq!(WindowPlace { monitor: None, ..saved }.rescue(&at(2100.0, 50.0), Some(&one)), Some(egui::pos2(360.0, 140.0)));
    assert_eq!(WindowPlace { maximized: true, ..saved }.rescue(&at(2100.0, 50.0), Some(&one)), None);
    // The primary monitor left of the origin, and at twice the pixels: in points, centred on it.
    let primary = egui::Rect::from_min_size(egui::pos2(-2560.0, 240.0), egui::vec2(2560.0, 1440.0));
    let hidpi = egui::ViewportInfo {
        native_pixels_per_point: Some(2.0),
        outer_rect: Some(egui::Rect::from_min_size(egui::pos2(5000.0, 50.0), egui::vec2(1200.0, 830.0))),
        ..at(0.0, 0.0)
    };
    let far = Monitors { all: vec![primary], primary: Some(primary) };
    assert_eq!(saved.rescue(&hidpi, Some(&far)), Some(egui::pos2(-1280.0 + 40.0, 120.0)));
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
    let (mut a, _) = app("bad-fonts");
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
}

/// Ctrl+Q is Mark as read (Outlook's): it marked the message read and then closed MailCraft,
/// egui's own quit shortcut being Ctrl+Q too. Now the window stays (a Mac keeps Cmd+Q to quit).
#[test]
fn ctrl_q_marks_read_and_keeps_the_window() {
    let (mut a, _) = app("ctrl-q");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| !m.flags.seen).map(|m| m.key.clone()).unwrap();
    a.run("list.select", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        events: vec![key(Key::Q, Modifiers::COMMAND)],
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let closes = out.viewport_output.values().flat_map(|v| v.commands.iter()).any(|c| matches!(c, egui::ViewportCommand::Close));
    assert!(a.session.summary(&k).is_some_and(|m| m.flags.seen), "marked read");
    if cfg!(target_os = "macos") {
        assert!(closes, "Cmd+Q quits on a Mac");
    } else {
        assert!(!closes, "the window stays");
    }
}

/// Every window of MailCraft's own carries its app id, as the main one does: a Wayland dock
/// shows MailCraft's icon on it and window rules can match it (as PhotoCraft's). Each is built
/// from `os_window`, which no window of the UI's own goes around.
#[test]
fn windows_of_their_own_carry_the_app_id() {
    assert_eq!(crate::os_window().app_id.as_deref(), Some(crate::APP_ID));
    fn walk(dir: &std::path::Path, found: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, found);
            } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("lib.rs") && !p.starts_with(dir.join("tests")) {
                let text = std::fs::read_to_string(&p).unwrap();
                if text.contains(concat!("ViewportBuilder::", "default()")) {
                    found.push(p.display().to_string());
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut found);
    assert!(found.is_empty(), "windows built without MailCraft's app id: {found:?}");
}

/// Services saying the window has no system title bar (Windows, issue #22).
struct OwnTitleBar(std::rc::Rc<std::cell::Cell<u32>>);

impl OwnTitleBar {
    fn new() -> OwnTitleBar {
        OwnTitleBar(std::rc::Rc::default())
    }
}

impl crate::Services for OwnTitleBar {
    stub_services!(clock, pick_files, save_path);
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("not available".into())
    }
    fn own_title_bar(&self) -> bool {
        true
    }
    fn before_zoom(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

/// MailCraft's own Maximize, Restore and double-click tell the window before winit does them, so
/// Windows can animate them and size a maximize clear of the taskbar; Minimize doesn't, and the
/// note is gone after the frame.
#[test]
fn the_window_is_told_before_its_own_title_bar_zooms_it() {
    let (a, _) = app("told-before-maximize");
    let told = OwnTitleBar::new();
    let count = std::rc::Rc::clone(&told.0);
    let mut a = MailApp::new(a.session, Box::new(NullBackend), Box::new(told));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = egui::vec2(1280.0, 800.0);
    let time = std::cell::Cell::new(0.0f64);
    let run = |a: &mut MailApp, events: Vec<Event>, maximized: bool| {
        time.set(time.get() + 1.0 / 60.0);
        let mut input =
            RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, time: Some(time.get()), ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().maximized = Some(maximized);
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        out
    };
    let click = |at: egui::Pos2| {
        vec![
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
            Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
        ]
    };
    for _ in 0..3 {
        run(&mut a, vec![], false);
    }
    let out = run(&mut a, vec![], false);
    let nodes = named_in(&out);
    let at = |label: &str| nodes.iter().filter(|n| n.label.as_deref() == Some(label)).find_map(Named::centre).unwrap_or_else(|| panic!("{label}"));
    let (max, min) = (at("Maximize"), at("Minimize"));
    let free = egui::pos2(at("Send / Receive all folders (F9)").x - 20.0, 20.0);
    run(&mut a, click(min), false);
    run(&mut a, vec![], false);
    assert_eq!(count.get(), 0, "Minimize says nothing");
    run(&mut a, click(max), false);
    assert_eq!(count.get(), 1, "the button says so, in the same frame as the command");
    run(&mut a, vec![], false);
    assert_eq!(count.get(), 1, "once");
    time.set(time.get() + 1.0);
    run(&mut a, click(free), false);
    run(&mut a, click(free), false);
    assert_eq!(count.get(), 2, "so does a double-click on the bar");
    run(&mut a, vec![], true);
    time.set(time.get() + 1.0);
    run(&mut a, click(max), true);
    run(&mut a, vec![], true);
    assert_eq!(count.get(), 3, "so does Restore");
}

/// Issue #22: on Windows the top bar is the window's title bar, in MailCraft's colour as
/// Outlook's is: the window's buttons at its end (named for a screen reader, clear of the bar's
/// own controls), its free space dragging the window and a double-click maximizing it, the
/// window's edges resizing it, and the File page keeping the bar. Without it, none of that.
#[test]
fn the_top_bar_is_the_window_title_bar_when_it_has_none_of_the_systems() {
    let (a, _) = app("own-title-bar");
    let mut a = MailApp::new(a.session, Box::new(NullBackend), Box::new(OwnTitleBar::new()));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = egui::vec2(1280.0, 800.0);
    // Frames a 60th of a second apart, a pause where the steps want one.
    let time = std::cell::Cell::new(0.0f64);
    let run = |a: &mut MailApp, events: Vec<Event>| {
        time.set(time.get() + 1.0 / 60.0);
        let input =
            RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, time: Some(time.get()), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        out
    };
    let named =
        |out: &egui::FullOutput| -> Vec<(String, egui::Rect)> { named_in(out).into_iter().filter_map(|n| Some((n.label?, n.rect?))).collect() };
    let sent = |out: &egui::FullOutput| out.viewport_output.values().flat_map(|v| v.commands.clone()).collect::<Vec<_>>();
    let click = |at: egui::Pos2| {
        vec![
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
            Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
        ]
    };
    for _ in 0..3 {
        run(&mut a, vec![]);
    }
    let out = run(&mut a, vec![]);
    let seen = named(&out);
    let at = |label: &str| seen.iter().find(|(l, _)| l == label).map(|(_, r)| *r).unwrap_or_else(|| panic!("{label}: {seen:?}"));
    let (min, max, close) = (at("Minimize"), at("Maximize"), at("Close"));
    assert!((close.right() - size.x).abs() < 1.0 && close.top() <= 0.5, "flush with the window's corner: {close:?}");
    assert!(min.right() <= max.left() + 0.5 && max.right() <= close.left() + 0.5);
    let settings = at("Accounts and settings");
    assert!(settings.right() <= min.left(), "the bar's controls keep clear of the window's buttons: {settings:?} {min:?}");
    // Each button does what the system's does.
    for (r, what) in [(close, "close"), (max, "maximize"), (min, "minimize")] {
        let out = run(&mut a, click(r.center()));
        let cmds = sent(&out);
        let done = match what {
            "close" => cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Close)),
            "maximize" => cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Maximized(true))),
            _ => cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Minimized(true))),
        };
        assert!(done, "{what}: {cmds:?}");
        run(&mut a, vec![]);
    }
    // The bar's free space, between the search box and the controls: a double-click maximizes,
    // a press and a move drags the window.
    // Between the search box and the bar's own controls at its end (Undo's room, kept while
    // there's nothing to undo).
    let free = egui::pos2(at("Send / Receive all folders (F9)").left() - 20.0, 20.0);
    assert!(at("Advanced search").right() < free.x, "past the search box");
    time.set(time.get() + 1.0);
    run(&mut a, click(free));
    let out = run(&mut a, click(free));
    assert!(sent(&out).iter().any(|c| matches!(c, egui::ViewportCommand::Maximized(true))), "{:?}", sent(&out));
    run(&mut a, vec![]);
    let press = vec![
        Event::PointerMoved(free),
        Event::PointerButton { pos: free, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
    ];
    run(&mut a, press);
    let out = run(&mut a, vec![Event::PointerMoved(free + egui::vec2(40.0, 30.0))]);
    assert!(sent(&out).iter().any(|c| matches!(c, egui::ViewportCommand::StartDrag)), "{:?}", sent(&out));
    run(&mut a, vec![Event::PointerButton { pos: free, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    // The window's left edge resizes it, from the press (a zone that only drags).
    let edge = egui::pos2(2.0, 400.0);
    let down = Event::PointerButton { pos: edge, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
    let out = run(&mut a, vec![Event::PointerMoved(edge), down]);
    assert!(sent(&out).iter().any(|c| matches!(c, egui::ViewportCommand::BeginResize(egui::ResizeDirection::West))), "{:?}", sent(&out));
    run(&mut a, vec![Event::PointerButton { pos: edge, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    // A dialog holding the window (the question as MailCraft closes, Add account): the bar under
    // it takes no click, so its drag and buttons are laid again above it (found in review).
    a.ui.dialog = Some(Dialog::SaveOnClose(1));
    for _ in 0..3 {
        run(&mut a, vec![]);
    }
    time.set(time.get() + 1.0);
    let out = run(&mut a, click(close.center()));
    assert!(sent(&out).iter().any(|c| matches!(c, egui::ViewportCommand::Close)), "closed over the dialog: {:?}", sent(&out));
    run(&mut a, vec![]);
    let press = vec![
        Event::PointerMoved(free),
        Event::PointerButton { pos: free, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
    ];
    run(&mut a, press);
    let out = run(&mut a, vec![Event::PointerMoved(free + egui::vec2(40.0, 30.0))]);
    assert!(sent(&out).iter().any(|c| matches!(c, egui::ViewportCommand::StartDrag)), "dragged over the dialog: {:?}", sent(&out));
    run(&mut a, vec![Event::PointerButton { pos: free, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    assert!(a.ui.dialog.is_some(), "the dialog stays");
    // One Minimize, Maximize and Close to a screen reader: the dimmed ones under the dialog aren't
    // named (found in review: two of each).
    let out = run(&mut a, vec![]);
    for name in ["Minimize", "Maximize", "Close"] {
        assert_eq!(named(&out).iter().filter(|(l, _)| l == name).count(), 1, "{name}");
    }
    // Not in the Tab order, as a system title bar isn't: Tab goes round the dialog's buttons and
    // never reaches the bar's, where Enter would have minimized or closed the window.
    for _ in 0..8 {
        let out = run(&mut a, vec![key(Key::Tab, Modifiers::NONE)]);
        let tree = out.platform_output.accesskit_update.as_ref().expect("AccessKit is on");
        let focus = tree.nodes.iter().find(|(id, _)| *id == tree.focus).and_then(|(_, n)| n.label().map(str::to_string));
        assert!(!matches!(focus.as_deref(), Some("Minimize" | "Maximize" | "Restore" | "Close")), "focus on {focus:?}");
        assert!(sent(&out).iter().all(|c| !matches!(c, egui::ViewportCommand::Minimized(_) | egui::ViewportCommand::Close)));
    }
    // A double-click on a tab with the dialog open: the bar's free space drags and maximizes, its
    // tabs don't (PhotoCraft's gap).
    let home = at("Home").center();
    time.set(time.get() + 1.0);
    a.ui.dialog = Some(Dialog::SaveOnClose(1));
    run(&mut a, vec![]);
    run(&mut a, click(home));
    let out = run(&mut a, click(home));
    assert!(!sent(&out).iter().any(|c| matches!(c, egui::ViewportCommand::Maximized(_))), "{:?}", sent(&out));
    a.ui.dialog = None;
    // The File page keeps the window's buttons.
    a.run("ui.file", json!({}));
    run(&mut a, vec![]);
    let out = run(&mut a, vec![]);
    assert!(named(&out).iter().any(|(l, _)| l == "Close"), "the File page's title bar");
    // With the system's title bar, MailCraft draws none of it.
    let (b, _) = app("system-title-bar");
    let mut b = b;
    for _ in 0..3 {
        run(&mut b, vec![]);
    }
    let out = run(&mut b, vec![]);
    assert!(!named(&out).iter().any(|(l, _)| l == "Minimize"));
}

/// The title bar laid over a dialog stays off a dialog that reaches into the bar (a short window,
/// a tall dialog): the dialog keeps its clicks there (PhotoCraft's
/// `modal_drag_overlay_does_not_cover_a_dialog_moved_into_the_gap`). And in a right-to-left
/// language the File page's mark stays clear of the window's buttons (found in review: it was
/// drawn over Close).
#[test]
fn the_title_bar_keeps_off_a_dialog_in_it_and_its_buttons_off_the_mark() {
    let (a, _) = app("own-title-bar-short");
    let mut a = MailApp::new(a.session, Box::new(NullBackend), Box::new(OwnTitleBar::new()));
    let ctx = egui::Context::default();
    let time = std::cell::Cell::new(0.0f64);
    let run = |a: &mut MailApp, size: egui::Vec2, events: Vec<Event>| {
        time.set(time.get() + 1.0 / 60.0);
        let input =
            RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, time: Some(time.get()), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        out
    };
    let short = egui::vec2(1280.0, 150.0);
    a.ui.dialog = Some(Dialog::SaveOnClose(1));
    for _ in 0..4 {
        run(&mut a, short, vec![]);
    }
    let modal = ctx.memory(|m| m.top_modal_layer()).and_then(|l| ctx.memory(|m| m.area_rect(l.id)));
    assert!(modal.is_some_and(|r| r.top() < 40.0), "the dialog reaches into the bar: {modal:?}");
    assert!(!crate::app::caption::laid_over_a_dialog(&ctx));
    let close = egui::pos2(short.x - 23.0, 20.0);
    let press = |down| Event::PointerButton { pos: close, button: egui::PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE };
    let out = run(&mut a, short, vec![Event::PointerMoved(close), press(true), press(false)]);
    let cmds: Vec<_> = out.viewport_output.values().flat_map(|v| v.commands.clone()).collect();
    assert!(!cmds.contains(&egui::ViewportCommand::Close), "{cmds:?}");
    // Where the dialog itself is in the bar, clear of the bar's controls (the search box, which
    // takes no drag anyway): a press and a move there don't drag the window, and a double-click
    // doesn't maximize it; the dialog has them.
    let (bar, _, controls) = crate::app::caption::noted_bar(&ctx).unwrap_or((egui::Rect::NOTHING, egui::Rect::NOTHING, Vec::new()));
    let both = modal.unwrap_or(egui::Rect::NOTHING).intersect(bar);
    let in_both = (0..40)
        .flat_map(|x| (0..8).map(move |y| (x, y)))
        .map(|(x, y)| egui::pos2(both.left() + 10.0 + x as f32 * 8.0, both.bottom() - 1.0 - y as f32 * 4.0))
        .find(|p| both.contains(*p) && !controls.iter().any(|c| c.contains(*p)))
        .unwrap_or_else(|| panic!("no free place on the dialog in the bar: {both:?}, {controls:?}"));
    let at =
        |p: egui::Pos2, down: bool| Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE };
    let mut cmds = Vec::new();
    for events in [
        vec![Event::PointerMoved(in_both), at(in_both, true)],
        vec![Event::PointerMoved(in_both + egui::vec2(40.0, 10.0))],
        vec![at(in_both + egui::vec2(40.0, 10.0), false)],
        vec![Event::PointerMoved(in_both), at(in_both, true), at(in_both, false)],
        vec![at(in_both, true), at(in_both, false)],
    ] {
        let out = run(&mut a, short, events);
        cmds.extend(out.viewport_output.values().flat_map(|v| v.commands.clone()));
    }
    assert!(!cmds.iter().any(|c| matches!(c, egui::ViewportCommand::StartDrag | egui::ViewportCommand::Maximized(_))), "{cmds:?}");
    a.ui.dialog = None;

    // Right to left, the File page: its mark (a 20-point tile) at the bar's start, clear of the
    // window's buttons at its end.
    a.ui.language = "he".into();
    a.run("ui.file", json!({}));
    let size = egui::vec2(1280.0, 800.0);
    let mut tiles = Vec::new();
    for _ in 0..4 {
        let out = run(&mut a, size, vec![]);
        tiles = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if (r.rect.width() - 20.0).abs() < 0.5 && (r.rect.height() - 20.0).abs() < 0.5 && r.rect.top() < 40.0 => {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
    }
    assert!(!tiles.is_empty(), "the mark drawn");
    assert!(tiles.iter().all(|r| r.right() <= size.x - crate::app::caption::CAPTIONS_W), "clear of the buttons: {tiles:?}");
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// Audit #473: a ribbon button asks whether the ribbon is the classic one without copying what
/// was noted of every button before it. A ribbon of 120 buttons in groups of 6, both kinds. Run
/// in release: `cargo test --release -p mailcraft-ui-egui --lib ribbon_buttons_measure --
/// --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn ribbon_buttons_measure() {
    let t = crate::theme::Tokens::light();
    for classic in [false, true] {
        let ctx = egui::Context::default();
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 200.0)));
        let ribbon = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(RawInput { screen_rect: screen, ..Default::default() }, |ui| {
                let id = crate::widgets::ribbon_fit_id();
                let start = crate::widgets::RibbonFit { active: true, classic, icons_from: usize::MAX, ..Default::default() };
                ui.ctx().data_mut(|d| d.insert_temp(id, start));
                ui.horizontal(|ui| {
                    for g in 0..20 {
                        for _ in 0..6 {
                            let _ = crate::widgets::tool_button(ui, crate::icons::Icon::Reply, "Reply", &t, true);
                        }
                        crate::widgets::group(ui, &t, "Respond", g == 19);
                    }
                });
                ui.ctx().data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<crate::widgets::RibbonFit>(id)));
            });
            out.textures_delta.clear();
        };
        for _ in 0..20 {
            ribbon(&ctx);
        }
        const ROUNDS: u32 = 1000;
        let at = std::time::Instant::now();
        for _ in 0..ROUNDS {
            ribbon(&ctx);
        }
        let kind = if classic { "classic" } else { "simplified" };
        println!("MEASURE a {kind} ribbon of 120 buttons: {:.1} µs a frame", at.elapsed().as_secs_f64() * 1e6 / f64::from(ROUNDS));
    }
}
