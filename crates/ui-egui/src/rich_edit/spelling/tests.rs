use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use super::*;

fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    crate::lock(LOCK.get_or_init(|| Mutex::new(())))
}

struct DirGuard {
    root: PathBuf,
    explicit: Vec<&'static str>,
}

impl DirGuard {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("mailcraft-spell-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::create_dir_all(&root);
        Self { root, explicit: Vec::new() }
    }
}

impl Drop for DirGuard {
    fn drop(&mut self) {
        set_dictionaries_dir(None);
        for l in self.explicit.drain(..) {
            memory::remove_explicit(l);
        }
        follow(true, None, &BTreeSet::new());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn marks(text: &str, typing: Option<usize>) -> Vec<String> {
    let mut m = Marks::default();
    m.follow(text, typing);
    m.marks.iter().map(|r| text.chars().skip(r.start).take(r.end - r.start).collect()).collect()
}

fn marks_in(m: &mut Marks, text: &str) -> Vec<String> {
    m.follow(text, None);
    m.marks.iter().map(|r| text.chars().skip(r.start).take(r.end - r.start).collect()).collect()
}

#[test]
fn misspelled_words_are_marked_by_character() {
    let _g = test_lock();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    follow(true, None, &BTreeSet::new());
    assert_eq!(marks("OK — the meeting is tomorow.\nWe recieve it.", None), ["tomorow", "recieve"]);
    // Addresses, links, capitals and words with digits are left alone; a quoted line too.
    assert!(marks("Write to ana@studio.example or see https://studio.example/xyzzq, NASA, mp3", None).is_empty());
    assert!(marks("> I wrot this", None).is_empty());
    // The word being typed waits until it's left.
    assert_eq!(marks("Hello wrld", Some(10)), Vec::<String>::new());
    assert_eq!(marks("Hello wrld ", Some(11)), ["wrld"]);
    // Off: none.
    follow(false, None, &BTreeSet::new());
    assert!(marks("tomorow", None).is_empty());
    follow(true, None, &BTreeSet::new());
}

#[test]
fn words_added_to_the_dictionary_are_spelled_right() {
    let _g = test_lock();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    follow(true, None, &BTreeSet::new());
    assert_eq!(marks("Ask Qwzorbit about it", None), ["Qwzorbit"]);
    follow(true, None, &BTreeSet::from(["qwzorbit".to_string()]));
    assert!(marks("Ask Qwzorbit about it", None).is_empty());
    follow(true, None, &BTreeSet::new());
    assert_eq!(marks("Ask Qwzorbit about it", None), ["Qwzorbit"], "taken out again");
}

/// Ignore all: the word is taken as spelled right until MailCraft closes (a word no other
/// test uses: the checker's list is the process's).
#[test]
fn a_word_ignored_is_marked_no_more() {
    let _g = test_lock();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    follow(true, None, &BTreeSet::new());
    assert_eq!(marks("Zxqvintor again", None), ["Zxqvintor"]);
    ignore("Zxqvintor");
    assert!(marks("Zxqvintor again", None).is_empty());
}

#[test]
fn a_misspelled_word_is_offered_its_suggestions() {
    let _g = test_lock();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    follow(true, None, &BTreeSet::new());
    let text = "See you tomorow.";
    let mut m = Marks::default();
    m.follow(text, None);
    let o = Offer::at(&m, text, 10).expect("a misspelled word there");
    assert_eq!((o.word.as_str(), o.range.clone()), ("tomorow", 8..15));
    assert!(o.suggestions.iter().any(|s| s == "tomorrow"), "{:?}", o.suggestions);
    assert!(Offer::at(&m, text, 2).is_none(), "a word spelled right");
}

/// The editor's frame with these input events: what it asked for and what it drew.
fn frame(
    ctx: &egui::Context,
    ed: &mut super::super::Editor,
    c: &mut mailcraft_engine::Compose,
    events: Vec<egui::Event>,
) -> (Vec<crate::Action>, Vec<egui::Shape>) {
    let t = crate::theme::Tokens::light();
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 600.0))), events, ..Default::default() };
    let mut acts = Vec::new();
    let mut out = ctx.run_ui(raw, |ui| super::super::body(ui, &t, ed, c, None, &mut acts));
    out.textures_delta.clear();
    (acts, out.shapes.into_iter().map(|s| s.shape).collect())
}

/// Where the red wavy line is drawn (its left end), and where `text` is drawn.
fn underline_at(shapes: &[egui::Shape]) -> Option<Pos2> {
    let red = crate::theme::Tokens::light().danger;
    shapes.iter().find_map(|s| match s {
        egui::Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(red) => p.points.first().copied(),
        _ => None,
    })
}
fn text_at(shapes: &[egui::Shape], text: &str) -> Option<Rect> {
    shapes.iter().rev().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
        _ => None,
    })
}

/// Issue #174, in the editor: a misspelled word is underlined; its right-click menu's first
/// suggestion takes its place; Add to dictionary asks for the command, Ignore all takes the
/// line off until MailCraft closes.
#[test]
fn the_editor_underlines_and_corrects_a_misspelled_word() {
    let _g = test_lock();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    follow(true, None, &BTreeSet::new());
    let ctx = egui::Context::default();
    let mut ed = super::super::Editor::new(7);
    let mut c = mailcraft_engine::Compose { body: "See you tomorow then".into(), ..Default::default() };
    c.styles = mailcraft_engine::mime::rich::Styles::plain(&c.body);
    let (_, shapes) = frame(&ctx, &mut ed, &mut c, vec![]);
    let line = underline_at(&shapes).expect("the word underlined");
    let word = line + egui::vec2(8.0, -6.0);
    let right = |pressed| egui::Event::PointerButton { pos: word, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &mut ed, &mut c, vec![egui::Event::PointerMoved(word)]);
    frame(&ctx, &mut ed, &mut c, vec![right(true)]);
    frame(&ctx, &mut ed, &mut c, vec![right(false)]);
    let (_, shapes) = frame(&ctx, &mut ed, &mut c, vec![]);
    for item in ["Ignore all", "Add to dictionary", "Cut"] {
        assert!(text_at(&shapes, item).is_some(), "{item} in the menu");
    }
    let fix = text_at(&shapes, "tomorrow").expect("the suggestion in the menu").center();
    let left = |pressed| egui::Event::PointerButton { pos: fix, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &mut ed, &mut c, vec![egui::Event::PointerMoved(fix)]);
    frame(&ctx, &mut ed, &mut c, vec![left(true)]);
    frame(&ctx, &mut ed, &mut c, vec![left(false)]);
    assert_eq!(c.body, "See you tomorrow then");
    let (_, shapes) = frame(&ctx, &mut ed, &mut c, vec![]);
    assert!(underline_at(&shapes).is_none(), "spelled right now");

    // Another: Add to dictionary asks for the command with it.
    c.body = "Ask Qwzorbat now".into();
    c.styles = mailcraft_engine::mime::rich::Styles::plain(&c.body);
    let (_, shapes) = frame(&ctx, &mut ed, &mut c, vec![]);
    let word = underline_at(&shapes).expect("underlined") + egui::vec2(8.0, -6.0);
    let right = |pressed| egui::Event::PointerButton { pos: word, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &mut ed, &mut c, vec![egui::Event::PointerMoved(word)]);
    frame(&ctx, &mut ed, &mut c, vec![right(true)]);
    frame(&ctx, &mut ed, &mut c, vec![right(false)]);
    let (_, shapes) = frame(&ctx, &mut ed, &mut c, vec![]);
    let add = text_at(&shapes, "Add to dictionary").expect("in the menu").center();
    let left = |pressed| egui::Event::PointerButton { pos: add, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &mut ed, &mut c, vec![egui::Event::PointerMoved(add)]);
    frame(&ctx, &mut ed, &mut c, vec![left(true)]);
    let (acts, _) = frame(&ctx, &mut ed, &mut c, vec![left(false)]);
    assert!(acts.iter().any(|a| matches!(a, crate::Action::Run("spelling.addWord", p) if p["word"] == "Qwzorbat")), "asked to add it");

    // A block of the original under a reply: left alone.
    let mut quoted = super::super::Editor::new(9);
    quoted.min_height = Some(0.0);
    let (_, shapes) = frame(&ctx, &mut quoted, &mut c, vec![]);
    assert!(underline_at(&shapes).is_none(), "the original isn't checked");
}

#[test]
fn discovers_multiple_dictionaries_across_user_system_and_libreoffice_dirs() {
    let guard = DirGuard::new("discover");
    let user = guard.root.join("user_dicts");
    let sys = guard.root.join("sys_hunspell");
    let lo_bg = guard.root.join("lo").join("dict-bg");
    let lo_de = guard.root.join("lo").join("dict-de");
    for d in [&user, &sys, &lo_bg, &lo_de] {
        std::fs::create_dir_all(d).unwrap();
    }
    // User dir overrides sys for en_GB; sys provides pl_PL plus hyphenation/thesaurus noise.
    std::fs::write(user.join("en_GB.aff"), "SET UTF-8\n").unwrap();
    std::fs::write(user.join("en_GB.dic"), "1\ncolour\n").unwrap();
    std::fs::write(sys.join("en_GB.aff"), "SET UTF-8\n").unwrap();
    std::fs::write(sys.join("en_GB.dic"), "1\nsystem\n").unwrap();
    std::fs::write(sys.join("pl_PL.aff"), "SET UTF-8\n").unwrap();
    std::fs::write(sys.join("pl_PL.dic"), "1\ndzień\n").unwrap();
    // Orphan .dic (no .aff), hyphenation and thesaurus files must be ignored.
    std::fs::write(sys.join("orphan.dic"), "1\nnope\n").unwrap();
    std::fs::write(sys.join("hyph_pl_PL.aff"), "SET UTF-8\n").unwrap();
    std::fs::write(sys.join("hyph_pl_PL.dic"), "UTF-8\n").unwrap();
    std::fs::write(sys.join("th_pl_PL_v2.aff"), "SET UTF-8\n").unwrap();
    std::fs::write(sys.join("th_pl_PL_v2.dic"), "UTF-8\n").unwrap();
    // LibreOffice non-uniform filenames (de_DE_frami -> de_DE, bg_BG -> bg_BG).
    std::fs::write(lo_bg.join("bg_BG.aff"), "SET UTF-8\n").unwrap();
    std::fs::write(lo_bg.join("bg_BG.dic"), "1\nздравей\n").unwrap();
    std::fs::write(lo_de.join("de_DE_frami.aff"), "SET UTF-8\n").unwrap();
    std::fs::write(lo_de.join("de_DE_frami.dic"), "1\nHallo\n").unwrap();

    let found = discover::scan_dirs(&[user.clone(), sys, lo_bg, lo_de]);
    let locales: Vec<&str> = found.iter().map(|e| e.locale.as_str()).collect();
    assert_eq!(locales, ["bg_BG", "de_DE", "en_GB", "pl_PL"]);
    let en_gb = found.iter().find(|e| e.locale == "en_GB").unwrap();
    assert_eq!(en_gb.aff.as_deref(), Some(user.join("en_GB.aff").as_path()), "user dir takes priority");
}

#[test]
fn loads_and_checks_dictionary_from_custom_user_directory() {
    let _g = test_lock();
    let guard = DirGuard::new("custom_dir");
    let dict_dir = guard.root.join("dictionaries");
    std::fs::create_dir_all(&dict_dir).unwrap();
    std::fs::write(dict_dir.join("bg_BG.aff"), "SET UTF-8\n").unwrap();
    std::fs::write(dict_dir.join("bg_BG.dic"), "2\nздравей\nписмо\n").unwrap();

    set_dictionaries_dir(Some(dict_dir));
    assert!(available_dictionaries().iter().any(|d| d.locale == "bg_BG"));

    follow(true, Some("bg_BG"), &BTreeSet::new());
    assert!(has_dictionary_for("bg_BG"));
    let mut m = Marks::default();
    assert_eq!(marks_in(&mut m, "здравей писмъ"), ["писмъ"]);
    let o = Offer::at(&m, "здравей писмъ", 10).expect("misspelled Bulgarian word");
    assert!(o.suggestions.iter().any(|s| s == "писмо"), "{:?}", o.suggestions);
}

#[test]
fn switching_language_invalidates_line_cache_and_preserves_user_and_ignored_words() {
    let _g = test_lock();
    let mut guard = DirGuard::new("switch");
    crate::i18n::set_current(crate::i18n::Lang::EN);

    // Language A: English (built-in) -> "demain" is misspelled, "hello" is valid.
    // Language B: French (in-memory Hunspell) -> "demain" and "monde" are valid, "hello" is misspelled.
    install_in_memory("fr_FR", b"SET UTF-8\n", b"2\ndemain\nmonde\n").unwrap();
    guard.explicit.push("fr_FR");

    let custom_word = "XyzzyKeepWord".to_string();
    let ignored_word = "ZqxIgnoredOnce";
    let words = BTreeSet::from([custom_word.clone()]);

    follow(true, Some("en_US"), &words);
    ignore(ignored_word);

    let text = "demain hello XyzzyKeepWord ZqxIgnoredOnce NASA mp3 ana@studio.example https://studio.example\n> hello quoted";
    let mut m = Marks::default();
    // Populate line cache under en_US.
    let marked_en = marks_in(&mut m, text);
    assert_eq!(marked_en, ["demain"]);

    // Hot-swap to fr_FR on the SAME Marks instance without recreating it.
    follow(true, Some("fr_FR"), &words);
    let marked_fr = marks_in(&mut m, text);
    assert_eq!(
        marked_fr,
        ["hello"],
        "line cache invalidated on language switch while preserving user word, ignored word, NASA, mp3, URL, email, quote"
    );
}

#[test]
fn in_memory_dictionary_works_without_filesystem() {
    let _g = test_lock();
    let mut guard = DirGuard::new("in_mem");
    install_in_memory("xx_YY", b"SET UTF-8\nSFX A Y 1\nSFX A 0 s .\n", b"2\nkazva/A\ndobre\n").unwrap();
    guard.explicit.push("xx_YY");

    assert!(has_dictionary_for("xx_YY"));
    follow(true, Some("xx_YY"), &BTreeSet::new());
    let mut m = Marks::default();
    assert_eq!(marks_in(&mut m, "kazvas dobre dober"), ["dober"]);
    let offer = Offer::at(&m, "kazvas dobre dober", 14).unwrap();
    assert!(offer.suggestions.iter().any(|s| s == "dobre"), "{:?}", offer.suggestions);
}

#[test]
fn hunspell_dictionaries_skip_other_scripts() {
    let _g = test_lock();
    let mut guard = DirGuard::new("other_scripts");

    // 25 Latin stems (>170 chars) + 1 stray 1-letter Cyrillic loanword: only Latin reaches 1%.
    let mut latin_dic = String::from("26\n");
    for w in [
        "hello", "world", "message", "letter", "morning", "evening", "meeting", "schedule", "calendar", "tomorrow", "project", "account", "sending",
        "reading", "writing", "folder", "archive", "subject", "preview", "example", "window", "options", "drafts", "outbox", "mailbox",
    ] {
        latin_dic.push_str(w);
        latin_dic.push('\n');
    }
    latin_dic.push_str("я\n");

    install_in_memory("xx_LA", b"SET UTF-8\n", latin_dic.as_bytes()).unwrap();
    guard.explicit.push("xx_LA");
    install_in_memory("xx_BG", b"SET UTF-8\n", "2\nздравей\nписмо\n".as_bytes()).unwrap();
    guard.explicit.push("xx_BG");

    let mut m = Marks::default();
    follow(true, Some("xx_LA"), &BTreeSet::new());
    assert_eq!(marks_in(&mut m, "hello tomorow здравей писмъ"), ["tomorow"]);

    follow(true, Some("xx_BG"), &BTreeSet::new());
    assert_eq!(marks_in(&mut m, "hello tomorow здравей писмъ"), ["писмъ"]);
}
