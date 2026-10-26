//! Right-to-left text (Arabic, Hebrew, Persian…) for egui.
//!
//! The Craft apps' egui (storytold/egui, `Cargo.toml`'s `[patch.crates-io]`) draws it itself:
//! each line in the order it is seen (the Unicode Bidirectional Algorithm, UAX #9), Arabic
//! joined by the shaper, brackets mirrored, right-to-left paragraphs against the right edge of a
//! wrapped layout, and a text box's caret and selection where they belong. MailCraft hands it
//! text as written, and decides only what egui can't know: where a single line it paints by hand
//! (a subject, a name) belongs, and whether the interface font has the scripts.

use std::sync::atomic::{AtomicU8, Ordering};

use unicode_bidi::BidiInfo;

const ARABIC: u8 = 1;
const HEBREW: u8 = 2;
/// What the main font draws: Arabic letters, Hebrew letters.
static COVERAGE: AtomicU8 = AtomicU8::new(ARABIC | HEBREW);

/// Note which right-to-left scripts the main interface font has (from its character map).
pub fn set_coverage(font: Option<&[u8]>) {
    let has = |c: char| font.is_some_and(|f| covers(f, c));
    let mut bits = 0;
    if has('\u{0627}') && has('\u{0644}') {
        bits |= ARABIC;
    }
    if has('\u{05D0}') {
        bits |= HEBREW;
    }
    COVERAGE.store(bits, Ordering::Relaxed);
}

/// Whether the main font has both Arabic and Hebrew.
pub fn covers_rtl() -> bool {
    COVERAGE.load(Ordering::Relaxed) & (ARABIC | HEBREW) == ARABIC | HEBREW
}

fn covers(font: &[u8], c: char) -> bool {
    use skrifa::MetadataProvider as _;
    skrifa::FontRef::new(font).is_ok_and(|f| f.charmap().map(c).is_some())
}

/// A right-to-left letter (or mark, or digit of a right-to-left script).
pub fn is_rtl(c: char) -> bool {
    matches!(u32::from(c), 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

pub fn has_rtl(s: &str) -> bool {
    s.chars().any(is_rtl)
}

/// One line of mail (a subject, a name) to paint, and whether it reads right to left (then it
/// belongs against the right edge, where it starts). The text is as written; egui orders it.
pub fn shown(text: &str) -> (std::borrow::Cow<'_, str>, bool) {
    (std::borrow::Cow::Borrowed(text), has_rtl(text) && paragraph_rtl(text))
}

/// A label for text that may be right to left: wrapped at the width there is, right-to-left
/// paragraphs against the right edge.
pub fn label(ui: &mut egui::Ui, text: &str, font: egui::FontId, color: egui::Color32) -> egui::Response {
    if !has_rtl(text) {
        return ui.label(egui::RichText::new(text).font(font).color(color));
    }
    add_label(ui, egui::text::LayoutJob::simple(text.to_string(), font, color, ui.available_width()))
}

/// A label for a laid-out job, wrapped at the width there is. One whose first paragraph reads
/// right to left goes against the right edge, where it starts; egui then puts each of its
/// right-to-left rows there too.
pub fn add_label(ui: &mut egui::Ui, mut job: egui::text::LayoutJob) -> egui::Response {
    job.wrap.max_width = ui.available_width();
    let rtl = has_rtl(&job.text) && paragraph_rtl(&job.text);
    let label = egui::Label::new(job).selectable(false);
    if rtl { ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| ui.add(label)).inner } else { ui.add(label) }
}

/// Whether a paragraph reads right to left (its first strong letter is).
pub fn paragraph_rtl(text: &str) -> bool {
    BidiInfo::new(text, None).paragraphs.first().is_some_and(|p| p.level.is_rtl())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What egui draws, left to right on the screen: the galley's glyphs by position.
    fn drawn(text: &str, wrap: f32) -> (Vec<Vec<char>>, Vec<f32>) {
        let ctx = egui::Context::default();
        let mut rows = (Vec::new(), Vec::new());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut j = egui::text::LayoutJob::simple(text.to_string(), egui::FontId::proportional(14.0), egui::Color32::WHITE, wrap);
            j.wrap.max_width = wrap;
            let g = ui.fonts_mut(|f| f.layout_job(j));
            for r in &g.rows {
                let mut glyphs: Vec<(f32, char)> = r.row.glyphs.iter().map(|gl| (gl.pos.x, gl.chr)).collect();
                glyphs.sort_by(|a, b| a.0.total_cmp(&b.0));
                rows.0.push(glyphs.iter().map(|(_, c)| *c).collect());
                rows.1.push(r.pos.x + r.row.glyphs.iter().map(|gl| gl.pos.x).fold(f32::INFINITY, f32::min));
            }
        });
        out.textures_delta.clear();
        rows
    }

    /// Text goes to egui as written, and egui draws it right: Hebrew from the right, Latin and
    /// digits inside it left to right, every letter and space kept.
    #[test]
    fn egui_draws_right_to_left_text_as_written() {
        let (rows, _) = drawn("שלום abc 123 עולם", f32::INFINITY);
        let line: String = rows.concat().into_iter().collect();
        // A left-to-right stretch inside keeps its own order ("abc 123").
        assert_eq!(line, "םלוע abc 123 םולש", "seen left to right on the screen");
        assert_eq!(shown("שלום abc").0, "שלום abc", "handed over unchanged");
        assert!(shown("שלום abc").1 && !shown("abc שלום").1 && !shown("plain").1);
    }

    /// A wrapped right-to-left paragraph keeps every word, and its short last line sits against
    /// the right edge.
    #[test]
    fn wrapped_paragraphs_keep_every_word() {
        let text = "أرفقنا الترجمة العربية للفيلم القصير (النسخة 2). راجعنا التوقيت مع ملف shot_list_v2 وعدد الأسطر 128.";
        let (rows, lefts) = drawn(text, 300.0);
        assert!(rows.len() > 1, "it wraps");
        let mut seen: Vec<char> = rows.concat().into_iter().filter(|c| !c.is_whitespace()).collect();
        let mut want: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
        seen.sort_unstable();
        want.sort_unstable();
        assert_eq!(seen, want);
        assert!(lefts.last().is_some_and(|l| *l > 20.0), "the last line starts away from the left: {lefts:?}");
    }
}
