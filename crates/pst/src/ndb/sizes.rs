//! The sizes, page types and block types MS-PST gives.

/// A page is 512 bytes (MS-PST 2.2.2.7).
pub(crate) const PAGE: usize = 512;
/// A block and its trailer take at most 8 KiB (MS-PST 2.2.2.8).
pub(crate) const MAX_BLOCK: usize = 8192;
/// A map page's bits, one per 64 bytes in an allocation map (MS-PST 2.2.2.7.2).
pub(crate) const MAP_BYTES: usize = 496;

// Page types (`ptype`).
pub(crate) const PT_BBT: u8 = 0x80;
pub(crate) const PT_NBT: u8 = 0x81;
pub(crate) const PT_FMAP: u8 = 0x82;
pub(crate) const PT_PMAP: u8 = 0x83;
pub(crate) const PT_AMAP: u8 = 0x84;
pub(crate) const PT_FPMAP: u8 = 0x85;
pub(crate) const PT_DLIST: u8 = 0x86;

// Block types (`btype`, MS-PST 2.2.2.8.3).
pub(crate) const BT_DATA: u8 = 0x01;
pub(crate) const BT_SUBNODE: u8 = 0x02;
