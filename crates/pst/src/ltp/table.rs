//! A node as a table of rows (a Table Context, MS-PST 2.3.4).

use std::io;
use std::rc::Rc;

use super::Heap;
use super::props::stored;
use super::value::{self, PropertyType, PropertyValue};
use crate::ndb::{NID_TYPE_MASK, PstFile, SubNodes};
use crate::util::{invalid, short, u8_at, u16_at, u32_at};

const TC_SIG: u8 = 0x7C;
/// Rows read at most, whatever the blocks hold.
const MAX_ROWS: usize = 1 << 24;
/// Cells of a table written again at most, and the bytes of text they may hold: past what the
/// biggest folder has, and short of what repeated rows would make of a few kilobytes.
#[cfg(not(target_arch = "wasm32"))]
const MAX_CELLS: usize = 1 << 25;
#[cfg(not(target_arch = "wasm32"))]
const MAX_CELL_BYTES: usize = 1 << 31;

/// A table's column (`TCOLDESC`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    prop_type: u16,
    prop_id: u16,
    pub(crate) offset: u16,
    pub(crate) size: u8,
    pub(crate) bit: u8,
}

impl Column {
    pub fn prop_id(&self) -> u16 {
        self.prop_id
    }

    pub fn prop_type(&self) -> u16 {
        self.prop_type
    }

    pub(crate) fn new(prop_type: u16, prop_id: u16, offset: u16, size: u8, bit: u8) -> Column {
        Column { prop_type, prop_id, offset, size, bit }
    }
}

/// A row: its ID and where it is in the table's blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    id: u32,
    block: u32,
    at: u32,
}

impl Row {
    pub fn id(&self) -> u32 {
        self.id
    }
}

/// What's in `TCINFO`, the head of a table.
pub(crate) struct TableInfo {
    /// `rgib`: where the 4-, 2- and 1-byte columns end and where the rows end.
    pub(crate) ends: [u16; 4],
    pub(crate) columns: Vec<Column>,
    pub(crate) hnid_rows: u32,
}

pub(crate) fn info(heap: &Heap) -> io::Result<TableInfo> {
    if heap.client != TC_SIG {
        return Err(invalid("a node that isn't a table"));
    }
    let b = heap.alloc(heap.root)?;
    if u8_at(b, 0)? != TC_SIG {
        return Err(invalid("a table's head is damaged"));
    }
    let count = usize::from(u8_at(b, 1)?);
    let ends = [u16_at(b, 2)?, u16_at(b, 4)?, u16_at(b, 6)?, u16_at(b, 8)?];
    let hnid_rows = u32_at(b, 14)?;
    let mut columns = Vec::with_capacity(count);
    for i in 0..count {
        let at = 22 + i * 8;
        let tag = u32_at(b, at)?;
        #[allow(clippy::cast_possible_truncation)] // the two halves of the tag
        columns.push(Column {
            prop_type: tag as u16,
            prop_id: (tag >> 16) as u16,
            offset: u16_at(b, at + 4)?,
            size: u8_at(b, at + 6)?,
            bit: u8_at(b, at + 7)?,
        });
    }
    Ok(TableInfo { ends, columns, hnid_rows })
}

/// A table: its columns and its rows.
pub struct Table {
    pst: Rc<PstFile>,
    heap: Heap,
    subs: SubNodes,
    columns: Vec<Column>,
    /// Where the existence bitmap starts, and its end, which is the row's width.
    bitmap: usize,
    width: usize,
    ends: [u16; 4],
    blocks: Vec<Vec<u8>>,
    rows: Vec<Row>,
    /// The bytes its values may still read from subnodes, over all of its rows.
    budget: std::cell::Cell<u64>,
}

impl Table {
    /// The table in the data tree at `data`, its subnodes those in the tree at `sub`.
    pub(crate) fn read(pst: Rc<PstFile>, data: u64, sub: u64) -> io::Result<Table> {
        let heap = Heap::read(&pst, data)?;
        let head = info(&heap)?;
        let subs = pst.subnodes(sub)?;
        let bitmap = usize::from(head.ends[2]);
        let width = usize::from(head.ends[3]);
        let blocks = if head.hnid_rows == 0 {
            Vec::new()
        } else if head.hnid_rows & NID_TYPE_MASK == 0 {
            vec![heap.alloc(head.hnid_rows)?.to_vec()]
        } else {
            let (node, _) = subs.get(&head.hnid_rows).ok_or_else(|| invalid("a table's rows aren't there"))?;
            pst.read_blocks(*node)?
        };
        let rows = rows_of(&blocks, width)?;
        let budget = super::props::budget(&pst);
        Ok(Table { pst, heap, subs, columns: head.columns, bitmap, width, ends: head.ends, blocks, rows, budget })
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn rows_matrix(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter()
    }

    /// The row's bytes.
    fn bytes(&self, row: &Row) -> Option<&[u8]> {
        let at = usize::try_from(row.at).ok()?;
        self.blocks.get(usize::try_from(row.block).ok()?)?.get(at..at.checked_add(self.width)?)
    }

    /// The row's value in column `index`: `None` when the row has none there, and when it
    /// can't be read.
    pub fn value(&self, row: &Row, index: usize) -> Option<PropertyValue> {
        let column = self.columns.get(index)?;
        match self.read_value(row, column) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("a table's value {:#06x} could not be read: {e}", column.prop_id);
                None
            }
        }
    }

    fn read_value(&self, row: &Row, column: &Column) -> io::Result<Option<PropertyValue>> {
        let bytes = self.bytes(row).ok_or_else(short)?;
        let bit = usize::from(column.bit);
        let flag = u8_at(bytes, self.bitmap + bit / 8)?;
        if flag & (0x80 >> (bit % 8)) == 0 {
            return Ok(None);
        }
        let start = usize::from(column.offset);
        let cell = bytes.get(start..start + usize::from(column.size)).ok_or_else(short)?;
        let kind = column.prop_type;
        // A cell too small for the value holds where it is kept (MS-PST 2.3.4.4.1).
        let held = matches!(
            kind,
            PropertyType::INT16
                | PropertyType::INT32
                | PropertyType::FLOAT32
                | PropertyType::ERROR
                | PropertyType::BOOLEAN
                | PropertyType::FLOAT64
                | PropertyType::CURRENCY
                | PropertyType::APP_TIME
                | PropertyType::INT64
                | PropertyType::TIME
        );
        if held {
            return value::parse(kind, cell).map(Some);
        }
        // Marked as there, but with nothing kept: no value.
        let hnid = u32_at(cell, 0)?;
        if hnid == 0 {
            return Ok(None);
        }
        stored(&self.pst, &self.heap, &self.subs, kind, hnid, &self.budget).map(Some)
    }

    /// `rgib`: where the rows' 4-, 2- and 1-byte columns end, then the bitmap.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn ends(&self) -> [u16; 4] {
        [self.ends[0], self.ends[1], u16::try_from(self.bitmap).unwrap_or(0), u16::try_from(self.width).unwrap_or(0)]
    }

    /// That each column lies in the row, before the bitmap, and on no other: what the table says
    /// of them is copied into every row it is written back with.
    #[cfg(not(target_arch = "wasm32"))]
    fn check_columns(&self) -> io::Result<()> {
        let mut spans: Vec<(usize, usize)> =
            self.columns.iter().map(|c| (usize::from(c.offset), usize::from(c.offset) + usize::from(c.size))).collect();
        spans.sort_unstable();
        let overlap = spans.windows(2).any(|w| matches!(w, [(_, end), (start, _)] if start < end));
        if overlap || spans.last().is_some_and(|(_, end)| *end > self.bitmap) {
            return Err(invalid("a table's columns overlap, or lie past its rows"));
        }
        Ok(())
    }

    /// Its rows, each as the cells it has, to be written again.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new_rows(&self) -> io::Result<Vec<crate::write_table::NewRow>> {
        use crate::write_table::{Cell, NewRow, kept_elsewhere};
        self.check_columns()?;
        if self.rows.len().saturating_mul(self.columns.len()) > MAX_CELLS {
            return Err(invalid("a table of more cells than can be written again"));
        }
        let mut held = 0usize;
        let mut out = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            let bytes = self.bytes(row).ok_or_else(short)?;
            let mut cells = Vec::with_capacity(self.columns.len());
            for c in &self.columns {
                let bit = usize::from(c.bit);
                if u8_at(bytes, self.bitmap + bit / 8)? & (0x80 >> (bit % 8)) == 0 {
                    cells.push(None);
                    continue;
                }
                let start = usize::from(c.offset);
                let raw = bytes.get(start..start + usize::from(c.size)).ok_or_else(short)?;
                cells.push(Some(if kept_elsewhere(c.prop_type) {
                    let hnid = u32_at(raw, 0)?;
                    if hnid == 0 {
                        Cell::Bytes(Vec::new())
                    } else if hnid & NID_TYPE_MASK == 0 {
                        let value = self.heap.alloc(hnid)?;
                        held = held.saturating_add(value.len());
                        if held > MAX_CELL_BYTES {
                            return Err(invalid("a table of more text than can be written again"));
                        }
                        Cell::Bytes(value.to_vec())
                    } else {
                        Cell::Node(hnid)
                    }
                } else {
                    Cell::Fixed(raw.to_vec())
                }));
            }
            out.push(NewRow { id: row.id, cells });
        }
        Ok(out)
    }
}

/// The rows the blocks hold, `width` bytes each, none crossing into the next block.
fn rows_of(blocks: &[Vec<u8>], width: usize) -> io::Result<Vec<Row>> {
    if blocks.iter().all(Vec::is_empty) {
        return Ok(Vec::new());
    }
    if width < 4 {
        return Err(invalid("a table's rows of a width none has"));
    }
    let mut rows = Vec::new();
    for (b, block) in blocks.iter().enumerate() {
        for (i, row) in block.chunks_exact(width).enumerate() {
            if rows.len() >= MAX_ROWS {
                return Err(invalid("a table of more rows than any has"));
            }
            rows.push(Row { id: u32_at(row, 0)?, block: u32::try_from(b).map_err(|_| short())?, at: u32::try_from(i * width).map_err(|_| short())? });
        }
    }
    Ok(rows)
}
