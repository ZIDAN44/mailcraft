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
pub fn add_label(ui: &mut egui::Ui, job: egui::text::LayoutJob) -> egui::Response {
    add_label_with(ui, job, false)
}

/// [`add_label`] whose text can be selected with the pointer and copied: a message's text. The
/// Craft apps' egui counts right-to-left letters right (stock egui 0.37 didn't, and a selection
/// there copied nothing), so Arabic and Hebrew select as they read.
pub fn add_selectable_label(ui: &mut egui::Ui, job: egui::text::LayoutJob) -> egui::Response {
    add_label_with(ui, job, true)
}

/// [`add_selectable_label`] for a plain string in one font and colour.
pub fn selectable_label(ui: &mut egui::Ui, text: &str, font: egui::FontId, color: egui::Color32) -> egui::Response {
    add_selectable_label(ui, egui::text::LayoutJob::simple(text.to_string(), font, color, ui.available_width()))
}

fn add_label_with(ui: &mut egui::Ui, mut job: egui::text::LayoutJob, selectable: bool) -> egui::Response {
    job.wrap.max_width = ui.available_width();
    let rtl = has_rtl(&job.text) && paragraph_rtl(&job.text);
    let label = egui::Label::new(job).selectable(selectable);
    if rtl { ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| ui.add(label)).inner } else { ui.add(label) }
}

/// Drawing by hand in a row, mirrored when the interface reads right to left (Arabic, Hebrew):
/// the icon that sits at the start of a row on the left goes on the right, the count at its end
/// on the left. Positions are written as for left to right; `x`, `rect` and `start` place them.
#[derive(Clone, Copy)]
pub struct Mirror {
    on: bool,
    sum: f32,
}

impl Mirror {
    /// For drawing in `r`, in the interface's direction.
    pub fn new(r: egui::Rect) -> Self {
        Self::with(r, crate::i18n::current().rtl())
    }

    pub fn with(r: egui::Rect, on: bool) -> Self {
        Self { on, sum: r.min.x + r.max.x }
    }

    pub fn on(self) -> bool {
        self.on
    }

    pub fn x(self, x: f32) -> f32 {
        if self.on { self.sum - x } else { x }
    }

    pub fn pos(self, p: egui::Pos2) -> egui::Pos2 {
        egui::pos2(self.x(p.x), p.y)
    }

    pub fn rect(self, r: egui::Rect) -> egui::Rect {
        if self.on { egui::Rect::from_x_y_ranges(self.x(r.max.x)..=self.x(r.min.x), r.y_range()) } else { r }
    }

    /// The left edge of something `width` wide that starts at `x` (its right edge, mirrored).
    pub fn start(self, x: f32, width: f32) -> f32 {
        if self.on { self.x(x) - width } else { x }
    }

    pub fn align(self, a: egui::Align2) -> egui::Align2 {
        let flip = |h: egui::Align| match h {
            egui::Align::Min => egui::Align::Max,
            egui::Align::Max => egui::Align::Min,
            egui::Align::Center => egui::Align::Center,
        };
        if self.on { egui::Align2([flip(a.x()), a.y()]) } else { a }
    }

    /// The arrow of something closed, pointing the way the text goes.
    pub fn closed(self) -> crate::icons::Icon {
        if self.on { crate::icons::Icon::ChevronLeft } else { crate::icons::Icon::ChevronRight }
    }
}

/// A row from where the interface's text starts: the left, or the right in a right-to-left
/// interface (Arabic, Hebrew).
pub fn row_from_start(align: egui::Align) -> egui::Layout {
    if crate::i18n::current().rtl() { egui::Layout::right_to_left(align) } else { egui::Layout::left_to_right(align) }
}

/// A row from where the interface's text ends: the right, or the left in a right-to-left one.
pub fn row_from_end(align: egui::Align) -> egui::Layout {
    if crate::i18n::current().rtl() { egui::Layout::left_to_right(align) } else { egui::Layout::right_to_left(align) }
}

/// A column against the side the interface's text starts on; egui runs each `horizontal` row in
/// it from that side too.
pub fn column() -> egui::Layout {
    egui::Layout::top_down(if crate::i18n::current().rtl() { egui::Align::Max } else { egui::Align::Min })
}

/// `ui.vertical` in the interface's direction (egui's own is always against the left, and its
/// rows run from there).
pub fn vertical<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    ui.scope_builder(egui::UiBuilder::new().layout(column()), add)
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
