//! Reading blocks (MS-PST 2.2.2.8): one by itself, a data tree of them, and a subnode tree.

use std::collections::BTreeMap;
use std::io;

use super::btree::id_at;
use super::*;
use crate::crc::matches;
use crate::util::{align64, invalid, size, u8_at, u16_at, u32_at, u64_at};

/// A subnode tree's entries read at most: what a tree of the two levels MS-PST allows holds.
const MAX_SUBNODES: usize = 1 << 20;

// `btype`.
const BT_DATA: u8 = 0x01;
const BT_SUBNODE: u8 = 0x02;

/// What a walk over a data tree hands over.
enum Piece {
    /// About how many bytes are to come, before the first block; never more than the tree can
    /// hold, whatever its top block claims.
    Expect(usize),
    /// A block's data, in order.
    Block(Vec<u8>),
}

/// A node's subnodes (`SLENTRY`s): for each, its data block and its own subnode tree.
pub(crate) type SubNodes = BTreeMap<u32, (u64, u64)>;

impl PstFile {
    /// The block `bid`'s data as it is in the file, its trailer checked.
    pub(crate) fn raw_block(&self, bid: u64) -> io::Result<Vec<u8>> {
        let format = self.header.format;
        let entry = self.block_entry(bid)?;
        let len = usize::from(entry.size);
        let trailer = format.block_trailer();
        let total = align64(len + trailer);
        if len > format.block_data() || total > MAX_BLOCK {
            return Err(invalid("a block of a size no block can have"));
        }
        let mut b = vec![0u8; total];
        self.read_at(entry.ib, &mut b)?;
        let at = total - trailer;
        let (cb, stored, id) = if format.wide() {
            (u16_at(&b, at)?, u32_at(&b, at + 4)?, u64_at(&b, at + 8)?)
        } else {
            (u16_at(&b, at)?, u32_at(&b, at + 8)?, u64::from(u32_at(&b, at + 4)?))
        };
        if usize::from(cb) != len || id & !1 != bid & !1 {
            return Err(invalid("a block isn't the one named"));
        }
        b.truncate(len);
        if !matches(&b, stored) {
            return Err(invalid("a block is damaged"));
        }
        Ok(b)
    }

    /// The data block `bid`'s data, readable.
    fn data_block(&self, bid: u64) -> io::Result<Vec<u8>> {
        let mut b = self.raw_block(bid)?;
        self.crypt().decode(&mut b, bid);
        Ok(b)
    }

    /// The leaf blocks of the data tree at `bid`, up to `limit` bytes in all (past it: an error,
    /// before any of it is read), handed to `each` in order. A tree is read only as far as the
    /// size its top block declares, so blocks named over and over can't make it bigger than
    /// that.
    fn walk_tree(&self, bid: u64, limit: u64, each: &mut dyn FnMut(Piece) -> io::Result<()>) -> io::Result<()> {
        if bid & BID_INTERNAL == 0 {
            let b = self.data_block(bid)?;
            if b.len() as u64 > limit {
                return Err(too_big(b.len() as u64, limit));
            }
            return each(Piece::Block(b));
        }
        let top = self.raw_block(bid)?;
        let list = Internal::parse(&top, self.header.format)?;
        if u64::from(list.total) > limit {
            return Err(too_big(u64::from(list.total), limit));
        }
        if !matches!(list.level, 1 | 2) {
            return Err(invalid("a data tree of a depth none has"));
        }
        let data = self.header.format.block_data();
        let per_list = if list.level == 2 { data / self.header.format.id_size() } else { 1 };
        let most = list.count.saturating_mul(per_list).saturating_mul(data);
        each(Piece::Expect(size(u64::from(list.total))?.min(most)))?;
        let mut left = u64::from(list.total);
        for i in 0..list.count {
            if left == 0 {
                break;
            }
            let child = list.id(&top, i)?;
            if list.level == 1 {
                if child & BID_INTERNAL != 0 {
                    return Err(invalid("a data tree names a list where a block should be"));
                }
                left = self.take_leaf(child, left, each)?;
            } else {
                if child & BID_INTERNAL == 0 {
                    return Err(invalid("a data tree names a block where a list should be"));
                }
                let middle = self.raw_block(child)?;
                let inner = Internal::parse(&middle, self.header.format)?;
                if inner.level != 1 {
                    return Err(invalid("a data tree doesn't go down level by level"));
                }
                for j in 0..inner.count {
                    if left == 0 {
                        break;
                    }
                    let leaf = inner.id(&middle, j)?;
                    if leaf & BID_INTERNAL != 0 {
                        return Err(invalid("a data tree names a list where a block should be"));
                    }
                    left = self.take_leaf(leaf, left, each)?;
                }
            }
        }
        Ok(())
    }

    fn take_leaf(&self, bid: u64, left: u64, each: &mut dyn FnMut(Piece) -> io::Result<()>) -> io::Result<u64> {
        let mut b = self.data_block(bid)?;
        let n = (b.len() as u64).min(left);
        b.truncate(size(n)?);
        each(Piece::Block(b))?;
        Ok(left - n)
    }

    /// The data tree at `bid` whole, up to the file's limit.
    pub(crate) fn read_value(&self, bid: u64) -> io::Result<Vec<u8>> {
        self.read_value_within(bid, self.max_value())
    }

    /// The data tree at `bid` whole, up to `limit` bytes (and the file's limit).
    pub(crate) fn read_value_within(&self, bid: u64, limit: u64) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        self.walk_tree(bid, limit.min(self.max_value()), &mut |piece| {
            match piece {
                Piece::Expect(n) => out.try_reserve_exact(n).map_err(|_| too_big(n as u64, limit))?,
                Piece::Block(b) if out.capacity() == 0 => out = b,
                Piece::Block(b) => {
                    out.try_reserve(b.len()).map_err(|_| too_big(b.len() as u64, limit))?;
                    out.extend_from_slice(&b);
                }
            }
            Ok(())
        })?;
        Ok(out)
    }

    /// The data tree at `bid`, block by block, up to the file's limit.
    pub(crate) fn read_blocks(&self, bid: u64) -> io::Result<Vec<Vec<u8>>> {
        let mut out = Vec::new();
        self.walk_tree(bid, self.max_value(), &mut |piece| {
            if let Piece::Block(b) = piece {
                out.push(b);
            }
            Ok(())
        })?;
        Ok(out)
    }

    /// The data tree at `bid` whole, up to the file's limit.
    pub fn read_block(&self, bid: u64) -> io::Result<Vec<u8>> {
        self.read_value(bid)
    }

    /// The subnodes in the tree at `bid` (0: none).
    pub(crate) fn subnodes(&self, bid: u64) -> io::Result<SubNodes> {
        let mut out = SubNodes::new();
        if bid == 0 {
            return Ok(out);
        }
        let format = self.header.format;
        let top = self.raw_block(bid)?;
        let list = SubnodeBlock::parse(&top, format)?;
        match list.level {
            0 => list.leaves(&top, format, &mut out)?,
            1 => {
                for i in 0..list.count {
                    let child = id_at(&top, list.first + i * list.entry + format.id_size(), format)?;
                    let inner = self.raw_block(child)?;
                    let leaf = SubnodeBlock::parse(&inner, format)?;
                    if leaf.level != 0 {
                        return Err(invalid("a subnode tree doesn't go down level by level"));
                    }
                    leaf.leaves(&inner, format, &mut out)?;
                }
            }
            _ => return Err(invalid("a subnode tree of a depth none has")),
        }
        Ok(out)
    }
}

fn too_big(n: u64, limit: u64) -> io::Error {
    io::Error::new(io::ErrorKind::FileTooLarge, format!("a value of {n} bytes, over the limit of {limit}"))
}

/// A block listing data blocks (`XBLOCK` or `XXBLOCK`).
struct Internal {
    level: u8,
    count: usize,
    total: u32,
    format: Format,
}

impl Internal {
    fn parse(b: &[u8], format: Format) -> io::Result<Internal> {
        if u8_at(b, 0)? != BT_DATA {
            return Err(invalid("a data tree's list is damaged"));
        }
        let level = u8_at(b, 1)?;
        let count = usize::from(u16_at(b, 2)?);
        let total = u32_at(b, 4)?;
        if count.saturating_mul(format.id_size()).saturating_add(8) > b.len() {
            return Err(invalid("a data tree's list ends past its block"));
        }
        Ok(Internal { level, count, total, format })
    }

    fn id(&self, b: &[u8], i: usize) -> io::Result<u64> {
        id_at(b, 8 + i * self.format.id_size(), self.format)
    }
}

/// A block of a subnode tree (`SLBLOCK` or `SIBLOCK`).
struct SubnodeBlock {
    level: u8,
    count: usize,
    first: usize,
    entry: usize,
}

impl SubnodeBlock {
    fn parse(b: &[u8], format: Format) -> io::Result<SubnodeBlock> {
        if u8_at(b, 0)? != BT_SUBNODE {
            return Err(invalid("a subnode tree is damaged"));
        }
        let level = u8_at(b, 1)?;
        let count = usize::from(u16_at(b, 2)?);
        let id = format.id_size();
        let (first, entry) = (if format.wide() { 8 } else { 4 }, if level == 0 { 3 * id } else { 2 * id });
        if count.saturating_mul(entry).saturating_add(first) > b.len() {
            return Err(invalid("a subnode tree's block ends past its data"));
        }
        Ok(SubnodeBlock { level, count, first, entry })
    }

    fn leaves(&self, b: &[u8], format: Format, out: &mut SubNodes) -> io::Result<()> {
        let w = format.id_size();
        for i in 0..self.count {
            if out.len() >= MAX_SUBNODES {
                return Err(invalid("a subnode tree of more entries than any has"));
            }
            let at = self.first + i * self.entry;
            #[allow(clippy::cast_possible_truncation)] // a node's ID is its key's low 32 bits
            let nid = id_at(b, at, format)? as u32;
            out.insert(nid, (id_at(b, at + w, format)?, id_at(b, at + 2 * w, format)?));
        }
        Ok(())
    }
}
