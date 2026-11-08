//! Changing the node and block B-trees: only the pages that change are written, each to new
//! space under a new ID, so the tree the header names stays whole until the header changes.

use std::collections::BTreeMap;
use std::io;
use std::ops::Bound;

use super::edit::Editor;
use crate::ndb::btree::BtPage;
use crate::ndb::{Bref, Format, PAGE};
use crate::util::invalid;

/// A leaf entry as it is kept in a page.
pub(crate) type Raw = Vec<u8>;
/// What to do to each key: put this entry, or take it out.
pub(crate) type Changes = BTreeMap<u64, Option<Raw>>;

/// A page's entries in a page's room: the most a page of `entry` bytes each holds.
fn page_room(format: Format, entry: usize) -> usize {
    (if format.wide() { 488 } else { 496 }) / entry
}

/// A page's bytes.
pub(crate) fn page_bytes(format: Format, ptype: u8, ib: u64, bid: u64, entry: usize, level: u8, entries: &[Raw]) -> io::Result<Vec<u8>> {
    let top = if format.wide() { 488 } else { 496 };
    let room = page_room(format, entry);
    if entries.len() > room {
        return Err(invalid("more entries than a page holds"));
    }
    let mut b = vec![0u8; PAGE];
    let body = b.get_mut(..top).ok_or_else(crate::util::short)?;
    for (slot, e) in body.chunks_exact_mut(entry).zip(entries) {
        slot.get_mut(..e.len()).ok_or_else(crate::util::short)?.copy_from_slice(e);
    }
    let foot = b.get_mut(top..top + 4).ok_or_else(crate::util::short)?;
    #[allow(clippy::cast_possible_truncation)] // both fit a byte: checked above
    foot.copy_from_slice(&[entries.len() as u8, room as u8, entry as u8, level]);
    finish_page(&mut b, format, ptype, ib, bid, true)?;
    Ok(b)
}

/// A page's signature (`ComputeSig`, MS-PST 5.5).
pub(crate) fn signature(ib: u64, bid: u64) -> u16 {
    let x = ib ^ bid;
    #[allow(clippy::cast_possible_truncation)] // the low 16 bits are the signature
    let sig = ((x >> 16) ^ x) as u16;
    sig
}

/// Writes a page's trailer: its type, signature (for B-tree pages), CRC and ID.
pub(crate) fn finish_page(b: &mut [u8], format: Format, ptype: u8, ib: u64, bid: u64, signed: bool) -> io::Result<()> {
    let data = format.page_data();
    let crc = crate::crc::crc(b.get(..data).ok_or_else(crate::util::short)?);
    let sig = if signed { signature(ib, bid) } else { 0 };
    let t = b.get_mut(data..PAGE).ok_or_else(crate::util::short)?;
    let mut tail = vec![ptype, ptype];
    tail.extend(sig.to_le_bytes());
    if format.wide() {
        tail.extend(crc.to_le_bytes());
        tail.extend(bid.to_le_bytes());
    } else {
        let narrow = u32::try_from(bid).map_err(|_| invalid("past what an ANSI data file can hold"))?;
        tail.extend(narrow.to_le_bytes());
        tail.extend(crc.to_le_bytes());
    }
    t.get_mut(..tail.len()).ok_or_else(crate::util::short)?.copy_from_slice(&tail);
    Ok(())
}

fn id_bytes(format: Format, id: u64) -> io::Result<Vec<u8>> {
    if format.wide() {
        Ok(id.to_le_bytes().to_vec())
    } else {
        Ok(u32::try_from(id).map_err(|_| invalid("past what an ANSI data file can hold"))?.to_le_bytes().to_vec())
    }
}

/// The keys a page may hold: from the first, up to but not including the second.
type KeyRange = (Option<u64>, Option<u64>);

/// That `keys` only go up: a page that doesn't can't be rewritten without losing what it holds.
fn ascending(mut keys: impl Iterator<Item = u64>) -> io::Result<()> {
    let Some(mut last) = keys.next() else { return Ok(()) };
    for key in keys {
        if key <= last {
            return Err(invalid("a B-tree page's keys are out of order"));
        }
        last = key;
    }
    Ok(())
}

/// A `BTENTRY`: the first key under a page, and the page.
fn index_entry(format: Format, key: u64, at: Bref) -> io::Result<Raw> {
    Ok([id_bytes(format, key)?, id_bytes(format, at.bid)?, id_bytes(format, at.ib)?].concat())
}

impl Editor {
    /// Writes a page of these entries to new space under a new ID.
    fn new_page(&mut self, ptype: u8, entry: usize, level: u8, entries: &[Raw]) -> io::Result<Bref> {
        let format = self.format();
        let ib = self.space.alloc(PAGE as u64, PAGE as u64)?;
        let bid = self.next_page_id();
        let bytes = page_bytes(format, ptype, ib, bid, entry, level, entries)?;
        self.queue(ib, &bytes)?;
        Ok(Bref { bid, ib })
    }

    /// Writes `entries` as however many pages they take, and says what's now above them.
    fn pages_of(&mut self, ptype: u8, entry: usize, level: u8, entries: &[(u64, Raw)]) -> io::Result<Vec<(u64, Bref)>> {
        let room = page_room(self.format(), entry).max(1);
        let mut out = Vec::with_capacity(entries.len() / room + 1);
        for chunk in entries.chunks(room) {
            let raws: Vec<Raw> = chunk.iter().map(|(_, r)| r.clone()).collect();
            let at = self.new_page(ptype, entry, level, &raws)?;
            out.push((chunk.first().map_or(0, |(k, _)| *k), at));
        }
        Ok(out)
    }

    /// The page at `at`, with `changes` applied: what should stand in its place in its parent.
    fn update_page(
        &mut self,
        at: Bref,
        ptype: u8,
        leaf: usize,
        above: Option<u8>,
        range: KeyRange,
        changes: &Changes,
    ) -> io::Result<Vec<(u64, Bref)>> {
        let format = self.format();
        let p = BtPage::parse(self.pst.page(at.ib, ptype, at.bid)?, format, leaf)?;
        if above.is_some_and(|l| p.level + 1 != l) || p.level > 7 {
            return Err(invalid("a B-tree doesn't go down level by level"));
        }
        let keys = (0..p.count).map(|i| p.key(i, format)).collect::<io::Result<Vec<_>>>()?;
        ascending(keys.iter().copied())?;
        if keys.iter().any(|k| range.0.is_some_and(|low| *k < low) || range.1.is_some_and(|high| *k >= high)) {
            return Err(invalid("a B-tree page holds keys its parent doesn't send to it"));
        }
        // Entries go back at the size this writes, whatever `cbEnt` the page had.
        let written = if p.level == 0 { leaf } else { BtPage::index_entry(format) };
        let entry_of = |i: usize| -> io::Result<Raw> {
            let start = i * p.entry;
            Ok(p.page.get(start..start + written).ok_or_else(crate::util::short)?.to_vec())
        };
        if p.level == 0 {
            let mut merged: BTreeMap<u64, Raw> = BTreeMap::new();
            for i in 0..p.count {
                merged.insert(p.key(i, format)?, entry_of(i)?);
            }
            for (k, v) in changes {
                match v {
                    Some(raw) => {
                        merged.insert(*k, raw.clone());
                    }
                    None => {
                        merged.remove(k);
                    }
                }
            }
            let list: Vec<(u64, Raw)> = merged.into_iter().collect();
            self.space.release(at.ib, PAGE as u64);
            return self.pages_of(ptype, leaf, 0, &list);
        }
        // Which keys go under which child: the first takes those before its key too.
        let mut list: Vec<(u64, Raw)> = Vec::with_capacity(p.count);
        for i in 0..p.count {
            let key = p.key(i, format)?;
            let low = if i == 0 { Bound::Unbounded } else { Bound::Included(key) };
            let high = if i + 1 < p.count { Bound::Excluded(p.key(i + 1, format)?) } else { Bound::Unbounded };
            let part: Changes = changes.range((low, high)).map(|(k, v)| (*k, v.clone())).collect();
            if part.is_empty() {
                list.push((key, entry_of(i)?));
                continue;
            }
            let child = p.child(i, format)?;
            let within = (if i == 0 { range.0 } else { Some(key) }, if i + 1 < p.count { Some(p.key(i + 1, format)?) } else { range.1 });
            for (k, new) in self.update_page(child, ptype, leaf, Some(p.level), within, &part)? {
                list.push((k, index_entry(format, k, new)?));
            }
        }
        ascending(list.iter().map(|(k, _)| *k))?;
        self.space.release(at.ib, PAGE as u64);
        self.pages_of(ptype, BtPage::index_entry(format), p.level, &list)
    }

    /// The tree at `root`, with `changes` applied: its new root.
    pub(crate) fn update_tree(&mut self, root: Bref, ptype: u8, leaf: usize, changes: &Changes) -> io::Result<Bref> {
        if changes.is_empty() {
            return Ok(root);
        }
        let format = self.format();
        let root_level = BtPage::parse(self.pst.page(root.ib, ptype, root.bid)?, format, leaf)?.level;
        let mut level = root_level;
        let mut pages = self.update_page(root, ptype, leaf, None, (None, None), changes)?;
        while pages.len() > 1 {
            level += 1;
            if level > 7 {
                return Err(invalid("a B-tree of more levels than any has"));
            }
            let entries = pages.iter().map(|(k, at)| Ok((*k, index_entry(format, *k, *at)?))).collect::<io::Result<Vec<_>>>()?;
            pages = self.pages_of(ptype, BtPage::index_entry(format), level, &entries)?;
        }
        match pages.first() {
            Some((_, at)) => Ok(*at),
            // Everything was taken out: an empty leaf is the tree.
            None => self.new_page(ptype, leaf, 0, &[]),
        }
    }

    /// Calls `leaf` for every leaf entry of the tree at `root` and `page` for every page.
    pub(crate) fn walk_tree(
        &self,
        root: Bref,
        ptype: u8,
        leaf_size: usize,
        leaf: &mut dyn FnMut(&BtPage, usize) -> io::Result<()>,
        page: &mut dyn FnMut(u64),
    ) -> io::Result<()> {
        let format = self.format();
        let mut seen = std::collections::HashSet::new();
        let mut todo = vec![(root, None::<u8>)];
        while let Some((at, above)) = todo.pop() {
            if !seen.insert(at.ib) {
                return Err(invalid("a B-tree names a page twice"));
            }
            let p = BtPage::parse(self.pst.page(at.ib, ptype, at.bid)?, format, leaf_size)?;
            if above.is_some_and(|l| p.level + 1 != l) || p.level > 7 {
                return Err(invalid("a B-tree doesn't go down level by level"));
            }
            page(at.ib);
            if p.level == 0 {
                for i in 0..p.count {
                    leaf(&p, i)?;
                }
            } else {
                for i in 0..p.count {
                    todo.push((p.child(i, format)?, Some(p.level)));
                }
            }
        }
        Ok(())
    }
}
