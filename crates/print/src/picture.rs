//! A message's pictures in the PDF: a JPEG goes in as it is (PDF reads JPEG itself), anything
//! else as its pixels, with what shows through it as a mask. Pictures are hostile input: sizes
//! and allocations are capped before anything is decoded.

/// Most bytes of a picture taken in.
const MAX_BYTES: usize = 30 << 20;
/// A picture is kept this many pixels a side at most (more is sampled down: the page is 595
/// points wide).
const MAX_SIDE: u32 = 1600;

/// A picture ready to embed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub(crate) data: Pixels,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pixels {
    /// A JPEG file, grey (1 component) or colour (3).
    Jpeg { file: Vec<u8>, components: u8 },
    /// Three bytes a pixel, and one a pixel for how solid each is when any isn't wholly.
    Rgb { rgb: Vec<u8>, alpha: Option<Vec<u8>> },
}

impl Picture {
    /// The picture in `bytes` (JPEG, PNG, GIF, WebP or BMP), or none when it isn't one that can
    /// be embedded.
    pub fn decode(bytes: &[u8]) -> Option<Picture> {
        if bytes.len() > MAX_BYTES {
            return None;
        }
        if let Some((width, height, components)) = jpeg_size(bytes) {
            // Grey and colour go in as they are; four components (CMYK) are decoded.
            if matches!(components, 1 | 3) && width <= 12_000 && height <= 12_000 {
                return Some(Picture { width, height, data: Pixels::Jpeg { file: bytes.to_vec(), components } });
            }
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(12_000);
        limits.max_image_height = Some(12_000);
        limits.max_alloc = Some(256 << 20);
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
        reader.limits(limits);
        let decoded = reader.decode().ok()?;
        let img = if decoded.width() > MAX_SIDE || decoded.height() > MAX_SIDE { decoded.thumbnail(MAX_SIDE, MAX_SIDE) } else { decoded }.to_rgba8();
        let (width, height) = (img.width(), img.height());
        if width == 0 || height == 0 {
            return None;
        }
        let raw = img.as_raw();
        let mut rgb = Vec::with_capacity(raw.len() / 4 * 3);
        let mut alpha = Vec::with_capacity(raw.len() / 4);
        for [r, g, b, a] in raw.as_chunks::<4>().0 {
            rgb.extend_from_slice(&[*r, *g, *b]);
            alpha.push(*a);
        }
        let solid = alpha.iter().all(|a| *a == 255);
        Some(Picture { width, height, data: Pixels::Rgb { rgb, alpha: (!solid).then_some(alpha) } })
    }
}

/// A JPEG's width, height and number of colour components, from its frame header.
fn jpeg_size(b: &[u8]) -> Option<(u32, u32, u8)> {
    if b.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut i = 2usize;
    // Segments: a marker, then a length that counts itself.
    for _ in 0..4096 {
        while *b.get(i)? == 0xFF && *b.get(i + 1)? == 0xFF {
            i += 1;
        }
        if *b.get(i)? != 0xFF {
            return None;
        }
        let marker = *b.get(i + 1)?;
        let len = usize::from(u16::from_be_bytes([*b.get(i + 2)?, *b.get(i + 3)?]));
        // Start of frame (not the tables that share its range).
        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let height = u32::from(u16::from_be_bytes([*b.get(i + 5)?, *b.get(i + 6)?]));
            let width = u32::from(u16::from_be_bytes([*b.get(i + 7)?, *b.get(i + 8)?]));
            let components = *b.get(i + 9)?;
            return (width > 0 && height > 0).then_some((width, height, components));
        }
        // The picture's data begins: no frame header came before it.
        if marker == 0xDA || len < 2 {
            return None;
        }
        i = i.checked_add(2)?.checked_add(len)?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(width, height, image::Rgba(pixel));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn pictures_are_read_and_capped() {
        let solid = Picture::decode(&png(4, 3, [10, 20, 30, 255])).unwrap();
        assert_eq!((solid.width, solid.height), (4, 3));
        assert_eq!(solid.data, Pixels::Rgb { rgb: [10, 20, 30].repeat(12), alpha: None });
        // What shows through is kept as a mask.
        let clear = Picture::decode(&png(2, 2, [1, 2, 3, 128])).unwrap();
        assert_eq!(clear.data, Pixels::Rgb { rgb: [1, 2, 3].repeat(4), alpha: Some(vec![128; 4]) });
        // A big one is sampled down, its shape kept.
        let big = Picture::decode(&png(4000, 1000, [0, 0, 0, 255])).unwrap();
        assert_eq!((big.width, big.height), (1600, 400));
        // A JPEG goes in as the file it is.
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(16, 8, image::Rgb([200, 100, 50])).write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
        let jpeg = jpeg.into_inner();
        let kept = Picture::decode(&jpeg).unwrap();
        assert_eq!((kept.width, kept.height), (16, 8));
        assert_eq!(kept.data, Pixels::Jpeg { file: jpeg.clone(), components: 3 });
    }

    #[test]
    fn what_isnt_a_picture_is_none() {
        for junk in [
            &b""[..],
            b"\xFF",
            b"\xFF\xD8",
            b"\xFF\xD8\xFF",
            b"\xFF\xD8\xFF\xC0",
            b"\xFF\xD8\xFF\xE0\x00\x00",
            b"GIF89a",
            b"\x89PNG\r\n\x1a\n",
            b"hello",
        ] {
            assert!(Picture::decode(junk).is_none(), "{junk:?}");
        }
        // A JPEG cut off anywhere, and one whose header says it's enormous.
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(16, 8, image::Rgb([200, 100, 50])).write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
        let jpeg = jpeg.into_inner();
        for cut in 0..jpeg.len().min(200) {
            let _ = Picture::decode(&jpeg[..cut]);
        }
        assert!(jpeg_size(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0xFF, 0xFF, 0xFF, 0xFF, 3]).is_some_and(|(w, h, _)| w == 65_535 && h == 65_535));
        assert!(
            Picture::decode(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0xFF, 0xFF, 0xFF, 0xFF, 3]).is_none(),
            "too big to take as it is, and no picture"
        );
    }
}
