//! Fonts from the optional craft-fonts build input (`CRAFT_FONTS_DIR`, see `build.rs`): empty
//! unless the app was built with it. They're the last fallbacks, after the system's fonts, so
//! Arabic, Hebrew, Japanese, Chinese, Korean and Thai can be drawn where the system has no font
//! for them (the browser has none at all).

/// A font compiled in from craft-fonts, with the licence that goes with it.
pub struct CraftFont {
    pub family: &'static str,
    pub style: &'static str,
    /// ISO 15924 scripts the font is for, e.g. `"Arab"`.
    pub scripts: &'static [&'static str],
    /// The licence's identifier (e.g. `OFL-1.1`) and its text, shown on the About page.
    pub licence: &'static str,
    pub licence_text: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

/// The fallback fonts to draw with: the regular style of each family (bold is drawn from it).
pub fn fallbacks() -> impl Iterator<Item = &'static CraftFont> {
    CRAFT_FONTS.iter().filter(|f| f.style == "Regular")
}

/// Whether a compiled-in font is for a script (`"Jpan"`, `"Hans"`, `"Arab"`…).
pub fn has_script(script: &str) -> bool {
    fallbacks().any(|f| f.scripts.contains(&script))
}
