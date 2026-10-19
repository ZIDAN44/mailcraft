//! Interface languages. Strings in the code stay English and are the lookup keys; a catalog per
//! language (`*.tsv`) maps them to what the interface shows. Command ids, mail, file names, the
//! CLI and agents never see translated text. A string without a translation shows in English, so
//! catalogs can grow one entry at a time.
//!
//! The format and habits are the Crafting Apps' (WordCraft, PdfCraft): three TAB-separated
//! columns (`context`, English, translation), `\n` `\t` `\\` escapes, `{name}` placeholders kept
//! in both columns, `#` comments.
//!
//! The language follows the system's (Windows display language, macOS preferred languages,
//! `LANG`/`LC_*`; `MAILCRAFT_LANGUAGE` overrides) unless one is chosen in Settings.
//!
//! # Adding a language
//! Add `xx.tsv` next to `es.tsv` (translate from the *meaning* of the English, never from another
//! product's localisation) and a row to [`LANGUAGES`]. The tests check every catalog's entries
//! and that it covers every string the interface draws.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The language setting that follows the system.
pub const AUTO: &str = "auto";

pub struct LangInfo {
    /// BCP 47, lower case (`pt-br`, `zh-hans`); also the setting's value.
    pub code: &'static str,
    /// The language's own name, for the Settings list.
    pub name: &'static str,
    source: &'static str,
    /// Needs a CJK font.
    pub cjk: bool,
    /// Written right to left (its text goes through [`crate::bidi`]).
    pub rtl: bool,
    catalog: OnceLock<HashMap<String, String>>,
}

pub static LANGUAGES: [LangInfo; 14] = [
    LangInfo { code: "en", name: "English", source: "", cjk: false, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "es", name: "Español", source: include_str!("es.tsv"), cjk: false, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "fr", name: "Français", source: include_str!("fr.tsv"), cjk: false, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "de", name: "Deutsch", source: include_str!("de.tsv"), cjk: false, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "nl", name: "Nederlands", source: include_str!("nl.tsv"), cjk: false, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "pt-br", name: "Português (Brasil)", source: include_str!("pt-br.tsv"), cjk: false, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "it", name: "Italiano", source: include_str!("it.tsv"), cjk: false, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "ru", name: "Русский", source: include_str!("ru.tsv"), cjk: false, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "ja", name: "日本語", source: include_str!("ja.tsv"), cjk: true, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "ko", name: "한국어", source: include_str!("ko.tsv"), cjk: true, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "zh-hans", name: "简体中文", source: include_str!("zh-hans.tsv"), cjk: true, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "zh-hant", name: "繁體中文", source: include_str!("zh-hant.tsv"), cjk: true, rtl: false, catalog: OnceLock::new() },
    LangInfo { code: "ar", name: "العربية", source: include_str!("ar.tsv"), cjk: false, rtl: true, catalog: OnceLock::new() },
    LangInfo { code: "he", name: "עברית", source: include_str!("he.tsv"), cjk: false, rtl: true, catalog: OnceLock::new() },
];

fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(o) => return Err(format!("unknown escape \\{o}")),
            None => return Err("trailing backslash".into()),
        }
    }
    Ok(out)
}

/// `{name}` placeholders of a template, sorted.
pub fn placeholders(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some((_, after)) = rest.split_once('{') {
        let Some((name, tail)) = after.split_once('}') else { break };
        names.push(name);
        rest = tail;
    }
    names.sort_unstable();
    names
}

/// The entries of a catalog, and the errors of the lines that were skipped.
pub fn parse(text: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut errors = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(ctx), Some(src), Some(tr), None) = (cols.next(), cols.next(), cols.next(), cols.next()) else {
            errors.push(format!("line {}: expected context<TAB>source<TAB>translation", i + 1));
            continue;
        };
        match (unescape(src), unescape(tr)) {
            (Ok(s), Ok(t)) if !s.is_empty() && !t.is_empty() => {
                if placeholders(&s) != placeholders(&t) {
                    errors.push(format!("line {}: placeholders differ: {s:?}", i + 1));
                } else if s.ends_with('…') != t.ends_with('…') {
                    errors.push(format!("line {}: trailing ellipsis differs: {s:?}", i + 1));
                } else {
                    let key = if ctx.is_empty() { s } else { format!("{ctx}{CTX}{s}") };
                    if entries.iter().any(|(e, _)| *e == key) {
                        errors.push(format!("line {}: duplicate {key:?}", i + 1));
                    } else {
                        entries.push((key, t));
                    }
                }
            }
            (Err(e), _) | (_, Err(e)) => errors.push(format!("line {}: {e}", i + 1)),
            _ => errors.push(format!("line {}: empty column", i + 1)),
        }
    }
    (entries, errors)
}

impl LangInfo {
    fn catalog(&self) -> &HashMap<String, String> {
        self.catalog.get_or_init(|| {
            let (entries, errors) = parse(self.source);
            for e in errors {
                log::warn!("{} catalog: {e} (shown in English)", self.code);
            }
            entries.into_iter().collect()
        })
    }
}

/// A language the interface can be shown in.
#[derive(Clone, Copy)]
pub struct Lang(&'static LangInfo);

impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        self.0.code == other.0.code
    }
}

impl std::fmt::Debug for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lang({})", self.0.code)
    }
}

impl Lang {
    pub const EN: Lang = Lang(&LANGUAGES[0]);

    pub fn code(self) -> &'static str {
        self.0.code
    }
    pub fn name(self) -> &'static str {
        self.0.name
    }
    pub fn cjk(self) -> bool {
        self.0.cjk
    }
    pub fn rtl(self) -> bool {
        self.0.rtl
    }
    pub fn all() -> impl Iterator<Item = Lang> {
        LANGUAGES.iter().map(Lang)
    }
    pub fn from_code(code: &str) -> Option<Lang> {
        LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code)).map(Lang)
    }
    /// The setting (`auto` or a code) as a language.
    pub fn from_pref(pref: &str) -> Lang {
        Lang::from_code(pref).unwrap_or_else(system_lang)
    }
}

/// The language for a locale tag (`pt_BR.UTF-8`, `zh-Hans-CN`, `ja-JP`), if there's a catalog.
pub fn lang_from_tag(tag: &str) -> Option<Lang> {
    let base = tag.split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    let parts: Vec<&str> = base.split('-').filter(|p| !p.is_empty()).collect();
    let primary = *parts.first()?;
    if matches!(primary, "c" | "posix") {
        return Some(Lang::EN);
    }
    if primary == "zh" {
        // The script decides; without one, the region (Taiwan, Hong Kong, Macau write Traditional).
        let traditional = parts.iter().any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo")) && !parts.contains(&"hans");
        return Lang::from_code(if traditional { "zh-hant" } else { "zh-hans" });
    }
    // Hebrew's old code.
    if primary == "iw" {
        return Lang::from_code("he");
    }
    if primary == "pt" {
        return parts.contains(&"br").then(|| Lang::from_code("pt-br")).flatten();
    }
    Lang::from_code(primary)
}

/// The system's language when there's a catalog for it; English otherwise (and in tests).
pub fn system_lang() -> Lang {
    if cfg!(test) {
        return Lang::EN;
    }
    static SYSTEM: OnceLock<Lang> = OnceLock::new();
    *SYSTEM.get_or_init(|| {
        if let Some(l) = std::env::var("MAILCRAFT_LANGUAGE").ok().and_then(|v| lang_from_tag(&v)) {
            return l;
        }
        sys_locale::get_locales().take(16).find_map(|t| lang_from_tag(&t)).unwrap_or(Lang::EN)
    })
}

thread_local! {
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::EN) };
    /// Text is wanted as written, not as egui draws it (for the system: window titles).
    static AS_WRITTEN: Cell<bool> = const { Cell::new(false) };
}

/// Run `f` with translations as written (the system lays out right-to-left text itself: window
/// titles, notifications).
pub fn as_written<R>(f: impl FnOnce() -> R) -> R {
    let before = AS_WRITTEN.replace(true);
    let r = f();
    AS_WRITTEN.set(before);
    r
}

/// Whether drawn text of the current language goes through [`crate::bidi`].
fn reorder() -> bool {
    current().rtl() && !AS_WRITTEN.get()
}

/// Set the language the interface is drawn in (each frame, from the setting).
pub fn set_current(lang: Lang) {
    CURRENT.set(lang);
}

pub fn current() -> Lang {
    CURRENT.get()
}

/// `s` in the current language, ready to draw.
pub fn t(s: &str) -> &str {
    shown(tr(current(), s))
}

/// A template in the current language with its `{name}` values filled in, ready to draw (see
/// `tf!`).
pub fn tfmt(template: &str, args: &[(&str, &str)]) -> String {
    let filled = fmt(tr(current(), template), args);
    if reorder() { crate::bidi::line(&filled, None) } else { filled }
}

/// Interface text of a right-to-left language in the order egui draws right ([`crate::bidi`]);
/// kept for the session, as catalogs are.
fn shown(s: &str) -> &str {
    if !reorder() || !crate::bidi::has_rtl(s) {
        return s;
    }
    thread_local! {
        static SHOWN: std::cell::RefCell<HashMap<String, &'static str>> = std::cell::RefCell::new(HashMap::new());
    }
    SHOWN.with(|m| {
        if let Some(v) = m.borrow().get(s) {
            return *v;
        }
        // The direction comes from the first letter: a line that starts with a name or an
        // address reads left to right, as people write it.
        let v: &'static str = Box::leak(crate::bidi::line(s, None).into_boxed_str());
        m.borrow_mut().insert(s.to_string(), v);
        v
    })
}

/// Separates a context from the English in catalog keys.
const CTX: char = '\u{4}';

/// `s` where its context tells words that are the same in English apart (`folder`: the
/// Archive folder, not the Archive command); the plain translation without one.
pub fn tc<'a>(ctx: &str, s: &'a str) -> &'a str {
    #[cfg(test)]
    if pseudo::on() {
        return pseudo::mark(s);
    }
    current().0.catalog().get(&format!("{ctx}{CTX}{s}")).map_or_else(|| t(s), |v| shown(v))
}

/// A folder's name as shown: the name its server gives a special folder as it is (Exchange
/// names them in the mailbox's language, as Outlook shows them), any other translated.
pub fn folder(f: &mailcraft_engine::model::Folder) -> String {
    match f.server_name() {
        Some(n) => n.to_string(),
        None => tc("folder", &f.display_name()).to_string(),
    }
}

/// `s` in `lang` (unchanged without a translation).
pub fn tr(lang: Lang, s: &str) -> &str {
    #[cfg(test)]
    if pseudo::on() {
        return pseudo::mark(s);
    }
    lang.0.catalog().get(s).map_or(s, String::as_str)
}

/// Fill `{name}` placeholders in one pass (values are never read as templates).
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        out.push_str(before);
        let Some((name, tail)) = after.split_once('}') else {
            out.push('{');
            out.push_str(after);
            return out;
        };
        match args.iter().find(|(k, _)| *k == name) {
            Some((_, v)) => out.push_str(v),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// A date or time from `dates` with its English day and month names translated.
pub fn date(s: &str) -> String {
    const NAMES: [&str; 26] = [
        "Mon",
        "Tue",
        "Wed",
        "Thu",
        "Fri",
        "Sat",
        "Sun",
        "Jan",
        "Feb",
        "Mar",
        "Apr",
        "May",
        "Jun",
        "Jul",
        "Aug",
        "Sep",
        "Oct",
        "Nov",
        "Dec",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    if current() == Lang::EN {
        return s.to_string();
    }
    let out = s.split(' ').map(|w| if NAMES.contains(&w) { tr(current(), w) } else { w }).collect::<Vec<_>>().join(" ");
    if reorder() { crate::bidi::line(&out, None) } else { out }
}

#[cfg(test)]
pub(crate) mod pseudo {
    //! A test-only pseudo-language: every looked-up string is recorded and comes back wrapped in
    //! `⟦…⟧`, so the coverage test can list what the interface draws.
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeSet, HashMap};

    thread_local! {
        static ON: Cell<bool> = const { Cell::new(false) };
        static CACHE: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new());
        static SEEN: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    }

    pub fn on() -> bool {
        ON.get()
    }
    pub fn set(on: bool) {
        ON.set(on);
    }
    pub fn seen() -> BTreeSet<String> {
        SEEN.with_borrow(Clone::clone)
    }
    pub fn mark(s: &str) -> &'static str {
        SEEN.with_borrow_mut(|l| l.insert(s.to_string()));
        CACHE.with_borrow_mut(|c| *c.entry(s.to_string()).or_insert_with(|| Box::leak(format!("⟦{s}⟧").into_boxed_str())))
    }
}

#[cfg(test)]
mod tests;
