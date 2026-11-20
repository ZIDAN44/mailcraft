//! The computer's fonts, to write in (issue #166): the Craft apps' font catalog finds them (the
//! font folders, and Windows' registry), looking in the background from the start, and a face
//! is handed to the window only once it's known to be one egui reads.

use std::sync::Arc;

use craft_font_catalog::Catalog;
use mailcraft_ui_egui::mail_fonts::FontSource;

/// The catalog, as the window's [`FontSource`].
pub(crate) struct CatalogFonts;

impl CatalogFonts {
    /// Start looking for the computer's fonts (on a thread of the catalog's own).
    pub(crate) fn start() -> Self {
        Catalog::global().scan_in_background();
        CatalogFonts
    }
}

impl FontSource for CatalogFonts {
    fn families(&self) -> Arc<[String]> {
        let catalog = Catalog::global();
        if catalog.is_ready() { catalog.families() } else { Arc::from([]) }
    }

    fn face(&self, family: &str, bold: bool) -> Option<(Arc<[u8]>, u32)> {
        let catalog = Catalog::global();
        let (face, _) = catalog.resolve(family, if bold { "Bold" } else { "Regular" })?;
        let data = catalog.load(&face).ok()?;
        readable(&data.bytes, data.index).then_some((data.bytes, data.index))
    }
}

/// Whether egui reads face `index` of `bytes`: as it does, with its own skrifa (the same
/// version, which the test below holds). A file it can't read stops egui, and MailCraft with it;
/// one damaged on the disk is left out instead.
fn readable(bytes: &[u8], index: u32) -> bool {
    skrifa::FontRef::from_index(bytes, index).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A damaged font file, or one that isn't a font, is never handed to egui; nor a face its
    /// file doesn't have.
    #[test]
    fn only_a_face_egui_reads_is_handed_over() {
        assert!(!readable(b"", 0));
        assert!(!readable(b"not a font at all", 0));
        // A collection (version 1.1) listing its one face only in the longer list ISO/IEC
        // 14496-22:2026 adds after the first: a newer skrifa reads it, egui's doesn't, and stops.
        let mut ttc = b"ttcf\0\x01\0\x01\0\0\0\0".to_vec();
        ttc.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 20]);
        ttc.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(!readable(&ttc, 0));
        let catalog = Catalog::global();
        catalog.rescan();
        // On a computer with fonts, its first family's regular face is read, and a face past the
        // file's last isn't.
        if let Some(family) = catalog.families().first().cloned()
            && let Some((face, _)) = catalog.resolve(&family, "Regular")
            && let Ok(data) = catalog.load(&face)
        {
            assert!(readable(&data.bytes, data.index), "{family} is read");
            assert!(!readable(&data.bytes, 10_000), "no face 10000 in {family}'s file");
        }
    }

    /// The skrifa the app checks a font with, as `Cargo.lock` has it: `skrifa` alone where the
    /// lock holds one version, `skrifa 0.44.0` where it holds several.
    fn skrifa_of(lock: &str, package: &str) -> Option<String> {
        let name = format!("name = \"{package}\"");
        lock.split("[[package]]").find(|p| p.lines().any(|l| l.trim() == name))?.lines().find_map(|l| {
            let dep = l.trim().trim_matches(|c| c == '"' || c == ',');
            (dep == "skrifa" || dep.starts_with("skrifa ")).then(|| dep.to_string())
        })
    }

    /// egui reads fonts with the skrifa epaint takes; a newer one reads files it doesn't (a
    /// collection's longer list of faces, in read-fonts 0.46), which egui would then stop on.
    #[test]
    fn a_font_is_checked_with_egui_s_own_skrifa() {
        let lock = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"));
        let (egui, app) = (skrifa_of(lock, "epaint"), skrifa_of(lock, "mailcraft"));
        assert!(egui.is_some(), "epaint reads fonts with skrifa");
        assert_eq!(app, egui, "apps/mailcraft's skrifa is epaint's");
    }
}
