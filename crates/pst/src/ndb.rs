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
pub(crate) use header::{Bref, CLIENT, Header, MAGIC, Root};
pub(crate) use sizes::*;

/// A node's type: the low five bits of its ID (MS-PST 2.2.2.1).
pub(crate) const NID_TYPE_MASK: u32 = 0x1F;
/// What the writer keeps in subnodes of its own: values too big for a heap, attachments' bytes.
pub(crate) const NID_TYPE_INTERNAL: u32 = 0x01;
pub(crate) const NID_TYPE_NORMAL_FOLDER: u32 = 0x02;
pub(crate) const NID_TYPE_SEARCH_FOLDER: u32 = 0x03;
pub(crate) const NID_TYPE_NORMAL_MESSAGE: u32 = 0x04;
pub(crate) const NID_TYPE_ATTACHMENT: u32 = 0x05;
pub(crate) const NID_TYPE_ASSOC_MESSAGE: u32 = 0x08;
pub(crate) const NID_TYPE_HIERARCHY_TABLE: u32 = 0x0D;
pub(crate) const NID_TYPE_CONTENTS_TABLE: u32 = 0x0E;
pub(crate) const NID_TYPE_ASSOC_CONTENTS_TABLE: u32 = 0x0F;
pub(crate) const NID_TYPE_SEARCH_CONTENTS_TABLE: u32 = 0x10;
pub(crate) const NID_TYPE_ATTACHMENT_TABLE: u32 = 0x11;
pub(crate) const NID_TYPE_RECIPIENT_TABLE: u32 = 0x12;

/// A block whose ID has this bit lists other blocks, and isn't encoded (MS-PST 2.2.2.2).
pub(crate) const BID_INTERNAL: u64 = 0x2;
