//! Where new blocks and pages go, and the allocation maps that say so (MS-PST 2.2.2.7.2, 2.6.1).
//!
//! The file is divided into runs of 253,952 bytes, each starting with an allocation map (`AMap`)
//! of one bit per 64 bytes. New space comes from what the maps of the last few runs mark free,
//! else from the end of the file, which grows a whole run at a time.

use std::io;

use crate::ndb::{Format, PAGE};
use crate::util::invalid;

/// Where the first map is.
pub(crate) const FIRST_AMAP: u64 = 0x4400;
/// The bytes one map covers, itself included.
pub(crate) const AMAP_BYTES: u64 = 496 * 8 * 64;
/// A page map (`PMap`) comes every 8 runs, a free map every 496 after the first 128, and a free
/// page map every 31,744 after the first 8,192: all deprecated, but still in their places.
const PMAP_EVERY: u64 = 8;
const FMAP_FIRST: u64 = 128;
const FMAP_EVERY: u64 = 496;
const FPMAP_FIRST: u64 = 128 * 64;
const FPMAP_EVERY: u64 = 496 * 64;
/// The most an ANSI file can hold: the end of the one free page map its header has.
pub(crate) const ANSI_MAX: u64 = FIRST_AMAP + FPMAP_FIRST * AMAP_BYTES;
/// Maps looked through for free space, from the end of the file.
const MAPS_SCANNED: u64 = 256;

const UNIT: u64 = 64;

/// A kind of map page at the start of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MapKind {
    Amap,
    Pmap,
    Fmap,
    Fpmap,
}

/// The map pages at the start of run `i`, in order.
pub(crate) fn map_pages(i: u64) -> Vec<MapKind> {
    let mut out = vec![MapKind::Amap];
    if i.is_multiple_of(PMAP_EVERY) {
        out.push(MapKind::Pmap);
        if i >= FMAP_FIRST && (i - FMAP_FIRST).is_multiple_of(FMAP_EVERY) {
            out.push(MapKind::Fmap);
        }
        if i >= FPMAP_FIRST && (i - FPMAP_FIRST).is_multiple_of(FPMAP_EVERY) {
            out.push(MapKind::Fpmap);
        }
    }
    out
}

/// The offset of run `i`.
pub(crate) fn run_at(i: u64) -> u64 {
    FIRST_AMAP.saturating_add(i.saturating_mul(AMAP_BYTES))
}

/// The run offset `at` falls in.
pub(crate) fn run_of(at: u64) -> Option<u64> {
    at.checked_sub(FIRST_AMAP).map(|n| n / AMAP_BYTES)
}

/// The bytes at the start of run `i` its map pages take.
fn reserved(i: u64) -> u64 {
    (map_pages(i).len() * PAGE) as u64
}

/// The space a set of changes takes and lets go of.
pub(crate) struct Space {
    format: Format,
    /// Runs the file has when opened.
    runs: u64,
    /// Where the next space from the end of the file goes.
    tail: u64,
    /// Free ranges to use first, in order, none crossing a run.
    free: Vec<(u64, u64)>,
    /// What's been taken from either.
    pub(crate) taken: Vec<(u64, u64)>,
    /// What's let go of: free only once the changes are committed.
    pub(crate) released: Vec<(u64, u64)>,
}

impl Space {
    pub(crate) fn new(format: Format, eof: u64) -> Space {
        let runs = eof.saturating_sub(FIRST_AMAP).div_ceil(AMAP_BYTES);
        Space { format, runs, tail: eof.max(FIRST_AMAP), free: Vec::new(), taken: Vec::new(), released: Vec::new() }
    }

    /// Runs the file has when opened.
    pub(crate) fn runs(&self) -> u64 {
        self.runs
    }

    /// How many of the last runs to look through for free space, and from which.
    pub(crate) fn scan(&self) -> (u64, u64) {
        (self.runs.saturating_sub(MAPS_SCANNED), self.runs)
    }

    /// Adds what a map marks free, `(start, end)`, in file order.
    pub(crate) fn add_free(&mut self, ranges: Vec<(u64, u64)>) {
        self.free.extend(ranges);
    }

    /// Lets go of everything the maps offered: they don't match what the file holds.
    pub(crate) fn distrust(&mut self) {
        self.free.clear();
    }

    pub(crate) fn free_ranges(&self) -> &[(u64, u64)] {
        &self.free
    }

    /// `size` bytes, a multiple of 64, aligned to `align`: from free space, else the end.
    pub(crate) fn alloc(&mut self, size: u64, align: u64) -> io::Result<u64> {
        let at = match self.take_free(size, align) {
            Some(at) => at,
            None => self.take_tail(size, align)?,
        };
        self.taken.push((at, at + size));
        Ok(at)
    }

    fn take_free(&mut self, size: u64, align: u64) -> Option<u64> {
        for i in 0..self.free.len() {
            let (start, end) = *self.free.get(i)?;
            let at = start.div_ceil(align).checked_mul(align)?;
            if at.checked_add(size)? <= end {
                let mut pieces = Vec::with_capacity(2);
                if start < at {
                    pieces.push((start, at));
                }
                if at + size < end {
                    pieces.push((at + size, end));
                }
                self.free.splice(i..=i, pieces);
                return Some(at);
            }
        }
        None
    }

    fn take_tail(&mut self, size: u64, align: u64) -> io::Result<u64> {
        let too_far = || invalid("the data file can't grow any further");
        let mut at = self.tail.div_ceil(align).checked_mul(align).ok_or_else(too_far)?;
        loop {
            let run = run_of(at).ok_or_else(too_far)?;
            let start = run_at(run);
            let usable = start + reserved(run);
            if at < usable {
                at = usable.div_ceil(align).checked_mul(align).ok_or_else(too_far)?;
                continue;
            }
            let next = start + AMAP_BYTES;
            if at.checked_add(size).ok_or_else(too_far)? > next {
                at = (next + reserved(run + 1)).div_ceil(align).checked_mul(align).ok_or_else(too_far)?;
                continue;
            }
            break;
        }
        if self.format == Format::Ansi && at + size > ANSI_MAX {
            return Err(invalid("an ANSI data file (Outlook 97–2002's format) can't grow past about 2 GB"));
        }
        self.tail = at + size;
        Ok(at)
    }

    /// Lets go of a range at the end of the change.
    pub(crate) fn release(&mut self, start: u64, size: u64) {
        self.released.push((start, start + size));
    }

    /// The runs the file ends up with.
    pub(crate) fn final_runs(&self) -> u64 {
        self.runs.max(self.tail.saturating_sub(FIRST_AMAP).div_ceil(AMAP_BYTES))
    }
}

/// The bits a map page would have for a new run: its map pages' space taken.
pub(crate) fn new_bits(i: u64) -> [u8; 496] {
    let mut bits = [0u8; 496];
    let count = map_pages(i).len();
    for b in bits.iter_mut().take(count) {
        *b = 0xFF;
    }
    bits
}

/// Free ranges in a map's bits, for run `i` (ending at `eof` at the latest).
pub(crate) fn free_in(i: u64, bits: &[u8], eof: u64) -> Vec<(u64, u64)> {
    let start = run_at(i);
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (byte, b) in bits.iter().enumerate() {
        if *b == 0xFF {
            continue;
        }
        for bit in 0..8u64 {
            if b & (0x80 >> bit) != 0 {
                continue;
            }
            let at = start + (byte as u64 * 8 + bit) * UNIT;
            if at + UNIT > eof {
                return out;
            }
            match out.last_mut() {
                Some((_, end)) if *end == at => *end += UNIT,
                _ => out.push((at, at + UNIT)),
            }
        }
    }
    out
}

/// Sets (`to`) the bits of `[start, end)`, within run `i`. How many bytes changed state, signed
/// for bytes that became free.
pub(crate) fn mark(bits: &mut [u8], i: u64, start: u64, end: u64, to: bool) -> io::Result<i64> {
    let base = run_at(i);
    if start < base || end > base + AMAP_BYTES || end < start {
        return Err(invalid("a range past its allocation map"));
    }
    let (first, last) = ((start - base) / UNIT, (end - base).div_ceil(UNIT));
    let mut changed = 0i64;
    for bit in first..last {
        let byte = crate::util::size(bit / 8)?;
        let mask = 0x80u8 >> (bit % 8);
        let slot = bits.get_mut(byte).ok_or_else(|| invalid("a range past its allocation map"))?;
        if (*slot & mask != 0) != to {
            *slot ^= mask;
            changed += UNIT as i64;
        }
    }
    Ok(if to { -changed } else { changed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_pages_are_where_ms_pst_puts_them() {
        assert_eq!(map_pages(0), vec![MapKind::Amap, MapKind::Pmap]);
        assert_eq!(map_pages(1), vec![MapKind::Amap]);
        assert_eq!(map_pages(128), vec![MapKind::Amap, MapKind::Pmap, MapKind::Fmap]);
        assert_eq!(map_pages(128 + 496), vec![MapKind::Amap, MapKind::Pmap, MapKind::Fmap]);
        assert_eq!(map_pages(8192), vec![MapKind::Amap, MapKind::Pmap, MapKind::Fpmap]);
    }

    #[test]
    fn space_comes_from_the_free_list_then_the_end() {
        let mut s = Space::new(Format::Unicode, FIRST_AMAP + AMAP_BYTES);
        s.add_free(vec![(FIRST_AMAP + 1024, FIRST_AMAP + 2048)]);
        assert_eq!(s.alloc(512, 512).unwrap(), FIRST_AMAP + 1024);
        assert_eq!(s.alloc(512, 512).unwrap(), FIRST_AMAP + 1536);
        // The free list is empty; the end of the file is the next run, past its map page.
        let next = FIRST_AMAP + AMAP_BYTES;
        assert_eq!(s.alloc(512, 512).unwrap(), next + 512);
        assert_eq!(s.final_runs(), 2);
    }

    #[test]
    fn a_block_never_crosses_a_run() {
        let mut s = Space::new(Format::Unicode, FIRST_AMAP + AMAP_BYTES);
        s.tail = FIRST_AMAP + AMAP_BYTES - 64;
        let at = s.alloc(8192, 64).unwrap();
        assert!(at >= FIRST_AMAP + AMAP_BYTES + 512, "{at:#x}");
    }

    #[test]
    fn marks_count_what_changed_and_free_runs_are_found() {
        let mut bits = new_bits(1);
        let i = 1;
        let base = run_at(i);
        assert_eq!(mark(&mut bits, i, base + 512, base + 576, true).unwrap(), -64);
        assert_eq!(mark(&mut bits, i, base + 512, base + 576, true).unwrap(), 0);
        let free = free_in(i, &bits, base + AMAP_BYTES);
        assert_eq!(free.first(), Some(&(base + 576, base + AMAP_BYTES)));
        assert_eq!(mark(&mut bits, i, base + 512, base + 576, false).unwrap(), 64);
        assert!(mark(&mut bits, i, base - 64, base, true).is_err());
    }

    #[test]
    fn an_ansi_file_stops_where_its_format_does() {
        let mut s = Space::new(Format::Ansi, ANSI_MAX - 512);
        assert!(s.alloc(1024, 64).is_err());
    }
}
