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

/// The smallest window MailCraft goes to (`apps/mailcraft`: its `with_min_inner_size`).
const SMALL: egui::Vec2 = egui::vec2(760.0, 480.0);

/// Frames in a window of `size`, in `lang`, until the test ends.
struct Dressed;

impl Dressed {
    fn new(size: egui::Vec2, lang: &'static str) -> Self {
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

/// In the smallest window the message read keeps a width it can be read in: the folders and the
/// list give way. It was a strip a word wide ("Storyboa / rd"), and a reply written there had a
/// ribbon of one button a row.
#[test]
fn a_small_window_keeps_room_to_read_and_write() {
    let _dressed = Dressed::new(SMALL, "de");
    let (mut a, dir) = app("layout-small");
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
    let _ = std::fs::remove_dir_all(dir);
}

/// The calendar, tasks, people and the File page in the smallest window, in languages with long
/// words: nothing cut mid-letter, nothing over anything else.
#[test]
fn the_other_pages_fit_a_small_window() {
    for lang in ["fr", "de", "ru"] {
        let _dressed = Dressed::new(SMALL, lang);
        let (mut a, dir) = app(&format!("layout-pages-{lang}"));
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
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// A message opened by a command is brought into view in the list (it stayed out of sight,
/// below the rows shown); and a notice is in the corner where lines end, the left one in Arabic.
#[test]
fn an_opened_message_and_a_notice_are_where_they_are_looked_for() {
    let _dressed = Dressed::new(SMALL, "ar");
    let (mut a, dir) = app("layout-opened");
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
    let _ = std::fs::remove_dir_all(dir);
}

/// A message in a window of its own: its toolbar fits the window, its last buttons without
/// their labels when it must (in French "Imprimer" ran under "Fermer", and what came after was
/// cut by the window's edge).
#[test]
fn a_message_windows_toolbar_fits() {
    let _dressed = Dressed::new(egui::vec2(900.0, 600.0), "fr");
    let (mut a, dir) = app("layout-window");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = first_message(&a);
    a.run("message.popOut", json!({"key": k}));
    let (drawn, found) = settled(&ctx, &mut a);
    assert!(drawn.iter().any(|p| p.text == crate::i18n::t("Reply all")), "the window's toolbar is drawn");
    assert!(found.is_empty(), "{found:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A subject of two thousand characters is a few lines over its message, not the whole pane;
/// and a conversation's messages in the pane keep clear of their times.
#[test]
fn a_long_subject_and_a_conversation_keep_their_room() {
    let _dressed = Dressed::new(SMALL, "en");
    let (mut a, dir) = app("layout-subject");
    let store = a.session.store.clone().unwrap();
    let long = "long subject words ".repeat(120);
    let key = MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid: 1 };
    let messages = vec![Summary { key: key.clone(), subject: long.clone(), date: NOW - 3600, ..Default::default() }];
    let state = FolderState { validity: 1, uid_next: 2, highest_modseq: 0, messages: messages.into_iter().map(Into::into).collect(), window: 0 };
    store.save_folder("sample", "Archive", &state).unwrap();
    store.write_body(&key, format!("Subject: {long}\r\n\r\nbody\r\n").as_bytes()).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "Archive".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "Archive"}));
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
    let _ = std::fs::remove_dir_all(dir);
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
