//! The CRC of MS-PST 5.3: CRC-32 (reflected polynomial 0xEDB88320), started at 0 and not inverted
//! at the end. Eight bytes are taken at a time ("slicing"), since every page and block is checked.

use std::sync::atomic::{AtomicBool, Ordering};

static CHECKSUMS_TAKEN_AS_RIGHT: AtomicBool = AtomicBool::new(false);

/// For the fuzz targets: every CRC is taken for right from now on, or damage would rarely reach
/// what lies behind the checks. Does nothing in any other build.
#[doc(hidden)]
#[allow(unexpected_cfgs)] // `cargo fuzz` sets it
pub fn take_checksums_as_right(on: bool) {
    if cfg!(fuzzing) {
        CHECKSUMS_TAKEN_AS_RIGHT.store(on, Ordering::Relaxed);
    }
}

/// Whether `data` has the CRC `stored` for it.
pub(crate) fn matches(data: &[u8], stored: u32) -> bool {
    CHECKSUMS_TAKEN_AS_RIGHT.load(Ordering::Relaxed) || crc(data) == stored
}

const fn tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            bit += 1;
        }
        t[0][i] = c;
        i += 1;
    }
    let mut i = 0;
    while i < 256 {
        let mut k = 1;
        while k < 8 {
            let prev = t[k - 1][i];
            t[k][i] = t[0][(prev & 0xFF) as usize] ^ (prev >> 8);
            k += 1;
        }
        i += 1;
    }
    t
}

static TABLES: [[u32; 256]; 8] = tables();

fn at(table: usize, byte: u32) -> u32 {
    TABLES.get(table).and_then(|t| t.get((byte & 0xFF) as usize)).copied().unwrap_or(0)
}

/// `data`'s CRC, carrying on from `crc`.
pub(crate) fn update(mut crc: u32, data: &[u8]) -> u32 {
    let (chunks, rest) = data.as_chunks::<8>();
    for c in chunks {
        let [a, b, c0, d, e, f, g, h] = *c;
        let low = crc ^ u32::from_le_bytes([a, b, c0, d]);
        let high = u32::from_le_bytes([e, f, g, h]);
        crc = at(7, low)
            ^ at(6, low >> 8)
            ^ at(5, low >> 16)
            ^ at(4, low >> 24)
            ^ at(3, high)
            ^ at(2, high >> 8)
            ^ at(1, high >> 16)
            ^ at(0, high >> 24);
    }
    for b in rest {
        crc = at(0, crc ^ u32::from(*b)) ^ (crc >> 8);
    }
    crc
}

/// `data`'s CRC.
pub(crate) fn crc(data: &[u8]) -> u32 {
    update(0, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One byte at a time, as MS-PST 5.3 writes it.
    fn plain(data: &[u8]) -> u32 {
        data.iter().fold(0u32, |crc, b| at(0, crc ^ u32::from(*b)) ^ (crc >> 8))
    }

    #[test]
    fn slicing_gives_what_one_byte_at_a_time_does() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i.wrapping_mul(31) >> 3) as u8).collect();
        for n in [0, 1, 7, 8, 9, 63, 64, 512, 8176, 10_000] {
            assert_eq!(crc(&data[..n]), plain(&data[..n]), "{n}");
        }
    }

    #[test]
    fn a_known_value() {
        // The usual CRC-32 of this text is 0xCBF43926: pre- and post-inverted; undone here.
        assert_eq!(update(u32::MAX, b"123456789") ^ u32::MAX, 0xCBF4_3926);
    }
}
