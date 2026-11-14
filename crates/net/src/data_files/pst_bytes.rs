//! A data file's bytes as the import and export tests read and change them by hand: MS-PST's
//! CRC, and a Unicode file's blocks, found from the trailers that end them (MS-PST 2.2.2.8).

use std::collections::HashMap;

pub(super) fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}

pub(super) fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

pub(super) fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

pub(super) fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

pub(super) fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// MS-PST's CRC: CRC-32's table, started at 0 and not inverted at the end.
pub(super) use mailcraft_pst::crc;

/// The size of a block of `size` bytes of data in the file: with its 16-byte trailer, in 64-byte
/// steps.
fn on_disk(size: usize) -> usize {
    (size + 16).div_ceil(64) * 64
}

/// Where the blocks of a Unicode file of `len` bytes may end: each 64-byte boundary, in order.
pub(super) fn block_ends(len: usize) -> impl Iterator<Item = usize> {
    (64..=len).step_by(64)
}

/// The block whose 16-byte trailer ends at `end` (one of [`block_ends`]): its id, where it
/// starts and how many bytes it holds (at most 8176); `None` unless the trailer's CRC matches
/// the bytes before it as they are now. Two blocks found so can overlap: once one of them is
/// changed, the other no longer matches.
pub(super) fn block_at(f: &[u8], end: usize) -> Option<(u64, (usize, usize))> {
    let size = usize::from(u16_at(f, end - 16));
    if size == 0 || size > 8176 {
        return None;
    }
    let start = end.checked_sub(on_disk(size))?;
    (crc(&f[start..start + size]) == u32_at(f, end - 12)).then(|| (u64_at(f, end - 8), (start, size)))
}

/// A Unicode file's blocks in the file's order ([`block_at`] each of its [`block_ends`]).
pub(super) fn block_list(f: &[u8]) -> Vec<(u64, (usize, usize))> {
    block_ends(f.len()).filter_map(|end| block_at(f, end)).collect()
}

/// [`block_list`] by id.
pub(super) fn blocks(f: &[u8]) -> HashMap<u64, (usize, usize)> {
    block_list(f).into_iter().collect()
}

/// Write a changed block's CRC into its trailer.
pub(super) fn seal(f: &mut [u8], (start, size): (usize, usize)) {
    let end = start + on_disk(size);
    let c = crc(&f[start..start + size]);
    put32(f, end - 12, c);
}
