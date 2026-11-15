//! Building tables (Table Contexts, MS-PST 2.3.4), new and from one already in the file.

use std::collections::{BTreeMap, HashSet};
use std::io;

use super::FIRST_SUBNODE;
use super::edit::Editor;
use super::heap::{HeapBuilder, MAX_ALLOC, build_tree};
use crate::ltp::{Column, PropertyType, TC_SIG, Table};
use crate::ndb::{Format, NID_TYPE_INTERNAL, NodeEntry, SubNodes};
use crate::util::{invalid, short};

pub(crate) const ROW_ID: u16 = 0x67F2;
pub(crate) const ROW_VERSION: u16 = 0x67F3;

/// A table's layout.
#[derive(Clone)]
pub(crate) struct Schema {
    pub(crate) columns: Vec<Column>,
    /// `rgib`: where the 4-, 2- and 1-byte columns end, then the existence bitmap.
    pub(crate) ends: [u16; 4],
}

/// What a cell holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cell {
    /// A value kept in the row, of the column's size.
    Fixed(Vec<u8>),
    /// A value kept elsewhere: bytes for the heap or a subnode.
    Bytes(Vec<u8>),
    /// A value kept in this subnode already (of a table being rebuilt).
    Node(u32),
}

/// A row being written.
#[derive(Clone)]
pub(crate) struct NewRow {
    pub(crate) id: u32,
    /// One for each column.
    pub(crate) cells: Vec<Option<Cell>>,
}

impl Schema {
    /// A layout for these columns (`(id, type)`): the row ID and version first, then 8- and
    /// 4-byte columns, 2-byte, 1-byte, and the bitmap saying which cells exist.
    pub(crate) fn new(list: &[(u16, u16)]) -> io::Result<Schema> {
        let mut columns = vec![Column::new(PropertyType::INT32, ROW_ID, 0, 4, 0), Column::new(PropertyType::INT32, ROW_VERSION, 4, 4, 1)];
        let mut offset = 8u16;
        let mut bit = 2u8;
        let mut ends = [0u16; 4];
        for (i, group) in [&[8u8, 4][..], &[2][..], &[1][..]].into_iter().enumerate() {
            for (id, kind) in list {
                // A value kept elsewhere: the row holds where, in four bytes.
                let size = PropertyType(*kind).row_width().unwrap_or(4);
                if group.contains(&size) {
                    columns.push(Column::new(*kind, *id, offset, size, bit));
                    offset += u16::from(size);
                    bit = bit.checked_add(1).ok_or_else(|| invalid("a table of too many columns"))?;
                }
            }
            if let Some(end) = ends.get_mut(i) {
                *end = offset;
            }
        }
        ends[3] = offset + u16::from(bit).div_ceil(8);
        columns.sort_by_key(|c| (u32::from(c.prop_id()) << 16) | u32::from(c.prop_type()));
        Ok(Schema { columns, ends })
    }

    pub(crate) fn width(&self) -> usize {
        usize::from(self.ends[3])
    }

    /// The position of column `id`, when the table has it.
    pub(crate) fn index_of(&self, id: u16) -> Option<usize> {
        self.columns.iter().position(|c| c.prop_id() == id)
    }
}

/// A table's bytes: its heap, its rows when they are in a subnode, and its big values.
pub(crate) struct Built {
    pub(crate) blocks: Vec<Vec<u8>>,
    pub(crate) rows: Option<(u32, Vec<Vec<u8>>)>,
    pub(crate) external: Vec<(u32, Vec<u8>)>,
}

/// `rows` as a table of `schema`. The bytes of their cells move into it.
pub(crate) fn build_table(format: Format, schema: &Schema, rows: Vec<NewRow>, keep_above: u32) -> io::Result<Built> {
    let width = schema.width();
    let mut heap = HeapBuilder::new(format);
    let mut external = Vec::new();
    let mut next = keep_above.max(FIRST_SUBNODE);
    let bitmap = usize::from(schema.ends[2]);
    let narrow = !format.wide();
    if narrow && rows.len() > 1 << 16 {
        return Err(invalid("an ANSI data file's folder holds 65,536 items at most"));
    }
    let mut encoded = Vec::with_capacity(rows.len());
    // The row index: each row's ID and its place (MS-PST 2.3.4.3).
    let mut index: Vec<(u32, u32)> = Vec::with_capacity(rows.len());
    for (i, row) in rows.into_iter().enumerate() {
        let mut b = vec![0u8; width];
        for (column, cell) in schema.columns.iter().zip(row.cells) {
            let Some(cell) = cell else { continue };
            let at = usize::from(column.offset);
            let size = usize::from(column.size);
            let slot = b.get_mut(at..at + size).ok_or_else(short)?;
            match cell {
                Cell::Fixed(v) => slot.get_mut(..v.len().min(size)).ok_or_else(short)?.copy_from_slice(v.get(..v.len().min(size)).ok_or_else(short)?),
                Cell::Bytes(v) => {
                    let hnid = if v.is_empty() {
                        0
                    } else if v.len() > MAX_ALLOC {
                        let nid = (next << 5) | NID_TYPE_INTERNAL;
                        next += 1;
                        external.push((nid, v));
                        nid
                    } else {
                        heap.alloc(v)?
                    };
                    slot.get_mut(..4).ok_or_else(short)?.copy_from_slice(&hnid.to_le_bytes());
                }
                Cell::Node(nid) => slot.get_mut(..4).ok_or_else(short)?.copy_from_slice(&nid.to_le_bytes()),
            }
            let bit = usize::from(column.bit);
            *b.get_mut(bitmap + bit / 8).ok_or_else(short)? |= 0x80 >> (bit % 8);
        }
        b.get_mut(..4).ok_or_else(short)?.copy_from_slice(&row.id.to_le_bytes());
        encoded.push(b);
        index.push((row.id, u32::try_from(i).map_err(|_| short())?));
    }
    index.sort_unstable();
    let records: Vec<Vec<u8>> = index
        .iter()
        .map(|(id, at)| {
            let mut r = id.to_le_bytes().to_vec();
            if narrow {
                #[allow(clippy::cast_possible_truncation)] // checked above
                r.extend((*at as u16).to_le_bytes());
            } else {
                r.extend(at.to_le_bytes());
            }
            r
        })
        .collect();
    let row_index = build_tree(&mut heap, 4, if narrow { 2 } else { 4 }, &records)?;
    // The rows: in the heap when they fit an allocation, else in a subnode a block at a time.
    let total = encoded.len().checked_mul(width).ok_or_else(short)?;
    let (hnid_rows, rows_out) = if encoded.is_empty() {
        (0, None)
    } else if total <= MAX_ALLOC {
        (heap.alloc(encoded.concat())?, None)
    } else {
        let per_block = (format.block_data() / width.max(1)).max(1);
        let blocks: Vec<Vec<u8>> = encoded
            .chunks(per_block)
            .enumerate()
            .map(|(i, chunk)| {
                let mut b = chunk.concat();
                // Every block but the last is full, so a row's place is found by arithmetic.
                if (i + 1) * per_block < encoded.len() {
                    b.resize(format.block_data(), 0);
                }
                b
            })
            .collect();
        let nid = (next << 5) | NID_TYPE_INTERNAL;
        (nid, Some((nid, blocks)))
    };
    let mut head = vec![TC_SIG, u8::try_from(schema.columns.len()).map_err(|_| short())?];
    for e in schema.ends {
        head.extend(e.to_le_bytes());
    }
    head.extend(row_index.to_le_bytes());
    head.extend(hnid_rows.to_le_bytes());
    head.extend(0u32.to_le_bytes());
    for c in &schema.columns {
        head.extend(((u32::from(c.prop_id()) << 16) | u32::from(c.prop_type())).to_le_bytes());
        head.extend(c.offset.to_le_bytes());
        head.extend([c.size, c.bit]);
    }
    let root = heap.alloc(head)?;
    Ok(Built { blocks: heap.finish(TC_SIG, root)?, rows: rows_out, external })
}

impl Editor {
    /// Writes a built table: its data tree and subnode tree, with `subs` already in that tree.
    pub(crate) fn write_built(&mut self, built: Built, mut subs: SubNodes) -> io::Result<(u64, u64)> {
        let data = self.write_leaves(&built.blocks)?;
        if let Some((nid, blocks)) = built.rows {
            let bid = self.write_leaves(&blocks)?;
            subs.insert(nid, (bid, 0));
        }
        for (nid, bytes) in built.external {
            let bid = self.write_tree(&bytes)?;
            subs.insert(nid, (bid, 0));
        }
        let sub = self.write_subnodes(&subs)?;
        Ok((data, sub))
    }

    /// The table at node `nid`, with `edit` applied to its rows: what's in the file is let go
    /// of and the node written again. `edit` can read each row as cells of the layout.
    pub(crate) fn rewrite_table(&mut self, nid: u32, edit: impl FnOnce(&Schema, &mut Vec<NewRow>) -> io::Result<()>) -> io::Result<usize> {
        let (entry, table) = self.read_table(nid)?;
        self.rewrite_read_table(nid, entry, &table, edit)
    }

    /// The table at node `nid`, as this change has it, and its node.
    pub(crate) fn read_table(&self, nid: u32) -> io::Result<(NodeEntry, Table)> {
        let entry = self.node(nid)?;
        let table = Table::read(self.pst.clone(), entry.data, entry.sub)?;
        Ok((entry, table))
    }

    /// [`Self::rewrite_table`], of `table` read from node `nid` (`entry`) already.
    pub(crate) fn rewrite_read_table(
        &mut self,
        nid: u32,
        entry: NodeEntry,
        table: &Table,
        edit: impl FnOnce(&Schema, &mut Vec<NewRow>) -> io::Result<()>,
    ) -> io::Result<usize> {
        let schema = Schema { columns: table.columns().to_vec(), ends: table.ends() };
        let mut rows = table.new_rows()?;
        edit(&schema, &mut rows)?;
        // Cells kept in subnodes stay where they are.
        let kept: HashSet<u32> =
            rows.iter().flat_map(|r| r.cells.iter().flatten()).filter_map(|c| if let Cell::Node(n) = c { Some(*n) } else { None }).collect();
        let mut subs = BTreeMap::new();
        let mut top = FIRST_SUBNODE;
        for (nid, (data, sub)) in table.subnodes().iter().filter(|(n, _)| kept.contains(*n)) {
            self.add_ref(*data);
            if *sub != 0 {
                self.add_ref(*sub);
            }
            subs.insert(*nid, (*data, *sub));
            top = top.max((nid >> 5).saturating_add(1));
        }
        let count = rows.len();
        let built = build_table(self.format(), &schema, rows, top)?;
        let (data, sub) = self.write_built(built, subs)?;
        self.release_node_data(entry.data, entry.sub)?;
        self.set_node(nid, data, sub, entry.parent);
        Ok(count)
    }
}
