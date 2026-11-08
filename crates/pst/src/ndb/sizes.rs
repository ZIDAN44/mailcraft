//! The sizes and page types MS-PST gives.

/// A page is 512 bytes (MS-PST 2.2.2.7).
pub(crate) const PAGE: usize = 512;
/// A block and its trailer take at most 8 KiB (MS-PST 2.2.2.8).
pub(crate) const MAX_BLOCK: usize = 8192;

// Page types (`ptype`).
pub(crate) const PT_BBT: u8 = 0x80;
pub(crate) const PT_NBT: u8 = 0x81;
