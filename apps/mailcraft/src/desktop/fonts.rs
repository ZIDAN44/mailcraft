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
        if !readable(&data.bytes, data.index) {
            return None;
        }
        Some(outlines_only(&data.bytes, data.index).unwrap_or((data.bytes, data.index)))
    }
}

/// The largest system font file read: a CJK collection is about 20 MB, Windows' largest about 40.
const SYSTEM_FONT_MAX: u64 = 96 << 20;

/// The first of `candidates` (a file, and the face in it) that can be read whole and has that
/// face as egui reads it. A file that's empty, cut short, too large or not a font, or a collection
/// without the face, is passed over for the next one: taken as it was, egui stopped MailCraft as
/// it started, every time (as CADCraft found with its CJK fonts).
pub(crate) fn first_readable(candidates: impl IntoIterator<Item = (std::path::PathBuf, u32)>) -> Option<(Vec<u8>, u32)> {
    candidates.into_iter().find_map(|(path, index)| {
        if std::fs::metadata(&path).ok()?.len() > SYSTEM_FONT_MAX {
            log::warn!("fonts: {} is too large to be a font", path.display());
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        if !readable(&bytes, index) {
            log::warn!("fonts: {} isn't a font MailCraft can read (face {index}); another is used", path.display());
            return None;
        }
        Some((bytes, index))
    })
}

/// Whether egui reads face `index` of `bytes`: as it does, with its own skrifa (the same
/// version, which the test below holds). A file it can't read stops egui, and MailCraft with it;
/// one damaged on the disk is left out instead.
fn readable(bytes: &[u8], index: u32) -> bool {
    let Ok(font) = skrifa::FontRef::from_index(bytes, index) else { return false };
    // And every table it lists is in the file: skrifa reads them only as they're used, so a
    // file cut short was taken, and its letters drew as nothing.
    font.table_directory.table_records().iter().all(|r| font.table_data(r.tag()).is_some())
}

/// The tables of black-and-white or grey bitmaps a face carries for some sizes (`EBDT`, `EBLC`,
/// `EBSC`, and Apple's `bdat` and `bloc`). egui takes a glyph that has one for a colour glyph,
/// and paints it white whatever the text's colour: most of Calibri's letters didn't show on the
/// page (issue #166). Its outlines draw them all, in the text's colour.
const BITMAP_TABLES: [&[u8; 4]; 5] = [b"EBDT", b"EBLC", b"EBSC", b"bdat", b"bloc"];

/// Face `index` of `bytes` as a font of its own without its [`BITMAP_TABLES`], when it has some
/// and outlines to draw instead: `None` to hand it over as it is (or when it can't be rebuilt).
fn outlines_only(bytes: &Arc<[u8]>, index: u32) -> Option<(Arc<[u8]>, u32)> {
    use skrifa::raw::{TableProvider, types::Tag};
    let font = skrifa::FontRef::from_index(bytes, index).ok()?;
    let bitmap = |tag: Tag| BITMAP_TABLES.iter().any(|t| tag == Tag::new(t));
    let records = font.table_directory.table_records();
    let outlines = font.glyf().is_ok() || font.cff().is_ok() || font.cff2().is_ok();
    if !outlines || !records.iter().any(|r| bitmap(r.tag())) {
        return None;
    }
    let tables: Vec<(Tag, u32, &[u8])> =
        records.iter().filter(|r| !bitmap(r.tag())).filter_map(|r| Some((r.tag(), r.checksum(), font.table_data(r.tag())?.as_bytes()))).collect();
    let out = sfnt(font.table_directory.sfnt_version(), &tables)?;
    readable(&out, 0).then(|| (Arc::from(out), 0))
}

/// A font file of `tables` (tag, checksum, data), sorted by tag as the format asks, each on a
/// four-byte boundary: `None` past the format's counts and sizes.
fn sfnt(version: u32, tables: &[(skrifa::raw::types::Tag, u32, &[u8])]) -> Option<Vec<u8>> {
    let mut tables = tables.to_vec();
    tables.sort_by_key(|t| t.0);
    let count = u16::try_from(tables.len()).ok()?;
    // The binary search fields: the largest power of two not over the count.
    let power = if count == 0 { 0 } else { 1u16 << (15 - count.leading_zeros()) };
    let search = power.checked_mul(16)?;
    let mut out = Vec::new();
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&search.to_be_bytes());
    out.extend_from_slice(&u16::try_from(power.max(1).trailing_zeros()).ok()?.to_be_bytes());
    out.extend_from_slice(&count.checked_mul(16)?.checked_sub(search)?.to_be_bytes());
    let mut offset = 12 + 16 * usize::from(count);
    for (tag, checksum, data) in &tables {
        out.extend_from_slice(&tag.to_be_bytes());
        out.extend_from_slice(&checksum.to_be_bytes());
        out.extend_from_slice(&u32::try_from(offset).ok()?.to_be_bytes());
        out.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
        offset = offset.checked_add(data.len().next_multiple_of(4))?;
    }
    for (_, _, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The computer's fonts for the interface (its own, Chinese, Japanese and Korean, Arabic and
    /// Hebrew): a file that's empty, cut short or not a font, one too large, or a collection
    /// without the face asked for, is passed over for the next candidate. Taken as it was, egui
    /// stopped MailCraft as it started, every time (as CADCraft found).
    #[test]
    fn a_damaged_system_font_is_passed_over() {
        let dir = std::env::temp_dir().join(format!("mailcraft-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (good, _) = with_a_bitmap();
        let file = |name: &str, bytes: &[u8]| {
            let p = dir.join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        };
        let empty = file("empty.ttf", b"");
        let junk = file("junk.ttf", b"<html>not found</html>");
        let cut = file("cut.ttf", &good[..good.len() / 3]);
        let font = file("good.ttf", &good);
        let missing = dir.join("missing.ttf");
        let big = file("big.ttf", b"");
        std::fs::File::options().write(true).open(&big).unwrap().set_len(SYSTEM_FONT_MAX + 1).unwrap();
        let candidates = [(missing, 0), (empty, 0), (junk, 0), (cut, 0), (big, 0), (font.clone(), 1), (font, 0)];
        let (bytes, index) = first_readable(candidates).expect("the good one");
        assert_eq!((bytes.as_slice(), index), (good.as_slice(), 0), "face 1 isn't in it either");
        assert!(first_readable([(dir.join("empty.ttf"), 0)]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    /// egui's own sans with a black-and-white bitmap of `a`, as Calibri has one for most letters:
    /// one 8×8 strike (`EBLC` version 2, index format 1; `EBDT` image format 1, small metrics).
    fn with_a_bitmap() -> (Vec<u8>, skrifa::GlyphId) {
        use skrifa::MetadataProvider;
        use skrifa::raw::types::Tag;
        let defs = egui::FontDefinitions::default();
        let sans = defs.font_data.values().next().expect("egui's own fonts");
        let bytes: &[u8] = sans.font.as_ref().as_ref();
        let font = skrifa::FontRef::from_index(bytes, sans.index).expect("egui's font reads");
        let a = font.charmap().map('a').expect("it has an a");
        let g = u16::try_from(a.to_u32()).expect("a small glyph id").to_be_bytes();
        let metrics = [8u8, 0, 8, 0, 0, 0, 0, 0, 8, 0, 0, 0];
        let mut eblc = vec![0, 2, 0, 0, 0, 0, 0, 1];
        eblc.extend_from_slice(&[0, 0, 0, 56, 0, 0, 0, 24, 0, 0, 0, 1, 0, 0, 0, 0]);
        eblc.extend_from_slice(&metrics);
        eblc.extend_from_slice(&metrics);
        eblc.extend_from_slice(&[g[0], g[1], g[0], g[1], 16, 16, 1, 1]);
        eblc.extend_from_slice(&[g[0], g[1], g[0], g[1], 0, 0, 0, 8]);
        eblc.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 13]);
        let mut ebdt = vec![0, 2, 0, 0, 8, 8, 0, 8, 8];
        ebdt.extend_from_slice(&[0xFF; 8]);
        let mut tables: Vec<(Tag, u32, &[u8])> =
            font.table_directory.table_records().iter().filter_map(|r| Some((r.tag(), r.checksum(), font.table_data(r.tag())?.as_bytes()))).collect();
        tables.push((Tag::new(b"EBLC"), 0, &eblc));
        tables.push((Tag::new(b"EBDT"), 0, &ebdt));
        (sfnt(font.table_directory.sfnt_version(), &tables).expect("a font"), a)
    }

    /// Whether egui draws `a` of face `index` of `bytes` as a colour glyph (painted white).
    fn a_in_colour(bytes: &[u8], index: u32) -> bool {
        let mut defs = egui::FontDefinitions::empty();
        let mut data = egui::FontData::from_owned(bytes.to_vec());
        data.index = index;
        defs.font_data.insert("face".into(), Arc::new(data));
        defs.families.insert(egui::FontFamily::Proportional, vec!["face".into()]);
        let ctx = egui::Context::default();
        ctx.set_fonts(defs);
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let galley = ctx.fonts_mut(|f| f.layout_no_wrap("a".into(), egui::FontId::proportional(16.0), egui::Color32::BLACK));
        galley.rows.first().and_then(|r| r.row.glyphs.first()).is_some_and(|g| g.is_color)
    }

    /// Issue #166: a face with black-and-white bitmaps (Calibri has them) is handed to egui
    /// without them: egui drew each letter with one in white, whatever the text's colour, and
    /// most of Calibri's didn't show on the page. Drawn from its outlines, it's in the text's
    /// colour; a face with none is handed over as it is.
    #[test]
    fn a_face_s_bitmaps_are_left_out_so_its_letters_take_the_text_s_colour() {
        let (bytes, a) = with_a_bitmap();
        assert!(readable(&bytes, 0));
        assert!(a_in_colour(&bytes, 0), "egui takes the bitmap for a colour glyph");
        let bytes: Arc<[u8]> = Arc::from(bytes);
        let (out, index) = outlines_only(&bytes, 0).expect("rebuilt without its bitmaps");
        assert!(!a_in_colour(&out, index), "drawn from its outline");
        use skrifa::MetadataProvider;
        let font = skrifa::FontRef::from_index(&out, index).expect("it reads");
        assert!(font.bitmap_strikes().glyph_for_size(skrifa::instance::Size::new(16.0), a).is_none());
        assert_eq!(font.charmap().map('a'), Some(a), "the rest as it was");
        let defs = egui::FontDefinitions::default();
        let sans = defs.font_data.values().next().expect("egui's own fonts");
        let plain: Arc<[u8]> = Arc::from(sans.font.as_ref().as_ref());
        assert!(outlines_only(&plain, sans.index).is_none(), "nothing to leave out: as it is");
        assert!(outlines_only(&Arc::from(&b"not a font"[..]), 0).is_none());
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
