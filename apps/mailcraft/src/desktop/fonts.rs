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

/// Whether egui reads face `index` of `bytes`: as it does, with skrifa. A file it can't read
/// stops egui, and MailCraft with it; one damaged on the disk is left out instead.
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
}
