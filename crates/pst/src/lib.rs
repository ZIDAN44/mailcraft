//! Outlook data files (`.pst`), as Microsoft's open specification [MS-PST] lays them out: read
//! them, add to them and write new ones, in the Unicode format and the older ANSI one.
//!
//! Three layers, as the specification has them: [`ndb`] (pages, blocks, the node and block
//! B-trees), [`ltp`] (heaps, property sets, tables) and [`messaging`] (folders, messages,
//! attachments). One set of types serves both formats: IDs and offsets are held 64 bits wide and
//! only the code that reads and writes them knows an ANSI file's are 32.
//!
//! Everything read is hostile until checked: nothing here panics on any file, every size is
//! capped, every tree walk goes down level by level and ends, every CRC is checked.
//!
//! [MS-PST]: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/

// The writer isn't built for the browser, so what only it uses goes unused there.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod crc;
mod crypt;
mod ltp;
mod messaging;
mod ndb;
mod util;
#[cfg(not(target_arch = "wasm32"))]
pub mod write;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use write::table_types as write_table;

pub use crc::take_checksums_as_right;
pub use crypt::Crypt;
pub use ltp::{Bytes, Column, GuidValue, ObjectRef, Properties, PropertyType, PropertyValue, Row, Table, Utf16};
pub use messaging::{
    Attachment, AttachmentData, MAX_EMBEDDED_DEPTH, Message, NamedEntry, NamedMap, NamedPropertyGuid, NamedPropertyId, PS_MAPI, PS_PUBLIC_STRINGS,
    PstFolder, PstStore,
};
pub use ndb::{Format, PstFile, ReadSeek};

/// A node's ID (MS-PST 2.2.2.1).
pub type NodeId = u32;
