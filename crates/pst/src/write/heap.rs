//! Building heaps, and the trees and property sets kept in them (MS-PST 2.3.1, 2.3.2, 2.3.3).

use std::io;

use crate::ltp::{HeapId, PropertyType, PropertyValue};
use crate::ndb::Format;
use crate::util::{invalid, short};

/// The most one allocation may be (MS-PST 2.3.1.1).
pub(crate) const MAX_ALLOC: usize = 3580;
/// Allocations a block may have: a heap ID's index is 11 bits.
const MAX_ALLOCS: usize = 2047;

// `bClientSig`.
pub(crate) const CLIENT_TC: u8 = 0x7C;
pub(crate) const CLIENT_PC: u8 = 0xBC;
const HN_SIG: u8 = 0xEC;
const BTH_SIG: u8 = 0xB5;

/// The nodes a value too big for a heap goes in have this type: `NID_TYPE_INTERNAL`.
pub(crate) const NID_TYPE_INTERNAL: u32 = 0x01;

/// One block being filled.
#[derive(Default)]
struct Page {
    allocs: Vec<Vec<u8>>,
    used: usize,
}

/// A heap being built.
pub(crate) struct HeapBuilder {
    format: Format,
    pages: Vec<Page>,
}

/// The header at the start of block `i`.
fn header_size(i: usize) -> usize {
    match i {
        0 => 12,
        i if i >= 8 && (i - 8) % 128 == 0 => 66,
        _ => 2,
    }
}

/// The page map for `n` allocations: its counts and `n + 1` offsets.
fn map_size(n: usize) -> usize {
    4 + (n + 1) * 2
}

/// `rgbFillLevel` for a block with this much free (MS-PST 2.3.1.2).
fn fill_level(free: usize) -> u8 {
    match free {
        3584.. => 0,
        2560.. => 1,
        2048.. => 2,
        1792.. => 3,
        1536.. => 4,
        1280.. => 5,
        1024.. => 6,
        768.. => 7,
        512.. => 8,
        256.. => 9,
        128.. => 10,
        64.. => 11,
        32.. => 12,
        16.. => 13,
        8.. => 14,
        _ => 15,
    }
}

impl HeapBuilder {
    pub(crate) fn new(format: Format) -> HeapBuilder {
        HeapBuilder { format, pages: vec![Page::default()] }
    }

    fn room(&self, i: usize) -> usize {
        self.format.block_data() - header_size(i)
    }

    /// The heap ID of a new allocation of `data`.
    pub(crate) fn alloc(&mut self, data: Vec<u8>) -> io::Result<HeapId> {
        if data.len() > MAX_ALLOC {
            return Err(invalid("a heap allocation bigger than one can be"));
        }
        let mut i = self.pages.len() - 1;
        let fits = |p: &Page, room: usize| {
            p.allocs.len() < MAX_ALLOCS && (p.used + data.len() + 1) & !1 <= room.saturating_sub(map_size(p.allocs.len() + 1))
        };
        let room = self.room(i);
        if !self.pages.last().is_some_and(|p| fits(p, room)) {
            self.pages.push(Page::default());
            i += 1;
        }
        let block = u32::try_from(i).map_err(|_| invalid("a heap of more blocks than one can have"))?;
        if block > 0xFFFF {
            return Err(invalid("a heap of more blocks than one can have"));
        }
        let page = self.pages.last_mut().ok_or_else(short)?;
        page.used += data.len();
        page.allocs.push(data);
        let index = u32::try_from(page.allocs.len()).map_err(|_| short())?;
        Ok((block << 16) | (index << 5))
    }

    /// The heap's blocks, its root allocation `root`, every block but the last full (MS-PST
    /// 2.2.2.8.3.1: a data tree's blocks are), the rest taken up by padding allocations.
    pub(crate) fn finish(mut self, client: u8, root: HeapId) -> io::Result<Vec<Vec<u8>>> {
        let count = self.pages.len();
        let size = self.format.block_data();
        let mut free_levels = Vec::with_capacity(count);
        for i in 0..count {
            let room = self.room(i);
            let page = self.pages.get_mut(i).ok_or_else(short)?;
            if i + 1 < count {
                loop {
                    let n = page.allocs.len();
                    let left = room.saturating_sub(page.used).saturating_sub(map_size(n + 1)) & !1;
                    if left == 0 || n >= MAX_ALLOCS {
                        break;
                    }
                    let pad = left.min(MAX_ALLOC);
                    page.used += pad;
                    page.allocs.push(vec![0u8; pad]);
                }
            }
            let free = room.saturating_sub(page.used).saturating_sub(map_size(page.allocs.len()));
            free_levels.push(fill_level(free));
        }
        let mut out = Vec::with_capacity(count);
        for (i, page) in self.pages.iter().enumerate() {
            let mut b = Vec::with_capacity(size);
            b.resize(header_size(i), 0);
            let mut offsets = Vec::with_capacity(page.allocs.len() + 1);
            for a in &page.allocs {
                offsets.push(u16::try_from(b.len()).map_err(|_| short())?);
                b.extend_from_slice(a);
            }
            offsets.push(u16::try_from(b.len()).map_err(|_| short())?);
            if b.len() % 2 != 0 {
                b.push(0);
            }
            let map = u16::try_from(b.len()).map_err(|_| short())?;
            b.extend(u16::try_from(page.allocs.len()).map_err(|_| short())?.to_le_bytes());
            b.extend(0u16.to_le_bytes());
            for o in offsets {
                b.extend(o.to_le_bytes());
            }
            if i + 1 < count {
                if b.len() > size {
                    return Err(invalid("a heap block bigger than a block can be"));
                }
                b.resize(size, 0);
            }
            b.get_mut(..2).ok_or_else(short)?.copy_from_slice(&map.to_le_bytes());
            if i == 0 {
                b.get_mut(2..4).ok_or_else(short)?.copy_from_slice(&[HN_SIG, client]);
                b.get_mut(4..8).ok_or_else(short)?.copy_from_slice(&root.to_le_bytes());
                // Four bits a block, for the first eight.
                write_levels(b.get_mut(8..12).ok_or_else(short)?, free_levels.get(..count.min(8)).unwrap_or_default());
            } else if header_size(i) == 66 {
                let group = free_levels.get(i..(i + 128).min(count)).unwrap_or_default();
                write_levels(b.get_mut(2..66).ok_or_else(short)?, group);
            }
            out.push(b);
        }
        Ok(out)
    }
}

fn write_levels(out: &mut [u8], levels: &[u8]) {
    for (byte, pair) in out.iter_mut().zip(levels.chunks(2)) {
        *byte = pair.first().copied().unwrap_or(0) | (pair.get(1).copied().unwrap_or(0) << 4);
    }
}

/// A tree in a heap (`BTH`) over sorted records: its header's heap ID.
pub(crate) fn build_tree(heap: &mut HeapBuilder, key: usize, data: usize, records: &[Vec<u8>]) -> io::Result<HeapId> {
    let width = key + data;
    let room = (MAX_ALLOC / width.max(1)).max(1);
    let mut children: Vec<(Vec<u8>, HeapId)> = Vec::with_capacity(records.len() / room + 1);
    for chunk in records.chunks(room) {
        let first = chunk.first().and_then(|r| r.get(..key)).ok_or_else(short)?.to_vec();
        children.push((first, heap.alloc(chunk.concat())?));
    }
    let mut levels = 0u8;
    let index = key + 4;
    while children.len() > 1 {
        let room = (MAX_ALLOC / index).max(1);
        let mut up = Vec::with_capacity(children.len() / room + 1);
        for chunk in children.chunks(room) {
            let first = chunk.first().ok_or_else(short)?.0.clone();
            let mut b = Vec::with_capacity(chunk.len() * index);
            for (k, hid) in chunk {
                b.extend_from_slice(k);
                b.extend(hid.to_le_bytes());
            }
            up.push((first, heap.alloc(b)?));
        }
        children = up;
        levels += 1;
    }
    let root = children.first().map_or(0, |c| c.1);
    let mut head = vec![BTH_SIG, u8::try_from(key).map_err(|_| short())?, u8::try_from(data).map_err(|_| short())?, levels];
    head.extend(root.to_le_bytes());
    heap.alloc(head)
}

/// A value's bytes as a heap allocation or subnode keeps them; a string without a NUL, as Outlook
/// keeps them.
pub(crate) fn value_bytes(value: &PropertyValue) -> Vec<u8> {
    use PropertyValue as V;
    fn with_offsets(items: Vec<Vec<u8>>) -> Vec<u8> {
        let mut at = 4 + 4 * items.len();
        let mut out = Vec::new();
        out.extend(u32::try_from(items.len()).unwrap_or(0).to_le_bytes());
        for i in &items {
            out.extend(u32::try_from(at).unwrap_or(0).to_le_bytes());
            at += i.len();
        }
        for i in items {
            out.extend(i);
        }
        out
    }
    let units = |u: &[u16]| u.iter().flat_map(|c| c.to_le_bytes()).collect::<Vec<u8>>();
    match value {
        V::Null => Vec::new(),
        V::Other(_, b) => b.buffer().to_vec(),
        V::Integer16(n) => n.to_le_bytes().to_vec(),
        V::Integer32(n) | V::ErrorCode(n) => n.to_le_bytes().to_vec(),
        V::Floating32(n) => n.to_bits().to_le_bytes().to_vec(),
        V::Floating64(n) | V::FloatingTime(n) => n.to_bits().to_le_bytes().to_vec(),
        V::Currency(n) | V::Integer64(n) | V::Time(n) => n.to_le_bytes().to_vec(),
        V::Boolean(b) => vec![u8::from(*b)],
        V::String8(s) => s.buffer().strip_suffix(&[0]).unwrap_or(s.buffer()).to_vec(),
        V::Unicode(s) => units(s.buffer().strip_suffix(&[0]).unwrap_or(s.buffer())),
        V::Guid(g) => g.to_bytes().to_vec(),
        V::Binary(b) => b.buffer().to_vec(),
        V::Object(o) => [o.node.to_le_bytes(), o.size.to_le_bytes()].concat(),
        V::MultipleInteger16(l) => l.iter().flat_map(|x| x.to_le_bytes()).collect(),
        V::MultipleInteger32(l) => l.iter().flat_map(|x| x.to_le_bytes()).collect(),
        V::MultipleFloating32(l) => l.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect(),
        V::MultipleFloating64(l) | V::MultipleFloatingTime(l) => l.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect(),
        V::MultipleCurrency(l) | V::MultipleInteger64(l) | V::MultipleTime(l) => l.iter().flat_map(|x| x.to_le_bytes()).collect(),
        V::MultipleGuid(l) => l.iter().flat_map(|g| g.to_bytes()).collect(),
        V::MultipleString8(l) => with_offsets(l.iter().map(|s| s.buffer().to_vec()).collect()),
        V::MultipleUnicode(l) => with_offsets(l.iter().map(|s| units(s.buffer())).collect()),
        V::MultipleBinary(l) => with_offsets(l.iter().map(|b| b.buffer().to_vec()).collect()),
    }
}

/// A value's type.
pub(crate) fn value_type(value: &PropertyValue) -> u16 {
    use PropertyValue as V;
    const M: u16 = PropertyType::MULTI;
    match value {
        V::Null => PropertyType::NULL,
        V::Other(k, _) => *k,
        V::Integer16(_) => PropertyType::INT16,
        V::Integer32(_) => PropertyType::INT32,
        V::Floating32(_) => PropertyType::FLOAT32,
        V::Floating64(_) => PropertyType::FLOAT64,
        V::Currency(_) => PropertyType::CURRENCY,
        V::FloatingTime(_) => PropertyType::APP_TIME,
        V::ErrorCode(_) => PropertyType::ERROR,
        V::Boolean(_) => PropertyType::BOOLEAN,
        V::Integer64(_) => PropertyType::INT64,
        V::String8(_) => PropertyType::STRING8,
        V::Unicode(_) => PropertyType::UNICODE,
        V::Time(_) => PropertyType::TIME,
        V::Guid(_) => PropertyType::GUID,
        V::Binary(_) => PropertyType::BINARY,
        V::Object(_) => PropertyType::OBJECT,
        V::MultipleInteger16(_) => M | PropertyType::INT16,
        V::MultipleInteger32(_) => M | PropertyType::INT32,
        V::MultipleFloating32(_) => M | PropertyType::FLOAT32,
        V::MultipleFloating64(_) => M | PropertyType::FLOAT64,
        V::MultipleCurrency(_) => M | PropertyType::CURRENCY,
        V::MultipleFloatingTime(_) => M | PropertyType::APP_TIME,
        V::MultipleInteger64(_) => M | PropertyType::INT64,
        V::MultipleString8(_) => M | PropertyType::STRING8,
        V::MultipleUnicode(_) => M | PropertyType::UNICODE,
        V::MultipleTime(_) => M | PropertyType::TIME,
        V::MultipleGuid(_) => M | PropertyType::GUID,
        V::MultipleBinary(_) => M | PropertyType::BINARY,
    }
}

/// A property set built: its heap's blocks and the values kept in subnodes of their own.
pub(crate) struct BuiltProps {
    pub(crate) blocks: Vec<Vec<u8>>,
    pub(crate) external: Vec<(u32, Vec<u8>)>,
}

/// `props` as a property set. Values too big for a heap get subnodes from index `next` on.
pub(crate) fn build_props(format: Format, props: Vec<(u16, PropertyValue)>, next: u32) -> io::Result<BuiltProps> {
    build_props_linked(format, props, next, None)
}

/// [`build_props`], one property (`(id, type, node)`) held in a subnode already written.
pub(crate) fn build_props_linked(
    format: Format,
    mut props: Vec<(u16, PropertyValue)>,
    mut next: u32,
    linked: Option<(u16, u16, u32)>,
) -> io::Result<BuiltProps> {
    props.sort_by_key(|(id, _)| *id);
    props.dedup_by_key(|(id, _)| *id);
    let mut heap = HeapBuilder::new(format);
    let mut external = Vec::new();
    let mut records = Vec::with_capacity(props.len());
    for (id, value) in &props {
        if matches!(value, PropertyValue::Null) {
            continue;
        }
        let kind = value_type(value);
        if let Some((link_id, link_kind, node)) = linked
            && link_id == *id
        {
            let mut r = Vec::with_capacity(8);
            r.extend(id.to_le_bytes());
            r.extend(link_kind.to_le_bytes());
            r.extend(node.to_le_bytes());
            records.push(r);
            continue;
        }
        let held: u32 = if PropertyType(kind).fits_record() {
            let b = value_bytes(value);
            let mut four = [0u8; 4];
            four.get_mut(..b.len()).ok_or_else(short)?.copy_from_slice(&b);
            u32::from_le_bytes(four)
        } else {
            let data = value_bytes(value);
            if data.is_empty() {
                0
            } else if data.len() > MAX_ALLOC {
                let nid = next.checked_mul(32).ok_or_else(short)? | NID_TYPE_INTERNAL;
                next += 1;
                external.push((nid, data));
                nid
            } else {
                heap.alloc(data)?
            }
        };
        let mut r = Vec::with_capacity(8);
        r.extend(id.to_le_bytes());
        r.extend(kind.to_le_bytes());
        r.extend(held.to_le_bytes());
        records.push(r);
    }
    let root = build_tree(&mut heap, 2, 6, &records)?;
    Ok(BuiltProps { blocks: heap.finish(CLIENT_PC, root)?, external })
}
