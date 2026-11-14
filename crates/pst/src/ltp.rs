//! The middle layer (MS-PST 2.3): a node's bytes as a heap, a heap as a tree of records, and
//! those as a set of properties or a table.

mod heap;
mod props;
mod table;
mod tree;
mod value;

pub(crate) use heap::{Heap, HeapId};
pub use props::Properties;
pub(crate) use props::{read as read_properties, read_some};
pub use table::{Column, Row, Table};
pub(crate) use tree::records;
pub use value::{Bytes, GuidValue, ObjectRef, PropertyType, PropertyValue, Utf16};

/// A heap's signature (`bSig`, MS-PST 2.3.1.2).
pub(crate) const HN_SIG: u8 = 0xEC;
// What a heap holds (`bClientSig`): a tree of records, a table, a set of properties.
pub(crate) const BTH_SIG: u8 = 0xB5;
pub(crate) const TC_SIG: u8 = 0x7C;
pub(crate) const PC_SIG: u8 = 0xBC;
