//! What the data file tests (and the import worker's) make up the same way each time: times as
//! Outlook keeps them, bytes that don't repeat, a signed message's content.

use super::outlook::EPOCH_1601;

/// `unix` seconds as a FILETIME (100 ns since 1601), as an item's times are kept.
pub(crate) const fn filetime(unix: i64) -> i64 {
    (unix + EPOCH_1601) * 10_000_000
}

/// `len` bytes of xorshift from `seed`: the same each time, and nothing a compressor or a writer
/// keeping repeated blocks once could make smaller.
pub(crate) fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// A CMS `ContentInfo` of this type (2 signed data, 3 enveloped data), its content made up.
pub(crate) fn cms(kind: u8) -> Vec<u8> {
    let mut b = vec![0x30, 0x80, 0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, kind, 0xA0, 0x80];
    b.extend([0x5A; 40]);
    b
}
