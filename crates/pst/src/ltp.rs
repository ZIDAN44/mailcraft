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
