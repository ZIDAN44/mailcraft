//! What a laid-out message adds to the PDF beside its text: filled boxes (backgrounds, lines,
//! rounded buttons), pictures, and the places a click follows a link.

use std::fmt::Write as _;

use super::picture::{Picture, Pixels};
use super::{compress, rgb};

/// What a page holds beside its text, in the page's own coordinates: points from its
/// bottom-left corner, a box by its bottom-left.
#[derive(Default)]
pub(super) struct Extra {
    pub(super) fills: Vec<Fill>,
    pub(super) images: Vec<Image>,
    pub(super) links: Vec<Link>,
}

pub(super) struct Fill {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
    pub(super) color: [u8; 3],
    pub(super) radius: f32,
}

pub(super) struct Image {
    pub(super) index: usize,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
}

pub(super) struct Link {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
    pub(super) url: String,
}

/// Links written on one page at most.
const MAX_LINKS: usize = 2000;

impl Extra {
    /// The page's filled boxes, in the order they were laid (a table's background under its
    /// cells').
    pub(super) fn fills_into(&self, c: &mut String) {
        for f in &self.fills {
            if !(f.w > 0.0 && f.h > 0.0) {
                continue;
            }
            let _ = write!(c, "{} rg ", rgb(f.color));
            let r = f.radius.min(f.w / 2.0).min(f.h / 2.0);
            if r >= 0.5 {
                c.push_str(&rounded(f.x, f.y, f.w, f.h, r));
            } else {
                let _ = writeln!(c, "{:.2} {:.2} {:.2} {:.2} re f", f.x, f.y, f.w, f.h);
            }
        }
        if !self.fills.is_empty() {
            c.push_str("0 g\n");
        }
    }

    /// The page's pictures, each drawn into its box.
    pub(super) fn images_into(&self, c: &mut String) {
        for i in &self.images {
            if i.w > 0.0 && i.h > 0.0 {
                let _ = writeln!(c, "q {:.2} 0 0 {:.2} {:.2} {:.2} cm /Im{} Do Q", i.w, i.h, i.x, i.y, i.index);
            }
        }
    }

    /// The page's links, as its dictionary's `/Annots` entry (nothing when it has none).
    pub(super) fn annots(&self) -> String {
        let mut out = String::new();
        for l in self.links.iter().take(MAX_LINKS) {
            let Some(uri) = uri(&l.url) else { continue };
            let _ = write!(
                out,
                "<< /Type /Annot /Subtype /Link /Rect [{:.2} {:.2} {:.2} {:.2}] /Border [0 0 0] /A << /S /URI /URI {uri} >> >> ",
                l.x,
                l.y,
                l.x + l.w,
                l.y + l.h
            );
        }
        if out.is_empty() { out } else { format!(" /Annots [ {out}]") }
    }
}

/// A filled box with round corners: four sides and four quarter circles (as Bézier curves).
fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> String {
    let k = r * 0.5523;
    let (x1, y1) = (x + w, y + h);
    format!(
        "{:.2} {y:.2} m {:.2} {y:.2} l {:.2} {y:.2} {x1:.2} {:.2} {x1:.2} {:.2} c {x1:.2} {:.2} l {x1:.2} {:.2} {:.2} {y1:.2} {:.2} {y1:.2} c \
         {:.2} {y1:.2} l {:.2} {y1:.2} {x:.2} {:.2} {x:.2} {:.2} c {x:.2} {:.2} l {x:.2} {:.2} {:.2} {y:.2} {:.2} {y:.2} c h f\n",
        x + r,
        x1 - r,
        x1 - r + k,
        y + r - k,
        y + r,
        y1 - r,
        y1 - r + k,
        x1 - r + k,
        x1 - r,
        x + r,
        x + r - k,
        y1 - r + k,
        y1 - r,
        y + r,
        y + r - k,
        x + r - k,
        x + r,
    )
}

/// A link's address as a PDF string: web and mail addresses only, in ASCII (anything else as
/// `%XX`), no longer than a browser takes.
fn uri(url: &str) -> Option<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")) || url.len() > 2000 {
        return None;
    }
    let mut out = String::from("(");
    for b in url.bytes() {
        match b {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(char::from(b));
            }
            0x21..=0x7E => out.push(char::from(b)),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out.push(')');
    Some(out)
}

/// Write a picture's objects into `objects`; the number of the one a page draws.
pub(super) fn picture(p: &Picture, objects: &mut Vec<Vec<u8>>) -> usize {
    let stream = |head: String, data: &[u8]| {
        let mut s = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /BitsPerComponent 8 {head} /Length {} >>\nstream\n",
            p.width,
            p.height,
            data.len()
        )
        .into_bytes();
        s.extend_from_slice(data);
        s.extend_from_slice(b"\nendstream");
        s
    };
    match &p.data {
        Pixels::Jpeg { file, components } => {
            let space = if *components == 1 { "/DeviceGray" } else { "/DeviceRGB" };
            objects.push(stream(format!("/ColorSpace {space} /Filter /DCTDecode"), file));
        }
        Pixels::Rgb { rgb, alpha } => {
            // What shows through it: a mask of its own, named by the picture.
            let mask = alpha.as_ref().map(|a| {
                objects.push(stream("/ColorSpace /DeviceGray /Filter /FlateDecode".into(), &compress(a)));
                objects.len()
            });
            let mask = mask.map(|m| format!(" /SMask {m} 0 R")).unwrap_or_default();
            objects.push(stream(format!("/ColorSpace /DeviceRGB /Filter /FlateDecode{mask}"), &compress(rgb)));
        }
    }
    objects.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_web_and_mail_addresses_in_ascii() {
        assert_eq!(uri("https://shop.example/a(b)?q=1").as_deref(), Some("(https://shop.example/a\\(b\\)?q=1)"));
        assert_eq!(uri(" mailto:zoe@studio.example ").as_deref(), Some("(mailto:zoe@studio.example)"));
        assert_eq!(uri("https://x.example/caf\u{e9} au lait").as_deref(), Some("(https://x.example/caf%C3%A9%20au%20lait)"));
        for no in
            ["", "javascript:alert(1)", "file:///etc/passwd", "ftp://x.example/", "//x.example", &format!("https://x.example/{}", "a".repeat(3000))]
        {
            assert!(uri(no).is_none(), "{no}");
        }
    }

    #[test]
    fn a_pages_boxes_pictures_and_links_are_written() {
        let e = Extra {
            fills: vec![
                Fill { x: 10.0, y: 20.0, w: 100.0, h: 30.0, color: [255, 0, 0], radius: 0.0 },
                Fill { x: 10.0, y: 60.0, w: 100.0, h: 30.0, color: [0, 0, 255], radius: 400.0 },
                Fill { x: 0.0, y: 0.0, w: 0.0, h: f32::NAN, color: [0, 0, 0], radius: 0.0 },
            ],
            images: vec![Image { index: 2, x: 5.0, y: 6.0, w: 40.0, h: 20.0 }],
            links: vec![
                Link { x: 1.0, y: 2.0, w: 3.0, h: 4.0, url: "https://x.example/".into() },
                Link { x: 0.0, y: 0.0, w: 1.0, h: 1.0, url: "javascript:x".into() },
            ],
        };
        let mut c = String::new();
        e.fills_into(&mut c);
        assert!(c.contains("1.000 0.000 0.000 rg 10.00 20.00 100.00 30.00 re f"), "{c}");
        // Corners no rounder than half the box: a pill.
        assert!(c.contains("25.00 60.00 m 95.00 60.00 l") && c.matches(" c ").count() == 4, "{c}");
        assert_eq!(c.matches(" rg ").count(), 2, "a box with no size isn't drawn: {c}");
        let mut c = String::new();
        e.images_into(&mut c);
        assert_eq!(c, "q 40.00 0 0 20.00 5.00 6.00 cm /Im2 Do Q\n");
        assert_eq!(
            e.annots(),
            " /Annots [ << /Type /Annot /Subtype /Link /Rect [1.00 2.00 4.00 6.00] /Border [0 0 0] /A << /S /URI /URI (https://x.example/) >> >> ]"
        );
        assert_eq!(Extra::default().annots(), "");
    }
}
