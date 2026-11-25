//! In-memory and dynamically loaded Hunspell affix dictionaries (`AffixDict`), safe on both
//! desktop and WASM with zero `unsafe`.

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use craft_proof::affix::AffixDict;

use super::{catalog, discover};

/// Maximum file size read into memory for a Hunspell `.aff` or `.dic` file (32 MiB).
#[cfg(not(target_arch = "wasm32"))]
const MAX_DICT_BYTES: u64 = 32 * 1024 * 1024;

/// Loaded Hunspell dictionary paired with its detected scripts.
pub(crate) struct LoadedDict {
    pub(crate) dict: AffixDict,
    scripts: Vec<u32>,
}

impl LoadedDict {
    fn new(dict: AffixDict) -> Self {
        let scripts = detect_scripts(&dict);
        Self { dict, scripts }
    }
}

#[derive(Default)]
struct MemoryStore {
    /// Explicitly installed byte dictionaries (WASM IndexedDB / downloaded bytes / tests).
    explicit: HashMap<String, Arc<LoadedDict>>,
    /// Lazily loaded dictionaries discovered on disk outside `craft-proof`'s built-in cache.
    disk: HashMap<String, Arc<LoadedDict>>,
}

static STORE: OnceLock<RwLock<MemoryStore>> = OnceLock::new();
static GEN: AtomicU64 = AtomicU64::new(0);

fn store() -> &'static RwLock<MemoryStore> {
    STORE.get_or_init(|| RwLock::new(MemoryStore::default()))
}

pub(crate) fn generation() -> u64 {
    GEN.load(Ordering::Relaxed)
}

pub(crate) fn bump_generation() {
    GEN.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn clear_disk_cache() {
    store().write().unwrap_or_else(std::sync::PoisonError::into_inner).disk.clear();
    bump_generation();
}

/// Validate a raw Hunspell `.aff`/`.dic` byte pair and parse it into an [`AffixDict`].
pub(crate) fn validate(locale: &str, aff: &[u8], dic: &[u8]) -> Result<AffixDict, String> {
    let (aff_s, dic_s) = craft_proof::hunspell::decode(aff, dic)?;
    let dict = if catalog::normalize_locale(locale).starts_with("en") {
        AffixDict::from_hunspell_english(&aff_s, &dic_s, &[])?
    } else {
        AffixDict::from_hunspell(&aff_s, &dic_s, &[])?
    };
    if dict.is_empty() {
        return Err("the dictionary contains no words".into());
    }
    Ok(dict)
}

/// Decode, validate, and install a Hunspell `.aff`/`.dic` byte pair in memory (for WASM and tests).
pub(crate) fn install(locale: &str, aff: &[u8], dic: &[u8]) -> Result<(), String> {
    let norm = catalog::normalize_locale(locale);
    let dict = validate(&norm, aff, dic)?;
    install_proof_slot(&norm, aff, dic);
    store().write().unwrap_or_else(std::sync::PoisonError::into_inner).explicit.insert(norm, Arc::new(LoadedDict::new(dict)));
    bump_generation();
    discover::invalidate_cache();
    Ok(())
}

/// Populate `craft-proof`'s own built-in slot when `locale` has one, so `craft-proof` tuning also
/// benefits from the in-memory bytes.
fn install_proof_slot(norm: &str, aff: &[u8], dic: &[u8]) {
    let Ok((aff_s, dic_s)) = craft_proof::hunspell::decode(aff, dic) else { return };
    let primary = norm.split('_').next().unwrap_or(norm);
    let _ = match primary {
        "pl" => craft_proof::pl::install_dictionary(&aff_s, &dic_s),
        "pt" => craft_proof::pt::install_dictionary(&aff_s, &dic_s),
        "it" => craft_proof::it::install_dictionary(&aff_s, &dic_s),
        "hr" => craft_proof::hr::install_dictionary(&aff_s, &dic_s),
        "tr" => craft_proof::tr::install_dictionary(&aff_s, &dic_s),
        "ru" => craft_proof::ru::install_dictionary(&aff_s, &dic_s),
        "cs" => craft_proof::cs::install_dictionary(&aff_s, &dic_s),
        "id" => craft_proof::id::install_dictionary(&aff_s, &dic_s),
        "es" => {
            let lang = craft_proof::ProofLang::from_tag(Some(norm));
            craft_proof::es::install_dictionary(lang, &aff_s, &dic_s)
        }
        _ => Ok(()),
    };
}

/// Remove an explicitly installed in-memory dictionary (used by tests for cleanup).
#[cfg(test)]
pub(crate) fn remove_explicit(locale: &str) {
    let norm = catalog::normalize_locale(locale);
    store().write().unwrap_or_else(std::sync::PoisonError::into_inner).explicit.remove(&norm);
    bump_generation();
    discover::invalidate_cache();
}

pub(crate) fn installed_locales() -> Vec<String> {
    store().read().unwrap_or_else(std::sync::PoisonError::into_inner).explicit.keys().cloned().collect()
}

pub(crate) fn has(locale: &str) -> bool {
    let norm = catalog::normalize_locale(locale);
    store().read().unwrap_or_else(std::sync::PoisonError::into_inner).explicit.contains_key(&norm)
}

/// Return the [`LoadedDict`] for `locale` if explicitly installed in memory or loadable from a
/// discovered disk `.aff`/`.dic` pair. Returns `None` for built-in English without custom files so
/// `craft-proof` handles it directly.
pub(crate) fn get_or_load(locale: &str) -> Option<Arc<LoadedDict>> {
    let norm = catalog::normalize_locale(locale);
    {
        let s = store().read().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(d) = s.explicit.get(&norm) {
            return Some(Arc::clone(d));
        }
        if let Some(d) = s.disk.get(&norm) {
            return Some(Arc::clone(d));
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let entry = discover::find_entry(&norm)?;
        let (Some(aff_p), Some(dic_p)) = (entry.aff, entry.dic) else { return None };
        // `craft_proof::packs::installed()` entries point to zip pack paths, not raw `.aff`/`.dic`.
        let is_pair = aff_p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("aff"))
            && dic_p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("dic"));
        if !is_pair {
            return None;
        }
        let aff_bytes = read_capped(&aff_p).ok()?;
        let dic_bytes = read_capped(&dic_p).ok()?;
        let dict = match validate(&norm, &aff_bytes, &dic_bytes) {
            Ok(d) => Arc::new(LoadedDict::new(d)),
            Err(e) => {
                log::warn!("spelling dictionary `{norm}` couldn't be loaded: {e}");
                return None;
            }
        };
        store().write().unwrap_or_else(std::sync::PoisonError::into_inner).disk.insert(norm, Arc::clone(&dict));
        Some(dict)
    }
    #[cfg(target_arch = "wasm32")]
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn read_capped(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    file.take(MAX_DICT_BYTES + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() as u64 > MAX_DICT_BYTES {
        return Err("dictionary file is too large".into());
    }
    Ok(out)
}

/// Check spelling on a single line against `loaded`, honoring MailCraft's ignore rules (other
/// scripts, URLs, email addresses, ALL-CAPS words, words with digits, `words`, and `ignored`).
pub(crate) fn check_line(line: &str, loaded: &LoadedDict, words_lower: &BTreeSet<String>, ignored: &BTreeSet<String>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut token = (0usize, 0usize, false);
    for (a, b) in craft_proof::words(line) {
        if a < token.0 || a >= token.1 {
            token = enclosing_token(line, a, b);
        }
        if token.2 {
            continue;
        }
        let Some(raw) = line.get(a..b) else { continue };
        let w = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’').trim_matches(['\'', '’']);
        if w.chars().count() <= 1
            || !w.chars().any(char::is_alphabetic)
            || w.chars().any(|c| c.is_ascii_digit())
            || in_capitals(w)
            || other_script(w, &loaded.scripts)
        {
            continue;
        }
        let known = |part: &str| {
            let lower = part.to_lowercase();
            words_lower.contains(&lower) || ignored.contains(&lower) || loaded.dict.check(part)
        };
        if known(w) {
            continue;
        }
        if w.contains('-') && w.split('-').all(|p| p.chars().count() <= 1 || known(p)) {
            continue;
        }
        out.push(a..b);
    }
    out
}

/// Suggest corrections for `word` from `loaded`.
pub(crate) fn suggest(word: &str, max: usize, loaded: &LoadedDict) -> Vec<String> {
    loaded.dict.suggest(word, max)
}

fn script_of(c: char) -> u32 {
    match c as u32 {
        0x0041..=0x02AF | 0x1E00..=0x1EFF | 0x2C60..=0x2C7F | 0xA720..=0xA7FF | 0xAB30..=0xAB6F | 0xFF21..=0xFF5A => 1,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => 2,
        0x0400..=0x052F | 0x1C80..=0x1C8F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => 3,
        0x0590..=0x05FF => 4,
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF => 5,
        code => 0x1000 + code / 128,
    }
}

fn detect_scripts(dict: &AffixDict) -> Vec<u32> {
    let mut counts: HashMap<u32, u64> = HashMap::new();
    let mut total: u64 = 0;
    for stem in dict.stems() {
        for c in stem.chars().filter(|c| c.is_alphabetic()) {
            *counts.entry(script_of(c)).or_insert(0) += 1;
            total += 1;
        }
    }
    if total == 0 {
        return Vec::new();
    }
    let mut scripts: Vec<u32> = counts.into_iter().filter(|(_, count)| count.saturating_mul(100) >= total).map(|(s, _)| s).collect();
    scripts.sort_unstable();
    scripts
}

fn other_script(w: &str, scripts: &[u32]) -> bool {
    if scripts.is_empty() {
        return false;
    }
    !w.chars().filter(|c| c.is_alphabetic()).any(|c| scripts.contains(&script_of(c)))
}

fn in_capitals(w: &str) -> bool {
    w.chars().any(char::is_uppercase) && w.chars().all(|c| !c.is_lowercase())
}

fn enclosing_token(text: &str, a: usize, b: usize) -> (usize, usize, bool) {
    let ts = text.get(..a).and_then(|t| t.char_indices().rev().find(|(_, c)| c.is_whitespace())).map_or(0, |(i, c)| i + c.len_utf8());
    let te = text.get(b..).and_then(|t| t.find(char::is_whitespace)).map_or(text.len(), |i| b + i);
    let addr = text.get(ts..te).is_some_and(|tok| tok.contains("://") || tok.contains('@') || tok.starts_with("www."));
    (ts, te, addr)
}
