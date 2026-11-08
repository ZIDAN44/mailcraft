//! A node's bytes as a heap of small allocations (MS-PST 2.3.1).

use std::io;

use crate::ndb::PstFile;
use crate::util::{invalid, short, u8_at, u16_at, u32_at};

/// Blocks a heap can have: a heap ID's block index is 16 bits.
const MAX_BLOCKS: usize = 1 << 16;

/// A heap's signature (`bSig`).
const HN_SIG: u8 = 0xEC;

/// An allocation in a heap (`HID`): five type bits (zero), its index from 1, then its block.
pub(crate) type HeapId = u32;

/// A node's data, read as a heap.
pub(crate) struct Heap {
    blocks: Vec<Vec<u8>>,
    /// What the heap is for (`bClientSig`).
    pub(crate) client: u8,
    /// The heap's root allocation.
    pub(crate) root: HeapId,
}

/// Where a block's map of allocations is (`ibHnpm`): always a block's first two bytes.
fn page_map_at(block: &[u8]) -> io::Result<usize> {
    u16_at(block, 0).map(usize::from)
}

impl Heap {
    /// The heap in the data tree at `bid`.
    pub(crate) fn read(pst: &PstFile, bid: u64) -> io::Result<Heap> {
        let blocks = pst.read_blocks(bid)?;
        Heap::of(blocks)
    }

    /// The heap held in these blocks.
    pub(crate) fn of(blocks: Vec<Vec<u8>>) -> io::Result<Heap> {
        if blocks.len() > MAX_BLOCKS {
            return Err(invalid("a heap of more blocks than one can have"));
        }
        let first = blocks.first().ok_or_else(|| invalid("a heap with no block"))?;
        if u8_at(first, 2)? != HN_SIG {
            return Err(invalid("a heap's header is damaged"));
        }
        let (client, root) = (u8_at(first, 3)?, u32_at(first, 4)?);
        Ok(Heap { blocks, client, root })
    }

    /// The allocation `hid`.
    pub(crate) fn alloc(&self, hid: HeapId) -> io::Result<&[u8]> {
        if hid & 0x1F != 0 {
            return Err(invalid("not a heap ID"));
        }
        let block_index = usize::try_from(hid >> 16).map_err(|_| short())?;
        let index = usize::try_from((hid >> 5) & 0x7FF).map_err(|_| short())?;
        let block = self.blocks.get(block_index).ok_or_else(|| invalid("a heap ID past its heap's blocks"))?;
        let map = page_map_at(block)?;
        let count = usize::from(u16_at(block, map)?);
        if index == 0 || index > count {
            return Err(invalid("a heap ID past its block's allocations"));
        }
        let offset = |i: usize| -> io::Result<usize> { u16_at(block, map + 4 + i * 2).map(usize::from) };
        let (start, end) = (offset(index - 1)?, offset(index)?);
        block.get(start..end).ok_or_else(|| invalid("a heap allocation past its block"))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A heap's one block, of these allocations.
    pub(crate) fn heap_block(client: u8, root: HeapId, allocations: &[&[u8]]) -> Vec<u8> {
        let mut b = vec![0u8; 12];
        let mut offsets = vec![12u16];
        for a in allocations {
            b.extend_from_slice(a);
            offsets.push(u16::try_from(b.len()).unwrap());
        }
        if !b.len().is_multiple_of(2) {
            b.push(0);
        }
        let map = u16::try_from(b.len()).unwrap();
        b.extend(u16::try_from(allocations.len()).unwrap().to_le_bytes());
        b.extend(0u16.to_le_bytes());
        for o in offsets {
            b.extend(o.to_le_bytes());
        }
        b[0..2].copy_from_slice(&map.to_le_bytes());
        b[2] = HN_SIG;
        b[3] = client;
        b[4..8].copy_from_slice(&root.to_le_bytes());
        b
    }

    pub(crate) fn hid(block: u32, index: u32) -> HeapId {
        (block << 16) | (index << 5)
    }

    #[test]
    fn allocations_read_by_their_ids() {
        let h = Heap::of(vec![heap_block(0xBC, hid(0, 1), &[b"one", b"", b"three"])]).unwrap();
        assert_eq!((h.client, h.root), (0xBC, hid(0, 1)));
        assert_eq!(h.alloc(hid(0, 1)).unwrap(), b"one");
        assert_eq!(h.alloc(hid(0, 2)).unwrap(), b"");
        assert_eq!(h.alloc(hid(0, 3)).unwrap(), b"three");
    }

    #[test]
    fn ids_past_the_heap_and_damaged_headers_are_errors() {
        let h = Heap::of(vec![heap_block(0xBC, hid(0, 1), &[b"one"])]).unwrap();
        for bad in [hid(0, 0), hid(0, 2), hid(1, 1), hid(0, 1) | 1, u32::MAX] {
            assert!(h.alloc(bad).is_err(), "{bad:#x}");
        }
        let mut block = heap_block(0xBC, hid(0, 1), &[b"one"]);
        block[2] = 0;
        assert!(Heap::of(vec![block]).is_err());
        assert!(Heap::of(vec![]).is_err());
        assert!(Heap::of(vec![vec![]]).is_err());
    }

    #[test]
    fn a_map_with_offsets_past_its_block_is_an_error() {
        let mut block = heap_block(0xBC, hid(0, 1), &[b"one"]);
        let map = usize::from(u16::from_le_bytes([block[0], block[1]]));
        block[map + 6] = 0xFF;
        block[map + 7] = 0xFF;
        let h = Heap::of(vec![block]).unwrap();
        assert!(h.alloc(hid(0, 1)).is_err());
    }
}
