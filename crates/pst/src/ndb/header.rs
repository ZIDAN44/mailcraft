//! The file's header (MS-PST 2.2.2.6), which says what kind of file it is and where it all begins.

use std::io;

use crate::crc::{crc, matches};
use crate::crypt::Crypt;
use crate::util::{invalid, short, u8_at, u16_at, u32_at, u64_at};

/// The file's two formats: Outlook 2003 and later, or Outlook 97–2002.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Unicode,
    Ansi,
}

/// A block or page and where it is (`BREF`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Bref {
    pub(crate) bid: u64,
    pub(crate) ib: u64,
}

/// `ROOT`: where the B-trees and allocation maps stand.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Root {
    pub(crate) eof: u64,
    pub(crate) amap_last: u64,
    pub(crate) amap_free: u64,
    pub(crate) pmap_free: u64,
    pub(crate) nbt: Bref,
    pub(crate) bbt: Bref,
    pub(crate) amap_valid: u8,
}

#[derive(Clone, Debug)]
pub(crate) struct Header {
    pub(crate) format: Format,
    pub(crate) crypt: Crypt,
    pub(crate) root: Root,
    pub(crate) next_page: u64,
    pub(crate) next_block: u64,
    pub(crate) unique: u32,
    /// `rgnid`: the next index for each node type.
    pub(crate) nids: [u32; 32],
    /// The bytes it was read from: what isn't changed stays as it was.
    pub(crate) raw: Vec<u8>,
}

/// Bytes read to take in either format's header.
pub(crate) const HEADER_READ: usize = 564;

const MAGIC: &[u8; 4] = b"!BDN";
const CLIENT: &[u8; 2] = b"SM";
const CRC_START: usize = 8;
const CRC_PARTIAL: usize = 471;
const CRC_FULL: usize = 516;

impl Format {
    /// The format of the file whose first bytes are `head`: ANSI when `wVer` says 14 or 15
    /// (MS-PST 2.2.2.6), else Unicode.
    pub fn of_head(head: &[u8]) -> Format {
        match head.get(10..12) {
            Some(&[14 | 15, 0]) => Format::Ansi,
            _ => Format::Unicode,
        }
    }

    /// Whether IDs and offsets are 64 bits wide.
    pub(crate) fn wide(self) -> bool {
        self == Format::Unicode
    }

    /// Bytes in a page before its trailer.
    pub(crate) fn page_data(self) -> usize {
        if self.wide() { 496 } else { 500 }
    }

    /// An ID's or an offset's size.
    pub(crate) fn id_size(self) -> usize {
        if self.wide() { 8 } else { 4 }
    }

    /// A block's trailer.
    pub(crate) fn block_trailer(self) -> usize {
        if self.wide() { 16 } else { 12 }
    }

    /// The most data a block holds.
    pub(crate) fn block_data(self) -> usize {
        crate::ndb::MAX_BLOCK - self.block_trailer()
    }
}

impl Header {
    /// `(rgnid, ROOT, bCryptMethod)`'s offsets.
    fn offsets(format: Format) -> (usize, usize, usize) {
        if format.wide() { (44, 180, 513) } else { (36, 164, 461) }
    }

    /// `wVer`.
    pub(crate) fn version(&self) -> u16 {
        u16_at(&self.raw, 10).unwrap_or(0)
    }

    pub(crate) fn parse(b: &[u8]) -> io::Result<Header> {
        if b.get(..4) != Some(&MAGIC[..]) || b.get(8..10) != Some(&CLIENT[..]) {
            return Err(invalid("not an Outlook data file"));
        }
        let format = match u16_at(b, 10)? {
            14 | 15 => Format::Ansi,
            // 23 or more (MS-PST 2.2.2.6); from 36, files keep 4 KiB pages, which it doesn't lay out.
            23..=35 => Format::Unicode,
            v => return Err(invalid(format!("data file version {v} isn't one this reads"))),
        };
        if !matches(b.get(CRC_START..CRC_START + CRC_PARTIAL).ok_or_else(short)?, u32_at(b, 4)?) {
            return Err(invalid("the file's header is damaged"));
        }
        if format.wide() && !matches(b.get(CRC_START..CRC_START + CRC_FULL).ok_or_else(short)?, u32_at(b, 524)?) {
            return Err(invalid("the file's header is damaged"));
        }
        let (nids_at, root_at, crypt_at) = Self::offsets(format);
        let crypt = Crypt::from_byte(u8_at(b, crypt_at)?).ok_or_else(|| invalid("an unknown block encoding"))?;
        let mut nids = [0u32; 32];
        for (i, n) in nids.iter_mut().enumerate() {
            *n = u32_at(b, nids_at + i * 4)?;
        }
        let root = if format.wide() { Self::root_wide(b, root_at)? } else { Self::root_narrow(b, root_at)? };
        let (next_page, next_block, unique) = if format.wide() {
            (u64_at(b, 32)?, u64_at(b, 516)?, u32_at(b, 40)?)
        } else {
            (u64::from(u32_at(b, 28)?), u64::from(u32_at(b, 24)?), u32_at(b, 32)?)
        };
        let len = if format.wide() { 564 } else { 512 };
        Ok(Header { format, crypt, root, next_page, next_block, unique, nids, raw: b.get(..len).ok_or_else(short)?.to_vec() })
    }

    fn root_wide(b: &[u8], at: usize) -> io::Result<Root> {
        Ok(Root {
            eof: u64_at(b, at + 4)?,
            amap_last: u64_at(b, at + 12)?,
            amap_free: u64_at(b, at + 20)?,
            pmap_free: u64_at(b, at + 28)?,
            nbt: Bref { bid: u64_at(b, at + 36)?, ib: u64_at(b, at + 44)? },
            bbt: Bref { bid: u64_at(b, at + 52)?, ib: u64_at(b, at + 60)? },
            amap_valid: u8_at(b, at + 68)?,
        })
    }

    fn root_narrow(b: &[u8], at: usize) -> io::Result<Root> {
        let w = |o: usize| u32_at(b, at + o).map(u64::from);
        Ok(Root {
            eof: w(4)?,
            amap_last: w(8)?,
            amap_free: w(12)?,
            pmap_free: w(16)?,
            nbt: Bref { bid: w(20)?, ib: w(24)? },
            bbt: Bref { bid: w(28)?, ib: w(32)? },
            amap_valid: u8_at(b, at + 36)?,
        })
    }

    /// The header as it goes in the file: its fields written over the bytes it was read from and
    /// its CRCs worked out again. An ID past what an ANSI file's fields hold is an error.
    pub(crate) fn to_bytes(&self) -> io::Result<Vec<u8>> {
        fn put(b: &mut [u8], at: usize, bytes: &[u8]) -> io::Result<()> {
            b.get_mut(at..at + bytes.len()).ok_or_else(short)?.copy_from_slice(bytes);
            Ok(())
        }
        let narrow = |n: u64| u32::try_from(n).map_err(|_| invalid("past what an ANSI data file can hold"));
        let mut b = self.raw.clone();
        let (nids_at, root_at, crypt_at) = Self::offsets(self.format);
        for (i, n) in self.nids.iter().enumerate() {
            put(&mut b, nids_at + i * 4, &n.to_le_bytes())?;
        }
        put(&mut b, crypt_at, &[self.crypt.byte()])?;
        let r = &self.root;
        if self.format.wide() {
            put(&mut b, 32, &self.next_page.to_le_bytes())?;
            put(&mut b, 516, &self.next_block.to_le_bytes())?;
            put(&mut b, 40, &self.unique.to_le_bytes())?;
            let fields = [
                (4, r.eof),
                (12, r.amap_last),
                (20, r.amap_free),
                (28, r.pmap_free),
                (36, r.nbt.bid),
                (44, r.nbt.ib),
                (52, r.bbt.bid),
                (60, r.bbt.ib),
            ];
            for (o, v) in fields {
                put(&mut b, root_at + o, &v.to_le_bytes())?;
            }
            put(&mut b, root_at + 68, &[r.amap_valid])?;
        } else {
            put(&mut b, 28, &narrow(self.next_page)?.to_le_bytes())?;
            put(&mut b, 24, &narrow(self.next_block)?.to_le_bytes())?;
            put(&mut b, 32, &self.unique.to_le_bytes())?;
            let fields = [
                (4, r.eof),
                (8, r.amap_last),
                (12, r.amap_free),
                (16, r.pmap_free),
                (20, r.nbt.bid),
                (24, r.nbt.ib),
                (28, r.bbt.bid),
                (32, r.bbt.ib),
            ];
            for (o, v) in fields {
                put(&mut b, root_at + o, &narrow(v)?.to_le_bytes())?;
            }
            put(&mut b, root_at + 36, &[r.amap_valid])?;
        }
        let partial = crc(b.get(CRC_START..CRC_START + CRC_PARTIAL).ok_or_else(short)?);
        put(&mut b, 4, &partial.to_le_bytes())?;
        if self.format.wide() {
            let full = crc(b.get(CRC_START..CRC_START + CRC_FULL).ok_or_else(short)?);
            put(&mut b, 524, &full.to_le_bytes())?;
        }
        Ok(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A header of `format`, valid, with nothing in it but what these tests look at.
    pub(crate) fn blank(format: Format) -> Vec<u8> {
        let mut b = vec![0u8; if format.wide() { 564 } else { 512 }];
        b[..4].copy_from_slice(MAGIC);
        b[8..10].copy_from_slice(CLIENT);
        b[10..12].copy_from_slice(&(if format.wide() { 23u16 } else { 14u16 }).to_le_bytes());
        let header = Header { format, crypt: Crypt::Permute, root: Root::default(), next_page: 7, next_block: 8, unique: 9, nids: [1; 32], raw: b };
        header.to_bytes().unwrap()
    }

    #[test]
    fn a_header_written_reads_back() {
        for format in [Format::Unicode, Format::Ansi] {
            let mut h = Header::parse(&blank(format)).unwrap();
            h.root.eof = 0x1_0000;
            h.root.nbt = Bref { bid: 0x20, ib: 0x4600 };
            h.next_block = 0x1234;
            h.nids[4] = 0x400;
            let again = Header::parse(&h.to_bytes().unwrap()).unwrap();
            assert_eq!((again.format, again.crypt, again.root.eof), (format, Crypt::Permute, 0x1_0000));
            assert_eq!((again.root.nbt, again.next_block, again.nids[4], again.next_page), (Bref { bid: 0x20, ib: 0x4600 }, 0x1234, 0x400, 7));
        }
    }

    #[test]
    fn a_damaged_header_is_refused() {
        for format in [Format::Unicode, Format::Ansi] {
            let mut b = blank(format);
            b[100] ^= 1;
            assert!(Header::parse(&b).is_err());
        }
        assert!(Header::parse(b"not a pst at all").is_err());
        assert!(Header::parse(&[]).is_err());
    }

    #[test]
    fn past_what_an_ansi_file_holds_is_an_error() {
        let mut h = Header::parse(&blank(Format::Ansi)).unwrap();
        h.root.eof = 1 << 33;
        assert!(h.to_bytes().is_err());
    }
}
