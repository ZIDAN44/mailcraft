//! Photos leave without saying where they were taken. A phone writes the place into every
//! picture it takes (the GPS block of its EXIF data, often an XMP copy of it too), and a photo
//! sent as it is tells every recipient where its sender lives or was standing.
//! [`without_location`] takes that out of JPEG and PNG files and leaves everything else as the
//! camera wrote it: the picture itself, its orientation, its colours, its date.
//!
//! Only the bytes that say where are touched: the GPS block is zeroed where it lies (so nothing
//! else in the file moves), and an XMP packet that carries a position is left out. The input is
//! whatever the user attached, so every offset is checked before it's followed.

/// The tag in a TIFF directory that points to the GPS block (EXIF 2.3, 4.6.3).
const GPS_POINTER: u16 = 0x8825;
/// More entries than this in one directory isn't a photo's metadata.
const ENTRIES_MAX: usize = 1000;
const JPEG_START: &[u8] = &[0xFF, 0xD8];
const PNG_START: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const EXIF_ID: &[u8] = b"Exif\0\0";
const XMP_IDS: [&[u8]; 2] = [b"http://ns.adobe.com/xap/1.0/\0", b"http://ns.adobe.com/xmp/extension/\0"];

/// `data` with where it was taken removed, or `None` when there was nothing to remove (or it
/// isn't a JPEG or PNG file, or not one that can be read).
pub fn without_location(data: &[u8]) -> Option<Vec<u8>> {
    if data.starts_with(JPEG_START) {
        jpeg(data)
    } else if data.starts_with(PNG_START) {
        png(data)
    } else {
        None
    }
}

/// Whether a picture's place is in an XMP packet (`exif:GPSLatitude`, `GPSLongitude`…).
fn xmp_has_place(packet: &[u8]) -> bool {
    packet.windows(3).any(|w| w == b"GPS")
}

fn jpeg(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(JPEG_START);
    let mut at = JPEG_START.len();
    let mut changed = false;
    loop {
        if *data.get(at)? != 0xFF {
            return None;
        }
        let marker = *data.get(at + 1)?;
        match marker {
            // Fill bytes before a marker.
            0xFF => {
                out.push(0xFF);
                at += 1;
                continue;
            }
            // Markers without a length: restart points and the like.
            0x01 | 0xD0..=0xD8 => {
                out.extend_from_slice(data.get(at..at + 2)?);
                at += 2;
                continue;
            }
            // The end, or the start of the picture itself: what follows is pixels, not metadata.
            0xD9 | 0xDA => {
                out.extend_from_slice(data.get(at..)?);
                break;
            }
            _ => {}
        }
        let len = usize::from(u16::from_be_bytes([*data.get(at + 2)?, *data.get(at + 3)?]));
        if len < 2 {
            return None;
        }
        let end = at.checked_add(2 + len)?;
        let segment = data.get(at..end)?;
        let body = segment.get(4..)?;
        if marker == 0xE1 && body.starts_with(EXIF_ID) {
            let mut segment = segment.to_vec();
            if let Some(tiff) = segment.get_mut(4 + EXIF_ID.len()..) {
                changed |= wipe_gps(tiff);
            }
            out.extend_from_slice(&segment);
        } else if marker == 0xE1 && XMP_IDS.iter().any(|id| body.starts_with(id)) && xmp_has_place(body) {
            changed = true;
        } else {
            out.extend_from_slice(segment);
        }
        at = end;
    }
    changed.then_some(out)
}

fn png(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(PNG_START);
    let mut at = PNG_START.len();
    let mut changed = false;
    while at < data.len() {
        let len = usize::try_from(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?)).ok()?;
        let kind = data.get(at + 4..at + 8)?;
        let end = at.checked_add(12)?.checked_add(len)?;
        let chunk = data.get(at..end)?;
        let body = chunk.get(8..chunk.len() - 4)?;
        let leave_out = match kind {
            // PNG's own EXIF block (PNG 1.5). Its CRC covers it, so it goes whole.
            b"eXIf" => true,
            // Text chunks holding an XMP packet, or EXIF as ImageMagick writes it.
            b"iTXt" | b"tEXt" | b"zTXt" => {
                let keyword = body.split(|b| *b == 0).next().unwrap_or_default();
                keyword == b"XML:com.adobe.xmp" && (kind == b"zTXt" || xmp_has_place(body))
                    || keyword.eq_ignore_ascii_case(b"Raw profile type exif")
                    || keyword.eq_ignore_ascii_case(b"Raw profile type APP1")
            }
            _ => false,
        };
        if leave_out {
            changed = true;
        } else {
            out.extend_from_slice(chunk);
        }
        at = end;
        if kind == b"IEND" {
            break;
        }
    }
    changed.then_some(out)
}

/// Zeroes the GPS block of a TIFF structure (an EXIF block's body) where it lies, with the values
/// it points to, and empties it. `true` when there was one with anything in it.
fn wipe_gps(tiff: &mut [u8]) -> bool {
    let little = match tiff.get(0..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return false,
    };
    let Some(first) = read32(tiff, 4, little) else { return false };
    let Some(n) = read16(tiff, first, little) else { return false };
    for i in 0..usize::from(n).min(ENTRIES_MAX) {
        let Some(entry) = first.checked_add(2 + 12 * i) else { return false };
        if read16(tiff, entry, little) == Some(GPS_POINTER)
            && let Some(gps) = read32(tiff, entry.saturating_add(8), little)
        {
            return wipe_directory(tiff, gps, little);
        }
    }
    false
}

fn wipe_directory(tiff: &mut [u8], at: usize, little: bool) -> bool {
    let Some(n) = read16(tiff, at, little) else { return false };
    let n = usize::from(n).min(ENTRIES_MAX);
    for i in 0..n {
        let Some(entry) = at.checked_add(2 + 12 * i) else { break };
        let (Some(kind), Some(count)) = (read16(tiff, entry.saturating_add(2), little), read32(tiff, entry.saturating_add(4), little)) else { break };
        // A value longer than four bytes lies elsewhere, at the offset the entry gives.
        let size = value_size(kind).saturating_mul(count);
        if size > 4
            && let Some(off) = read32(tiff, entry.saturating_add(8), little)
            && let Some(value) = off.checked_add(size).and_then(|end| tiff.get_mut(off..end))
        {
            value.fill(0);
        }
        if let Some(e) = entry.checked_add(12).and_then(|end| tiff.get_mut(entry..end)) {
            e.fill(0);
        }
    }
    // No entries left: the block reads as empty, its "next" pointer is now zero as well.
    if let Some(count) = tiff.get_mut(at..at.saturating_add(2)) {
        count.fill(0);
    }
    n > 0
}

/// Bytes per value of a TIFF field type (TIFF 6.0, section 2); unknown types count as 1.
fn value_size(kind: u16) -> usize {
    match kind {
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => 1,
    }
}

fn read16(b: &[u8], at: usize, little: bool) -> Option<u16> {
    let v: [u8; 2] = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
    Some(if little { u16::from_le_bytes(v) } else { u16::from_be_bytes(v) })
}

fn read32(b: &[u8], at: usize, little: bool) -> Option<usize> {
    let v: [u8; 4] = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
    usize::try_from(if little { u32::from_le_bytes(v) } else { u32::from_be_bytes(v) }).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TIFF block: IFD0 with an orientation tag and the GPS pointer, then a GPS block with a
    /// latitude (three rationals, stored out of line).
    fn tiff(little: bool) -> Vec<u8> {
        let w16 = |v: u16| if little { v.to_le_bytes() } else { v.to_be_bytes() };
        let w32 = |v: u32| if little { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut t = Vec::new();
        t.extend_from_slice(if little { b"II" } else { b"MM" });
        t.extend_from_slice(&w16(42));
        t.extend_from_slice(&w32(8));
        // IFD0 at 8: two entries.
        t.extend_from_slice(&w16(2));
        t.extend_from_slice(&w16(0x0112)); // orientation
        t.extend_from_slice(&w16(3));
        t.extend_from_slice(&w32(1));
        t.extend_from_slice(&w16(6));
        t.extend_from_slice(&[0, 0]);
        t.extend_from_slice(&w16(GPS_POINTER));
        t.extend_from_slice(&w16(4));
        t.extend_from_slice(&w32(1));
        t.extend_from_slice(&w32(38)); // the GPS block
        t.extend_from_slice(&w32(0));
        assert_eq!(t.len(), 38);
        // GPS block at 38: one entry, latitude 51/1 30/1 4123/100 at 56.
        t.extend_from_slice(&w16(1));
        t.extend_from_slice(&w16(2)); // GPSLatitude
        t.extend_from_slice(&w16(5));
        t.extend_from_slice(&w32(3));
        t.extend_from_slice(&w32(56));
        t.extend_from_slice(&w32(0));
        assert_eq!(t.len(), 56);
        for v in [51u32, 1, 30, 1, 4123, 100] {
            t.extend_from_slice(&w32(v));
        }
        t
    }

    fn jpeg_with(segments: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut j = JPEG_START.to_vec();
        for (marker, body) in segments {
            j.extend_from_slice(&[0xFF, *marker]);
            j.extend_from_slice(&u16::try_from(body.len() + 2).unwrap_or(0).to_be_bytes());
            j.extend_from_slice(body);
        }
        // The picture: a scan header and its data, then the end.
        j.extend_from_slice(&[0xFF, 0xDA, 0, 2, 0x12, 0x34, 0xFF, 0x00, 0x56, 0xFF, 0xD9]);
        j
    }

    fn exif(little: bool) -> Vec<u8> {
        [EXIF_ID.to_vec(), tiff(little)].concat()
    }

    fn has_latitude(b: &[u8], little: bool) -> bool {
        let v = if little { 4123u32.to_le_bytes() } else { 4123u32.to_be_bytes() };
        b.windows(4).any(|w| w == v)
    }

    #[test]
    fn the_place_goes_and_the_rest_stays() {
        for little in [true, false] {
            let photo = jpeg_with(&[(0xE0, b"JFIF\0\x01\x01".to_vec()), (0xE1, exif(little))]);
            assert!(has_latitude(&photo, little));
            let clean = without_location(&photo).unwrap_or_default();
            assert_eq!(clean.len(), photo.len(), "nothing moves");
            assert!(!has_latitude(&clean, little), "little-endian {little}");
            // Orientation, the JFIF header and the picture are as they were.
            let orientation = if little { [0x12, 0x01, 3, 0] } else { [0x01, 0x12, 0, 3] };
            assert!(clean.windows(4).any(|w| w == orientation));
            assert!(clean.windows(4).any(|w| w == b"JFIF"));
            assert!(clean.ends_with(&[0xFF, 0xDA, 0, 2, 0x12, 0x34, 0xFF, 0x00, 0x56, 0xFF, 0xD9]));
            // Done twice, there's nothing left to take out.
            assert_eq!(without_location(&clean), None);
        }
    }

    #[test]
    fn an_xmp_position_goes_and_other_xmp_stays() {
        let place = [XMP_IDS[0], b"<x:xmpmeta><rdf:Description exif:GPSLatitude=\"51,30.6N\"/></x:xmpmeta>"].concat();
        let rating = [XMP_IDS[0], b"<x:xmpmeta><rdf:Description xmp:Rating=\"5\"/></x:xmpmeta>"].concat();
        let clean = without_location(&jpeg_with(&[(0xE1, place), (0xE1, rating)])).unwrap_or_default();
        assert!(!clean.windows(3).any(|w| w == b"GPS"));
        assert!(clean.windows(10).any(|w| w == b"xmp:Rating"));
    }

    #[test]
    fn photos_without_a_place_and_other_files_are_left_alone() {
        assert_eq!(without_location(&jpeg_with(&[(0xE0, b"JFIF\0".to_vec())])), None);
        assert_eq!(without_location(b"%PDF-1.7 GPS"), None);
        assert_eq!(without_location(b""), None);
    }

    #[test]
    fn png_location_chunks_go() {
        let chunk = |kind: &[u8], body: &[u8]| {
            let mut c = u32::try_from(body.len()).unwrap_or(0).to_be_bytes().to_vec();
            c.extend_from_slice(kind);
            c.extend_from_slice(body);
            c.extend_from_slice(&[1, 2, 3, 4]); // CRC: not checked here
            c
        };
        let png = [
            PNG_START.to_vec(),
            chunk(b"IHDR", &[0; 13]),
            chunk(b"eXIf", &tiff(false)),
            chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<exif:GPSLatitude>51</exif:GPSLatitude>"),
            chunk(b"tEXt", b"Title\0Harbour"),
            chunk(b"IDAT", &[9; 20]),
            chunk(b"IEND", &[]),
        ]
        .concat();
        let clean = without_location(&png).unwrap_or_default();
        assert!(!clean.windows(4).any(|w| w == b"eXIf"));
        assert!(!clean.windows(3).any(|w| w == b"GPS"));
        assert!(clean.windows(7).any(|w| w == b"Harbour"));
        assert!(clean.windows(4).any(|w| w == b"IDAT"));
    }

    #[test]
    fn broken_photos_never_panic() {
        let photo = jpeg_with(&[(0xE1, exif(true))]);
        for cut in 0..photo.len() {
            let _ = without_location(photo.get(..cut).unwrap_or_default());
        }
        // A GPS pointer, entry count and value offset that point anywhere.
        let mut t = exif(true);
        for at in [EXIF_ID.len() + 30, EXIF_ID.len() + 38, EXIF_ID.len() + 48] {
            if let Some(b) = t.get_mut(at..at + 4) {
                b.copy_from_slice(&[0xFF, 0xFF, 0xFF, 0x7F]);
            }
        }
        let _ = without_location(&jpeg_with(&[(0xE1, t)]));
        let mut png = PNG_START.to_vec();
        png.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, b'e', b'X', b'I', b'f']);
        assert_eq!(without_location(&png), None);
    }
}
