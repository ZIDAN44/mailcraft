//! Thumbnails for attachments that aren't pictures: the first page of a PDF (rendered here, in
//! pure Rust) and a frame of a video (from the platform, when it has a way: see
//! [`Services::video_frame`](crate::Services::video_frame)).
//!
//! Attachments are hostile input: sizes are capped and the renderer runs behind a panic guard.

use std::sync::Arc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::font::{FontData, FontQuery};
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

/// Largest PDF rendered for a thumbnail.
const MAX_PDF: usize = 40 << 20;

/// A video frame the platform is working on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoFrame {
    /// Still being made: ask again.
    Pending,
    /// An encoded picture (PNG, JPEG…).
    Ready(Vec<u8>),
    /// No frame (no decoder installed, or the file can't be read).
    Unavailable,
}

/// The first page of a PDF, at most `max` pixels on its longer side. Text in fonts the PDF
/// doesn't carry (its "standard" fonts) is drawn with `fonts` (regular, bold), the interface's
/// system fonts, when given.
pub fn pdf_first_page(data: &[u8], max: u32, fonts: (Option<Vec<u8>>, Option<Vec<u8>>)) -> Option<egui::ColorImage> {
    pdf_page(data, 0, Fit::Longer(max), fonts).map(|(img, _)| img)
}

/// How big a page is drawn: its longer side at most this many pixels, or this wide.
#[derive(Clone, Copy, Debug)]
pub enum Fit {
    Longer(u32),
    Width(u32),
}

/// Page `n` (from 0) of a PDF, and how many pages it has, for the reading pane's preview.
pub fn pdf_page(data: &[u8], n: usize, fit: Fit, fonts: (Option<Vec<u8>>, Option<Vec<u8>>)) -> Option<(egui::ColorImage, usize)> {
    if data.len() > MAX_PDF || !data.windows(5).take(1024).any(|w| w == b"%PDF-") {
        return None;
    }
    let data = data.to_vec();
    let regular: Option<FontData> = fonts.0.map(|f| Arc::new(f) as FontData);
    let bold: Option<FontData> = fonts.1.map(|f| Arc::new(f) as FontData).or_else(|| regular.clone());
    let interpreter = InterpreterSettings {
        font_resolver: Arc::new(move |q: &FontQuery| {
            let heavy = matches!(q, FontQuery::Standard(s) if format!("{s:?}").contains("Bold"));
            let f = if heavy { bold.clone() } else { regular.clone() };
            f.map(|f| (f, 0))
        }),
        ..Default::default()
    };
    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let pdf = Pdf::new(data).ok()?;
        let pages = pdf.pages();
        let count = pages.len();
        let page = pages.get(n)?;
        let (w, h) = page.render_dimensions();
        if !(w.is_finite() && h.is_finite()) || w < 1.0 || h < 1.0 {
            return None;
        }
        let scale = match fit {
            Fit::Longer(max) => max as f32 / w.max(h),
            Fit::Width(px) => px as f32 / w,
        }
        // At most 4x, and no page more than about 16 megapixels.
        .min(4.0)
        .min((16_000_000.0 / (w * h)).sqrt());
        let settings = PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE };
        let pixmap = hayro::render(page, &RenderCache::new(), &interpreter, &RenderSettings::default(), &settings);
        let size = [usize::from(pixmap.width()), usize::from(pixmap.height())];
        if size[0] == 0 || size[1] == 0 {
            return None;
        }
        let bytes = pixmap.data_as_u8_slice();
        (bytes.len() == size[0] * size[1] * 4).then(|| (egui::ColorImage::from_rgba_premultiplied(size, bytes), count))
    }));
    rendered.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF (100 x 50 points) with a blue box in it.
    fn tiny_pdf() -> Vec<u8> {
        let content = "0 0 1 rg 10 10 40 30 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 50] /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for off in offsets {
            out.extend(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        out
    }

    #[test]
    fn pdf_pages_render_and_junk_does_not() {
        let img = pdf_first_page(&tiny_pdf(), 200, (None, None)).expect("a page");
        assert_eq!(img.size, [200, 100], "the longer side is the asked size");
        assert!(img.pixels.iter().any(|p| p.b() > 200 && p.r() < 60), "the blue box is drawn");
        assert!(img.pixels.iter().any(|p| p.r() > 250 && p.g() > 250), "on a white page");
        for junk in [&b""[..], b"%PDF-1.7\n garbage", b"not a pdf at all", &[0xffu8; 4096]] {
            assert!(pdf_first_page(junk, 200, (None, None)).is_none());
        }
    }
}
