//! A tree of fixed-size records kept in a heap (`BTH`, MS-PST 2.3.2).

use std::io;

use super::{Heap, HeapId};
use crate::util::{invalid, short, u8_at, u32_at};

const BTH_SIG: u8 = 0xB5;
/// Index levels a tree may have: a heap ID is 32 bits, and no real tree comes near.
const MAX_LEVELS: u8 = 8;
/// Records and index records read at most, over the whole tree: records that point back up it
/// can't keep the walk going.
const MAX_RECORDS: usize = 1 << 24;

/// The header's fields: a key's size, a record's data size, the levels and the first allocation.
pub(crate) struct TreeHeader {
    pub(crate) key: usize,
    pub(crate) data: usize,
    pub(crate) levels: u8,
    pub(crate) root: HeapId,
}

pub(crate) fn header(heap: &Heap, hid: HeapId) -> io::Result<TreeHeader> {
    let b = heap.alloc(hid)?;
    if u8_at(b, 0)? != BTH_SIG {
        return Err(invalid("a heap's tree is damaged"));
    }
    let header = TreeHeader { key: usize::from(u8_at(b, 1)?), data: usize::from(u8_at(b, 2)?), levels: u8_at(b, 3)?, root: u32_at(b, 4)? };
    if header.key == 0 || header.data == 0 || header.levels >= MAX_LEVELS {
        return Err(invalid("a heap's tree says sizes no tree has"));
    }
    Ok(header)
}

/// Every leaf record of the tree whose header is `hid`, as `(key, data)` slices of the heap, in
/// order of their keys. The tree's records must be `key` and `data` bytes, which is checked
/// before anything of it is read.
pub(crate) fn records(heap: &Heap, hid: HeapId, key: usize, data: usize) -> io::Result<(TreeHeader, Vec<(&[u8], &[u8])>)> {
    let h = header(heap, hid)?;
    if h.key != key || h.data != data {
        return Err(invalid("a heap's tree of records of the wrong size"));
    }
    let mut out: Vec<(&[u8], &[u8])> = Vec::new();
    if h.root == 0 {
        return Ok((h, out));
    }
    let mut seen = 0usize;
    let mut todo = vec![(h.root, h.levels)];
    while let Some((at, level)) = todo.pop() {
        let b = heap.alloc(at)?;
        if level == 0 {
            let width = h.key + h.data;
            for rec in b.chunks_exact(width) {
                let (k, d) = rec.split_at(h.key);
                // Little-endian, so from the last byte.
                if out.last().is_some_and(|(last, _)| k.iter().rev().cmp(last.iter().rev()).is_le()) {
                    return Err(invalid("a heap's tree with its keys out of order"));
                }
                out.push((k, d));
                seen += 1;
                if seen > MAX_RECORDS {
                    return Err(invalid("a heap's tree of more records than any has"));
                }
            }
        } else {
            let width = h.key + 4;
            let mut children = Vec::with_capacity(b.len() / width);
            for rec in b.chunks_exact(width) {
                seen += 1;
                if seen > MAX_RECORDS {
                    return Err(invalid("a heap's tree of more records than any has"));
                }
                children.push((u32_at(rec.get(h.key..).ok_or_else(short)?, 0)?, level - 1));
            }
            // Last first, so they come off the stack in order.
            todo.extend(children.into_iter().rev());
        }
    }
    Ok((h, out))
}

#[cfg(test)]
mod tests {
    use super::super::heap::tests::{heap_block, hid};
    use super::*;

    fn tree_header(key: u8, data: u8, levels: u8, root: HeapId) -> Vec<u8> {
        let mut b = vec![BTH_SIG, key, data, levels];
        b.extend(root.to_le_bytes());
        b
    }

    fn rec(key: u16, data: u32) -> Vec<u8> {
        let mut b = key.to_le_bytes().to_vec();
        b.extend(data.to_le_bytes());
        b
    }

    #[test]
    fn a_one_level_tree_reads_in_order() {
        let leaf: Vec<u8> = [rec(1, 10), rec(2, 20)].concat();
        let h = Heap::of(vec![heap_block(0xBC, hid(0, 1), &[&tree_header(2, 4, 0, hid(0, 2)), &leaf])]).unwrap();
        let (_, r) = records(&h, h.root, 2, 4).unwrap();
        assert_eq!(r.iter().map(|(k, _)| k[0]).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn a_two_level_tree_reads_in_order() {
        let (left, right) = (rec(1, 10), rec(5, 50));
        let mut index = 1u16.to_le_bytes().to_vec();
        index.extend(hid(0, 3).to_le_bytes());
        index.extend(5u16.to_le_bytes());
        index.extend(hid(0, 4).to_le_bytes());
        let h = Heap::of(vec![heap_block(0xBC, hid(0, 1), &[&tree_header(2, 4, 1, hid(0, 2)), &index, &left, &right])]).unwrap();
        let (_, r) = records(&h, h.root, 2, 4).unwrap();
        assert_eq!(r.iter().map(|(k, _)| k[0]).collect::<Vec<_>>(), vec![1, 5]);
    }

    #[test]
    fn an_empty_tree_has_no_records() {
        let h = Heap::of(vec![heap_block(0xBC, hid(0, 1), &[&tree_header(2, 4, 0, 0)])]).unwrap();
        assert!(records(&h, h.root, 2, 4).unwrap().1.is_empty());
    }

    #[test]
    fn a_tree_that_points_back_up_ends() {
        // Level 7, its one index allocation naming itself twice: 2^7 walks at the most.
        let mut index = 1u16.to_le_bytes().to_vec();
        index.extend(hid(0, 2).to_le_bytes());
        index.extend(2u16.to_le_bytes());
        index.extend(hid(0, 2).to_le_bytes());
        let h = Heap::of(vec![heap_block(0xBC, hid(0, 1), &[&tree_header(2, 4, 7, hid(0, 2)), &index])]).unwrap();
        // It ends one way or the other (a leaf-sized reading of the index, here), and quickly.
        let _ = records(&h, h.root, 2, 4);
    }

    #[test]
    fn a_header_with_impossible_sizes_is_an_error() {
        for bad in [tree_header(0, 4, 0, 0), tree_header(2, 0, 0, 0), tree_header(2, 4, 200, 0)] {
            let h = Heap::of(vec![heap_block(0xBC, hid(0, 1), &[&bad])]).unwrap();
            assert!(records(&h, h.root, 2, 4).is_err());
        }
    }

    #[test]
    fn records_of_the_wrong_size_are_refused_before_the_tree_is_read() {
        // The root names an allocation that isn't there: reading the tree would say so instead.
        let h = Heap::of(vec![heap_block(0xBC, hid(0, 1), &[&tree_header(2, 4, 0, hid(0, 9))])]).unwrap();
        let e = records(&h, h.root, 2, 6).err().unwrap();
        assert!(e.to_string().contains("wrong size"), "{e}");
    }

    #[test]
    fn keys_out_of_order_are_refused() {
        for leaf in [[rec(2, 20), rec(1, 10)].concat(), [rec(1, 10), rec(1, 11)].concat(), [rec(0x0100, 1), rec(0x00FF, 2)].concat()] {
            let h = Heap::of(vec![heap_block(0xBC, hid(0, 1), &[&tree_header(2, 4, 0, hid(0, 2)), &leaf])]).unwrap();
            assert!(records(&h, h.root, 2, 4).is_err());
        }
    }
}
