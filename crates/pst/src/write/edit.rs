//! One change to a data file: everything new goes in free space, and the file becomes the changed
//! one when its header is written, last (MS-PST 2.6).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::Path;
use std::rc::Rc;

use super::btree::{Changes, finish_page};
use super::space::{self, AMAP_BYTES, MapKind, Space, run_at, run_of};
use crate::messaging::NID_TYPE_NORMAL_FOLDER;
use crate::messaging::PstStore;
use crate::ndb::btree::{block_entry, block_entry_size, node_entry_size};
use crate::ndb::*;
use crate::util::{align64, invalid, short};

/// The most any value may be when a file is added to.
pub const ADD_MAX_VALUE_SIZE: u64 = 256 * 1024 * 1024;

/// Writes waiting to be made, to keep runs of blocks in one call.
const WRITE_BUFFER: usize = 1 << 20;

// Allocation map validity (`fAMapValid`).
const AMAP_INVALID: u8 = 0;
const AMAP_VALID: u8 = 2;

const PT_AMAP: u8 = 0x84;
const PT_PMAP: u8 = 0x83;
const PT_FMAP: u8 = 0x82;
const PT_FPMAP: u8 = 0x85;
const PT_DLIST: u8 = 0x86;
const DLIST_AT: u64 = 0x4200;

/// A data file being changed.
pub(crate) struct Editor {
    file: File,
    /// The file as it is until the change is committed.
    pub(crate) pst: Rc<PstFile>,
    pub(crate) store: Rc<PstStore>,
    pub(crate) header: Header,
    pub(crate) space: Space,
    /// Nodes changed: their new entry, or gone.
    nodes: BTreeMap<u32, Option<NodeEntry>>,
    /// Blocks written.
    new_blocks: BTreeMap<u64, BlockEntry>,
    /// Blocks of the file that gained or lost references.
    refs: HashMap<u64, i32>,
    buffer: Vec<u8>,
    buffer_at: u64,
    /// The maps can be used and must be kept. When they can't (the header says so, or they don't
    /// match what the file holds), space comes only from the end of the file, the maps of the
    /// runs already there aren't touched, and the header goes on saying they aren't valid, which
    /// is what has Outlook rebuild them rather than allocate over what they miss.
    maps_valid: bool,
    /// Blocks already let go of in this change: one named again doesn't go through its children
    /// again, so a tree that names itself ends.
    released: HashSet<u64>,
}

impl Editor {
    /// Opens the data file at `path` to change it.
    pub(crate) fn open(path: &Path) -> io::Result<Editor> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let reader = file.try_clone()?;
        Self::over(file, Box::new(reader), false)
    }

    /// Opens a file that has no nodes yet.
    pub(crate) fn open_bare(path: &Path) -> io::Result<Editor> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let reader = file.try_clone()?;
        Self::over(file, Box::new(reader), true)
    }

    fn over(file: File, reader: Box<dyn ReadSeek>, bare: bool) -> io::Result<Editor> {
        let pst = PstFile::read_from(reader)?;
        // Others read as far as they read; added to, only those whose layout is known for sure.
        if !matches!(pst.header.version(), 14 | 15 | 23) {
            return Err(invalid(format!("mail can't be added to a data file of version {}", pst.header.version())));
        }
        pst.set_max_value_size(ADD_MAX_VALUE_SIZE);
        let pst = Rc::new(pst);
        let store = if bare { PstStore::bare(pst.clone()) } else { PstStore::read(pst.clone())? };
        let header = pst.header.clone();
        let maps_valid = header.root.amap_valid != AMAP_INVALID;
        let space = Space::new(header.format, header.root.eof);
        let mut editor = Editor {
            file,
            pst,
            store,
            header,
            space,
            nodes: BTreeMap::new(),
            new_blocks: BTreeMap::new(),
            refs: HashMap::new(),
            buffer: Vec::new(),
            buffer_at: 0,
            maps_valid,
            released: HashSet::new(),
        };
        if editor.maps_valid {
            editor.find_free_space()?;
        }
        Ok(editor)
    }

    pub(crate) fn format(&self) -> Format {
        self.header.format
    }

    /// What the last few maps mark free, if every map reads and none of it holds anything live.
    fn find_free_space(&mut self) -> io::Result<()> {
        let (from, to) = self.space.scan();
        let eof = self.header.root.eof;
        let mut free = Vec::new();
        for i in from..to {
            let at = run_at(i);
            let Ok(page) = self.pst.page(at, PT_AMAP, at) else { return Ok(()) };
            let bits = page.get(self.map_bits_at()..self.map_bits_at() + 496).ok_or_else(short)?;
            free.extend(space::free_in(i, bits, eof));
        }
        if free.is_empty() {
            return Ok(());
        }
        self.space.add_free(free);
        if !self.free_space_is_free().unwrap_or(false) {
            log::warn!("a data file's allocation map doesn't match what it holds: not used");
            self.space.distrust();
            self.maps_valid = false;
        }
        Ok(())
    }

    fn map_bits_at(&self) -> usize {
        if self.format().wide() { 0 } else { 4 }
    }

    /// Whether nothing the file holds lies in the free ranges.
    fn free_space_is_free(&self) -> io::Result<bool> {
        let free = self.space.free_ranges();
        let hits = |start: u64, end: u64| {
            let i = free.partition_point(|(_, e)| *e <= start);
            free.get(i).is_some_and(|(s, _)| *s < end)
        };
        let format = self.format();
        let trailer = format.block_trailer();
        let page_size = PAGE as u64;
        let mut clean = true;
        let root = self.header.root;
        let mut on_page = |ib: u64| clean &= !hits(ib, ib.saturating_add(page_size));
        self.walk_tree(root.nbt, PT_NBT, node_entry_size(format), &mut |_, _| Ok(()), &mut on_page)?;
        let mut blocks_clean = true;
        self.walk_tree(
            root.bbt,
            PT_BBT,
            block_entry_size(format),
            &mut |p, i| {
                let e = block_entry(p, i, format)?;
                let size = align64(usize::from(e.size) + trailer) as u64;
                blocks_clean &= !hits(e.ib, e.ib.saturating_add(size));
                Ok(())
            },
            &mut on_page,
        )?;
        Ok(clean && blocks_clean)
    }

    // --- IDs -------------------------------------------------------------------------------

    pub(crate) fn next_page_id(&mut self) -> u64 {
        let id = self.header.next_page;
        self.header.next_page = id.saturating_add(1);
        id
    }

    fn next_block_id(&mut self, internal: bool) -> u64 {
        loop {
            let id = self.header.next_block;
            self.header.next_block = id.saturating_add(4);
            if self.pst.block_entry(id).is_err() && self.pst.block_entry(id | BID_INTERNAL).is_err() {
                return if internal { id | BID_INTERNAL } else { id };
            }
        }
    }

    /// A new node ID of type `kind`.
    pub(crate) fn next_node_id(&mut self, kind: u32) -> io::Result<u32> {
        let slot = self.header.nids.get_mut(crate::util::size(u64::from(kind))?).ok_or_else(short)?;
        let mut index = (*slot).max(0x400);
        let nid = loop {
            let nid = index.checked_mul(32).ok_or_else(|| invalid("the file has run out of node IDs"))? | kind;
            index += 1;
            if !self.nodes.contains_key(&nid) && self.pst.node(nid).is_err() {
                break nid;
            }
        };
        if let Some(slot) = self.header.nids.get_mut(crate::util::size(u64::from(kind))?) {
            *slot = index;
        }
        // A folder's three tables are numbered with it.
        if kind == NID_TYPE_NORMAL_FOLDER
            && let Some(tables) = self.header.nids.get_mut(13..16)
        {
            tables.fill(index);
        }
        Ok(nid)
    }

    // --- Bytes -----------------------------------------------------------------------------

    /// Queues `bytes` to be written at `at`.
    pub(crate) fn queue(&mut self, at: u64, bytes: &[u8]) -> io::Result<()> {
        if self.buffer.is_empty() || self.buffer_at + self.buffer.len() as u64 != at || self.buffer.len() + bytes.len() > WRITE_BUFFER {
            self.flush()?;
            self.buffer_at = at;
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            self.file.seek(SeekFrom::Start(self.buffer_at))?;
            self.file.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
    }

    /// Writes a block of `data`: a data block (encoded as the file's are) or, `internal`, one
    /// that lists others. Its ID.
    pub(crate) fn write_block(&mut self, data: &[u8], internal: bool) -> io::Result<u64> {
        let format = self.format();
        let id = self.next_block_id(internal);
        let trailer = format.block_trailer();
        let total = align64(data.len() + trailer);
        if data.len() > format.block_data() {
            return Err(invalid("a block bigger than a block can be"));
        }
        let ib = self.space.alloc(total as u64, 64)?;
        let mut out = vec![0u8; total];
        let body = out.get_mut(..data.len()).ok_or_else(short)?;
        body.copy_from_slice(data);
        if !internal {
            self.header.crypt.encode(body, id);
        }
        let crc = crate::crc::crc(out.get(..data.len()).ok_or_else(short)?);
        let sig = super::btree::signature(ib, id);
        let cb = u16::try_from(data.len()).map_err(|_| short())?;
        let tail = out.get_mut(total - trailer..).ok_or_else(short)?;
        tail.get_mut(..2).ok_or_else(short)?.copy_from_slice(&cb.to_le_bytes());
        tail.get_mut(2..4).ok_or_else(short)?.copy_from_slice(&sig.to_le_bytes());
        if format.wide() {
            tail.get_mut(4..8).ok_or_else(short)?.copy_from_slice(&crc.to_le_bytes());
            tail.get_mut(8..16).ok_or_else(short)?.copy_from_slice(&id.to_le_bytes());
        } else {
            let narrow = u32::try_from(id).map_err(|_| invalid("past what an ANSI data file can hold"))?;
            tail.get_mut(4..8).ok_or_else(short)?.copy_from_slice(&narrow.to_le_bytes());
            tail.get_mut(8..12).ok_or_else(short)?.copy_from_slice(&crc.to_le_bytes());
        }
        self.queue(ib, &out)?;
        self.new_blocks.insert(id, BlockEntry { bid: id, ib, size: cb, refs: 2 });
        Ok(id)
    }

    // --- Nodes and references --------------------------------------------------------------

    /// Puts node `nid` (new, or changed).
    pub(crate) fn set_node(&mut self, nid: u32, data: u64, sub: u64, parent: u32) {
        self.nodes.insert(nid, Some(NodeEntry { nid, data, sub, parent }));
    }

    pub(crate) fn drop_node(&mut self, nid: u32) {
        self.nodes.insert(nid, None);
    }

    /// The node as this change has it.
    pub(crate) fn node(&self, nid: u32) -> io::Result<NodeEntry> {
        match self.nodes.get(&nid) {
            Some(Some(n)) => Ok(*n),
            Some(None) => Err(io::Error::new(io::ErrorKind::NotFound, format!("no node {nid:#x}"))),
            None => self.pst.node(nid),
        }
    }

    /// One more reference to block `bid`.
    pub(crate) fn add_ref(&mut self, bid: u64) {
        let key = bid & !1;
        if let Some(e) = self.new_blocks.get_mut(&key) {
            e.refs = e.refs.saturating_add(1);
        } else {
            *self.refs.entry(key).or_default() += 1;
        }
    }

    /// One less reference to block `bid`; whether nothing refers to it any more.
    pub(crate) fn drop_ref(&mut self, bid: u64) -> io::Result<bool> {
        let key = bid & !1;
        if let Some(e) = self.new_blocks.get_mut(&key) {
            e.refs = e.refs.saturating_sub(1);
            return Ok(e.refs <= 1);
        }
        let entry = self.pst.block_entry(key)?;
        let delta = self.refs.entry(key).or_default();
        *delta -= 1;
        Ok(i32::from(entry.refs) + *delta <= 1)
    }

    /// Whether block `bid` is let go of for the first time in this change.
    pub(crate) fn first_release(&mut self, bid: u64) -> bool {
        self.released.insert(bid & !1)
    }

    // --- Commit ----------------------------------------------------------------------------

    /// Makes the change the file: new blocks and pages are on disk first, the maps next, and
    /// the header, which alone says where everything is, last. A crash before that leaves the
    /// file as it was.
    pub(crate) fn commit(mut self) -> io::Result<()> {
        let format = self.format();
        // Blocks that lost their last reference.
        let trailer = format.block_trailer();
        let mut block_changes = Changes::new();
        for (key, delta) in std::mem::take(&mut self.refs) {
            if delta == 0 {
                continue;
            }
            let entry = self.pst.block_entry(key)?;
            let left = i32::from(entry.refs) + delta;
            if left <= 1 {
                self.space.release(entry.ib, align64(usize::from(entry.size) + trailer) as u64);
                block_changes.insert(entry.bid, None);
            } else {
                let refs = u16::try_from(left).map_err(|_| short())?;
                block_changes.insert(entry.bid, Some(block_raw(format, BlockEntry { refs, ..entry })?));
            }
        }
        for entry in std::mem::take(&mut self.new_blocks).into_values() {
            block_changes.insert(entry.bid, Some(block_raw(format, entry)?));
        }
        let mut node_changes = Changes::new();
        for (nid, entry) in std::mem::take(&mut self.nodes) {
            node_changes.insert(u64::from(nid), entry.map(|e| node_raw(format, e)).transpose()?);
        }
        let root = self.header.root;
        let bbt = self.update_tree(root.bbt, PT_BBT, block_entry_size(format), &block_changes)?;
        let nbt = self.update_tree(root.nbt, PT_NBT, node_entry_size(format), &node_changes)?;
        // The maps, worked out before anything in the file changes: an error here changes nothing.
        let (pages, free_delta, runs) = self.map_pages()?;
        self.flush()?;
        self.file.sync_data()?;

        // The maps will disagree with the header until it is written: said first.
        let mut interim = self.header.clone();
        interim.root = root;
        interim.root.amap_valid = AMAP_INVALID;
        self.write_header(&interim)?;
        self.file.sync_data()?;
        for (at, page) in pages {
            self.queue(at, &page)?;
        }
        self.flush()?;
        let eof = run_at(runs);
        if eof > self.file.metadata()?.len() {
            self.file.set_len(eof)?;
        }
        self.file.sync_data()?;

        self.touch_density_list()?;
        let h = &mut self.header;
        h.root.nbt = nbt;
        h.root.bbt = bbt;
        h.root.eof = eof.max(h.root.eof);
        h.root.amap_last = run_at(runs.saturating_sub(1));
        h.root.amap_free = apply_delta(h.root.amap_free, free_delta);
        h.root.amap_valid = if self.maps_valid { AMAP_VALID } else { AMAP_INVALID };
        h.unique = h.unique.wrapping_add(1);
        let final_header = self.header.clone();
        self.write_header(&final_header)?;
        self.file.sync_data()
    }

    fn write_header(&mut self, header: &Header) -> io::Result<()> {
        let bytes = header.to_bytes()?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&bytes)
    }

    /// The map pages as the change leaves them, how much more is free, and how many runs.
    fn map_pages(&mut self) -> io::Result<(Vec<(u64, Vec<u8>)>, i64, u64)> {
        let format = self.format();
        let old_runs = self.space.runs();
        let runs = self.space.final_runs();
        let mut delta = 0i64;
        let bits_at = self.map_bits_at();
        let mut amaps: BTreeMap<u64, [u8; 496]> = BTreeMap::new();
        let mut out = Vec::new();
        for i in old_runs..runs {
            let bits = space::new_bits(i);
            let taken: i64 = bits.iter().map(|b| i64::from(b.count_ones())).sum();
            delta += AMAP_BYTES as i64 - taken * 64;
            amaps.insert(i, bits);
            let base = run_at(i);
            let mut at = base;
            for kind in space::map_pages(i) {
                let filler = match kind {
                    MapKind::Amap => {
                        at += PAGE as u64;
                        continue;
                    }
                    MapKind::Pmap | MapKind::Fpmap => 0xFFu8,
                    MapKind::Fmap => 0u8,
                };
                let ptype = match kind {
                    MapKind::Pmap => PT_PMAP,
                    MapKind::Fmap => PT_FMAP,
                    _ => PT_FPMAP,
                };
                let mut page = vec![0u8; PAGE];
                page.get_mut(bits_at..bits_at + 496).ok_or_else(short)?.fill(filler);
                finish_page(&mut page, format, ptype, at, at, false)?;
                out.push((at, page));
                at += PAGE as u64;
            }
        }
        let taken = self.space.taken.clone();
        let released = self.space.released.clone();
        for (ranges, to) in [(released, false), (taken, true)] {
            for (start, end) in ranges {
                let i = run_of(start).ok_or_else(|| invalid("a range before the first allocation map"))?;
                if i >= old_runs || self.maps_valid {
                    if let std::collections::btree_map::Entry::Vacant(e) = amaps.entry(i) {
                        let at = run_at(i);
                        let page = self.pst.page(at, PT_AMAP, at)?;
                        let mut bits = [0u8; 496];
                        bits.copy_from_slice(page.get(bits_at..bits_at + 496).ok_or_else(short)?);
                        e.insert(bits);
                    }
                    if let Some(bits) = amaps.get_mut(&i) {
                        delta += space::mark(bits, i, start, end, to)?;
                    }
                }
            }
        }
        for (i, bits) in amaps {
            let at = run_at(i);
            let mut page = vec![0u8; PAGE];
            page.get_mut(bits_at..bits_at + 496).ok_or_else(short)?.copy_from_slice(&bits);
            finish_page(&mut page, format, PT_AMAP, at, at, false)?;
            out.push((at, page));
        }
        Ok((out, delta, runs))
    }

    /// A density list carries the header's next page ID in its trailer: kept in step.
    fn touch_density_list(&mut self) -> io::Result<()> {
        let format = self.format();
        // One that doesn't read, or is damaged, is left as it is: not made to look sound.
        let Ok(found) = self.pst.page(DLIST_AT, PT_DLIST, 0) else { return Ok(()) };
        let mut page = found.to_vec();
        finish_page(&mut page, format, PT_DLIST, DLIST_AT, self.header.next_page, true)?;
        self.queue(DLIST_AT, &page)?;
        self.flush()
    }
}

fn apply_delta(v: u64, delta: i64) -> u64 {
    if delta >= 0 { v.saturating_add(delta.unsigned_abs()) } else { v.saturating_sub(delta.unsigned_abs()) }
}

fn put_id(out: &mut Vec<u8>, format: Format, id: u64) -> io::Result<()> {
    if format.wide() {
        out.extend(id.to_le_bytes());
    } else {
        out.extend(u32::try_from(id).map_err(|_| invalid("past what an ANSI data file can hold"))?.to_le_bytes());
    }
    Ok(())
}

/// A block entry as a leaf page keeps it.
pub(crate) fn block_raw(format: Format, e: BlockEntry) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(24);
    put_id(&mut out, format, e.bid)?;
    put_id(&mut out, format, e.ib)?;
    out.extend(e.size.to_le_bytes());
    out.extend(e.refs.to_le_bytes());
    if format.wide() {
        out.extend([0u8; 4]);
    }
    Ok(out)
}

/// A node entry as a leaf page keeps it.
pub(crate) fn node_raw(format: Format, e: NodeEntry) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(32);
    put_id(&mut out, format, u64::from(e.nid))?;
    put_id(&mut out, format, e.data)?;
    put_id(&mut out, format, e.sub)?;
    out.extend(e.parent.to_le_bytes());
    if format.wide() {
        out.extend([0u8; 4]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
