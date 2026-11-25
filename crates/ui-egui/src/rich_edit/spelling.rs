//! Spelling as you type (issue #174), as Outlook checks it: a word the dictionary doesn't know
//! is underlined with a red wavy line, and its right-click menu offers the dictionary's
//! suggestions, Ignore all (until MailCraft closes) and Add to dictionary (kept with the
//! preferences). The spelling language can be chosen independently of the interface language and
//! switched live in the compose editor; missing Hunspell dictionaries can be downloaded directly.
//!
//! The checking is powered by `craft-proof`: URLs, addresses, words in capitals, words with
//! digits, `>` quoted lines, and original reply blocks are skipped.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::PathBuf;

use craft_proof::ProofLang;
use egui::{Color32, Galley, Pos2, Rect, RichText, Stroke, Ui};
use mailcraft_engine::Compose;
use serde_json::json;

use super::carets::Carets;
use super::{Editor, replace, typing_style};
use crate::theme::Tokens;
use crate::{Action, Dialog, DictionaryDownload};

pub(crate) mod catalog;
pub(crate) mod discover;
pub(crate) mod memory;
#[cfg(test)]
mod tests;

pub(crate) use catalog::{DictSource, display_name, find_source as download_source};
pub(crate) use discover::discover_all as available_dictionaries;

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
    static PREF_LANG: RefCell<Option<String>> = const { RefCell::new(None) };
    static WORDS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    static WORDS_LOWER: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    static IGNORED: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    static DOWNLOAD_STATUS: RefCell<(bool, DictionaryDownload)> = const { RefCell::new((false, DictionaryDownload::Idle)) };
}

/// Register the per-user `<app_data_dir>/dictionaries/` directory.
pub fn set_dictionaries_dir(dir: Option<PathBuf>) {
    discover::set_dictionaries_dir(dir);
}

/// Validate a raw Hunspell `.aff`/`.dic` pair before saving or installing.
pub fn validate_hunspell(locale: &str, aff: &[u8], dic: &[u8]) -> Result<(), String> {
    memory::validate(locale, aff, dic).map(|_| ())
}

/// Install an in-memory Hunspell `.aff`/`.dic` pair for `locale` (used by WASM IndexedDB cache
/// and tests).
pub fn install_in_memory(locale: &str, aff: &[u8], dic: &[u8]) -> Result<(), String> {
    memory::install(locale, aff, dic)
}

/// Reload Hunspell dictionaries from disk after installing new files.
pub fn reload_dictionaries() {
    discover::reload_dictionaries();
}

/// All curated downloadable dictionaries.
pub(crate) fn downloadable_dictionaries() -> &'static [DictSource] {
    catalog::CATALOG
}

/// Update the UI-visible download status snapshot (synced every frame in `MailApp::pump`).
pub(crate) fn set_download_status(can_download: bool, state: DictionaryDownload) {
    DOWNLOAD_STATUS.with_borrow_mut(|s| *s = (can_download, state));
}

/// Current dictionary download status snapshot (`(can_download, DictionaryDownload)`).
pub(crate) fn download_status() -> (bool, DictionaryDownload) {
    DOWNLOAD_STATUS.with_borrow(Clone::clone)
}

/// Format active dictionary download progress using `checked_div` to avoid checked-division lints.
pub(crate) fn download_progress_text(done: u64, total: u64) -> String {
    if let Some(pct) = done.saturating_mul(100).checked_div(total) {
        tf!("Downloading… {pct}%", pct = pct.min(100))
    } else {
        tl!("Downloading…").to_string()
    }
}

/// Each frame, from the preferences: whether to check as you type, the explicit spelling language
/// (`None` follows UI language), and the words added to the dictionary.
pub(crate) fn follow(on: bool, spelling_language: Option<&str>, words: &BTreeSet<String>) {
    ON.set(on);
    let next_lang = spelling_language.map(catalog::normalize_locale).filter(|s| !s.is_empty());
    PREF_LANG.with_borrow_mut(|cur| {
        if *cur != next_lang {
            *cur = next_lang;
        }
    });
    WORDS.with_borrow_mut(|had| {
        if had != words {
            for w in had.difference(words) {
                craft_proof::remove_word(w);
            }
            for w in words.difference(had) {
                craft_proof::add_word(w);
            }
            had.clone_from(words);
            WORDS_LOWER.with_borrow_mut(|low| {
                *low = words.iter().map(|w| w.to_lowercase()).collect();
            });
            memory::bump_generation();
        }
    });
}

/// Whether spelling is checked as you type.
pub(crate) fn on() -> bool {
    ON.get()
}

/// The explicitly configured spelling language preference (`None`: follows UI language).
pub(crate) fn pref_language() -> Option<String> {
    PREF_LANG.with_borrow(Clone::clone)
}

/// The active Hunspell locale code (`en_US`, `bg_BG`, …).
pub(crate) fn active_locale() -> String {
    if let Some(explicit) = pref_language() {
        return explicit;
    }
    catalog::ui_code_to_locale(crate::i18n::current().code()).to_string()
}

/// Compact badge label for the active spelling language on the compose toolbar (`EN`, `BG`, …).
pub(crate) fn active_badge() -> String {
    let active = active_locale();
    let primary = active.split('_').next().unwrap_or("");
    let available = available_dictionaries();
    let same_primary = available.iter().filter(|d| d.locale.split('_').next() == Some(primary)).count();
    catalog::short_badge(&active, same_primary > 1)
}

/// Whether there is an installed or built-in dictionary for `locale`.
pub(crate) fn has_dictionary_for(locale: &str) -> bool {
    let norm = catalog::normalize_locale(locale);
    if norm.is_empty() {
        return false;
    }
    let lang = ProofLang::from_tag(Some(&norm));
    if lang == ProofLang::Other {
        return false;
    }
    if memory::has(&norm) || discover::find_entry(&norm).is_some() {
        return true;
    }
    lang.is_english() || craft_proof::missing_dictionary(Some(&norm)).is_none()
}

/// The misspelled words of a message, as last worked out, and each line's own (a change checks
/// the lines it changed).
#[derive(Default)]
pub(crate) struct Marks {
    /// What the marks are for: text hash, active locale, craft-proof gen, memory gen, typing caret.
    key: Option<(u64, String, u64, u64, Option<usize>)>,
    /// Locale and generations that populated `lines`.
    lines_for: Option<(String, u64, u64)>,
    /// The misspelled words, by character.
    pub(super) marks: Vec<Range<usize>>,
    /// Each line's misspelled words (bytes of the line), by its text.
    lines: HashMap<String, Vec<Range<usize>>>,
    /// What the right-click menu offers for the word right-clicked.
    pub(super) offer: Option<Offer>,
}

impl Marks {
    /// Drop cached line results and current marks immediately (called on language switch).
    pub(crate) fn invalidate(&mut self) {
        self.lines.clear();
        self.marks.clear();
        self.key = None;
        self.lines_for = None;
        self.offer = None;
    }

    /// Work the marks out for `text` when it changed: every misspelled word but the one at
    /// `typing` (a character position, while a word is being typed there), and none in a line
    /// quoted with `>`.
    pub(super) fn follow(&mut self, text: &str, typing: Option<usize>) {
        let locale = active_locale();
        if !on() || !has_dictionary_for(&locale) {
            self.marks.clear();
            self.lines.clear();
            self.key = None;
            self.lines_for = None;
            return;
        }
        let cp_gen = craft_proof::dictionary_generation();
        let mem_gen = memory::generation();
        let key = (hash(text), locale.clone(), cp_gen, mem_gen, typing);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        let line_key = (locale.clone(), cp_gen, mem_gen);
        if self.lines_for.as_ref() != Some(&line_key) || self.lines.len() > LINES_KEPT {
            self.lines.clear();
            self.lines_for = Some(line_key);
        }
        self.key = Some(key);
        self.marks.clear();
        let lang = ProofLang::from_tag(Some(&locale));
        let mut at = 0usize;
        for line in text.split('\n') {
            let n = line.chars().count();
            if !line.trim_start().starts_with('>') && line.len() <= LINE_MAX && self.marks.len() < MARKS_MAX {
                let found = self.lines.entry(line.to_string()).or_insert_with(|| check_line_with(line, &locale, lang));
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

fn check_line_with(line: &str, locale: &str, lang: ProofLang) -> Vec<Range<usize>> {
    if let Some(dict) = memory::get_or_load(locale) {
        return WORDS_LOWER.with_borrow(|words| IGNORED.with_borrow(|ignored| memory::check_line(line, &dict, words, ignored)));
    }
    craft_proof::check_spelling_in(line, lang).into_iter().filter_map(|i| line.get(i.start..i.end).map(|_| i.start..i.end)).collect()
}

fn suggest_with(word: &str, max: usize, locale: &str) -> Vec<String> {
    if let Some(dict) = memory::get_or_load(locale) {
        return memory::suggest(word, max, &dict);
    }
    craft_proof::suggest_in(word, max, ProofLang::from_tag(Some(locale)))
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
        let locale = active_locale();
        let suggestions = suggest_with(&word, SUGGESTIONS, &locale);
        Some(Offer { range, word, suggestions })
    }
}

/// Ignore a word until MailCraft closes (Ignore all).
pub(super) fn ignore(word: &str) {
    craft_proof::ignore_word(word);
    IGNORED.with_borrow_mut(|set| {
        if set.insert(word.to_lowercase()) {
            memory::bump_generation();
        }
    });
}

/// Language switcher menu items shared between the compose ribbon dropdown and editor context menu.
pub(super) fn language_menu_items(ui: &mut Ui, ed: &mut Editor, acts: &mut Vec<Action>) {
    let explicit = pref_language();
    if ui.add(egui::Button::selectable(explicit.is_none(), tl!("Follow interface language"))).clicked() {
        acts.push(Action::Run("options.spellingLanguage", json!({ "language": null })));
        ed.spelling.invalidate();
        ed.refocus = Some(ed.sel.clone());
        ui.close();
    }
    ui.separator();
    for entry in available_dictionaries() {
        let on = explicit.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(&entry.locale));
        let label = format!("{} ({})", entry.name, entry.locale);
        if ui.add(egui::Button::selectable(on, label)).clicked() {
            acts.push(Action::Run("options.spellingLanguage", json!({ "language": entry.locale })));
            ed.spelling.invalidate();
            ed.refocus = Some(ed.sel.clone());
            ui.close();
        }
    }
    if download_status().0 {
        ui.separator();
        if ui.button(tl!("Download more dictionaries…")).clicked() {
            acts.push(Action::Dialog(Dialog::Dictionaries { filter: String::new() }));
            ui.close();
        }
    }
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
