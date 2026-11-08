//! The file's lowest layer (MS-PST 2.2): its header, the two B-trees and the blocks they name.

mod block;
pub(crate) mod btree;
mod file;
mod header;
mod sizes;

pub(crate) use block::SubNodes;
pub(crate) use btree::{BlockEntry, NodeEntry};
pub use file::{PstFile, ReadSeek};
pub use header::Format;
pub(crate) use header::{Bref, Header, Root};
pub(crate) use sizes::*;

/// A node's type: the low five bits of its ID (MS-PST 2.2.2.1).
pub(crate) const NID_TYPE_MASK: u32 = 0x1F;
pub(crate) const NID_TYPE_ATTACHMENT: u32 = 0x05;
pub(crate) const NID_TYPE_HIERARCHY_TABLE: u32 = 0x0D;
pub(crate) const NID_TYPE_CONTENTS_TABLE: u32 = 0x0E;
pub(crate) const NID_TYPE_ASSOC_CONTENTS_TABLE: u32 = 0x0F;

/// A block whose ID has this bit lists other blocks, and isn't encoded (MS-PST 2.2.2.2).
pub(crate) const BID_INTERNAL: u64 = 0x2;
