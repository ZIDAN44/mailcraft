//! Supported downloadable Hunspell dictionaries, locale normalization, and display names.

/// Raw GitHub base URL for the official LibreOffice dictionaries repository.
pub(crate) const BASE_URL: &str = "https://raw.githubusercontent.com/LibreOffice/dictionaries/master";

/// A downloadable Hunspell dictionary entry in the LibreOffice dictionaries repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DictSource {
    /// Canonical MailCraft locale code (saved locally as `<locale>.aff` / `<locale>.dic`).
    pub locale: &'static str,
    /// Human-readable display name.
    pub name: &'static str,
    /// Relative `.aff` path inside LibreOffice/dictionaries `master`.
    pub aff_path: &'static str,
    /// Relative `.dic` path inside LibreOffice/dictionaries `master`.
    pub dic_path: &'static str,
}

impl DictSource {
    pub(crate) fn aff_url(&self) -> String {
        format!("{BASE_URL}/{}", self.aff_path)
    }

    pub(crate) fn dic_url(&self) -> String {
        format!("{BASE_URL}/{}", self.dic_path)
    }
}

/// Curated manifest of Hunspell dictionaries available for one-click download.
pub(crate) const CATALOG: &[DictSource] = &[
    DictSource { locale: "ar", name: "العربية (Arabic)", aff_path: "ar/ar.aff", dic_path: "ar/ar.dic" },
    DictSource { locale: "bg_BG", name: "Български (Bulgarian)", aff_path: "bg_BG/bg_BG.aff", dic_path: "bg_BG/bg_BG.dic" },
    DictSource { locale: "cs_CZ", name: "Čeština (Czech)", aff_path: "cs_CZ/cs_CZ.aff", dic_path: "cs_CZ/cs_CZ.dic" },
    DictSource { locale: "da_DK", name: "Dansk (Danish)", aff_path: "da_DK/da_DK.aff", dic_path: "da_DK/da_DK.dic" },
    DictSource { locale: "de_DE", name: "Deutsch (German)", aff_path: "de/de_DE_frami.aff", dic_path: "de/de_DE_frami.dic" },
    DictSource { locale: "el_GR", name: "Ελληνικά (Greek)", aff_path: "el_GR/el_GR.aff", dic_path: "el_GR/el_GR.dic" },
    DictSource { locale: "en_GB", name: "English (UK)", aff_path: "en/en_GB.aff", dic_path: "en/en_GB.dic" },
    DictSource { locale: "en_US", name: "English (US)", aff_path: "en/en_US.aff", dic_path: "en/en_US.dic" },
    DictSource { locale: "es_ES", name: "Español (Spanish)", aff_path: "es/es_ES.aff", dic_path: "es/es_ES.dic" },
    DictSource { locale: "fr_FR", name: "Français (French)", aff_path: "fr_FR/fr.aff", dic_path: "fr_FR/fr.dic" },
    DictSource { locale: "he_IL", name: "עברית (Hebrew)", aff_path: "he_IL/he_IL.aff", dic_path: "he_IL/he_IL.dic" },
    DictSource { locale: "hr_HR", name: "Hrvatski (Croatian)", aff_path: "hr_HR/hr_HR.aff", dic_path: "hr_HR/hr_HR.dic" },
    DictSource { locale: "hu_HU", name: "Magyar (Hungarian)", aff_path: "hu_HU/hu_HU.aff", dic_path: "hu_HU/hu_HU.dic" },
    DictSource { locale: "id_ID", name: "Bahasa Indonesia (Indonesian)", aff_path: "id/id_ID.aff", dic_path: "id/id_ID.dic" },
    DictSource { locale: "it_IT", name: "Italiano (Italian)", aff_path: "it_IT/it_IT.aff", dic_path: "it_IT/it_IT.dic" },
    DictSource { locale: "nl_NL", name: "Nederlands (Dutch)", aff_path: "nl_NL/nl_NL.aff", dic_path: "nl_NL/nl_NL.dic" },
    DictSource { locale: "pl_PL", name: "Polski (Polish)", aff_path: "pl_PL/pl_PL.aff", dic_path: "pl_PL/pl_PL.dic" },
    DictSource { locale: "pt_BR", name: "Português (Brasil)", aff_path: "pt_BR/pt_BR.aff", dic_path: "pt_BR/pt_BR.dic" },
    DictSource { locale: "ro_RO", name: "Română (Romanian)", aff_path: "ro/ro_RO.aff", dic_path: "ro/ro_RO.dic" },
    DictSource { locale: "ru_RU", name: "Русский (Russian)", aff_path: "ru_RU/ru_RU.aff", dic_path: "ru_RU/ru_RU.dic" },
    DictSource { locale: "sk_SK", name: "Slovenčina (Slovak)", aff_path: "sk_SK/sk_SK.aff", dic_path: "sk_SK/sk_SK.dic" },
    DictSource { locale: "sv_SE", name: "Svenska (Swedish)", aff_path: "sv_SE/sv_SE.aff", dic_path: "sv_SE/sv_SE.dic" },
    DictSource { locale: "tr_TR", name: "Türkçe (Turkish)", aff_path: "tr_TR/tr_TR.aff", dic_path: "tr_TR/tr_TR.dic" },
    DictSource { locale: "uk_UA", name: "Українська (Ukrainian)", aff_path: "uk_UA/uk_UA.aff", dic_path: "uk_UA/uk_UA.dic" },
];

/// Additional display names for common locales outside [`CATALOG`].
const EXTRA_NAMES: &[(&str, &str)] = &[
    ("ca_ES", "Català (Catalan)"),
    ("en_AU", "English (Australia)"),
    ("en_CA", "English (Canada)"),
    ("es_419", "Español (Latinoamérica)"),
    ("es_MX", "Español (México)"),
    ("eu", "Euskara (Basque)"),
    ("fi_FI", "Suomi (Finnish)"),
    ("fr_CA", "Français (Canada)"),
    ("gl_ES", "Galego (Galician)"),
    ("ja", "日本語 (Japanese)"),
    ("ko", "한국어 (Korean)"),
    ("nb_NO", "Norsk bokmål (Norwegian)"),
    ("nn_NO", "Norsk nynorsk (Norwegian Nynorsk)"),
    ("pt_PT", "Português (Portugal)"),
    ("sl_SI", "Slovenščina (Slovenian)"),
    ("sr_RS", "Српски (Serbian)"),
    ("vi_VN", "Tiếng Việt (Vietnamese)"),
    ("zh_CN", "简体中文 (Simplified Chinese)"),
    ("zh_TW", "繁體中文 (Traditional Chinese)"),
];

/// Normalize a file stem or user tag (`de_DE_frami`, `fr`, `en-us`) into a canonical locale code.
pub(crate) fn normalize_locale(raw: &str) -> String {
    let trimmed = raw.trim().split('.').next().unwrap_or("").trim();
    if let Some(mapped) = match trimmed.to_ascii_lowercase().as_str() {
        "de_de_frami" | "de-de-frami" | "de_frami" => Some("de_DE"),
        "de_at_frami" | "de-at-frami" => Some("de_AT"),
        "de_ch_frami" | "de-ch-frami" => Some("de_CH"),
        "fr" | "fr-classique" | "fr-moderne" | "fr-reforme1990" | "fr-toutesvariantes" | "fr_classique" => Some("fr_FR"),
        "he" => Some("he_IL"),
        "uk" => Some("uk_UA"),
        "cs" => Some("cs_CZ"),
        "da" => Some("da_DK"),
        "el" => Some("el_GR"),
        "hr" => Some("hr_HR"),
        "bg" => Some("bg_BG"),
        "ro" => Some("ro_RO"),
        "sv" => Some("sv_SE"),
        _ => None,
    } {
        return mapped.to_string();
    }
    if let Some(norm) = mailcraft_engine::spelling_language_code(trimmed) {
        return norm;
    }
    trimmed.replace('-', "_")
}

/// Map a MailCraft UI language code (`crate::i18n::current().code()`) to a Hunspell locale code.
pub(crate) fn ui_code_to_locale(code: &str) -> &'static str {
    match code.trim().to_ascii_lowercase().as_str() {
        "ar" => "ar",
        "bg" => "bg_BG",
        "cs" => "cs_CZ",
        "da" => "da_DK",
        "de" => "de_DE",
        "el" => "el_GR",
        "en" => "en_US",
        "en-gb" => "en_GB",
        "es" => "es_ES",
        "fr" => "fr_FR",
        "he" => "he_IL",
        "hr" => "hr_HR",
        "hu" => "hu_HU",
        "id" => "id_ID",
        "it" => "it_IT",
        "ja" => "ja",
        "ko" => "ko",
        "nl" => "nl_NL",
        "pl" => "pl_PL",
        "pt-br" => "pt_BR",
        "ro" => "ro_RO",
        "ru" => "ru_RU",
        "sk" => "sk_SK",
        "sv" => "sv_SE",
        "tr" => "tr_TR",
        "uk" => "uk_UA",
        "zh-hans" => "zh_CN",
        "zh-hant" => "zh_TW",
        _ => "en_US",
    }
}

/// Find a downloadable source entry matching `locale` (exact or by primary language tag).
pub(crate) fn find_source(locale: &str) -> Option<&'static DictSource> {
    let norm = normalize_locale(locale);
    if let Some(exact) = CATALOG.iter().find(|s| s.locale.eq_ignore_ascii_case(&norm)) {
        return Some(exact);
    }
    let primary = norm.split('_').next().unwrap_or(&norm);
    CATALOG.iter().find(|s| s.locale.split('_').next().is_some_and(|p| p.eq_ignore_ascii_case(primary)))
}

/// Human-readable display name for `locale`.
pub(crate) fn display_name(locale: &str) -> String {
    let norm = normalize_locale(locale);
    if let Some(s) = CATALOG.iter().find(|s| s.locale.eq_ignore_ascii_case(&norm)) {
        return s.name.to_string();
    }
    if let Some((_, name)) = EXTRA_NAMES.iter().find(|(c, _)| c.eq_ignore_ascii_case(&norm)) {
        return (*name).to_string();
    }
    if let Some(s) = find_source(&norm) {
        return format!("{} ({norm})", s.name.split(" (").next().unwrap_or(s.name));
    }
    norm
}

/// Short badge shown on the compose toolbar language button (`EN`, `BG`, or `EN-GB` if `region`).
pub(crate) fn short_badge(locale: &str, show_region: bool) -> String {
    let norm = normalize_locale(locale);
    let mut parts = norm.split('_');
    let lang = parts.next().unwrap_or("EN").to_ascii_uppercase();
    match (show_region, parts.next()) {
        (true, Some(reg)) if !reg.is_empty() => format!("{lang}-{}", reg.to_ascii_uppercase()),
        _ => lang,
    }
}
