//! Typing right-to-left text into MailCraft's boxes: egui 0.37 lays a Hebrew or Arabic run out
//! with more places than it has letters ("שלום", four letters, ended at seven: emilk/egui#8576),
//! and Ctrl+Backspace or Alt+Left from there ran a count under zero. The Craft apps' egui fork
//! counts right (the fix of emilk/egui#8698).

use super::*;

/// Hebrew in the search box, then End and Ctrl+Backspace: the last word goes, nothing panics
/// (it did in debug builds; release builds took the whole box).
#[test]
fn deleting_a_word_after_hebrew_takes_one_word() {
    let (mut a, dir) = app("rtl-search");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let word = if cfg!(target_os = "macos") { Modifiers::ALT } else { Modifiers::CTRL };
    for (typed, left) in [("hello world", "hello "), ("שלום עולם", "שלום "), ("مرحبا بكم", "مرحبا ")] {
        a.ui.search_text.clear();
        // The box takes the focus, the words are typed, End, then a word deleted and a word moved over.
        a.ui.focus_search = true;
        for _ in 0..2 {
            frame(&ctx, &mut a, vec![]);
        }
        frame(&ctx, &mut a, vec![Event::Text(typed.into())]);
        assert_eq!(a.ui.search_text, typed, "typed into the search box");
        frame(&ctx, &mut a, vec![key(Key::End, Modifiers::NONE)]);
        frame(&ctx, &mut a, vec![key(Key::Backspace, word)]);
        assert_eq!(a.ui.search_text, left, "one word deleted from {typed:?}");
        frame(&ctx, &mut a, vec![key(Key::End, Modifiers::NONE)]);
        frame(&ctx, &mut a, vec![key(Key::ArrowLeft, word)]);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The text drawn in a frame: each piece with where its letters are, left to right on the screen.
fn drawn_text(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> Vec<(String, egui::Rect, egui::Rect)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => {
                let ink = t.galley.rows.iter().flat_map(|r| r.row.glyphs.iter().map(move |g| (r.pos.x + g.pos.x, r.pos.x + g.max_x())));
                let (l, r) = ink.fold((f32::INFINITY, f32::NEG_INFINITY), |(l, r), (a, b)| (l.min(a), r.max(b)));
                let rect = egui::Rect::from_x_y_ranges(t.pos.x + l..=t.pos.x + r, t.pos.y..=t.pos.y + t.galley.size().y);
                Some((t.galley.job.text.clone(), rect, s.clip_rect))
            }
            _ => None,
        })
        .collect()
}

/// A Hebrew subject too long for the list keeps its start, on the right, and loses its end ("…"
/// on the left): it was laid against the left and cut on the right, its first words gone.
#[test]
fn a_long_hebrew_subject_keeps_its_start() {
    let (mut a, dir) = app("rtl-subject");
    let store = a.session.store.clone().unwrap();
    let subject = "שלום לכולם, זהו נושא ארוך מאוד בעברית שלא נכנס בשורה אחת של הרשימה בשום אופן";
    let messages = vec![Summary {
        key: MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid: 1 },
        subject: subject.into(),
        date: NOW - 3600,
        ..Default::default()
    }];
    store.save_folder("sample", "Archive", &FolderState { validity: 1, uid_next: 2, highest_modseq: 0, messages, window: 0 }).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "Archive".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "Archive"}));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let texts = drawn_text(&ctx, &mut a, egui::vec2(1280.0, 800.0));
    let shown: Vec<_> = texts.iter().filter(|(s, ..)| s == subject).collect();
    assert_eq!(shown.len(), 1, "{texts:?}");
    let (_, ink, clip) = shown[0];
    // Within the list (nothing cut by the clip), its start against the right of the room.
    assert!(clip.min.x <= ink.min.x && ink.max.x <= clip.max.x, "{ink:?} in {clip:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Folder names in the narrowest folder pane stay on one line, "…" where they don't fit (they
/// wrapped: two or three lines in a row of one), in every language.
#[test]
fn folder_names_stay_on_one_line() {
    for lang in ["ar", "he", "de", "ru", "en"] {
        let (a, dir) = app(&format!("rtl-folders-{lang}"));
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap_or(crate::i18n::Lang::EN));
        let ctx = egui::Context::default();
        let t = crate::theme::Tokens::light();
        let mut acts = Vec::new();
        let mut out = ctx.run_ui(RawInput::default(), |ui| {
            ui.allocate_ui(egui::vec2(158.0, 800.0), |ui| crate::nav::show(&a.session, &t, ui, &mut acts));
        });
        out.textures_delta.clear();
        for s in &out.shapes {
            if let egui::Shape::Text(t) = &s.shape {
                assert_eq!(t.galley.rows.len(), 1, "{lang}: {:?} on one line", t.galley.job.text);
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// In Arabic the main window is mirrored: File is the rightmost tab, a folder's count is left of
/// its name, a message's time left of its sender; in English they're the other way round.
#[test]
fn a_right_to_left_interface_is_mirrored() {
    for (lang, file, view, inbox) in [("ar", "ملف", "عرض", "علبة الوارد"), ("en", "File", "View", "Inbox")] {
        let (mut a, dir) = app(&format!("rtl-mirror-{lang}"));
        a.ui.language = lang.into();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&ctx, &mut a, vec![]);
        }
        let texts = drawn_text(&ctx, &mut a, egui::vec2(1280.0, 800.0));
        let at =
            |name: &str| texts.iter().find(|(s, ..)| s == name).map(|(_, r, _)| *r).unwrap_or_else(|| panic!("{lang}: no {name:?} in {texts:?}"));
        let rtl = lang == "ar";
        assert_eq!(at(file).center().x > at(view).center().x, rtl, "{lang}: File and View");
        // The Inbox among the favourites: its unread count beside it.
        let name = at(inbox);
        let number = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
        let count = texts.iter().filter(|(s, r, _)| number(s) && (r.center().y - name.center().y).abs() < 4.0).map(|(_, r, _)| *r).next();
        let count = count.unwrap_or_else(|| panic!("{lang}: no count by the Inbox"));
        assert_eq!(count.center().x < name.center().x, rtl, "{lang}: the count's side");
        // The first message: its sender and its time on one line.
        let who = at("Sam Okafor");
        let time = texts
            .iter()
            .filter(|(s, r, _)| s.contains(':') && s.len() <= 12 && (r.center().y - who.center().y).abs() < 4.0)
            .map(|(_, r, _)| *r)
            .next();
        let time = time.unwrap_or_else(|| panic!("{lang}: no time by the sender"));
        assert_eq!(time.center().x < who.center().x, rtl, "{lang}: the time's side");
        let _ = std::fs::remove_dir_all(dir);
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// A right-to-left ribbon too wide for the window starts at its right, New mail in view, and
/// what doesn't fit can be scrolled to (it ran off the left edge, out of reach).
#[test]
fn a_narrow_right_to_left_ribbon_scrolls() {
    let (mut a, dir) = app("rtl-ribbon");
    a.ui.language = "ar".into();
    let ctx = egui::Context::default();
    let size = egui::vec2(480.0, 700.0);
    for _ in 0..4 {
        drawn_text(&ctx, &mut a, size);
    }
    let texts = drawn_text(&ctx, &mut a, size);
    let new_mail = texts.iter().find(|(s, ..)| s == "بريد جديد").map(|(_, r, _)| *r);
    let new_mail = new_mail.unwrap_or_else(|| panic!("no New mail in {texts:?}"));
    assert!(new_mail.max.x <= size.x && new_mail.min.x > size.x / 2.0, "New mail at the right, in view: {new_mail:?}");
    // The ribbon's content starts within reach: nothing laid out left of the window.
    let ribbon_y = new_mail.center().y;
    let left_of_window = texts.iter().filter(|(_, r, _)| (r.center().y - ribbon_y).abs() < 12.0 && r.min.x < -1.0).count();
    assert_eq!(left_of_window, 0, "{texts:?}");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
}

/// Gap 2: a message's Hebrew and Arabic text is selected with the pointer, as it reads, and copied
/// whole (with stock egui 0.37's count, a selection there copied nothing, so it couldn't be
/// selected at all), Latin names, times and dates inside it included. Dragged from the start of
/// the line on the right to its end on the left. (A line ending in a number or Latin word reads it
/// left to right at the far left, so its left edge is that run's start, as in any bidi layout.)
#[test]
fn right_to_left_text_is_selected_and_copied() {
    for text in ["שלום עולם, מה שלומך?", "مرحبا بكم في البريد", "שלום Riley, הפגישה ב-14:30 ביום שלישי"]
    {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 200.0));
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let run = |events: Vec<Event>| {
            let mut out = ctx.run_ui(RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    rect.set(crate::bidi::selectable_label(ui, text, egui::FontId::proportional(16.0), egui::Color32::BLACK).rect);
                });
            });
            out.textures_delta.clear();
            out.platform_output.commands
        };
        run(vec![]);
        run(vec![]);
        let rect = rect.get();
        let (start, end) = (egui::pos2(rect.right() - 1.0, rect.center().y), egui::pos2(rect.left() + 1.0, rect.center().y));
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        run(vec![Event::PointerMoved(start), button(start, true)]);
        run(vec![Event::PointerMoved(rect.center())]);
        run(vec![Event::PointerMoved(end)]);
        run(vec![button(end, false)]);
        let copied: Vec<String> = run(vec![Event::Copy])
            .into_iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(copied, [text.to_string()], "dragged across {text:?}");
    }
}
