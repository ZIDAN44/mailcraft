//! Spelling as you type (issue #174), as Outlook checks it: a word the dictionary doesn't know
//! is underlined with a red wavy line, and its right-click menu offers the dictionary's
//! suggestions, Ignore all (until MailCraft closes) and Add to dictionary (kept with the
//! preferences). The text is checked in the interface's language, when there's a dictionary for
//! it (English always: the Craft apps' own word list; others from the Hunspell dictionaries the
//! system has); a word being typed isn't marked until it's left.
//!
//! The checking is the Craft apps' shared `craft-proof`: it takes URLs, addresses, words in
//! capitals and words with digits as spelled right.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

use craft_proof::ProofLang;
use egui::{Color32, Galley, Pos2, Rect, RichText, Stroke, Ui};
use mailcraft_engine::Compose;

use super::carets::Carets;
use super::{Editor, replace, typing_style};
use crate::Action;
use crate::theme::Tokens;

/// Most lines' results kept, and most marks drawn.
const LINES_KEPT: usize = 4_000;
const MARKS_MAX: usize = 1_000;
/// The longest line checked (a pasted block with no line breaks is checked as far as this).
const LINE_MAX: usize = 20_000;
/// Suggestions offered for a word.
pub(super) const SUGGESTIONS: usize = 5;
/// A word typed isn't checked for this long after the last change under the cursor.
const TYPING_SECS: f64 = 1.5;

thread_local! {
    static ON: Cell<bool> = const { Cell::new(true) };
    static WORDS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

/// Each frame, from the preferences: whether to check as you type, and the words added to the
/// dictionary (given to the checker when they change).
pub(crate) fn follow(on: bool, words: &BTreeSet<String>) {
    ON.set(on);
    WORDS.with_borrow_mut(|had| {
        if had != words {
            for w in had.difference(words) {
                craft_proof::remove_word(w);
            }
            for w in words.difference(had) {
                craft_proof::add_word(w);
            }
            had.clone_from(words);
        }
    });
}

/// Whether spelling is checked as you type.
pub(crate) fn on() -> bool {
    ON.get()
}

/// The language the text is checked in: the interface's.
pub(crate) fn language() -> ProofLang {
    ProofLang::from_tag(Some(crate::i18n::current().code()))
}

/// Whether there's a dictionary for `lang` here (English always has one).
pub(crate) fn has_dictionary(lang: ProofLang) -> bool {
    lang.is_proofed()
}

/// The misspelled words of a message, as last worked out, and each line's own (a change checks
/// the lines it changed).
#[derive(Default)]
pub(crate) struct Marks {
    /// What the marks are for: the text's hash, the language, the dictionary's generation and
    /// the word left alone while it's typed.
    key: Option<(u64, String, u64, Option<usize>)>,
    /// The misspelled words, by character.
    pub(super) marks: Vec<Range<usize>>,
    /// Each line's misspelled words (bytes of the line), by its text.
    lines: HashMap<String, Vec<Range<usize>>>,
    /// What the right-click menu offers for the word right-clicked.
    pub(super) offer: Option<Offer>,
}

impl Marks {
    /// Work the marks out for `text` when it changed: every misspelled word but the one at
    /// `typing` (a character position, while a word is being typed there), and none in a line
    /// quoted with `>`.
    pub(super) fn follow(&mut self, text: &str, typing: Option<usize>) {
        let lang = language();
        if !on() || !has_dictionary(lang) {
            self.marks.clear();
            self.key = None;
            return;
        }
        let generation = craft_proof::dictionary_generation();
        let key = (hash(text), lang.tag().into_owned(), generation, typing);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        if self.key.as_ref().is_some_and(|k| k.1 != key.1 || k.2 != key.2) || self.lines.len() > LINES_KEPT {
            self.lines.clear();
        }
        self.key = Some(key);
        self.marks.clear();
        let mut at = 0usize;
        for line in text.split('\n') {
            let n = line.chars().count();
            if !line.trim_start().starts_with('>') && line.len() <= LINE_MAX && self.marks.len() < MARKS_MAX {
                let found = self.lines.entry(line.to_string()).or_insert_with(|| {
                    craft_proof::check_spelling_in(line, lang).into_iter().filter_map(|i| line.get(i.start..i.end).map(|_| i.start..i.end)).collect()
                });
                for r in found.iter() {
                    let (Some(a), Some(b)) = (line.get(..r.start), line.get(..r.end)) else { continue };
                    let chars = at + a.chars().count()..at + b.chars().count();
                    if typing.is_some_and(|t| chars.start <= t && t <= chars.end) {
                        continue;
                    }
                    self.marks.push(chars);
                }
            }
            at += n + 1;
        }
        self.marks.truncate(MARKS_MAX);
    }

    /// The misspelled word at character `at`, if one is marked there.
    pub(super) fn at(&self, at: usize) -> Option<Range<usize>> {
        self.marks.iter().find(|r| r.start <= at && at <= r.end).cloned()
    }
}

fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// What the right-click menu offers for a misspelled word: the word (its characters and
/// text) and the dictionary's suggestions.
#[derive(Clone, Debug, Default)]
pub(crate) struct Offer {
    pub(super) range: Range<usize>,
    pub(super) word: String,
    pub(super) suggestions: Vec<String>,
}

impl Offer {
    /// The offer for the misspelled word at character `at` of `text`, if one is marked there.
    pub(super) fn at(marks: &Marks, text: &str, at: usize) -> Option<Offer> {
        let range = marks.at(at)?;
        let word: String = text.chars().skip(range.start).take(range.end - range.start).collect();
        let suggestions = craft_proof::suggest_in(&word, SUGGESTIONS, language());
        Some(Offer { range, word, suggestions })
    }
}

/// Ignore a word until MailCraft closes (Ignore all).
pub(super) fn ignore(word: &str) {
    craft_proof::ignore_word(word);
}

/// The editor's misspelled words, once its text is laid out: worked out, underlined, and the one
/// right-clicked kept for the menu. The one being typed (the text changed under the cursor a
/// moment ago) waits until it's left, or the moment is over. Not in the original under a reply
/// (its blocks are text boxes of their own), as Outlook leaves it alone.
pub(super) fn marked(ui: &Ui, t: &Tokens, ed: &mut Editor, c: &Compose, out: &egui::text_edit::TextEditOutput, right_press: bool) {
    let now = ui.input(|i| i.time);
    let typing = (out.response.has_focus() && now - ed.last_change < TYPING_SECS).then_some(ed.sel.end);
    if ed.min_height.is_none() {
        ed.spelling.follow(&c.body, typing);
    } else {
        ed.spelling.marks.clear();
    }
    if typing.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(TYPING_SECS));
    }
    underline(ui.painter(), &out.galley, out.galley_pos, &ed.spelling.marks, ui.clip_rect(), t.danger);
    if right_press && out.response.hovered() {
        ed.spelling.offer = out.response.hover_pos().and_then(|p| {
            let at = out.galley.cursor_from_pos(p - out.galley_pos).index.0;
            Offer::at(&ed.spelling, &c.body, at)
        });
    }
}

/// A misspelled word's part of the right-click menu: its suggestions (one replaces it, in its
/// look), Ignore all, Add to dictionary.
pub(super) fn menu_items(ui: &mut Ui, ed: &mut Editor, c: &mut Compose, offer: &Offer, acts: &mut Vec<Action>) {
    if offer.suggestions.is_empty() {
        ui.add_enabled(false, egui::Button::new(tl!("No suggestions")));
    }
    for s in &offer.suggestions {
        if ui.add_enabled(!c.sending, egui::Button::new(RichText::new(s).strong())).clicked() {
            let style = typing_style(&c.body, &c.styles, offer.range.start + 1);
            replace(ed, c, offer.range.clone(), s, style);
            ed.refocus = Some(ed.sel.clone());
            ed.spelling.offer = None;
            ui.close();
        }
    }
    if ui.button(tl!("Ignore all")).clicked() {
        ignore(&offer.word);
        ed.refocus = Some(ed.sel.clone());
        ed.spelling.offer = None;
        ui.close();
    }
    if ui.button(tl!("Add to dictionary")).clicked() {
        acts.push(Action::Run("spelling.addWord", serde_json::json!({ "word": offer.word })));
        ed.refocus = Some(ed.sel.clone());
        ed.spelling.offer = None;
        ui.close();
    }
    ui.separator();
}

/// The red wavy line under each misspelled word, those in `clip`.
pub(super) fn underline(painter: &egui::Painter, g: &Galley, origin: Pos2, marks: &[Range<usize>], clip: Rect, color: Color32) {
    if marks.is_empty() {
        return;
    }
    let mut carets = Carets::new(g);
    let stroke = Stroke::new(1.0, color);
    for r in marks {
        let (a, b) = (carets.at(r.start), carets.at(r.end));
        // A word broken over two rows: under the part on the first.
        let right = if (b.min.y - a.min.y).abs() < 0.5 { b.min.x } else { a.min.x + 6.0 };
        let (x0, x1, y) = (origin.x + a.min.x, origin.x + right, origin.y + a.max.y - 1.0);
        if y < clip.top() || y - 4.0 > clip.bottom() || x1 <= x0 {
            continue;
        }
        let mut points = Vec::new();
        let mut x = x0;
        let mut up = false;
        while x < x1 {
            points.push(egui::pos2(x, if up { y - 1.5 } else { y }));
            x += 2.0;
            up = !up;
        }
        points.push(egui::pos2(x1, if up { y - 1.5 } else { y }));
        painter.add(egui::Shape::line(points, stroke));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(text: &str, typing: Option<usize>) -> Vec<String> {
        let mut m = Marks::default();
        m.follow(text, typing);
        m.marks.iter().map(|r| text.chars().skip(r.start).take(r.end - r.start).collect()).collect()
    }

    #[test]
    fn misspelled_words_are_marked_by_character() {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        follow(true, &BTreeSet::new());
        assert_eq!(marks("OK — the meeting is tomorow.\nWe recieve it.", None), ["tomorow", "recieve"]);
        // Addresses, links, capitals and words with digits are left alone; a quoted line too.
        assert!(marks("Write to ana@studio.example or see https://studio.example/xyzzq, NASA, mp3", None).is_empty());
        assert!(marks("> I wrot this", None).is_empty());
        // The word being typed waits until it's left.
        assert_eq!(marks("Hello wrld", Some(10)), Vec::<String>::new());
        assert_eq!(marks("Hello wrld ", Some(11)), ["wrld"]);
        // Off: none.
        follow(false, &BTreeSet::new());
        assert!(marks("tomorow", None).is_empty());
        follow(true, &BTreeSet::new());
    }

    #[test]
    fn words_added_to_the_dictionary_are_spelled_right() {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        follow(true, &BTreeSet::new());
        assert_eq!(marks("Ask Qwzorbit about it", None), ["Qwzorbit"]);
        follow(true, &BTreeSet::from(["qwzorbit".to_string()]));
        assert!(marks("Ask Qwzorbit about it", None).is_empty());
        follow(true, &BTreeSet::new());
        assert_eq!(marks("Ask Qwzorbit about it", None), ["Qwzorbit"], "taken out again");
    }

    /// Ignore all: the word is taken as spelled right until MailCraft closes (a word no other
    /// test uses: the checker's list is the process's).
    #[test]
    fn a_word_ignored_is_marked_no_more() {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        follow(true, &BTreeSet::new());
        assert_eq!(marks("Zxqvintor again", None), ["Zxqvintor"]);
        ignore("Zxqvintor");
        assert!(marks("Zxqvintor again", None).is_empty());
    }

    #[test]
    fn a_misspelled_word_is_offered_its_suggestions() {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        follow(true, &BTreeSet::new());
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
        crate::i18n::set_current(crate::i18n::Lang::EN);
        follow(true, &BTreeSet::new());
        let ctx = egui::Context::default();
        let mut ed = super::super::Editor::new(7);
        let mut c = mailcraft_engine::Compose { body: "See you tomorow then".into(), ..Default::default() };
        c.styles = mailcraft_engine::mime::rich::Styles::plain(&c.body);
        let (_, shapes) = frame(&ctx, &mut ed, &mut c, vec![]);
        let line = underline_at(&shapes).expect("the word underlined");
        let word = line + egui::vec2(8.0, -6.0);
        let right =
            |pressed| egui::Event::PointerButton { pos: word, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
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
        let right =
            |pressed| egui::Event::PointerButton { pos: word, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
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
}
