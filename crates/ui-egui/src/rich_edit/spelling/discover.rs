//! Multi-path Hunspell dictionary discovery (`<app_data_dir>/dictionaries/`, system paths,
//! LibreOffice extension directories, installed language packs, and in-memory dictionaries).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use super::{catalog, memory};

/// A discovered spelling dictionary available to the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DictEntry {
    pub locale: String,
    pub name: String,
    pub aff: Option<PathBuf>,
    pub dic: Option<PathBuf>,
}

static USER_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
static CACHE: OnceLock<Mutex<Option<(u64, Vec<DictEntry>)>>> = OnceLock::new();

fn user_dir_lock() -> &'static Mutex<Option<PathBuf>> {
    USER_DIR.get_or_init(|| Mutex::new(None))
}

fn cache_lock() -> &'static Mutex<Option<(u64, Vec<DictEntry>)>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Drop cached discovery results so the next lookup rescans disk and memory.
pub(crate) fn invalidate_cache() {
    *crate::lock(cache_lock()) = None;
}

/// Register MailCraft's user dictionaries directory (`<app_data_dir>/dictionaries/`), forwarding
/// it to `craft-proof` and invalidating all caches.
pub(crate) fn set_dictionaries_dir(dir: Option<PathBuf>) {
    *crate::lock(user_dir_lock()) = dir.clone();
    #[cfg(not(target_arch = "wasm32"))]
    craft_proof::set_dictionaries_folder(dir);
    memory::clear_disk_cache();
    invalidate_cache();
}

/// Reload Hunspell dictionaries from disk after files were added or changed.
pub(crate) fn reload_dictionaries() {
    let dir = crate::lock(user_dir_lock()).clone();
    set_dictionaries_dir(dir);
}

/// Scan `dirs` in priority order for valid `.aff` + `.dic` Hunspell dictionary pairs.
/// Skips hyphenation (`hyph_*`) and thesaurus (`th_*`) dictionaries, requires both files to exist,
/// and keeps the first match when multiple directories provide the same normalized locale.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn scan_dirs(dirs: &[PathBuf]) -> Vec<DictEntry> {
    let mut found: BTreeMap<String, DictEntry> = BTreeMap::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for dic in paths {
            let ext_ok = dic.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("dic"));
            if !ext_ok || !dic.is_file() {
                continue;
            }
            let Some(stem) = dic.file_stem().and_then(|s| s.to_str()) else { continue };
            let lower_stem = stem.to_ascii_lowercase();
            if lower_stem.starts_with("hyph_") || lower_stem.starts_with("th_") {
                continue;
            }
            let aff = dic.with_extension("aff");
            if !aff.is_file() {
                continue;
            }
            let locale = catalog::normalize_locale(stem);
            if locale.is_empty() {
                continue;
            }
            found.entry(locale.clone()).or_insert_with(|| DictEntry { name: catalog::display_name(&locale), locale, aff: Some(aff), dic: Some(dic) });
        }
    }
    found.into_values().collect()
}

/// Directories searched for Hunspell dictionaries on desktop platforms, in priority order.
#[cfg(not(target_arch = "wasm32"))]
fn platform_search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(user) = crate::lock(user_dir_lock()).clone() {
        dirs.push(user);
    }
    dirs.extend(craft_proof::hunspell::search_dirs(None));
    let mut ext_roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from) {
                ext_roots.push(base.join("LibreOffice").join("share").join("extensions"));
            }
        }
    } else if cfg!(target_os = "macos") {
        ext_roots.push(PathBuf::from("/Applications/LibreOffice.app/Contents/Resources/extensions"));
    } else {
        for base in [
            "/usr/lib/libreoffice/share/extensions",
            "/usr/lib64/libreoffice/share/extensions",
            "/usr/share/libreoffice/share/extensions",
            "/opt/libreoffice/share/extensions",
            "/usr/local/lib/libreoffice/share/extensions",
        ] {
            ext_roots.push(PathBuf::from(base));
        }
    }
    for root in ext_roots {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        let mut children: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("dict-")))
            .collect();
        children.sort();
        dirs.extend(children);
    }
    // Deduplicate while preserving order.
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

/// All dictionaries currently available (cached per dictionary generation).
pub(crate) fn discover_all() -> Vec<DictEntry> {
    let generation = memory::generation() ^ craft_proof::dictionary_generation();
    if let Some((cached_gen, entries)) = crate::lock(cache_lock()).as_ref()
        && *cached_gen == generation
    {
        return entries.clone();
    }
    let mut map: BTreeMap<String, DictEntry> = BTreeMap::new();
    #[cfg(not(target_arch = "wasm32"))]
    {
        for entry in scan_dirs(&platform_search_dirs()) {
            map.insert(entry.locale.clone(), entry);
        }
        for pack in craft_proof::packs::installed() {
            let locale = catalog::normalize_locale(pack.locale.as_str());
            map.entry(locale.clone()).or_insert_with(|| DictEntry {
                name: catalog::display_name(&locale),
                locale,
                aff: Some(pack.path.clone()),
                dic: Some(pack.path),
            });
        }
    }
    for locale in memory::installed_locales() {
        map.entry(locale.clone()).or_insert_with(|| DictEntry { name: catalog::display_name(&locale), locale, aff: None, dic: None });
    }
    // Built-in English is always available via craft-proof even when no external file exists.
    map.entry("en_US".into()).or_insert_with(|| DictEntry { locale: "en_US".into(), name: catalog::display_name("en_US"), aff: None, dic: None });
    let mut out: Vec<DictEntry> = map.into_values().collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.locale.cmp(&b.locale)));
    *crate::lock(cache_lock()) = Some((generation, out.clone()));
    out
}

/// Find the discovered entry matching `locale`.
pub(crate) fn find_entry(locale: &str) -> Option<DictEntry> {
    let norm = catalog::normalize_locale(locale);
    discover_all().into_iter().find(|e| e.locale.eq_ignore_ascii_case(&norm))
}
