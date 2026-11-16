//! What's drawn, looked at as a reader would: text cut by its room's edge, and two texts drawn
//! over one another. In a small window and in languages with long words, where both happened.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use serde_json::json;

use super::*;

thread_local! {
    /// The window's size for the frames drawn on this thread.
    pub(crate) static SIZE: Cell<egui::Vec2> = const { Cell::new(egui::vec2(1280.0, 800.0)) };
    /// The language they're drawn in ("": as the app has it).
    static LANG: Cell<&'static str> = const { Cell::new("") };
    /// What a tour found, while one is looking: the finding, and the frame it was first in.
    static FOUND: RefCell<Option<BTreeMap<String, String>>> = const { RefCell::new(None) };
    static FRAME: Cell<usize> = const { Cell::new(0) };
}

/// The smallest window MailCraft goes to.
const SMALL: egui::Vec2 = crate::MIN_WINDOW_SIZE;

/// Frames in a window of `size`, in `lang` ("": as the app has it), until the test ends.
pub(crate) struct Dressed;

impl Dressed {
    pub(crate) fn new(size: egui::Vec2, lang: &'static str) -> Self {
        SIZE.with(|s| s.set(size));
        LANG.with(|l| l.set(lang));
        Self
    }
}

impl Drop for Dressed {
    fn drop(&mut self) {
        SIZE.with(|s| s.set(egui::vec2(1280.0, 800.0)));
        LANG.with(|l| l.set(""));
        crate::i18n::set_current(crate::i18n::Lang::EN);
    }
}

pub(crate) fn dress(app: &mut MailApp) {
    let lang = LANG.with(Cell::get);
    if !lang.is_empty() {
        app.ui.language = lang.into();
    }
}

/// A line of text as drawn: its letters' reach, and the room it's cut to.
struct Piece {
    text: String,
    ink: egui::Rect,
    clip: egui::Rect,
}

fn pieces(shapes: &[egui::epaint::ClippedShape]) -> Vec<Piece> {
    fn walk(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<Piece>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            egui::Shape::Text(t) if t.opacity_factor > 0.0 => {
                for r in &t.galley.rows {
                    let ink = r.row.glyphs.iter().filter(|g| !g.chr.is_whitespace()).map(|g| (g.pos.x, g.max_x()));
                    let (l, rt) = ink.fold((f32::INFINITY, f32::NEG_INFINITY), |(l, r), (a, b)| (l.min(a), r.max(b)));
                    if l >= rt {
                        continue;
                    }
                    let top = t.pos.y + r.pos.y;
                    let ink = egui::Rect::from_x_y_ranges(t.pos.x + r.pos.x + l..=t.pos.x + r.pos.x + rt, top..=top + r.row.size.y);
                    out.push(Piece { text: r.row.glyphs.iter().map(|g| g.chr).collect(), ink, clip });
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in shapes {
        walk(&s.shape, s.clip_rect, &mut out);
    }
    out
}

/// What reads badly in a frame: `cut` text (its letters run past the edge of its room, so it
/// stops mid-letter with no "…"), and text drawn `over` other text in the same room.
fn findings(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    let all = pieces(shapes);
    let mut out = Vec::new();
    for p in &all {
        let seen = p.ink.intersect(p.clip);
        if seen.width() > 0.0 && seen.height() > 2.0 && (p.ink.min.x < p.clip.min.x - 1.5 || p.ink.max.x > p.clip.max.x + 1.5) {
            out.push(format!("cut {:?}: {:?} in {:?}", p.text.chars().take(50).collect::<String>(), p.ink.x_range(), p.clip.x_range()));
        }
    }
    for (i, a) in all.iter().enumerate() {
        for b in all.iter().skip(i + 1).filter(|b| a.clip == b.clip && a.text != b.text) {
            let both = a.ink.intersect(b.ink).intersect(a.clip);
            if both.width() > 3.0 && both.height() > 4.0 {
                let short = |p: &Piece| p.text.chars().take(30).collect::<String>();
                out.push(format!("over {:?} / {:?}: {:?} and {:?}", short(a), short(b), a.ink, b.ink));
            }
        }
    }
    out
}

/// Every frame the tests draw comes by here; a tour that's looking keeps what it finds.
pub(crate) fn look(app: &MailApp, shapes: &[egui::epaint::ClippedShape]) {
    let n = FRAME.with(|f| {
        f.set(f.get() + 1);
        f.get()
    });
    FOUND.with(|found| {
        if let Some(found) = found.borrow_mut().as_mut() {
            let dialog = app.ui.dialog.as_ref().map(|d| format!("{d:?}").chars().take(28).collect::<String>()).unwrap_or_default();
            for f in findings(shapes) {
                found.entry(f).or_insert_with(|| format!("frame {n} {dialog}"));
            }
        }
    });
}

/// The frame as it is once it has settled (the ribbon fits itself over a frame or two), and
/// what reads badly in it.
fn settled(ctx: &egui::Context, a: &mut MailApp) -> (Vec<Piece>, Vec<String>) {
    for _ in 0..4 {
        frame(ctx, a, vec![]);
    }
    dress(a);
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE.with(Cell::get))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    (pieces(&out.shapes), findings(&out.shapes))
}

fn first_message(a: &MailApp) -> String {
    a.session.list.first().map(|m| m.key.to_id()).unwrap_or_default()
}

/// A frame in a window `w` wide and 800 tall (not dressed or looked at, as [`frame`]'s are).
fn sized(ctx: &egui::Context, a: &mut MailApp, w: f32) {
    sized_frame(ctx, a, egui::vec2(w, 800.0), vec![]);
}

/// In the smallest window the message read keeps a width it can be read in: the folders and the
/// list give way. It was a strip a word wide ("Storyboa / rd"), and a reply written there had a
/// ribbon of one button a row.
#[test]
fn a_small_window_keeps_room_to_read_and_write() {
    let _dressed = Dressed::new(SMALL, "de");
    let (mut a, _) = app("layout-small");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = first_message(&a);
    a.run("list.open", json!({"key": k}));
    let (drawn, found) = settled(&ctx, &mut a);
    let subject = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.subject.clone()).unwrap_or_default();
    let first_words: String = subject.split(' ').take(2).collect::<Vec<_>>().join(" ");
    assert!(first_words.contains(' '), "{subject:?}");
    // Right of the list (which shows the subject too), in a pane wide enough for words.
    let pane = drawn.iter().filter(|p| p.ink.min.x > 440.0 && p.text.starts_with(&first_words)).count();
    assert_eq!(pane, 1, "the subject's words whole in the pane: {found:?}");
    assert!(found.is_empty(), "{found:?}");
    // A reply there: the ribbon's rows one under the other, no room between them.
    // As on the desktop, where a message can go to a window of its own: Pop out is drawn.
    ctx.set_embed_viewports(false);
    a.run("mail.reply", json!({}));
    let (drawn, found) = settled(&ctx, &mut a);
    // Save draft and its row aren't under Discard and Pop out (a text box may be cut: it scrolls).
    let over: Vec<_> = found.iter().filter(|f| f.starts_with("over")).collect();
    assert!(over.is_empty(), "{over:?}");
    let top = |name: &str| drawn.iter().find(|p| p.text == crate::i18n::t(name)).map(|p| p.ink.min.y);
    let (book, link) = (top("Address Book"), top("Link"));
    assert!(book.is_some() && link.is_some(), "the ribbon's buttons are drawn");
    let apart = link.unwrap_or(0.0) - book.unwrap_or(0.0);
    assert!((0.0..=40.0).contains(&apart), "rows {apart} apart");
}

/// The calendar, tasks, people and the File page in the smallest window, in languages with long
/// words: nothing cut mid-letter, nothing over anything else.
#[test]
fn the_other_pages_fit_a_small_window() {
    for lang in ["fr", "de", "ru"] {
        let _dressed = Dressed::new(SMALL, lang);
        let (mut a, _) = app(&format!("layout-pages-{lang}"));
        let ctx = egui::Context::default();
        // Mail first: the folder pane is at its narrowest from there on.
        frame(&ctx, &mut a, vec![]);
        for (page, cmd) in [("calendar", "view.calendar"), ("tasks", "view.tasks"), ("people", "view.people")] {
            a.run(cmd, json!({}));
            let (drawn, found) = settled(&ctx, &mut a);
            assert!(found.is_empty(), "{lang} {page}: {found:?}");
            // The A–Z index beside the people: all of it above the status bar, within reach.
            if page == "people" {
                let index: Vec<_> = drawn.iter().filter(|p| p.text.chars().count() == 1 && p.text.chars().all(|c| c.is_ascii_lowercase())).collect();
                assert!(!index.is_empty(), "{lang}: the index is drawn");
                assert!(index.iter().all(|p| p.ink.max.y <= p.clip.max.y && p.ink.max.y < SMALL.y - 26.0), "{lang}: the index fits its height");
            }
        }
        a.run("view.calendar", json!({}));
        let (drawn, _) = settled(&ctx, &mut a);
        let all_day = crate::i18n::t("All day");
        assert!(drawn.iter().any(|p| p.text == all_day && p.ink.min.x >= p.clip.min.x), "{lang}: {all_day:?} whole");
        a.run("view.mail", json!({}));
        a.act(crate::Action::Backstage(crate::backstage::Page::Info));
        let (_, found) = settled(&ctx, &mut a);
        assert!(found.is_empty(), "{lang} File: {found:?}");
    }
}

/// A message opened by a command is brought into view in the list (it stayed out of sight,
/// below the rows shown); and a notice is in the corner where lines end, the left one in Arabic.
#[test]
fn an_opened_message_and_a_notice_are_where_they_are_looked_for() {
    let _dressed = Dressed::new(SMALL, "ar");
    let (mut a, _) = app("layout-opened");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let last = a.session.list.last().cloned().unwrap();
    let start: String = last.subject.chars().take(8).collect();
    let in_list = |drawn: &[Piece]| drawn.iter().any(|p| p.clip.width() < 295.0 && p.text.contains(&start) && p.ink.max.y <= p.clip.max.y);
    let (drawn, _) = settled(&ctx, &mut a);
    assert!(!in_list(&drawn), "the last message starts out of sight: {start:?}");
    a.run("list.open", json!({"key": last.key.to_id()}));
    // The list glides there over some frames.
    for _ in 0..12 {
        settled(&ctx, &mut a);
    }
    let (drawn, _) = settled(&ctx, &mut a);
    let seen: Vec<_> = drawn.iter().filter(|p| p.text.contains(&start)).map(|p| (p.ink, p.clip)).collect();
    assert!(in_list(&drawn), "opened, it's in view in the list: {seen:?}");
    a.session.notice("Archive.pst: 100 messages imported", false);
    let (drawn, _) = settled(&ctx, &mut a);
    let notice = drawn.iter().find(|p| p.text.starts_with("Archive.pst")).map(|p| p.ink);
    assert!(notice.is_some_and(|r| r.max.x < SMALL.x / 2.0), "{notice:?}");
}

/// A message in a window of its own: its toolbar fits the window, its last buttons without
/// their labels when it must (in French "Imprimer" ran under "Fermer", and what came after was
/// cut by the window's edge).
#[test]
fn a_message_windows_toolbar_fits() {
    let _dressed = Dressed::new(egui::vec2(900.0, 600.0), "fr");
    let (mut a, _) = app("layout-window");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = first_message(&a);
    a.run("message.popOut", json!({"key": k}));
    let (drawn, found) = settled(&ctx, &mut a);
    assert!(drawn.iter().any(|p| p.text == crate::i18n::t("Reply all")), "the window's toolbar is drawn");
    assert!(found.is_empty(), "{found:?}");
}

/// A subject of two thousand characters is a few lines over its message, not the whole pane;
/// and a conversation's messages in the pane keep clear of their times.
#[test]
fn a_long_subject_and_a_conversation_keep_their_room() {
    let _dressed = Dressed::new(SMALL, "en");
    let (mut a, _) = app("layout-subject");
    let store = a.session.store.clone().unwrap();
    let long = "long subject words ".repeat(120);
    let key = MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid: 1 };
    store.write_body(&key, format!("Subject: {long}\r\n\r\nbody\r\n").as_bytes()).unwrap();
    archive_of(&mut a, vec![Summary { key: key.clone(), subject: long.clone(), date: NOW - 3600, ..Default::default() }]);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("list.open", json!({"key": key.to_id()}));
    let (drawn, _) = settled(&ctx, &mut a);
    let lines = drawn.iter().filter(|p| p.clip.min.x > 400.0 && p.text.contains("long subject")).count();
    assert!((1..=4).contains(&lines), "{lines} lines of subject");
    assert!(drawn.iter().any(|p| p.text == "body"), "the message is in view under it");
    // The conversation in the sample Inbox, read in the pane.
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("view.conversations", json!({"on": true}));
    frame(&ctx, &mut a, vec![]);
    a.run("list.open", json!({"key": first_message(&a)}));
    let (drawn, found) = settled(&ctx, &mut a);
    assert!(drawn.iter().any(|p| p.text.contains("messages in this conversation")), "the conversation is listed");
    assert!(found.is_empty(), "{found:?}");
}

/// A tour of every pane and dialog at three window sizes in five languages, printing what
/// reads badly: `cargo test -p mailcraft-ui-egui layout::tour_findings -- --ignored --nocapture`.
/// Not everything it prints is wrong (a text box scrolls, a frame before the ribbon has fitted).
#[test]
#[ignore = "a probe: prints what it finds"]
fn tour_findings() {
    for (w, h) in [(1320.0, 820.0), (1000.0, 700.0), (760.0, 480.0)] {
        for lang in ["en", "de", "ru", "fr", "ar"] {
            let _dressed = Dressed::new(egui::vec2(w, h), lang);
            FRAME.with(|f| f.set(0));
            FOUND.with(|f| *f.borrow_mut() = Some(BTreeMap::new()));
            tour();
            let found = FOUND.with(|f| f.borrow_mut().take()).unwrap_or_default();
            println!("==== {w}x{h} {lang}: {}", found.len());
            for (k, v) in found {
                println!("{k}   <- {v}");
            }
        }
    }
}

/// Issue #8: the classic ribbon on every tab and module, light and dark, in a wide window and
/// a narrow one (it scrolls rather than dropping labels): drawn without trouble and taller than
/// the simplified one; switched off, the simplified ribbon comes back.
#[test]
fn the_classic_ribbon_draws_every_tab() {
    let (mut a, _) = app("classic-ribbon");
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let simplified = a.ui.ribbon_rect.expect("the ribbon is drawn").height();
    a.run("view.classicRibbon", json!({"on": true}));
    assert!(a.session.prefs.classic_ribbon);
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        for w in [1280.0, 520.0] {
            for module in ["mail", "calendar", "people", "tasks", "notes"] {
                a.run("view.module", json!({"name": module}));
                for tab in [crate::RibbonTab::Home, crate::RibbonTab::SendReceive, crate::RibbonTab::View] {
                    a.ui.tab = tab;
                    for _ in 0..3 {
                        sized(&ctx, &mut a, w);
                    }
                }
            }
            a.run("view.module", json!({"name": "mail"}));
            a.ui.tab = crate::RibbonTab::Home;
            sized(&ctx, &mut a, w);
            assert!(a.ui.ribbon_rect.unwrap().height() > simplified + 30.0, "{} vs {simplified}", a.ui.ribbon_rect.unwrap().height());
        }
    }
    a.run("view.classicRibbon", json!({}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!((a.ui.ribbon_rect.unwrap().height() - simplified).abs() < 1.0);
}

/// Not shown in groups, the list has no headings (Today, Yesterday…) between its messages;
/// shown in groups again, the headings are back.
#[test]
fn the_list_without_groups_has_no_headings() {
    let (mut a, _) = app("list-no-groups");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    let headings: Vec<&'static str> =
        a.session.rows().iter().filter_map(|r| if let mailcraft_engine::Row::Header(h) = r { Some(*h) } else { None }).collect();
    assert!(headings.len() > 1, "the sample's list has headings: {headings:?}");
    let drawn_headings = |ctx: &egui::Context, a: &mut MailApp| {
        let mut drawn = Vec::new();
        for _ in 0..2 {
            drawn = drawn_text(ctx, a, size);
        }
        drawn.iter().filter(|d| d.seen() && headings.contains(&d.text.as_str())).count()
    };
    assert!(drawn_headings(&ctx, &mut a) > 0, "headings are drawn");
    a.run("view.groups", json!({}));
    assert!(!a.session.prefs.groups);
    assert_eq!(drawn_headings(&ctx, &mut a), 0, "no heading is drawn");
    a.run("view.groups", json!({}));
    assert!(drawn_headings(&ctx, &mut a) > 0, "headings again");
}

/// The ribbon hidden: its buttons go and the list has their room, the tabs stay on the top bar;
/// a click on a tab shows its buttons until a click elsewhere, and Ctrl+F1 brings it back.
#[test]
fn the_ribbon_hides_and_a_tab_shows_it_for_now() {
    let (mut a, _) = app("ribbon-hidden");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let ribbon = a.ui.ribbon_rect.expect("the ribbon is drawn");
    let rows_before = a.ui.list_page;
    a.run("view.hideRibbon", json!({}));
    assert!(a.session.prefs.ribbon_hidden);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(a.ui.ribbon_rect.is_none(), "the ribbon's buttons are gone");
    assert!(a.ui.top_bar_rect.is_some(), "the top bar, with the tabs, stays");
    assert!(a.ui.list_page > rows_before, "{} rows, {rows_before} before", a.ui.list_page);
    // A tab shows its buttons, over where they were.
    click_named(&ctx, &mut a, "View");
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.tab, crate::RibbonTab::View);
    let shown = a.ui.ribbon_rect.expect("a tab clicked shows the ribbon");
    assert!((shown.height() - ribbon.height()).abs() < 1.0);
    // A click on the ribbon keeps it, one in the window below puts it away.
    click(&ctx, &mut a, egui::pos2(shown.right() - 4.0, shown.center().y));
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.ribbon_rect.is_some(), "a click on the ribbon keeps it");
    let size = SIZE.with(std::cell::Cell::get);
    click(&ctx, &mut a, egui::pos2(size.x - 30.0, size.y - 60.0));
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.ribbon_rect.is_none(), "a click elsewhere puts it away");
    assert!(a.session.prefs.ribbon_hidden);
    // The same tab clicked twice: shown, then put away.
    click_named(&ctx, &mut a, "Home");
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.ribbon_rect.is_some());
    click_named(&ctx, &mut a, "Home");
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.ribbon_rect.is_none(), "its tab clicked again puts it away");
    // Ctrl+F1 brings it back for good.
    frame(&ctx, &mut a, vec![key(egui::Key::F1, egui::Modifiers::COMMAND)]);
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(!a.session.prefs.ribbon_hidden);
    assert!(a.ui.ribbon_rect.is_some());
    assert_eq!(a.ui.list_page, rows_before);
}

/// The single-line list's head is one line: the folder's name, and on the same line the filter,
/// in a menu named by the one chosen; choosing another there filters the list.
#[test]
fn the_single_line_list_has_its_head_on_one_line() {
    let (mut a, _) = app("single-line-head");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = egui::vec2(1280.0, 800.0);
    let title = crate::view_title(&a.session);
    // The filter's menu, and the folder's name where it's drawn on that line (the folder pane has it too).
    let places = |ctx: &egui::Context, a: &mut MailApp, filter: &str| {
        let mut drawn = Vec::new();
        for _ in 0..2 {
            drawn = drawn_text(ctx, a, size);
        }
        let menu = drawn.iter().find(|d| d.text == filter && d.seen()).map(|d| d.rect).unwrap_or_else(|| panic!("{filter} isn't drawn"));
        let name = drawn.iter().filter(|d| d.text == title && d.seen()).map(|d| d.rect).find(|r| r.top() < menu.bottom() && r.bottom() > menu.top());
        (name, menu)
    };
    a.run("view.singleLine", json!({}));
    let (name, filter) = places(&ctx, &mut a, "All ⏷");
    let name = name.expect("the folder's name is on the filter's line");
    assert!(filter.left() > name.right(), "the filter after the name: {name:?} {filter:?}");
    // Its menu has the filters.
    click(&ctx, &mut a, filter.center());
    frame(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Unread");
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.filter, mailcraft_engine::ListFilter::Unread);
    places(&ctx, &mut a, "Unread ⏷");
}

/// The single-line list's head keeps the Inbox's Focused and Other: one of them clicked shows
/// that part of the Inbox, as in the list's usual head.
#[test]
fn the_single_line_list_keeps_focused_and_other() {
    let (mut a, _) = app("single-line-focused");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.session.prefs.focused_inbox = true;
    a.run("view.singleLine", json!({}));
    assert!(a.session.focus_split(), "the sample opens on its Inbox");
    let size = egui::vec2(1280.0, 800.0);
    let seen = |ctx: &egui::Context, a: &mut MailApp, text: &str| {
        let mut drawn = Vec::new();
        for _ in 0..2 {
            drawn = drawn_text(ctx, a, size);
        }
        drawn.iter().any(|d| d.text == text && d.seen())
    };
    assert!(seen(&ctx, &mut a, "Focused") && seen(&ctx, &mut a, "Other"), "Focused and Other on the head's line");
    assert!(!a.session.focus_other);
    click_named(&ctx, &mut a, "Other");
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.focus_other, "Other clicked shows the Inbox's other mail");
}

/// Shown for now by a tab while hidden, then kept (Ctrl+F1) and hidden again (Ctrl+F1): it hides,
/// as it would had no tab been clicked.
#[test]
fn the_ribbon_hidden_again_hides_after_a_tab_showed_it() {
    let (mut a, _) = app("ribbon-hidden-again");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("view.hideRibbon", json!({}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "View");
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.ribbon_rect.is_some(), "a tab clicked shows it");
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![key(egui::Key::F1, egui::Modifiers::COMMAND)]);
        frame(&ctx, &mut a, vec![]);
    }
    assert!(a.session.prefs.ribbon_hidden);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.ribbon_rect.is_none(), "hidden again, it's hidden");
}

/// Issue #22: the ribbon's tabs and the search box share the top bar, one row where there were
/// two, with the ribbon's buttons right under it, as tall on every tab and in every module, in a
/// wide window, the narrowest and a narrow one; the File page has the window to itself.
#[test]
fn the_tabs_and_search_share_one_bar() {
    let (mut a, _) = app("one-bar");
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let bar = a.ui.top_bar_rect.expect("the top bar is drawn");
    let ribbon = a.ui.ribbon_rect.expect("the ribbon is drawn");
    assert_eq!(bar.top(), 0.0);
    assert!((bar.height() - crate::app::frame::TOP_BAR_H).abs() < 0.5, "{bar:?}");
    assert!(ribbon.top() >= bar.bottom() && ribbon.top() - bar.bottom() < 8.0, "{bar:?} {ribbon:?}");
    // The bar and the ribbon together: 40 points less than the bar, the tabs and the buttons were.
    assert!(ribbon.bottom() < 82.0, "{ribbon:?}");
    let height = ribbon.height();
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        for plain in [false, true] {
            a.session.prefs.plain_top_bar = plain;
            for w in [2560.0, 1280.0, 760.0, 520.0] {
                for module in ["mail", "calendar", "people", "tasks", "notes"] {
                    a.run("view.module", json!({"name": module}));
                    for tab in [crate::RibbonTab::Home, crate::RibbonTab::SendReceive, crate::RibbonTab::View] {
                        a.ui.tab = tab;
                        for _ in 0..2 {
                            sized(&ctx, &mut a, w);
                        }
                        let r = a.ui.ribbon_rect.unwrap();
                        assert!((r.height() - height).abs() < 1.0, "{module} {tab:?} at {w}: {} vs {height}", r.height());
                        // Nothing pushed past the window's edge: the narrow one has the box on a
                        // row of its own.
                        assert!((a.ui.top_bar_rect.unwrap().width() - w).abs() < 1.0, "{module} {tab:?} at {w}: {:?}", a.ui.top_bar_rect);
                        assert_eq!(a.ui.bar_two_rows, w < 700.0, "{module} {tab:?} at {w}");
                    }
                }
            }
        }
    }
    a.session.prefs.plain_top_bar = false;
    a.run("view.module", json!({"name": "mail"}));
    // Searching: the Search tab joins the others on the bar (and stays while the search does),
    // the ribbon as tall.
    a.run("search.run", json!({"query": "storyboard"}));
    a.ui.tab = crate::RibbonTab::Search;
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(a.ui.tab, crate::RibbonTab::Search);
    assert!((a.ui.ribbon_rect.unwrap().height() - height).abs() < 1.0);
    a.run("search.clear", json!({}));
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.top_bar_rect.is_none() && a.ui.ribbon_rect.is_none(), "the File page has no bar");
}

/// The single-line list: a message's sender, subject and time on one line, the subject after
/// the sender's column, and more messages in the same height; switched off, the subject is
/// back under the sender.
#[test]
fn the_single_line_list_has_a_message_on_one_line() {
    let (mut a, _) = app("single-line-list");
    let ctx = egui::Context::default();
    let (name, subject) = ("Alexandra Whitfield", "Quarterly figures for the board");
    let d = mailcraft_engine::mime::Draft { subject: subject.into(), body: "Attached.".into(), ..Default::default() };
    let from = mailcraft_engine::model::EmailAddress::new(name, "alexandra@example.com");
    let raw = mailcraft_engine::mime::build(&d, &from, NOW, "<single@x>").unwrap();
    with_raw(&mut a, 9902, &raw);
    a.session.prefs.reading_pane = ReadingPane::Bottom;
    let size = egui::vec2(1280.0, 800.0);
    let places = |ctx: &egui::Context, a: &mut MailApp| {
        let mut drawn = Vec::new();
        for _ in 0..2 {
            drawn = drawn_text(ctx, a, size);
        }
        let of = |text: &str| drawn.iter().find(|d| d.text == text && d.seen()).map(|d| d.rect).unwrap_or_else(|| panic!("{text} isn't drawn"));
        (of(name), of(subject))
    };
    let (who, what) = places(&ctx, &mut a);
    assert!(what.top() > who.bottom() - 4.0, "the subject under the sender: {who:?} {what:?}");
    let page = a.ui.list_page;
    a.run("view.singleLine", json!({}));
    assert!(a.session.prefs.single_line);
    let (who, what) = places(&ctx, &mut a);
    assert!((what.center().y - who.center().y).abs() < 3.0, "on one line: {who:?} {what:?}");
    assert!(what.left() > who.right(), "the subject after the sender: {who:?} {what:?}");
    assert!(a.ui.list_page > page, "more rows a page: {} then {page}", a.ui.list_page);
    // Every other setting of the list with it, light and dark, wide and narrow, right to left.
    for (compact, preview) in [(true, false), (false, true)] {
        a.session.prefs.compact = compact;
        a.session.prefs.show_preview = preview;
        for pane in [ReadingPane::Right, ReadingPane::Off, ReadingPane::Bottom] {
            a.session.prefs.reading_pane = pane;
            for w in [1280.0, 520.0] {
                sized(&ctx, &mut a, w);
            }
        }
    }
    let (who, what) = places(&ctx, &mut a);
    assert!((what.center().y - who.center().y).abs() < 3.0, "still on one line: {who:?} {what:?}");
    a.run("view.singleLine", json!({"on": false}));
    let (who, what) = places(&ctx, &mut a);
    assert!(what.top() > who.bottom() - 4.0, "the subject under the sender again: {who:?} {what:?}");
}
