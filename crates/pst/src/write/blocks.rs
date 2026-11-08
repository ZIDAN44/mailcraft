//! Data trees and subnode trees written, and let go of (MS-PST 2.2.2.8.3).

use std::io::{self, Read};

use super::edit::Editor;
use crate::ndb::btree::id_at;
use crate::ndb::*;
use crate::util::{invalid, short, u8_at, u16_at};

const BT_DATA: u8 = 0x01;
const BT_SUBNODE: u8 = 0x02;
/// Subnode trees nested at most when one is let go of: attached messages in attached messages,
/// two levels each, and a file whose tree names itself ends here.
const MAX_RELEASE_DEPTH: usize = 2 * crate::messaging::MAX_EMBEDDED_DEPTH + 8;

impl Editor {
    /// How many IDs a list block (`XBLOCK`, `XXBLOCK`) holds.
    fn list_room(&self) -> usize {
        let f = self.format();
        (f.block_data() - 8) / f.id_size()
    }

    /// Writes a block listing `ids`, at `level`, of `total` bytes under it.
    fn write_list(&mut self, level: u8, ids: &[u64], total: u32) -> io::Result<u64> {
        let f = self.format();
        let mut b = Vec::with_capacity(8 + ids.len() * f.id_size());
        b.extend([BT_DATA, level]);
        b.extend(u16::try_from(ids.len()).map_err(|_| short())?.to_le_bytes());
        b.extend(total.to_le_bytes());
        for id in ids {
            put_id(&mut b, f, *id)?;
        }
        self.write_block(&b, true)
    }

    /// The tree over these leaf blocks (their IDs and sizes): one by itself needs none.
    fn tree_over(&mut self, leaves: &[(u64, usize)]) -> io::Result<u64> {
        if let [(bid, _)] = leaves {
            return Ok(*bid);
        }
        let room = self.list_room().max(1);
        let total = |part: &[(u64, usize)]| -> io::Result<u32> {
            u32::try_from(part.iter().map(|(_, n)| *n as u64).sum::<u64>()).map_err(|_| invalid("a value too big to keep"))
        };
        let mut lists = Vec::with_capacity(leaves.len() / room + 1);
        for part in leaves.chunks(room) {
            let ids: Vec<u64> = part.iter().map(|(b, _)| *b).collect();
            let sum = total(part)?;
            lists.push((self.write_list(1, &ids, sum)?, sum));
        }
        if let [(bid, _)] = lists.as_slice() {
            return Ok(*bid);
        }
        if lists.len() > room {
            return Err(invalid("a value too big to keep"));
        }
        let ids: Vec<u64> = lists.iter().map(|(b, _)| *b).collect();
        self.write_list(2, &ids, total(leaves)?)
    }

    /// Writes `data` as a data tree, every block full but the last.
    pub(crate) fn write_tree(&mut self, data: &[u8]) -> io::Result<u64> {
        let size = self.format().block_data();
        let mut leaves = Vec::with_capacity(data.len() / size + 1);
        if data.is_empty() {
            leaves.push((self.write_block(&[], false)?, 0));
        }
        for chunk in data.chunks(size) {
            leaves.push((self.write_block(chunk, false)?, chunk.len()));
        }
        self.tree_over(&leaves)
    }

    /// Writes these blocks as a data tree (a heap's, or a table's rows).
    pub(crate) fn write_leaves(&mut self, blocks: &[Vec<u8>]) -> io::Result<u64> {
        let mut leaves = Vec::with_capacity(blocks.len().max(1));
        for b in blocks {
            leaves.push((self.write_block(b, false)?, b.len()));
        }
        if leaves.is_empty() {
            leaves.push((self.write_block(&[], false)?, 0));
        }
        self.tree_over(&leaves)
    }

    /// Writes exactly `size` bytes read from `source` as a data tree, a block at a time.
    pub(crate) fn write_stream(&mut self, source: &mut dyn Read, size: u64) -> io::Result<u64> {
        let block = self.format().block_data();
        let mut buf = vec![0u8; block];
        let mut leaves = Vec::new();
        let mut left = size;
        while left > 0 {
            let want = crate::util::size(left.min(block as u64))?;
            let chunk = buf.get_mut(..want).ok_or_else(short)?;
            source.read_exact(chunk)?;
            leaves.push((self.write_block(chunk, false)?, want));
            left -= want as u64;
        }
        let mut extra = [0u8; 1];
        if source.read(&mut extra)? != 0 {
            return Err(invalid("a file gave more than its size"));
        }
        if leaves.is_empty() {
            leaves.push((self.write_block(&[], false)?, 0));
        }
        self.tree_over(&leaves)
    }

    /// Writes a subnode tree of these entries; 0 when there are none.
    pub(crate) fn write_subnodes(&mut self, subs: &SubNodes) -> io::Result<u64> {
        if subs.is_empty() {
            return Ok(0);
        }
        let f = self.format();
        let id = f.id_size();
        let head = if f.wide() { 8 } else { 4 };
        let room = (f.block_data() - head) / (id * 3);
        let list: Vec<(u32, (u64, u64))> = subs.iter().map(|(k, v)| (*k, *v)).collect();
        let mut leaves = Vec::new();
        for part in list.chunks(room.max(1)) {
            let mut b = Vec::with_capacity(head + part.len() * id * 3);
            b.extend([BT_SUBNODE, 0]);
            b.extend(u16::try_from(part.len()).map_err(|_| short())?.to_le_bytes());
            b.resize(head, 0);
            for (nid, (data, sub)) in part {
                put_id(&mut b, f, u64::from(*nid))?;
                put_id(&mut b, f, *data)?;
                put_id(&mut b, f, *sub)?;
            }
            leaves.push((part.first().map_or(0, |(k, _)| *k), self.write_block(&b, true)?));
        }
        if let [(_, bid)] = leaves.as_slice() {
            return Ok(*bid);
        }
        let room = (f.block_data() - head) / (id * 2);
        if leaves.len() > room {
            return Err(invalid("more subnodes than a tree holds"));
        }
        let mut b = Vec::with_capacity(head + leaves.len() * id * 2);
        b.extend([BT_SUBNODE, 1]);
        b.extend(u16::try_from(leaves.len()).map_err(|_| short())?.to_le_bytes());
        b.resize(head, 0);
        for (nid, bid) in &leaves {
            put_id(&mut b, f, u64::from(*nid))?;
            put_id(&mut b, f, *bid)?;
        }
        self.write_block(&b, true)
    }

    /// Lets go of the data tree at `bid`: the blocks nothing else refers to go.
    pub(crate) fn release_tree(&mut self, bid: u64) -> io::Result<()> {
        if bid == 0 {
            return Ok(());
        }
        let Some(raw) = self.released_list(bid)? else { return Ok(()) };
        let f = self.format();
        let level = u8_at(&raw, 1)?;
        for i in 0..usize::from(u16_at(&raw, 2)?) {
            let child = id_at(&raw, 8 + i * f.id_size(), f)?;
            match level {
                1 => {
                    self.drop_ref(child)?;
                }
                2 => {
                    if child & BID_INTERNAL == 0 {
                        return Err(invalid("a data tree's list names a block where a list should be"));
                    }
                    let Some(list) = self.released_list(child)? else { continue };
                    if u8_at(&list, 1)? != 1 {
                        return Err(invalid("a data tree doesn't go down level by level"));
                    }
                    for j in 0..usize::from(u16_at(&list, 2)?) {
                        self.drop_ref(id_at(&list, 8 + j * f.id_size(), f)?)?;
                    }
                }
                _ => return Err(invalid("a data tree of a depth none has")),
            }
        }
        Ok(())
    }

    /// One reference less to the block `bid`: the list it holds when that was its last and it
    /// wasn't let go of before, whose children then lose one too.
    fn released_list(&mut self, bid: u64) -> io::Result<Option<Vec<u8>>> {
        let raw = if bid & BID_INTERNAL != 0 { Some(self.pst.raw_block(bid)?) } else { None };
        if !self.drop_ref(bid)? || !self.first_release(bid) {
            return Ok(None);
        }
        let Some(raw) = raw else { return Ok(None) };
        if u8_at(&raw, 0)? != BT_DATA {
            return Err(invalid("a data tree's list is damaged"));
        }
        Ok(Some(raw))
    }

    /// Lets go of a node's data and everything under it.
    pub(crate) fn release_node_data(&mut self, data: u64, sub: u64) -> io::Result<()> {
        self.release_node_within(data, sub, 0)
    }

    fn release_node_within(&mut self, data: u64, sub: u64, depth: usize) -> io::Result<()> {
        if depth > MAX_RELEASE_DEPTH {
            return Err(invalid("subnodes nested deeper than any are"));
        }
        self.release_tree(data)?;
        self.release_subnodes(sub, depth)
    }

    /// Lets go of the subnode tree at `bid`, and of what its subnodes hold.
    fn release_subnodes(&mut self, bid: u64, depth: usize) -> io::Result<()> {
        if bid == 0 {
            return Ok(());
        }
        let subs = self.pst.subnodes(bid)?;
        let raw = self.pst.raw_block(bid)?;
        // Still in use elsewhere: what it lists stays, and so does what its subnodes hold.
        if !self.drop_ref(bid)? || !self.first_release(bid) {
            return Ok(());
        }
        // The tree's own blocks: those it lists when it is two levels.
        if u8_at(&raw, 1)? == 1 {
            let f = self.format();
            let head = if f.wide() { 8 } else { 4 };
            for i in 0..usize::from(u16_at(&raw, 2)?) {
                self.drop_ref(id_at(&raw, head + i * 2 * f.id_size() + f.id_size(), f)?)?;
            }
        }
        for (data, sub) in subs.into_values() {
            self.release_node_within(data, sub, depth + 1)?;
        }
        Ok(())
    }
}

fn put_id(out: &mut Vec<u8>, format: Format, id: u64) -> io::Result<()> {
    if format.wide() {
        out.extend(id.to_le_bytes());
    } else {
        out.extend(u32::try_from(id).map_err(|_| invalid("past what an ANSI data file can hold"))?.to_le_bytes());
    }
    Ok(())
}
