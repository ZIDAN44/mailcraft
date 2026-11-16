//! The unread count on the notification area's icon (issue #117): a red badge in the icon's
//! corner with the Inboxes' unread mail in white, "99+" past 99, drawn here in code over
//! MailCraft's own icon. None, the icon as it is.

/// The badge's red, and the white of its figures and rim.
const RED: [u8; 4] = [0xD3, 0x2F, 0x2F, 0xFF];
const WHITE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];

/// The figures, 5 by 7: a row's bits, the leftmost the highest of five.
const GLYPHS: [(char, [u8; 7]); 11] = [
    ('0', [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110]),
    ('1', [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    ('2', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111]),
    ('3', [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110]),
    ('4', [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010]),
    ('5', [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110]),
    ('6', [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110]),
    ('7', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000]),
    ('8', [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110]),
    ('9', [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100]),
    ('+', [0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000]),
];

/// What the badge says: the count, "99+" past 99; nothing for none.
pub(super) fn label(unread: u32) -> Option<String> {
    match unread {
        0 => None,
        1..=99 => Some(unread.to_string()),
        _ => Some("99+".to_string()),
    }
}

/// `icon` (RGBA, `size` square) with the badge for `unread` over its lower right corner; as it is
/// for none, or a picture that isn't `size` square.
pub(super) fn badged(icon: &[u8], size: u32, unread: u32) -> Vec<u8> {
    let mut out = icon.to_vec();
    let side = usize::try_from(size).unwrap_or(0);
    let Some(text) = label(unread) else { return out };
    if side < 16 || out.len() != side * side * 4 {
        return out;
    }
    // Its figures as big as fit: a pixel of the font is `scale` of the icon's.
    let chars: Vec<[u8; 7]> = text.chars().filter_map(|c| GLYPHS.iter().find(|(g, _)| *g == c).map(|(_, rows)| *rows)).collect();
    let n = chars.len();
    let scale = if n <= 2 { side / 21 } else { side / 32 }.max(1);
    let gap = scale;
    let text_w = n * 5 * scale + n.saturating_sub(1) * gap;
    let text_h = 7 * scale;
    // A disc for one figure, a pill for more; in the corner, its rim a pixel in from the edge.
    let h = (text_h + 4 * scale).min(side);
    let w = (text_w + 4 * scale).max(h).min(side);
    let (right, bottom) = (side as f32 - 0.5, side as f32 - 0.5);
    let (x0, y0) = (right - w as f32, bottom - h as f32);
    let radius = h as f32 / 2.0;
    let rim = (scale as f32 * 0.8).max(1.0);
    // Inside the pill, as a share of the pixel (4 by 4 samples): smooth edges at any size.
    let inside = |px: usize, py: usize, grow: f32| -> f32 {
        let mut hits = 0u32;
        for sy in 0..4u32 {
            for sx in 0..4u32 {
                let x = px as f32 + (sx as f32 + 0.5) / 4.0;
                let y = py as f32 + (sy as f32 + 0.5) / 4.0;
                let cx = x.clamp(x0 + radius, x0 + w as f32 - radius);
                let cy = y0 + radius;
                if (x - cx).hypot(y - cy) <= radius + grow {
                    hits += 1;
                }
            }
        }
        hits as f32 / 16.0
    };
    let first_x = (x0 - rim).floor().max(0.0) as usize;
    let first_y = (y0 - rim).floor().max(0.0) as usize;
    for py in first_y..side {
        for px in first_x..side {
            let (outer, fill) = (inside(px, py, rim), inside(px, py, 0.0));
            if outer <= 0.0 {
                continue;
            }
            // The white rim first, the red over it.
            blend(&mut out, side, px, py, WHITE, outer);
            blend(&mut out, side, px, py, RED, fill);
        }
    }
    // The figures, centred in it.
    let left = (x0 + (w - text_w) as f32 / 2.0).round().max(0.0) as usize;
    let top = (y0 + (h - text_h) as f32 / 2.0).round().max(0.0) as usize;
    for (i, rows) in chars.iter().enumerate() {
        let gx = left + i * (5 * scale + gap);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if bits & (0b10000 >> col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        blend(&mut out, side, gx + col * scale + dx, top + row * scale + dy, WHITE, 1.0);
                    }
                }
            }
        }
    }
    out
}

/// `color` over the pixel at (`x`, `y`) of a `side` square picture, `share` of it.
fn blend(rgba: &mut [u8], side: usize, x: usize, y: usize, color: [u8; 4], share: f32) {
    if x >= side || y >= side || share <= 0.0 {
        return;
    }
    let at = (y * side + x) * 4;
    let Some(px) = rgba.get_mut(at..at + 4) else { return };
    let a = share.clamp(0.0, 1.0);
    for (c, new) in px.iter_mut().take(3).zip(color) {
        *c = (f32::from(*c) * (1.0 - a) + f32::from(new) * a).round() as u8;
    }
    if let Some(alpha) = px.get_mut(3) {
        *alpha = (f32::from(*alpha) * (1.0 - a) + 255.0 * a).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIDE: u32 = 64;

    fn blank() -> Vec<u8> {
        vec![0u8; (SIDE * SIDE * 4) as usize]
    }

    fn pixel(p: &[u8], x: u32, y: u32) -> [u8; 4] {
        let at = ((y * SIDE + x) * 4) as usize;
        [p[at], p[at + 1], p[at + 2], p[at + 3]]
    }

    #[test]
    fn the_badge_says_the_count_or_99_plus() {
        assert_eq!(label(0), None);
        assert_eq!(label(7).as_deref(), Some("7"));
        assert_eq!(label(42).as_deref(), Some("42"));
        assert_eq!(label(99).as_deref(), Some("99"));
        assert_eq!(label(100).as_deref(), Some("99+"));
        assert_eq!(label(u32::MAX).as_deref(), Some("99+"));
    }

    /// None unread: the icon as it was. Some: red in the lower right corner, white figures in
    /// it, the upper left as it was.
    #[test]
    fn the_badge_sits_in_the_corner() {
        let icon = blank();
        assert_eq!(badged(&icon, SIDE, 0), icon);
        for n in [1, 8, 42, 120] {
            let p = badged(&icon, SIDE, n);
            assert_eq!(pixel(&p, 2, 2), [0, 0, 0, 0], "{n}: the rest of the icon as it was");
            let corner: Vec<[u8; 4]> = (36..SIDE).flat_map(|y| (24..SIDE).map(move |x| (x, y))).map(|(x, y)| pixel(&p, x, y)).collect();
            assert!(corner.contains(&RED), "{n}: red in the corner");
            assert!(corner.contains(&WHITE), "{n}: white figures and rim");
            // The upper half of the icon is left as it was (the badge and its rim are below).
            assert!((0..SIDE / 2 - 6).all(|y| (0..SIDE).all(|x| pixel(&p, x, y) == [0, 0, 0, 0])), "{n}: above it untouched");
        }
    }

    /// One figure takes a disc, two a wider pill; "99+" has smaller figures to fit; none goes
    /// past the icon.
    #[test]
    fn wider_counts_take_a_wider_badge() {
        let reds = |n: u32| {
            let p = badged(&blank(), SIDE, n);
            (0..SIDE).filter(|&x| (0..SIDE).any(|y| pixel(&p, x, y) == RED)).count()
        };
        assert!(reds(7) < reds(42), "{} {}", reds(7), reds(42));
        for n in [7, 42, 120] {
            assert!(reds(n) < SIDE as usize, "{n}: within the icon");
        }
    }

    /// A picture that isn't the size given, or too small for figures, comes back as it is.
    #[test]
    fn odd_pictures_come_back_as_they_are() {
        let short = vec![1u8; 10];
        assert_eq!(badged(&short, SIDE, 5), short);
        let tiny = vec![9u8; 8 * 8 * 4];
        assert_eq!(badged(&tiny, 8, 5), tiny);
    }
}
