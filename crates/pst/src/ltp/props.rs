//! A node as a set of properties (a Property Context, MS-PST 2.3.3).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io;

use super::value::{self, Bytes, PropertyType, PropertyValue};
use super::{Heap, records};
use crate::ndb::{NID_TYPE_MASK, PstFile, SubNodes};
use crate::util::{invalid, u16_at, u32_at};

const PC_SIG: u8 = 0xBC;
/// A record's key (the property's ID) and data (its type, and its value or where it is kept).
const RECORD_KEY: usize = 2;
const RECORD_DATA: usize = 6;

/// An object's properties, by ID.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Properties(pub(crate) BTreeMap<u16, PropertyValue>);

impl Properties {
    pub fn get(&self, id: u16) -> Option<&PropertyValue> {
        self.0.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u16, &PropertyValue)> {
        self.0.iter()
    }

    /// `PidTagDisplayName`, without its NUL.
    pub fn display_name(&self) -> io::Result<String> {
        match self.get(0x3001) {
            Some(PropertyValue::Unicode(s)) => Ok(String::from_utf16_lossy(s.buffer()).trim_end_matches('\0').to_string()),
            Some(PropertyValue::String8(s)) => Ok(s.buffer().iter().map(|b| char::from(*b)).collect::<String>().trim_end_matches('\0').to_string()),
            _ => Err(invalid("no display name")),
        }
    }
}

/// What the values one object keeps in subnodes may come to, in values as big as the file's limit
/// for one: the body and its HTML both at that limit fit, and a file that names one huge value a
/// thousand times from a few kilobytes doesn't.
const VALUES_PER_OBJECT: u64 = 2;

/// The bytes one object may still read from subnodes.
pub(crate) fn budget(pst: &PstFile) -> Cell<u64> {
    Cell::new(pst.max_value().saturating_mul(VALUES_PER_OBJECT))
}

/// The value kept at `hnid`, of type `kind`: in the heap, or a subnode of its own, whose bytes
/// come out of `budget`.
pub(crate) fn stored(pst: &PstFile, heap: &Heap, subs: &SubNodes, kind: u16, hnid: u32, budget: &Cell<u64>) -> io::Result<PropertyValue> {
    if hnid == 0 {
        // Nothing kept: a zero-byte value, as Outlook keeps an empty file (MS-PST 2.3.3.3).
        return match kind {
            PropertyType::STRING8 | PropertyType::UNICODE | PropertyType::BINARY => value::parse(kind, &[]),
            PropertyType::NULL => Ok(PropertyValue::Null),
            // Empty, of a type with more to it: kept as it is, for writing back.
            _ => Ok(PropertyValue::Other(kind, Bytes::default())),
        };
    }
    if hnid & NID_TYPE_MASK == 0 {
        return value::parse(kind, heap.alloc(hnid)?);
    }
    let (data, _) = subs.get(&hnid).ok_or_else(|| invalid(format!("no subnode {hnid:#x}")))?;
    let bytes = pst.read_value_within(*data, budget.get())?;
    budget.set(budget.get().saturating_sub(bytes.len() as u64));
    value::parse(kind, &bytes)
}

/// The properties of the node with heap `heap` and subnodes `subs`. A record of a type no
/// property has is passed over with a warning: the others are still good.
pub(crate) fn read(pst: &PstFile, heap: &Heap, subs: &SubNodes) -> io::Result<Properties> {
    read_some(pst, heap, subs, &|_| true)
}

/// Those of the properties [`read`] finds that `wanted` asks for: a value it doesn't ask for
/// isn't read, so a damaged one costs nothing.
pub(crate) fn read_some(pst: &PstFile, heap: &Heap, subs: &SubNodes, wanted: &dyn Fn(u16) -> bool) -> io::Result<Properties> {
    if heap.client != PC_SIG {
        return Err(invalid("a node that isn't a set of properties"));
    }
    let (_, recs) = records(heap, heap.root, RECORD_KEY, RECORD_DATA)?;
    let budget = budget(pst);
    let mut out = BTreeMap::new();
    for (key, data) in recs {
        let id = u16_at(key, 0)?;
        if !wanted(id) {
            continue;
        }
        let kind = u16_at(data, 0)?;
        if !PropertyType(kind).known() {
            log::warn!("a property {id:#06x} of type {kind:#06x}, which none has, was passed over");
            continue;
        }
        let value = if PropertyType(kind).fits_record() {
            value::parse(kind, data.get(2..).unwrap_or_default())?
        } else {
            stored(pst, heap, subs, kind, u32_at(data, 2)?, &budget)?
        };
        out.insert(id, value);
    }
    Ok(Properties(out))
}
