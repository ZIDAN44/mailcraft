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
