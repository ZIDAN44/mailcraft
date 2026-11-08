//! Finding a node or a block in the file's two B-trees (MS-PST 2.2.2.7.7).

use std::io;
use std::rc::Rc;

use super::*;
use crate::util::{invalid, u8_at, u16_at, u32_at, u64_at};

/// Levels a B-tree may have: each page must be one level below the page naming it, so a tree
/// that names itself ends here.
const MAX_LEVELS: u8 = 8;

/// A node's entry in the node B-tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NodeEntry {
    pub(crate) nid: u32,
    pub(crate) data: u64,
    pub(crate) sub: u64,
    pub(crate) parent: u32,
}

/// A block's entry in the block B-tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlockEntry {
    pub(crate) bid: u64,
    pub(crate) ib: u64,
    pub(crate) size: u16,
    pub(crate) refs: u16,
}

/// What a B-tree page says about itself (`BTPAGE`).
pub(crate) struct BtPage {
    pub(crate) page: Rc<[u8]>,
    pub(crate) count: usize,
    pub(crate) entry: usize,
    pub(crate) level: u8,
}

impl BtPage {
    /// Takes in a page and checks its entries fit it. `leaf`: the size of the tree's leaf
    /// entries.
    pub(crate) fn parse(page: Rc<[u8]>, format: Format, leaf: usize) -> io::Result<BtPage> {
        let top = if format.wide() { 488 } else { 496 };
        let count = usize::from(u8_at(&page, top)?);
        let entry = usize::from(u8_at(&page, top + 2)?);
        let level = u8_at(&page, top + 3)?;
        let wanted = if level == 0 { leaf } else { Self::index_entry(format) };
        // `cbEnt` may be more than the entries need (MS-PST 2.2.2.7.7.4): each is read by it.
        if entry < wanted || count.saturating_mul(entry) > top {
            return Err(invalid("a B-tree page is damaged"));
        }
        Ok(BtPage { page, count, entry, level })
    }

    /// The size of an entry naming a page one level down (`BTENTRY`).
    pub(crate) fn index_entry(format: Format) -> usize {
        format.id_size() + format.id_size() * 2
    }

    /// The first field of entry `i`: its key.
    pub(crate) fn key(&self, i: usize, format: Format) -> io::Result<u64> {
        let at = i.checked_mul(self.entry).ok_or_else(crate::util::short)?;
        id_at(&self.page, at, format)
    }

    /// The last entry whose key is at most `key`.
    fn floor(&self, key: u64, format: Format) -> io::Result<Option<usize>> {
        let (mut low, mut high) = (0, self.count);
        while low < high {
            let mid = low + (high - low) / 2;
            if self.key(mid, format)? <= key {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Ok(low.checked_sub(1))
    }

    /// The page entry `i` names (only for pages above the leaves).
    pub(crate) fn child(&self, i: usize, format: Format) -> io::Result<Bref> {
        let at = i.checked_mul(self.entry).ok_or_else(crate::util::short)? + format.id_size();
        Ok(Bref { bid: id_at(&self.page, at, format)?, ib: id_at(&self.page, at + format.id_size(), format)? })
    }
}

pub(crate) fn id_at(b: &[u8], at: usize, format: Format) -> io::Result<u64> {
    if format.wide() { u64_at(b, at) } else { u32_at(b, at).map(u64::from) }
}

pub(crate) fn node_entry_size(format: Format) -> usize {
    if format.wide() { 32 } else { 16 }
}

pub(crate) fn block_entry_size(format: Format) -> usize {
    if format.wide() { 24 } else { 12 }
}

/// Node entry `i` of a leaf page.
pub(crate) fn node_entry(p: &BtPage, i: usize, format: Format) -> io::Result<NodeEntry> {
    let at = i.checked_mul(p.entry).ok_or_else(crate::util::short)?;
    let w = format.id_size();
    #[allow(clippy::cast_possible_truncation)] // a node's ID is its key's low 32 bits
    let nid = id_at(&p.page, at, format)? as u32;
    Ok(NodeEntry { nid, data: id_at(&p.page, at + w, format)?, sub: id_at(&p.page, at + 2 * w, format)?, parent: u32_at(&p.page, at + 3 * w)? })
}

/// Block entry `i` of a leaf page.
pub(crate) fn block_entry(p: &BtPage, i: usize, format: Format) -> io::Result<BlockEntry> {
    let at = i.checked_mul(p.entry).ok_or_else(crate::util::short)?;
    let w = format.id_size();
    Ok(BlockEntry {
        bid: id_at(&p.page, at, format)?,
        ib: id_at(&p.page, at + w, format)?,
        size: u16_at(&p.page, at + 2 * w)?,
        refs: u16_at(&p.page, at + 2 * w + 2)?,
    })
}

impl PstFile {
    /// The leaf page `key` falls in, and where in it.
    fn descend(&self, root: Bref, ptype: u8, leaf: usize, key: u64) -> io::Result<Option<(BtPage, usize)>> {
        let format = self.header.format;
        let mut at = root;
        let mut above: Option<u8> = None;
        loop {
            let p = BtPage::parse(self.page(at.ib, ptype, at.bid)?, format, leaf)?;
            if p.level >= MAX_LEVELS || above.is_some_and(|l| p.level + 1 != l) {
                return Err(invalid("a B-tree doesn't go down level by level"));
            }
            let Some(i) = p.floor(key, format)? else { return Ok(None) };
            if p.level == 0 {
                return Ok(Some((p, i)));
            }
            above = Some(p.level);
            at = p.child(i, format)?;
        }
    }

    /// The node `nid`.
    pub(crate) fn node(&self, nid: u32) -> io::Result<NodeEntry> {
        let format = self.header.format;
        let found = self.descend(self.header.root.nbt, PT_NBT, node_entry_size(format), u64::from(nid))?;
        if let Some((p, i)) = found {
            let e = node_entry(&p, i, format)?;
            if e.nid == nid {
                return Ok(e);
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, format!("no node {nid:#x}")))
    }

    /// The block `bid` (its lowest bit is ignored: MS-PST 2.2.2.2).
    pub(crate) fn block_entry(&self, bid: u64) -> io::Result<BlockEntry> {
        let format = self.header.format;
        let key = bid & !1;
        let found = self.descend(self.header.root.bbt, PT_BBT, block_entry_size(format), key)?;
        if let Some((p, i)) = found {
            let e = block_entry(&p, i, format)?;
            if e.bid & !1 == key {
                return Ok(e);
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, format!("no block {bid:#x}")))
    }
}
